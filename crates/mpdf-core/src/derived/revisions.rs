use super::{DerivedDocument, DERIVED_SCHEMA_VERSION};
use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fs;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;
pub const MAX_REVISION_RECORDS: usize = 100_000;
pub const MAX_REVISION_TEXT_BYTES: usize = 1_048_576;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionRecord {
    pub revision_id: String,
    pub target_ref: String,
    pub kind: RevisionKind,
    pub text: String,
    pub base_evidence_digest: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    Human,
    AiSuggested,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RevisionStore {
    pub schema: String,
    pub schema_version: String,
    pub revisions: Vec<RevisionRecord>,
}
impl RevisionStore {
    pub fn empty() -> Self {
        Self {
            schema: "mpdf-revisions".into(),
            schema_version: DERIVED_SCHEMA_VERSION.into(),
            revisions: vec![],
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        if self.schema != "mpdf-revisions"
            || self.schema_version != DERIVED_SCHEMA_VERSION
            || self.revisions.len() > MAX_REVISION_RECORDS
        {
            return Err(CoreError::InvalidDocument("invalid revision store".into()));
        }
        for r in &self.revisions {
            if r.revision_id.is_empty()
                || r.revision_id.len() > 256
                || !ids.insert(&r.revision_id)
                || r.target_ref.is_empty()
                || r.target_ref.len() > 512
                || r.base_evidence_digest.len() != 64
                || !r
                    .base_evidence_digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || r.text.len() > MAX_REVISION_TEXT_BYTES
            {
                return Err(CoreError::InvalidDocument("invalid revision record".into()));
            }
        }
        Ok(())
    }
}
impl DerivedDocument {
    pub fn apply_revisions(&mut self, store: &RevisionStore) -> Result<()> {
        store.validate()?;
        for r in &store.revisions {
            let mut found = None;
            for p in &mut self.pages {
                for b in &mut p.blocks {
                    for l in &mut b.lines {
                        for w in &mut l.words {
                            if w.id == r.target_ref {
                                if p.evidence_digest != r.base_evidence_digest {
                                    return Err(CoreError::InvalidDocument(
                                        "stale revision base evidence".into(),
                                    ));
                                }
                                found = Some(w);
                            }
                        }
                    }
                }
            }
            let Some(w) = found else {
                return Err(CoreError::InvalidDocument(
                    "revision target does not exist".into(),
                ));
            };
            if r.kind == RevisionKind::Human {
                w.effective_text = r.text.clone();
                w.effective_normalized_text = r.text.nfc().collect();
            }
        }
        self.rebuild_chunks();
        self.manifest.revision_digest = digest(
            &serde_json::to_vec(store).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
        );
        Ok(())
    }

    /// Rebuild the derived text chunks after an overlay changes effective text.
    /// Chunk IDs include their structural path and resulting text, so a changed
    /// human revision cannot leave a stale chunk behind in an export.
    fn rebuild_chunks(&mut self) {
        self.chunks.clear();
        for page in &self.pages {
            for block in &page.blocks {
                for line in &block.lines {
                    let text = line
                        .words
                        .iter()
                        .map(|word| word.effective_normalized_text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.is_empty() {
                        continue;
                    }
                    self.chunks.push(super::DerivedChunk {
                        id: super::stable("chunk", &[&page.page_id, &line.id, &text]),
                        document_id: self.manifest.document_id.clone(),
                        page_id: page.page_id.clone(),
                        page_index: page.page_index,
                        bbox: line.bbox,
                        coordinate_space: line.coordinate_space.clone(),
                        structural_path: format!("{}.chunk", line.structural_path),
                        constituent_word_refs: line.words.iter().map(|w| w.id.clone()).collect(),
                        text,
                        reading_order: line.reading_order,
                    });
                }
            }
        }
        self.chunks.sort_by(|a, b| {
            (a.page_index, &a.structural_path).cmp(&(b.page_index, &b.structural_path))
        });
    }
}
pub fn deterministic_revision_id(
    target: &str,
    base: &str,
    kind: RevisionKind,
    text: &str,
) -> String {
    let k = match kind {
        RevisionKind::Human => "human",
        RevisionKind::AiSuggested => "ai_suggested",
    };
    format!(
        "revision-{}",
        &digest(format!("{target}\0{base}\0{k}\0{text}").as_bytes())[..24]
    )
}
fn digest(v: &[u8]) -> String {
    sha2::Sha256::digest(v)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn load_revisions(root: &Path) -> Result<RevisionStore> {
    let dir = root.join("derived");
    if let Ok(m) = fs::symlink_metadata(&dir) {
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err(CoreError::InvalidDocument(
                "derived directory must be real".into(),
            ));
        }
    }
    let p = dir.join("revisions.json");
    let m = match fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RevisionStore::empty()),
        Err(e) => return Err(CoreError::io(&p, e)),
    };
    if m.file_type().is_symlink() || !m.is_file() || m.len() > MAX_REVISION_TEXT_BYTES as u64 * 2 {
        return Err(CoreError::InvalidDocument("invalid revision file".into()));
    };
    let s: RevisionStore = serde_json::from_slice(&fs::read(&p).map_err(|e| CoreError::io(&p, e))?)
        .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
    s.validate()?;
    Ok(s)
}
pub fn save_revisions(root: &Path, s: &RevisionStore) -> Result<()> {
    s.validate()?;
    let d = root.join("derived");
    if let Ok(m) = fs::symlink_metadata(&d) {
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err(CoreError::InvalidDocument(
                "derived directory must be real".into(),
            ));
        }
    } else {
        fs::create_dir_all(&d).map_err(|e| CoreError::io(&d, e))?
    };
    let p = d.join("revisions.json");
    if let Ok(m) = fs::symlink_metadata(&p) {
        if m.file_type().is_symlink() || !m.is_file() {
            return Err(CoreError::InvalidDocument(
                "invalid revision destination".into(),
            ));
        }
    };
    let t = p.with_extension("json.tmp");
    fs::write(
        &t,
        format!(
            "{}\n",
            serde_json::to_string_pretty(s)
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))?
        ),
    )
    .map_err(|e| CoreError::io(&t, e))?;
    fs::rename(&t, &p).map_err(|e| CoreError::io(&p, e))
}
