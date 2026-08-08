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
as the first release. Milestone 7A made the repository *capable* of
producing an RC; the `0.1.0-rc.1` release-prep work is what actually
produces one.

### Prerelease versioning across packaging targets

Before adopting `0.1.0-rc.1` as the real version, every packaging target
was checked empirically (real local and CI builds, not assumption) for
whether it accepts a SemVer prerelease identifier cleanly:

| Target | Result |
|---|---|
| Cargo workspace (`[workspace.package].version`) | Accepts it natively — Rust's `semver` crate is fully SemVer-compliant. Verified via `cargo metadata`. |
| `apps/desktop/package.json` | Accepts it natively (node-semver). |
| `tauri.conf.json` `version` | Accepts it; flows through to `CFBundleShortVersionString`/`CFBundleVersion` unchanged on macOS. Verified with a real local build. |
| macOS bundle (`.app`, `.dmg`) | Full build/sign/package pipeline verified locally with `0.1.0-rc.1` end to end — no rejection, no transformation. |
| **Windows MSI (WiX)** | **Rejects a non-numeric prerelease identifier in `ProductVersion`** — Windows Installer's `ProductVersion` is strictly `major.minor.build`, numeric only (Microsoft's own documented limit: build field ≤ 65,535, no fourth field recognized). Tauri's `tauri-bundler` added msi-specific prerelease/build-metadata support that must also be numeric-only. `0.1.0-rc.1`'s `rc.1` prerelease identifier is neither. **Fix**: `bundle.windows.wix.version` (`tauri.dist.conf.json`) overrides the MSI-internal `ProductVersion` with a numeric-only value (`0.1.0.1` for this RC) while every other version field — including the installer's own filename — keeps the real `0.1.0-rc.1`. Verified via a real Windows CI build: `Museion Binarize_0.1.0-rc.1_x64_en-US.msi` built successfully. |
| Windows NSIS | Accepts `0.1.0-rc.1` directly, no override needed — verified via the same real Windows CI build (`Museion Binarize_0.1.0-rc.1_x64-setup.exe`). |
| Linux `.deb`/`.rpm`/AppImage | Accepts `0.1.0-rc.1` directly — verified via a real Linux CI build (`Museion Binarize_0.1.0-rc.1_amd64.deb`, `...-0.1.0-rc.1-1.x86_64.rpm`, `..._0.1.0-rc.1_amd64.AppImage`). |

`scripts/distribution/check_version_consistency.py`'s SemVer regex
already accepted prerelease identifiers before this milestone (no code
change needed there); `scripts/distribution/test_distribution.py`'s
`test_dist_config_wix_version_is_msi_compatible` guards the one real
fix above from regressing.

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

**Fixed this milestone**: the Windows and Linux jobs previously never
copied their actual desktop installer into `dist-out/` at all, and never
generated a `release-manifest.json` entry for one — only the CLI archive
and its own checksum ever left those two jobs. Only discovered while
building the release-wide aggregation tooling below, since nothing had
tried to aggregate Windows/Linux desktop artifacts before. Fixed via
`scripts/distribution/collect_desktop_artifact.py` (shared by both
jobs), with the same "exactly one match or fail loudly" discipline the
macOS job's `.dmg` collector already used.

### Windows artifact selection

`tauri.conf.json`'s `bundle.targets: "all"` builds **both** an MSI and
an NSIS installer on Windows. Only the **MSI** is collected into
`dist-out`/published — chosen for its native Windows upgrade/uninstall
tracking (`ProductVersion`, "Programs & Features" integration). NSIS
still builds and is validated in CI; it is a deliberate, documented
exclusion from the published asset set, not an oversight.

### Linux artifact selection

Linux similarly builds `.deb`, `.rpm`, and `.AppImage` from the same
`"all"` targets setting. **`.deb` and `.AppImage`** are collected and
published — the most common Debian/Ubuntu package format plus a
distro-independent format that needs no package manager at all. `.rpm`
still builds and is validated in CI; also a deliberate, documented
exclusion.

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
  "signing_state": "unsigned | ad_hoc | signed | pending_credentials",
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

**Exercised for real, repeatedly, via `workflow_dispatch`** across
Milestones 7A, 7B1, and the `0.1.0-rc.1` release-prep work — including
successful macOS arm64, Windows x64, and Linux x86_64 runs building and
packaging the real desktop app and CLI archive on GitHub-hosted runners.
See `docs/desktop-testing.md`'s verification-state table for exactly
which platform/step combinations have real run evidence versus which
remain human-runtime-unverified — "the workflow ran successfully" and
"a human clicked through the resulting app" are still two different
claims, kept separate throughout that document.

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

### Ad-hoc signing fallback (macOS, always, when the step above didn't run)

"Unsigned" does not mean "the bundler leaves the `.app` alone." Rust's
linker ad-hoc-signs each Apple Silicon Mach-O binary at build time (arm64
requires every binary to carry some signature), but without
`bundle.macOS.signingIdentity` in `tauri.conf.json`, `tauri-bundler`
never resigns the *whole app bundle*, which leaves
`Contents/_CodeSignature/CodeResources` missing. macOS Gatekeeper
reports that specific mismatch as "**is damaged and can't be opened**,"
not the expected "unidentified developer" prompt a plain, properly
ad-hoc-signed unsigned app would get — this is a real defect a real
user hit, not a hypothetical. See
[`desktop-testing.md`](desktop-testing.md), "macOS arm64: 'is damaged'
bug found by human runtime testing," for how it was found, diagnosed,
and fixed.

The build workflow now always signs the whole `.app` bundle ad-hoc
(`codesign --force --deep --sign -`, via
`scripts/distribution/sign_macos_app.py`) whenever the real
Developer-ID step above didn't run, then packages the `.dmg` from that
already-signed `.app` directly with `hdiutil`
(`scripts/distribution/package_macos_dmg.py`) rather than through
Tauri's own dmg bundler — which was confirmed to recompile and
re-bundle the `.app` from scratch as part of producing a `.dmg`,
silently discarding any signature applied beforehand. Ad-hoc signing
fixes the damaged-bundle defect and makes `codesign --verify --deep
--strict` pass, but it does not satisfy `spctl -a` or notarization —
those still require the real Developer ID credentials this project does
not have.

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
