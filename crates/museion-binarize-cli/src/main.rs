//! Command-line interface for Museion Binarize.
//!
//! `info`, `inspect`, `analyze`, `process`, `preview` — see `docs/cli.md`
//! for the full contract: stdout/stderr behaviour, `--json` mode, and the
//! exit-code table.

mod cli;
mod commands;
mod errors;
mod output;
mod progress;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => commands::info::run(cli::InfoArgs {
            output: cli::OutputArgs::default(),
            probe_pdfium: false,
            pdfium: cli::PdfiumArgs {
                pdfium_library: None,
                allow_system_pdfium: false,
            },
        }),
        Some(Command::Info(args)) => commands::info::run(args),
        Some(Command::Inspect(args)) => commands::inspect::run(args),
        Some(Command::Analyze(args)) => commands::analyze::run(args),
        Some(Command::Process(args)) => commands::process::run(args),
        Some(Command::Preview(args)) => commands::preview::run(args),
    }
}
