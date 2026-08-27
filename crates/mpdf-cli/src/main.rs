//! Command-line interface for M PDF Processor.
//!
//! `info`, `inspect`, `analyze`, `estimate`, `process`, `preview` — see
//! `docs/cli.md` for the full contract: stdout/stderr behaviour, `--json`
//! mode, and the exit-code table.

mod cli;
mod commands;
mod errors;
mod output;
mod progress;

use std::process::ExitCode;

use clap::Parser;

use cli::{
    BenchmarkCommand, BookmarkCommand, Cli, Command, JobCommand, PackageCommand, PdfCommand,
    RevisionCommand,
};

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
        Some(Command::Estimate(args)) => commands::estimate::run(args),
        Some(Command::Process(args)) => commands::process::run(args),
        Some(Command::Preview(args)) => commands::preview::run(args),
        Some(Command::Benchmark(BenchmarkCommand::Run(args))) => commands::benchmark::run(args),
        Some(Command::Benchmark(BenchmarkCommand::Validate(args))) => {
            commands::benchmark::validate(args)
        }
        Some(Command::Package(PackageCommand::Create(args))) => commands::package::create(args),
        Some(Command::Package(PackageCommand::Validate(args))) => commands::package::validate(args),
        Some(Command::Job(JobCommand::Create(args))) => commands::job::create(args),
        Some(Command::Job(JobCommand::Status(args))) => commands::job::status(args),
        Some(Command::Job(JobCommand::Cancel(args))) => commands::job::cancel(args),
        Some(Command::Ocr(args)) => commands::ocr::run(args),
        Some(Command::Export(args)) => commands::derived::export(args),
        Some(Command::Review(args)) => commands::derived::review(args),
        Some(Command::Revision(RevisionCommand::Add(args))) => {
            commands::derived::revision_add(args)
        }
        Some(Command::Revision(RevisionCommand::List(args))) => {
            commands::derived::revision_list(args)
        }
        Some(Command::Bookmark(BookmarkCommand::Generate(args))) => {
            commands::bookmarks::generate(args)
        }
        Some(Command::Bookmark(BookmarkCommand::List(args))) => commands::bookmarks::list(args),
        Some(Command::Bookmark(BookmarkCommand::Confirm(args))) => {
            commands::bookmarks::confirm(args)
        }
        Some(Command::Bookmark(BookmarkCommand::Reject(args))) => commands::bookmarks::reject(args),
        Some(Command::Bookmark(BookmarkCommand::Edit(args))) => commands::bookmarks::edit(args),
        Some(Command::Bookmark(BookmarkCommand::Reparent(args))) => {
            commands::bookmarks::reparent(args)
        }
        Some(Command::Pdf(PdfCommand::BuildSearchable(args))) => {
            commands::pdf::build_searchable(args)
        }
    }
}
