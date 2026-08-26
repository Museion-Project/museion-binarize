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

use cli::{BenchmarkCommand, Cli, Command, JobCommand, PackageCommand};

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
    }
}
