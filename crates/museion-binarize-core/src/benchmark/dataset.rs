//! Versioned benchmark dataset manifest: `museion-binarize-benchmark-dataset`.
//!
//! A dataset manifest is untrusted input — it may name a path a user
//! points the CLI at. Every file path in it is resolved relative to the
//! manifest's own directory (the "dataset root") and is required, after
//! canonicalization, to remain **inside** that root; this rejects both
//! `../../secret`-style traversal and a symlink that points outside the
//! root. See `docs/benchmark-datasets.md` for the full manifest format.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::benchmark::limits::{MAX_DATASET_PAGES, MAX_ROIS_PER_PAGE};
use crate::benchmark::mask_io::GroundTruthPolarity;
use crate::benchmark::roi::Roi;
use crate::error::{CoreError, Result};

pub const DATASET_SCHEMA: &str = "museion-binarize-benchmark-dataset";
pub const DATASET_SCHEMA_VERSION: &str = "1.0";

/// The raw, deserialized dataset manifest (before path resolution).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub schema: String,
    pub schema_version: String,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub license: String,
    pub provenance: String,
    pub ground_truth_method: String,
    #[serde(default)]
    pub ground_truth_polarity: GroundTruthPolarity,
    #[serde(default)]
    pub ground_truth_creator: Option<String>,
    #[serde(default)]
    pub ground_truth_review_status: Option<String>,
    #[serde(default)]
    pub ground_truth_review_method: Option<String>,
    pub pages: Vec<PageManifest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageManifest {
    pub id: String,
    pub input: String,
    pub ground_truth: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub rights_holder: Option<String>,
    #[serde(default)]
    pub expected_width: Option<u32>,
    #[serde(default)]
    pub expected_height: Option<u32>,
    #[serde(default)]
    pub rois: Vec<Roi>,
}

/// A dataset manifest that has been parsed, schema-checked, and had every
/// referenced file path resolved and containment-checked against its
/// dataset root. This is what the benchmark runner actually consumes —
/// never the raw [`DatasetManifest`] with its relative, unchecked paths.
#[derive(Debug, Clone)]
pub struct LoadedDataset {
    pub manifest: DatasetManifest,
    pub root: PathBuf,
    /// Raw manifest bytes, for digest computation — kept alongside rather
    /// than only in [`Self::manifest_digest`] so a caller can also embed
    /// or re-verify the exact bytes if needed.
    manifest_bytes: Vec<u8>,
    /// Resolved, containment-checked `(input_path, ground_truth_path)`
    /// per page, in manifest order, parallel to `manifest.pages`.
    pub resolved_paths: Vec<(PathBuf, PathBuf)>,
}

impl LoadedDataset {
    /// Loads and validates a dataset manifest from `manifest_path`. The
    /// manifest's parent directory becomes the dataset root that every
    /// page's `input`/`ground_truth` path must stay inside.
    pub fn load(manifest_path: &Path) -> Result<Self> {
        let manifest_bytes = std::fs::read(manifest_path)
            .map_err(|e| CoreError::io(manifest_path.to_path_buf(), e))?;
        let manifest_text = String::from_utf8_lossy(&manifest_bytes);
        let manifest: DatasetManifest = toml::from_str(&manifest_text).map_err(|e| {
            CoreError::InvalidBenchmarkManifest(format!(
                "could not parse {}: {e}",
                manifest_path.display()
            ))
        })?;

        if manifest.schema != DATASET_SCHEMA {
            return Err(CoreError::UnsupportedBenchmarkSchema(format!(
                "expected schema '{DATASET_SCHEMA}', got '{}'",
                manifest.schema
            )));
        }
        if manifest.schema_version != DATASET_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedBenchmarkSchema(format!(
                "expected dataset schema version '{DATASET_SCHEMA_VERSION}', got '{}'",
                manifest.schema_version
            )));
        }
        if manifest.pages.is_empty() {
            return Err(CoreError::InvalidBenchmarkManifest(
                "dataset has no pages".to_string(),
            ));
        }
        if manifest.pages.len() > MAX_DATASET_PAGES {
            return Err(CoreError::ResourceLimitExceeded(format!(
                "dataset declares {} pages, exceeding the maximum of {MAX_DATASET_PAGES}",
                manifest.pages.len()
            )));
        }

        let mut seen_ids = BTreeSet::new();
        for page in &manifest.pages {
            if !seen_ids.insert(page.id.as_str()) {
                return Err(CoreError::InvalidBenchmarkManifest(format!(
                    "duplicate page id '{}'",
                    page.id
                )));
            }
            if page.rois.len() > MAX_ROIS_PER_PAGE {
                return Err(CoreError::ResourceLimitExceeded(format!(
                    "page '{}' declares {} ROIs, exceeding the maximum of {MAX_ROIS_PER_PAGE}",
                    page.id,
                    page.rois.len()
                )));
            }
            let mut roi_ids = BTreeSet::new();
            for roi in &page.rois {
                if !roi_ids.insert(roi.id.as_str()) {
                    return Err(CoreError::InvalidBenchmarkManifest(format!(
                        "page '{}' has a duplicate ROI id '{}'",
                        page.id, roi.id
                    )));
                }
            }
        }

        let manifest_dir = manifest_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let root = std::fs::canonicalize(manifest_dir)
            .map_err(|e| CoreError::io(manifest_dir.to_path_buf(), e))?;

        let mut resolved_paths = Vec::with_capacity(manifest.pages.len());
        for page in &manifest.pages {
            let input = resolve_contained(&root, &page.input)?;
            let ground_truth = resolve_contained(&root, &page.ground_truth)?;
            resolved_paths.push((input, ground_truth));
        }

        Ok(Self {
            manifest,
            root,
            manifest_bytes,
            resolved_paths,
        })
    }

    /// SHA-256 hex digest of the raw manifest file bytes.
    pub fn manifest_digest(&self) -> String {
        sha256_hex(&self.manifest_bytes)
    }

    /// SHA-256 hex digests of every referenced input/ground-truth file,
    /// in manifest page order, as `(page_id, input_digest, ground_truth_digest)`.
    pub fn page_file_digests(&self) -> Result<Vec<(String, String, String)>> {
        let mut out = Vec::with_capacity(self.manifest.pages.len());
        for (page, (input, ground_truth)) in self.manifest.pages.iter().zip(&self.resolved_paths) {
            let input_digest = sha256_file(input)?;
            let gt_digest = sha256_file(ground_truth)?;
            out.push((page.id.clone(), input_digest, gt_digest));
        }
        Ok(out)
    }
}

/// Resolves `relative` against `root` and requires the canonicalized
/// result to remain inside the canonicalized `root` — the core defense
/// against both `../..` traversal and a symlink that escapes the dataset
/// directory. The file must exist (canonicalization requires it), which
/// is also exactly what a benchmark needs anyway: a manifest pointing at
/// a nonexistent file is invalid regardless of the traversal check.
fn resolve_contained(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() {
        return Err(CoreError::InvalidBenchmarkManifest(
            "empty path in dataset manifest".to_string(),
        ));
    }
    let candidate = root.join(relative);
    let canonical = std::fs::canonicalize(&candidate).map_err(|e| {
        CoreError::MissingGroundTruth(format!(
            "could not resolve '{relative}' under dataset root {}: {e}",
            root.display()
        ))
    })?;
    if !canonical.starts_with(root) {
        return Err(CoreError::DatasetPathTraversal(format!(
            "'{relative}' resolves outside the dataset root {}",
            root.display()
        )));
    }
    Ok(canonical)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| CoreError::io(path.to_path_buf(), e))?;
    Ok(sha256_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, contents: &[u8]) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn minimal_manifest_toml() -> String {
        r#"
schema = "museion-binarize-benchmark-dataset"
schema_version = "1.0"
id = "test-dataset"
title = "Test dataset"
license = "CC0-1.0"
provenance = "unit test"
ground_truth_method = "synthetic"

[[pages]]
id = "page-1"
input = "input/page-1.png"
ground_truth = "ground-truth/page-1.png"
category = "text"
"#
        .to_string()
    }

    #[test]
    fn loads_a_valid_minimal_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "dataset.toml",
            minimal_manifest_toml().as_bytes(),
        );
        write(dir.path(), "input/page-1.png", b"fake-png-bytes");
        write(dir.path(), "ground-truth/page-1.png", b"fake-gt-bytes");

        let dataset = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap();
        assert_eq!(dataset.manifest.id, "test-dataset");
        assert_eq!(dataset.resolved_paths.len(), 1);
        assert!(dataset.resolved_paths[0].0.ends_with("input/page-1.png"));
    }

    #[test]
    fn rejects_wrong_schema() {
        let dir = tempfile::tempdir().unwrap();
        let bad = minimal_manifest_toml()
            .replace("museion-binarize-benchmark-dataset", "some-other-schema");
        write(dir.path(), "dataset.toml", bad.as_bytes());
        let err = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap_err();
        assert!(matches!(err, CoreError::UnsupportedBenchmarkSchema(_)));
    }

    #[test]
    fn rejects_path_traversal_outside_the_dataset_root() {
        let dir = tempfile::tempdir().unwrap();
        let manifest =
            minimal_manifest_toml().replace("input/page-1.png", "../../../../../../etc/passwd");
        write(dir.path(), "dataset.toml", manifest.as_bytes());
        write(dir.path(), "ground-truth/page-1.png", b"x");
        let err = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap_err();
        assert!(
            matches!(err, CoreError::DatasetPathTraversal(_))
                || matches!(err, CoreError::MissingGroundTruth(_)),
            "expected traversal or missing-file error, got {err:?}"
        );
    }

    #[test]
    fn rejects_a_symlink_that_escapes_the_dataset_root() {
        #[cfg(unix)]
        {
            let dataset_dir = tempfile::tempdir().unwrap();
            let outside_dir = tempfile::tempdir().unwrap();
            write(outside_dir.path(), "secret.png", b"secret");
            write(
                dataset_dir.path(),
                "dataset.toml",
                minimal_manifest_toml().as_bytes(),
            );
            write(dataset_dir.path(), "ground-truth/page-1.png", b"x");
            std::fs::create_dir_all(dataset_dir.path().join("input")).unwrap();
            std::os::unix::fs::symlink(
                outside_dir.path().join("secret.png"),
                dataset_dir.path().join("input/page-1.png"),
            )
            .unwrap();

            let err = LoadedDataset::load(&dataset_dir.path().join("dataset.toml")).unwrap_err();
            assert!(matches!(err, CoreError::DatasetPathTraversal(_)));
        }
    }

    #[test]
    fn rejects_missing_referenced_file() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "dataset.toml",
            minimal_manifest_toml().as_bytes(),
        );
        // ground-truth/page-1.png intentionally not written.
        write(dir.path(), "input/page-1.png", b"x");
        let err = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap_err();
        assert!(matches!(err, CoreError::MissingGroundTruth(_)));
    }

    #[test]
    fn rejects_duplicate_page_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = minimal_manifest_toml();
        manifest.push_str(
            "\n[[pages]]\nid = \"page-1\"\ninput = \"input/page-1.png\"\nground_truth = \"ground-truth/page-1.png\"\ncategory = \"text\"\n",
        );
        write(dir.path(), "dataset.toml", manifest.as_bytes());
        write(dir.path(), "input/page-1.png", b"x");
        write(dir.path(), "ground-truth/page-1.png", b"x");
        let err = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap_err();
        assert!(matches!(err, CoreError::InvalidBenchmarkManifest(_)));
    }

    #[test]
    fn manifest_digest_is_stable_for_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "dataset.toml",
            minimal_manifest_toml().as_bytes(),
        );
        write(dir.path(), "input/page-1.png", b"x");
        write(dir.path(), "ground-truth/page-1.png", b"x");
        let a = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap();
        let b = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap();
        assert_eq!(a.manifest_digest(), b.manifest_digest());
    }

    #[test]
    fn page_file_digests_differ_for_different_content() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "dataset.toml",
            minimal_manifest_toml().as_bytes(),
        );
        write(dir.path(), "input/page-1.png", b"aaa");
        write(dir.path(), "ground-truth/page-1.png", b"bbb");
        let dataset = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap();
        let digests = dataset.page_file_digests().unwrap();
        assert_eq!(digests.len(), 1);
        assert_ne!(digests[0].1, digests[0].2);
    }

    #[test]
    fn rejects_a_dataset_with_no_pages() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = minimal_manifest_toml()
            .lines()
            .take_while(|l| !l.starts_with("[[pages]]"))
            .collect::<Vec<_>>()
            .join("\n");
        write(dir.path(), "dataset.toml", manifest.as_bytes());
        let err = LoadedDataset::load(&dir.path().join("dataset.toml")).unwrap_err();
        assert!(matches!(err, CoreError::InvalidBenchmarkManifest(_)));
    }
}
