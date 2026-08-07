# Contributing to Museion Binarize

Thank you for your interest in contributing. Museion Binarize is an
open-source project maintained by Pei Haoran under the Museion Project
organization. This document explains how to work with the codebase and what
is expected of contributions.

Development may be assisted by AI coding tools. Architectural decisions,
testing, review, scholarly requirements, and releases remain under human
responsibility.

## Project status

The project is in **Phase 1, early development**. See
[`docs/roadmap.md`](docs/roadmap.md) for the milestone plan and
[`docs/limitations.md`](docs/limitations.md) for what is not yet implemented.

## Getting the code building

### Prerequisites

- Rust, pinned via [`rust-toolchain.toml`](rust-toolchain.toml) (installed
  automatically by `rustup` when you run `cargo` inside the repository)
- Node.js, version pinned in [`.nvmrc`](.nvmrc)
- [pnpm](https://pnpm.io/), the only supported JavaScript package manager for
  this repository (enable via `corepack enable pnpm`)

### Rust workspace

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Desktop application

```bash
pnpm install
pnpm --filter museion-binarize-desktop dev
```

## Development guidelines

- **Formatting.** Rust code must pass `cargo fmt --check`. Frontend code must
  pass the configured linter (`pnpm lint`) and type checker (`pnpm typecheck`).
- **Tests.** New behavior must include tests. `cargo test --workspace` and
  `pnpm test` must pass before a pull request is merged.
- **No copyrighted sample pages without permission.** Do not add scanned book
  pages, page images, or other copyrighted material to `test-data/` or
  `benchmarks/` unless you hold the rights or have documented, explicit
  permission. Public-domain or synthetically generated fixtures are
  preferred; see [`test-data/README.md`](test-data/README.md).
- **No generated corpus uploads without documented provenance.** Any
  fixture, sample, or benchmark input added to the repository must include a
  note on where it came from and under what license or permission it is
  included.
- **No performance or preservation claims without reproducible evidence.**
  Claims about accuracy, compression, speed, or typographic/script
  preservation (e.g. for polytonic Ancient Greek) must be backed by a
  reproducible benchmark described in [`docs/benchmarking.md`](docs/benchmarking.md).
  Do not add such claims to documentation, commit messages, or release notes
  without the underlying data and method.
- **Scope.** The processing core (`crates/museion-binarize-core`) must not
  depend on Tauri or any GUI framework. See [`docs/architecture.md`](docs/architecture.md).

## Commit and pull request conventions

- Keep commits small and focused; prefer several understandable commits over
  one large one.
- Write commit messages that explain intent, not just the diff.
- Fill out the pull request template, including the testing you performed.
- Draft pull requests are welcome for early feedback on architecture or
  approach.

## Reporting issues

Please use the issue templates under `.github/ISSUE_TEMPLATE/`. For security
issues, follow [`SECURITY.md`](SECURITY.md) instead of filing a public issue.

## License

By contributing, you agree that your contributions will be licensed under
the same dual MIT OR Apache-2.0 terms as the rest of the project.
