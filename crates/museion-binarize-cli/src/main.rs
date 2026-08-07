//! Command-line interface for Museion Binarize.
//!
//! At this stage (Milestone 0), this CLI does not convert PDF files. It
//! exists to confirm the workspace builds end to end and to expose basic
//! project information. See `docs/roadmap.md` for when PDF conversion
//! commands will be added.

use clap::{Parser, Subcommand};
use museion_binarize_core::ProjectInfo;

#[derive(Parser)]
#[command(name = "museion-binarize", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print project information (name and current development phase).
    Info,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Info) | None => {
            let info = ProjectInfo::current();
            println!("{} — {}", info.name, info.phase);
            println!("PDF conversion is not implemented yet; see docs/limitations.md.");
        }
    }
}
