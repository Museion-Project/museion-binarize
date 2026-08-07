//! One module per CLI command: argument -> core call -> human or JSON
//! output. Command parsing lives in `crate::cli`; JSON/report mechanics
//! live in `crate::output`; exit-code and error-envelope mapping live in
//! `crate::errors`. Each command function here only wires those together.

pub mod analyze;
pub mod benchmark;
pub mod estimate;
pub mod info;
pub mod inspect;
pub mod preview;
pub mod process;

/// Password read from the environment, never from a command-line flag —
/// so it never appears in a command line, shell history, or process
/// listing (`ps`). Documented in `docs/cli.md`.
pub const PASSWORD_ENV: &str = "MUSEION_PDF_PASSWORD";

pub fn password_from_env() -> Option<String> {
    std::env::var(PASSWORD_ENV).ok().filter(|p| !p.is_empty())
}
