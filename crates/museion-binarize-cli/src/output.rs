//! JSON output and atomic report-file writing.
//!
//! Kept separate from progress reporting (`main.rs`'s `StderrProgress`)
//! and from human-readable formatting (`commands/*.rs`) so a JSON-mode
//! command can never accidentally interleave prose or progress text with
//! its one JSON document. See the module-level stdout/stderr contract in
//! `docs/cli.md`.

use std::io::Write as _;
use std::path::Path;

use serde::Serialize;

/// Serializes `value` to stdout as exactly one JSON document, followed by
/// exactly one trailing newline: no leading or trailing prose, no ANSI
/// escapes. `pretty` selects two-space-indented formatting.
///
/// Panics only if `value` cannot be represented as JSON at all (e.g. a
/// `NaN`/`Infinity` float slipped through construction) — every report
/// type in this crate is built to make that impossible, so a panic here
/// would mean a bug in report construction, not a runtime condition a
/// caller should recover from.
pub fn print_json<T: Serialize>(value: &T, pretty: bool) {
    let text = render_json(value, pretty)
        .expect("report types are built from finite, well-formed data and always serialize");
    println!("{text}");
}

fn render_json<T: Serialize>(value: &T, pretty: bool) -> serde_json::Result<String> {
    if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
}

/// Writes `value` as JSON to `path`, refusing to silently overwrite an
/// existing file and persisting atomically (temporary file + rename),
/// mirroring the main output PDF's own write-then-rename discipline.
pub fn write_report_atomic<T: Serialize>(
    value: &T,
    path: &Path,
    pretty: bool,
    overwrite: bool,
) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!("{} is a directory, not a file", path.display()));
    }
    if path.exists() && !overwrite {
        return Err(format!(
            "{} already exists; pass --overwrite to replace it",
            path.display()
        ));
    }

    let text =
        render_json(value, pretty).map_err(|e| format!("could not serialize the report: {e}"))?;

    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let mut temp = match directory {
        Some(dir) => tempfile::Builder::new()
            .prefix(".museion-binarize-report-")
            .suffix(".json.partial")
            .tempfile_in(dir),
        None => tempfile::Builder::new()
            .prefix(".museion-binarize-report-")
            .suffix(".json.partial")
            .tempfile(),
    }
    .map_err(|e| format!("could not create a temporary report file: {e}"))?;

    temp.write_all(text.as_bytes())
        .map_err(|e| format!("could not write the temporary report file: {e}"))?;
    temp.flush()
        .map_err(|e| format!("could not flush the temporary report file: {e}"))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("could not sync the temporary report file: {e}"))?;

    // Same platform caveat as the main output PDF: Unix/macOS rename is a
    // single atomic replacement; Windows must unlink an existing
    // destination first, leaving a narrow window. See `docs/pdf-output.md`.
    #[cfg(windows)]
    if overwrite && path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("could not remove the existing report: {e}"))?;
    }

    temp.persist(path).map_err(|e| {
        format!(
            "could not move the completed report into place: {}",
            e.error
        )
    })?;
    Ok(())
}

/// Rejects a set of named paths (e.g. `input`, `output`, `report`) that
/// alias each other on the filesystem. Writing two different outputs to
/// the same file would silently corrupt whichever is written second, so
/// this is checked up front for every combination of paths a command
/// accepts, not only the input/output pair the core already protects.
pub fn reject_path_aliases(named: &[(&str, &Path)]) -> Result<(), String> {
    for i in 0..named.len() {
        for j in (i + 1)..named.len() {
            let (name_a, path_a) = named[i];
            let (name_b, path_b) = named[j];
            if paths_refer_to_same_file(path_a, path_b) {
                return Err(format!(
                    "--{name_a} and --{name_b} must not refer to the same file ({})",
                    path_a.display()
                ));
            }
        }
    }
    Ok(())
}

fn paths_refer_to_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Sample {
        a: u32,
        b: String,
    }

    fn sample() -> Sample {
        Sample {
            a: 1,
            b: "x".to_string(),
        }
    }

    #[test]
    fn render_json_compact_has_no_extra_whitespace() {
        let text = render_json(&sample(), false).unwrap();
        assert!(!text.contains('\n'));
        assert_eq!(text, r#"{"a":1,"b":"x"}"#);
    }

    #[test]
    fn render_json_pretty_is_multi_line_and_round_trips() {
        let text = render_json(&sample(), true).unwrap();
        assert!(text.contains('\n'));
        let back: Sample = serde_json::from_str(&text).unwrap();
        assert_eq!(back, sample());
    }

    #[test]
    fn write_report_atomic_refuses_to_overwrite_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, b"old").unwrap();
        let err = write_report_atomic(&sample(), &path, false, false).unwrap_err();
        assert!(err.contains("already exists"));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn write_report_atomic_overwrites_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, b"old").unwrap();
        write_report_atomic(&sample(), &path, false, true).unwrap();
        let back: Sample = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, sample());
    }

    #[test]
    fn write_report_atomic_leaves_no_partial_file_and_no_temp_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        write_report_atomic(&sample(), &path, true, false).unwrap();
        assert!(path.exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "report.json")
            .collect();
        assert!(leftovers.is_empty(), "leftover files: {leftovers:?}");
    }

    #[test]
    fn write_report_atomic_rejects_a_directory_target() {
        let dir = tempfile::tempdir().unwrap();
        let err = write_report_atomic(&sample(), dir.path(), false, true).unwrap_err();
        assert!(err.contains("directory"));
    }

    #[test]
    fn reject_path_aliases_catches_identical_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.pdf");
        std::fs::write(&path, b"x").unwrap();
        let err = reject_path_aliases(&[("output", &path), ("report", &path)]).unwrap_err();
        assert!(err.contains("--output"));
        assert!(err.contains("--report"));
    }

    #[test]
    fn reject_path_aliases_allows_distinct_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.json");
        assert!(reject_path_aliases(&[("output", &a), ("report", &b)]).is_ok());
    }

    #[test]
    fn reject_path_aliases_checks_every_pair_not_only_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.json");
        std::fs::write(&b, b"x").unwrap();
        let err =
            reject_path_aliases(&[("output", &a), ("report", &b), ("preview", &b)]).unwrap_err();
        assert!(err.contains("--report") && err.contains("--preview"));
    }
}
