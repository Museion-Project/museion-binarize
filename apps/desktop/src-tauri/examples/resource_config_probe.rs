//! Configuration parsing only: no App, window, recognizer, download or signing.
//! A caller-owned directory contains tauri.conf.json and tauri.macos.conf.json.
//! Tauri's actual locked parser merges them, then its Config schema validates it.
use std::path::PathBuf;
use tauri::utils::{config, platform::Target};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().ok_or("configuration directory required")?);
    if args.next().is_some() || !directory.is_dir() {
        return Err("exactly one existing configuration directory required".into());
    }
    let (raw, inputs) = config::parse::read_from(Target::MacOS, &directory)?;
    let typed: config::Config = serde_json::from_value(raw.clone())?;
    println!(
        "{}",
        serde_json::json!({
            "schema": "local-App-resource-config-probe/1",
            "path": "locked Tauri parser + Config schema; same JSON Merge Patch as TAURI_CONFIG",
            "inputs": inputs,
            "effective_resources": raw.pointer("/bundle/resources"),
            "typed_resources": typed.bundle.resources,
            "App_started": false,
            "GUI": 0,
            "OCR": 0,
            "provider": 0,
            "signing": false,
            "release_ready": false
        })
    );
    Ok(())
}
