//! Resolver for an auditable bundled OCR runtime.
//!
//! Explicit sidecar/model arguments and their environment overrides are
//! handled by front ends first. This module only resolves the optional
//! executable-adjacent/resource layout and fails closed when its manifest is
//! absent or inconsistent. It never searches for, or falls back to, system
//! Python/Tesseract.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledOcrRuntime {
    pub engine: PathBuf,
    pub sidecar: PathBuf,
    pub model_dir: PathBuf,
    pub manifest_sha256: String,
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn entry<'a>(manifest: &'a Value, path: &str) -> Option<&'a Value> {
    manifest
        .get("files")?
        .as_array()?
        .iter()
        .find(|item| item.get("path").and_then(Value::as_str) == Some(path))
}

fn verify_file(
    root: &Path,
    manifest: &Value,
    relative: &str,
    role: &str,
) -> Result<PathBuf, String> {
    let item = entry(manifest, relative)
        .ok_or_else(|| format!("bundled OCR manifest is missing {role} entry"))?;
    if item.get("role").and_then(Value::as_str) != Some(role) {
        return Err(format!("bundled OCR manifest role mismatch for {relative}"));
    }
    let path = root.join(relative);
    if !path.is_file() || path.is_symlink() {
        return Err(format!("bundled OCR {role} is unavailable"));
    }
    let bytes = std::fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len();
    if item.get("size_bytes").and_then(Value::as_u64) != Some(bytes) {
        return Err(format!("bundled OCR size mismatch for {relative}"));
    }
    if item.get("sha256").and_then(Value::as_str) != Some(sha256_file(&path)?.as_str()) {
        return Err(format!("bundled OCR SHA-256 mismatch for {relative}"));
    }
    Ok(path)
}

fn is_macho(path: &Path) -> bool {
    std::fs::read(path)
        .map(|bytes| {
            bytes.get(..4).is_some_and(|magic| {
                magic == b"\xfe\xed\xfa\xce"
                    || magic == b"\xce\xfa\xed\xfe"
                    || magic == b"\xfe\xed\xfa\xcf"
                    || magic == b"\xcf\xfa\xed\xfe"
                    || magic == b"\xca\xfe\xba\xbe"
                    || magic == b"\xbe\xba\xfe\xca"
            })
        })
        .unwrap_or(false)
}

/// Validate one staged runtime and return the two paths consumed by the
/// sidecar. The target is explicit so tests can validate a macOS layout on
/// another host without weakening the production resolver.
pub fn resolve_under(
    root: &Path,
    target: &str,
    language_profile: &str,
) -> Result<BundledOcrRuntime, String> {
    let manifest_path = root.join("runtime-manifest.json");
    if !root.is_dir() || root.is_symlink() || !manifest_path.is_file() || manifest_path.is_symlink()
    {
        return Err("bundled OCR runtime manifest is unavailable".into());
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("invalid bundled OCR runtime manifest: {error}"))?;
    if manifest.get("schema").and_then(Value::as_str) != Some("mpdf-ocr-runtime-bundle")
        || manifest.get("schema_version").and_then(Value::as_str) != Some("1.0")
        || manifest.get("release").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION"))
        || manifest.get("target").and_then(Value::as_str) != Some(target)
        || manifest.get("engine").and_then(Value::as_str) != Some("tesseract")
        || manifest.get("sidecar_protocol").and_then(Value::as_str) != Some("mpdf-ocr/0.1")
        || manifest
            .get("requires_system_python")
            .and_then(Value::as_bool)
            != Some(false)
        || manifest
            .get("requires_system_tesseract")
            .and_then(Value::as_bool)
            != Some(false)
        || (target.ends_with("-apple-darwin")
            && (manifest.get("inspection_mode").and_then(Value::as_str) != Some("mach-o")
                || manifest
                    .get("post_rewrite_verified")
                    .and_then(Value::as_bool)
                    != Some(true)))
    {
        return Err("bundled OCR runtime manifest identity/policy mismatch".into());
    }
    if manifest.get("model_set").and_then(Value::as_str) != Some(crate::ocr::PRODUCTION_MODEL_SET)
        || manifest.get("model_set_version").and_then(Value::as_str)
            != Some(crate::ocr::PRODUCTION_MODEL_SET_VERSION)
    {
        return Err("bundled OCR model set mismatch".into());
    }
    let engine = verify_file(root, &manifest, "bin/tesseract", "engine")?;
    let sidecar = verify_file(root, &manifest, "bin/mpdf-ocr-sidecar", "sidecar")?;
    if target.ends_with("-apple-darwin") && (!is_macho(&engine) || !is_macho(&sidecar)) {
        return Err("bundled OCR executable is not Mach-O".into());
    }
    let model_dir = root.join("tessdata");
    if !model_dir.is_dir() || model_dir.is_symlink() {
        return Err("bundled OCR model directory is unavailable".into());
    }
    for model in
        crate::ocr::required_model_files(language_profile).ok_or("unknown OCR language profile")?
    {
        if model == "manifest.json" {
            continue;
        }
        let relative = format!("tessdata/{model}");
        verify_file(root, &manifest, &relative, "model")?;
    }
    verify_file(root, &manifest, "tessdata/manifest.json", "model-manifest")?;
    let manifest_sha256 = sha256_file(&manifest_path)?;
    Ok(BundledOcrRuntime {
        engine,
        sidecar,
        model_dir,
        manifest_sha256,
    })
}

/// Resolve only the production executable-adjacent layout on macOS.
pub fn resolve_executable_adjacent(
    language_profile: &str,
) -> Result<Option<BundledOcrRuntime>, String> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64-apple-darwin"
    } else {
        return Ok(None);
    };
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let root = executable
        .parent()
        .ok_or("CLI has no executable parent")?
        .join("ocr-runtime");
    resolve_under(&root, target, language_profile).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_manifest_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let error = resolve_under(dir.path(), "aarch64-apple-darwin", "english").unwrap_err();
        assert!(error.contains("manifest"));
    }
    #[test]
    fn relative_layout_requires_manifest_identity() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("tessdata")).unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        fs::write(dir.path().join("bin/mpdf-ocr-sidecar"), b"x").unwrap();
        fs::write(dir.path().join("runtime-manifest.json"), b"{}").unwrap();
        let error = resolve_under(dir.path(), "aarch64-apple-darwin", "english").unwrap_err();
        assert!(error.contains("identity"));
    }
}
