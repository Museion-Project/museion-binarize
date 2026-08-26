//! Development-only wiring for the durable M2 job store. This command does
//! not invoke OCR; it is intentionally limited to creating and inspecting
//! local state so a provider adapter can be added without changing the CLI
//! contract.

use std::process::ExitCode;

use mpdf_core::jobs::JobStore;

use crate::cli::{JobCancelArgs, JobCreateArgs, JobStatusArgs};
use crate::errors::ExitReason;

pub fn create(args: JobCreateArgs) -> ExitCode {
    match JobStore::open(&args.db).and_then(|store| store.create_job(&args.job_id, args.pages)) {
        Ok(job) => {
            println!(
                "{}",
                serde_json::to_string(&job).expect("job record is serializable")
            );
            ExitReason::Success.exit_code()
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitReason::InputError.exit_code()
        }
    }
}

pub fn status(args: JobStatusArgs) -> ExitCode {
    match JobStore::open(&args.db).and_then(|store| store.progress(&args.job_id)) {
        Ok(Some(progress)) => {
            println!(
                "{}",
                serde_json::to_string(&progress).expect("job progress is serializable")
            );
            ExitReason::Success.exit_code()
        }
        Ok(None) => {
            eprintln!("error: job does not exist: {}", args.job_id);
            ExitReason::InputError.exit_code()
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitReason::InputError.exit_code()
        }
    }
}

pub fn cancel(args: JobCancelArgs) -> ExitCode {
    match JobStore::open(&args.db).and_then(|store| store.request_cancel(&args.job_id)) {
        Ok(()) => {
            println!("cancel requested: {}", args.job_id);
            ExitReason::Success.exit_code()
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitReason::InputError.exit_code()
        }
    }
}
