//! Development helper: write the synthetic PDF fixtures to a directory.
//!
//! ```text
//! cargo run -p mpdf-core --example gen_fixtures -- /tmp/fx
//! ```
//!
//! The directory is created if it does not already exist.

use std::path::Path;
use std::process::ExitCode;

use mpdf_core::test_fixtures as f;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gen_fixtures: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(dir) = std::env::args().nth(1) else {
        return Err("usage: gen_fixtures <dir>".to_string());
    };
    let dir = Path::new(&dir);

    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    for (name, bytes) in [
        ("orientation.pdf", f::orientation_and_polarity()),
        ("mixed.pdf", f::mixed_page_sizes()),
        ("rotations.pdf", f::page_rotations()),
        ("threshold.pdf", f::threshold_patterns()),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        println!("{name}: {} bytes", bytes.len());
    }
    Ok(())
}
