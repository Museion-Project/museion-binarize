# Mac App Store readiness audit (M7B scope only)

**This is an audit, not an implementation.** M7A implements the
Developer-ID (GitHub) distribution path. M7B — paid Mac App Store
distribution — is a future, separate milestone. Nothing here adds
StoreKit, App Sandbox, submission tooling, or store metadata; this
document only classifies what M7B would need to address, based on the
application as it exists after M7A.

Classification legend:

- **Ready** — already compatible with Mac App Store requirements as-is.
- **Likely compatible** — expected to work, not independently verified
  against a real App Store submission/review.
- **Needs M7B work** — a concrete change would be required.
- **Unknown / requires real Store validation** — cannot be classified
  without an actual submission or Apple documentation review beyond
  this audit's scope.

## Bundle identifier

**Ready.** `org.museionproject.binarize` (see `tauri.conf.json`) is a
reverse-DNS identifier matching the project's own domain/organization,
not a placeholder. Unchanged by M7A (see `docs/releasing.md`, "Product
identity was not silently changed").

## Native file picker (open)

**Ready.** The app already uses `tauri-plugin-dialog`'s native open
dialog exclusively (see `docs/desktop.md`) — no custom file browser, no
raw filesystem path entry from the frontend.

## Arbitrary user-selected input

**Likely compatible.** The app opens whatever PDF the user selects
through the native dialog. Under App Sandbox, a user-selected file
grants a security-scoped bookmark valid for that session; the current
architecture (one document open per window, opened once via
`open_document`, never re-opened from a stashed raw path outside that
flow) does not obviously conflict with that model, but this has not
been exercised under an actual sandboxed entitlement.

## Arbitrary user-selected output

**Needs M7B work.** `process`'s output path currently comes from the
native save dialog (also sandbox-compatible in principle), but the
backend's `PdfProcessingOptions`/output-writing path
(`crates/museion-binarize-core/src/pipeline.rs`) writes via a plain
`std::fs`/`tempfile` temp-file-then-rename sequence in the destination
directory. Under App Sandbox this requires the security-scoped bookmark
from the save-panel selection to still be active for that temp-file
write; this has not been tested under a real sandbox entitlement and
would need verification, not just code review.

## Temp-file handling

**Needs M7B work (verification).** The core pipeline creates its
temporary output file via `tempfile` in the *destination's own
directory* (see `docs/pdf-output.md`) so the final rename is atomic on
the same filesystem. Under App Sandbox, whether that directory remains
writable for the temp file depends on the same security-scoped bookmark
as the final output — the same open question as above, not a separate
one.

## Report export (`--report` equivalent from the desktop app)

**Not applicable in M7A's desktop app.** The current desktop UI does not
expose a `--report`-equivalent file-save action (JSON reports are a CLI-
only feature — see `docs/desktop.md`). Nothing to audit here until that
changes.

## Security-scoped file access

**Unknown / requires real Store validation.** This audit did not add or
test `com.apple.security.files.user-selected.read-write` or related
entitlements — App Sandbox is not enabled at all in the M7A GitHub
build (see "Do not sandbox the GitHub build" below). Real validation
requires building an actual sandboxed, entitled bundle and testing the
open/convert/save flow under it, which is App Store submission
preparation work, not an M7A concern.

## App Sandbox implications

**Needs M7B work.** The M7A Developer-ID build is intentionally *not*
sandboxed — Developer-ID and Mac App Store distribution have different
requirements, and forcing App Sandbox onto the GitHub build would add
complexity with no M7A benefit (per the milestone specification). M7B
would need to: add the sandbox entitlement, audit every filesystem
access path (input open, output save, PDFium load) under it, and
re-verify the full desktop workflow with sandboxing enabled.

## Bundled PDFium under App Sandbox

**Likely compatible, unverified.** PDFium is loaded via `dlopen`-style
dynamic loading from the app's own `Contents/Resources/` (see
`docs/pdfium-bundling.md`) — a location inside the app bundle itself,
which App Sandbox permits without a special entitlement (unlike loading
a library from outside the bundle). This was not tested under an actual
sandboxed build.

## Dynamic loading of PDFium generally

**Likely compatible.** The resolver
(`museion_binarize_core::pdfium_backend::resolve_library`) never loads
a library the app itself didn't ship or the user didn't explicitly name
via `MUSEION_PDFIUM_LIBRARY` (development-only) — no untrusted dynamic
code loading. A Mac App Store build would presumably want to disable
(or simply never expose) the `MUSEION_PDFIUM_LIBRARY` override and
`--allow-system-pdfium`-equivalent behavior, since a Store build should
only ever load its own bundled, notarized copy — this is a policy
decision for M7B, not a technical blocker.

## Entitlements

**Needs M7B work.** No entitlements file exists yet beyond what Tauri's
default Developer-ID build implies. M7B needs: App Sandbox, user-
selected file read/write, and a review of whether any other entitlement
(e.g. outgoing network — the app has none at runtime, see
`docs/limitations.md`) is required or should be explicitly absent.

## Signing differences: Developer ID vs. Mac App Store

**Needs M7B work.** M7A's signing integration
(`docs/releasing.md`, "Signing and notarization") targets a **Developer
ID Application** certificate and the notarization service — the correct
flow for a GitHub-distributed build. Mac App Store distribution instead
requires a **Mac App Distribution** / **Mac Installer Distribution**
certificate pair, App Store Connect provisioning, and no notarization
step (the Store's own review replaces it). These are different
credentials and a different build configuration, not an extension of
the Developer-ID flow — M7B would need its own signing setup, not a
toggle on M7A's.

## Summary

| Item | Classification |
|---|---|
| Bundle identifier | Ready |
| Native file picker | Ready |
| User-selected input | Likely compatible |
| User-selected output | Needs M7B work |
| Temp-file handling | Needs M7B work (verification) |
| Report export | Not applicable (feature doesn't exist in desktop app) |
| Security-scoped file access | Unknown / requires real Store validation |
| App Sandbox | Needs M7B work |
| Bundled PDFium under sandbox | Likely compatible, unverified |
| Dynamic PDFium loading policy | Likely compatible (policy decision for M7B) |
| Entitlements | Needs M7B work |
| Signing (Developer ID vs. MAS) | Needs M7B work |

No StoreKit, App Store Connect metadata, submission tooling, or paid-app
agreement work exists anywhere in this repository as of M7A. This audit
does not start M7B.
