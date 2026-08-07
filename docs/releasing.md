# Releasing (Milestone 7A infrastructure)

This document describes the release **infrastructure** Milestone 7A
built. It does not describe a public release — none has been published.
See [`distribution.md`](distribution.md) for the broader distribution
model and [`pdfium-bundling.md`](pdfium-bundling.md) for how packaged
builds get a trusted PDFium.

## Versioning

**Single source of truth: the workspace `Cargo.toml`'s
`[workspace.package].version`.** Every other version field in the
repository must match it exactly:

- `apps/desktop/package.json`'s `version`
- `apps/desktop/src-tauri/tauri.conf.json`'s `version`
- every workspace-member crate's `Cargo.toml`, via `version.workspace =
  true` (never a literal per-crate version)

`scripts/distribution/check_version_consistency.py` enforces this and
runs in ordinary CI (`.github/workflows/ci.yml`'s `version-consistency`
job) — a release cannot accidentally contain, e.g., CLI `0.1.0`
alongside desktop `package.json` `0.1.1`.

### SemVer policy

Recommended trajectory toward the first tagged release:

```
0.1.0-rc.1
0.1.0-rc.2
...
0.1.0
```

**No tag was created during Milestone 7A**, and `1.0.0` is not planned
as the first release. This milestone makes the repository *capable* of
producing an RC; it does not produce one.

## Artifact naming

Deterministic, defined once in `scripts/distribution/naming.py` and used
by every packaging script — never a workflow-run-number or timestamp in
a filename:

```
Museion-Binarize-<version>-macos-arm64.dmg
Museion-Binarize-<version>-macos-x64.dmg
Museion-Binarize-<version>-windows-x64.msi
Museion-Binarize-<version>-linux-x86_64.AppImage
Museion-Binarize-<version>-linux-x86_64.deb
museion-binarize-cli-<version>-macos-arm64.tar.gz
museion-binarize-cli-<version>-macos-x64.tar.gz
museion-binarize-cli-<version>-windows-x64.zip
museion-binarize-cli-<version>-linux-x86_64.tar.gz
```

## Checksums

`scripts/distribution/checksums.py <dir>` writes one `SHA256SUMS` file
covering every artifact in a directory. **Must run after any mutation**
(signing, notarization stapling) — a checksum computed before those
steps describes a different file. If a signed/notarized artifact is
produced later, its checksum must be regenerated; the pre-signing
checksum is never published as if it described the signed file.

## Release manifest

Schema `museion-binarize-release-manifest` v1.0
(`scripts/distribution/release_manifest.py`). Per artifact:

```json
{
  "target_triple": "aarch64-apple-darwin",
  "os": "macos",
  "arch": "arm64",
  "artifact_filename": "Museion-Binarize-0.1.0-macos-arm64.dmg",
  "artifact_sha256": "...",
  "pdfium_build": "7920",
  "pdfium_version": "151.0.7920.0",
  "pdfium_sha256": "...",
  "signing_state": "unsigned | signed | pending_credentials",
  "notarization_state": "not_applicable | notarized | pending_credentials"
}
```

Deliberately excludes username, hostname, home directory, secret
names/values, and absolute developer filesystem paths. A manifest
refuses to mix artifacts from two different `(project_version, git_sha)`
pairs into one file (`release_manifest.load_or_init`'s check) — a
release's provenance record must describe one build, not an
accidentally merged history of several.

## PDFium provenance

See [`pdfium-bundling.md`](pdfium-bundling.md) and
`distribution/pdfium/manifest.toml`: every packaging target's PDFium
build is pinned by exact upstream release URL and verified by
downloading and hashing the real asset — never `curl .../latest`, never
a checksum accepted with only a warning on mismatch. Verified for real
during this milestone (network access was available): all four pinned
assets (`aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, all PDFium build
7920 / version 151.0.7920.0) were actually downloaded from
`github.com/bblanchon/pdfium-binaries`'s release, hashed, and their
architecture confirmed via `file` (`arm64`, `x86_64` Mach-O, `x86-64`
ELF, `x86-64` PE32+, respectively).

## GitHub Actions distribution workflow

`.github/workflows/build-distribution.yml`, trigger: `workflow_dispatch`
only — **never** runs on a normal push or pull request, and never
publishes a public GitHub Release. It validates version consistency,
fetches and checksum-verifies PDFium per target, builds and packages the
desktop app and CLI archive, inspects bundled-dependency architecture
(fails the job on a mismatch), generates the release manifest and
`SHA256SUMS`, and uploads everything as a private workflow-run artifact
— nothing public is produced by running it.

**Not exercised in a real GitHub Actions run during this milestone** —
triggering it would require pushing to the repository and dispatching
the workflow, which this milestone's implementation phase did not do.
Its YAML has been validated for syntax; the individual steps were
validated by running the equivalent commands directly on this machine
(see `docs/desktop-testing.md`'s Milestone 7A section for exact
transcripts of the macOS build/package/smoke-test steps this workflow
automates).

### Normal PR CI vs. this workflow

Ordinary PR CI (`.github/workflows/ci.yml`) stays fast: Rust
fmt/clippy/test, `cargo-deny`, frontend lint/typecheck/test/build,
version consistency, and the distribution scripts' own unit tests — no
multi-platform packaging on every PR. Full packaging only runs when this
separate workflow is manually dispatched.

## Signing and notarization

**Integration implemented; no artifact produced during this milestone
is actually signed or notarized — no Apple Developer credentials were
available to this implementation.**

| | Integration state | Actual state |
|---|---|---|
| Developer ID signing | Implemented (conditional workflow step, secret-gated) | Pending owner credentials |
| Notarization | Integration point documented | Pending owner credentials |

Required secret names (values never committed, logged, or placed in any
artifact/manifest):

```
APPLE_CERTIFICATE
APPLE_CERTIFICATE_PASSWORD
APPLE_SIGNING_IDENTITY
APPLE_TEAM_ID
APPLE_API_KEY
APPLE_API_ISSUER
```

The workflow's signing step is conditional on `APPLE_CERTIFICATE` being
set — an unsigned build still runs the full architecture-validation
pipeline (bundled PDFium present, correct architecture, no dev-path
leakage) and produces a useful artifact on its own; signing is an
addition on top, not a prerequisite for the rest of the pipeline to be
meaningful. **Never recommend disabling Gatekeeper** (`sudo spctl
--master-disable`) as an installation step for any build this project
produces; an unsigned development/CI artifact is clearly non-production
and requires an explicit, informed developer action to open, not a
system-wide security downgrade.

### Windows signing

Authenticode signing is not required for the architecture to be
considered complete — no certificate is configured. **Windows package
unsigned.** No SmartScreen reputation/trust claim is made anywhere in
this repository's documentation.

## Publication is a separate deliberate step

This milestone's workflow produces workflow-run artifacts only. The
intended future publication flow:

```
validated packaged artifacts (this workflow)
  -> owner-triggered publish action
  -> Draft GitHub Release
  -> upload final artifacts + SHA256SUMS + release-manifest.json
  -> owner review
  -> mark prerelease/public
```

No stable release is created automatically by merging a PR, and none
was created by this milestone's implementation work.
