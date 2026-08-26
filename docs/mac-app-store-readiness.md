# Mac App Store readiness (M7B1)

M7B1 implements a Mac App Store-specific build path — configuration,
entitlements, PDFium bundling, and signing/packaging scripts — separate
from M7A's GitHub Developer-ID distribution path. **It does not submit
anything to Apple, publish a release, or add StoreKit/pricing/paywall
code.** This document classifies exactly what is done, what only the
owner can do, and what remains unverified — using the same
verification-state discipline as `docs/releasing.md`:

```
"configured" != "structurally built" != "signed" != "sandbox-verified" != "submitted"
```

## Status

**Technical sandbox readiness: COMPLETE.** App Sandbox enforcement, the
entitlement set, the sandboxed output-save architecture, and PDFium
bundling have all been verified against a real, kernel-enforced
sandboxed build, including a full interactive human acceptance pass —
see "Human acceptance: PASSED" below. There is no open BLOCKING or HIGH
technical finding for the sandbox itself as of this milestone.

This is a distinct claim from, and must never be conflated with:

- **Production Apple signing**: pending owner credentials (an Apple
  Distribution certificate — not yet obtained).
- **Provisioning**: pending owner credentials (App ID registration and a
  Mac App Store provisioning profile — not yet obtained).
- **App Store Connect upload / submission / review**: not started —
  no tooling for it has been exercised, and none of this milestone's
  builds have ever left this machine.

"Technical sandbox readiness" means: the code and configuration are
correct and demonstrated to work under real sandbox enforcement. It
does not mean an App Store submission exists or has been attempted.

## Commercial model (documentation only — no code enforces or changes this)

- GitHub source remains open source (MIT OR Apache-2.0), unchanged.
- GitHub builds remain fully featured — nothing is held back to create
  a Store-only tier.
- The Mac App Store edition is a **paid, one-time** convenience
  distribution — packaging and platform-integration work, not a
  separate feature set. No exact price appears here or anywhere in the
  repository; pricing is an App Store Connect configuration step for
  the owner, not a code or documentation concern.
- **No subscription. No feature tiering. No DRM, license keys, or
  activation servers** — nothing in this milestone adds any of these,
  and nothing planned does either.
- No in-app purchase exists or is implemented; one would require
  separate, explicit future approval.

## READY / IMPLEMENTED (repository-side technical work completed this milestone)

- **Permanent product identifier finalized**: `me.mpdf.processor`
  (previously `org.museionproject.binarize`), owner-approved, changed
  before any Apple App ID or App Store Connect identity was created.
  Declared exactly once, in the base `tauri.conf.json`; every overlay
  (GitHub dist, MAS) inherits it rather than redeclaring it, enforced by
  `BundleIdentifierConsistencyTests` in `test_distribution.py`, including
  a repo-wide `git grep` assertion that the old identifier does not
  reappear in any active file. The GitHub-distributed macOS app and the
  future Mac App Store version share this one identity, per the intended
  `me.mpdf.<product>` namespace for future PDF tools.
- **Sandboxed output-save architecture — human-verified**: `OutputWriteStrategy`
  in `crates/mpdf-core/src/pipeline.rs`, selected at compile
  time by the `mas-sandbox` Cargo feature (MAS build only). Real,
  credential-free App Sandbox enforcement was demonstrated locally
  (ad-hoc signing plus the sandbox entitlement — no Apple Developer
  Program membership required for this), the write path was redesigned
  to work within what a sandboxed save panel's grant actually covers
  (the resulting guarantee change — no longer crash-atomic on the final
  write, everything else unchanged — explicitly documented, not glossed
  over), and the owner then ran the full interactive open/save/convert/
  cancel/overwrite/relaunch checklist against a real sandboxed build,
  with every step passing and the sandbox log showing zero file-access
  denials. See "Sandboxed output-save architecture" below and
  `docs/pdf-output.md`.
- **`network.client` entitlement found necessary and added, with A/B
  evidence**: the original entitlement set (`app-sandbox` +
  `files.user-selected.read-write` only) launched but rendered a
  completely blank window — `WKWebView` runs its renderer in a separate,
  out-of-process `WebContent` service that does not start under App
  Sandbox without this entitlement, even for purely local content. Found
  by the owner during real human acceptance testing, then isolated by an
  A/B test on one identical binary varying only the signed entitlements.
  A dedicated network-capability audit (see "Entitlements audit" below)
  confirms this is architectural, not a sign of any actual networking in
  the application.
- **MAS-specific Tauri config overlay**: `apps/desktop/src-tauri/tauri.mas.conf.json`,
  parallel to M7A's `tauri.dist.conf.json`, merged in only via `--config
  src-tauri/tauri.mas.conf.json`. It never redeclares `identifier` or
  `version` (both come from the base `tauri.conf.json` alone, so they
  cannot drift between the GitHub and MAS builds of the same app), and
  carries no hard-coded signing identity.
- **Entitlements**: `apps/desktop/src-tauri/entitlements.mas.plist.template`
  (committed) + `scripts/distribution/render_mas_entitlements.py`
  (renders the real, gitignored `entitlements.mas.plist` from the
  template, substituting the Team ID from `APPLE_TEAM_ID` and the
  bundle identifier read directly from `tauri.conf.json`). Exactly two
  capability entitlements are declared:
  - `com.apple.security.app-sandbox`
  - `com.apple.security.files.user-selected.read-write`

  plus the identity-binding keys Apple's own Mac App Store guide
  requires (`com.apple.application-identifier`,
  `com.apple.developer.team-identifier`). See "Entitlements audit"
  below for why nothing else is present.
- **PDFium bundling**: the MAS config reuses M7A's exact pinned,
  checksum-verified provenance chain (`distribution/pdfium/manifest.toml`,
  `scripts/distribution/fetch_pdfium.py`,
  `scripts/distribution/stage_desktop_pdfium.py`) — no new fetch/trust
  path was added. No runtime network fetch, no PATH/system-library
  discovery, no developer-machine path in the production resolution
  order (see "PDFium strategy" below).
- **Build/validation script**: `scripts/distribution/package_mas.py`
  — structural build (works with no Apple credentials at all), and a
  separate, explicitly-requested (`--sign`, `--package-pkg`) real-signing
  path that requires `APPLE_SIGNING_IDENTITY`/`APPLE_INSTALLER_SIGNING_IDENTITY`
  and never silently falls back to ad-hoc signing (see "Signing
  architecture" below).
- **Regression tests** (`scripts/distribution/test_distribution.py`,
  `MasConfigTests`/`RenderMasEntitlementsTests`/`PackageMasEntitlementValidationTests`/
  `PackageMasNeverAdHocSignsGuardTests`): the base and GitHub-dist
  configs are asserted to never declare `bundle.macOS.entitlements`
  (App Sandbox can never leak into the unsandboxed GitHub build); the
  MAS config is asserted to be a pure overlay with no unexpected keys
  and no hard-coded identity; the entitlements template is asserted to
  contain exactly the intended capability keys and none of the
  evaluated-and-rejected broad ones; `package_mas.py --sign` is
  asserted to fail closed (before any build step runs) when
  `APPLE_SIGNING_IDENTITY` is unset, and to never import or fall back to
  `sign_macos_app.py`'s ad-hoc path.
- **M0–M7A behavior unchanged for every existing caller.** This is no
  longer a literal zero-diff claim against `main` at
  `afff0de46e47be7ec18b699606b3679003e0e098` — `pipeline.rs` gained
  `OutputWriteStrategy` (see above) — but the change is additive and
  behavior-preserving: `OutputWriteStrategy::default()` is exactly the
  old, only, unconditional behavior, every existing caller (CLI, GitHub
  desktop build, every pre-existing test) explicitly uses it, and
  `cargo test --workspace` — the same suite M7A's own tests all still
  belong to — passes unchanged, with new tests added alongside rather
  than any existing one modified. The identifier change is the other
  non-additive edit, and is an explicit owner-approved decision (see
  above), not an accidental drift.

## Entitlements audit

Evaluated and **not** included, because nothing in the current
application demonstrates a need for them (see the M7B1 preflight audit
below):

| Entitlement category | Evidence found |
|---|---|
| Network **server** (incoming connections) | Nothing in this app listens on a socket. Explicitly kept in `package_mas.py`'s forbidden list so a future addition fails the build. (Network *client* is a different matter — see the corrected finding below.) |
| Automation / Apple Events | No AppleScript/Scripting Bridge/`NSAppleEventDescriptor` usage. The one Finder interaction (`revealItemInDir`, the "reveal output" button) goes through `NSWorkspace`'s file-viewer API via the `tauri-plugin-opener`/`open` crate, not Apple Events. |
| Camera / microphone / location / contacts | No usage anywhere. |
| Downloads-folder entitlement | The app never targets `~/Downloads` specially — output destination is always the exact user-selected save path. |
| Temporary exceptions | None of the narrow, App-Review-discouraged temporary-exception keys are used; the ordinary `files.user-selected.read-write` entitlement covers this app's actual file-access pattern (see below). |

**Included, with evidence**:

| Entitlement | Why |
|---|---|
| `com.apple.security.app-sandbox` | Required unconditionally for Mac App Store submission. |
| `com.apple.security.files.user-selected.read-write` | The app opens PDFs and saves output exclusively through `tauri-plugin-dialog`'s native open/save panels (`apps/desktop/src/lib/tauri.ts`'s `pickPdfToOpen`/`pickOutputDestination`) — no custom file browser, no raw path entry, no `tauri-plugin-fs` (not a dependency at all). This is exactly the entitlement Apple's own documentation pairs with that access pattern. |
| `com.apple.security.network.client` | **Required by `WKWebView`, not by any network code in this app.** See the corrected finding immediately below. |

### Corrected finding: `network.client` is mandatory for WKWebView under App Sandbox

An earlier revision of this document listed "network client/server" as
*excluded*, reasoning that the app has zero network code (true — and
still true: it makes no outbound connection at runtime, see
`docs/limitations.md`). **That reasoning was wrong**, because it only
considered the app's own code and not `WKWebView`'s process
architecture. The error was caught by the owner actually running the
sandboxed acceptance build: the app launched, the window appeared with
its title bar, and the content area stayed completely blank.

Diagnosed by A/B test on **one identical binary**, varying only the
entitlements passed to `codesign`, using new-`WebContent`-process spawn
as the objective signal (`WKWebView` cannot render without one):

| Entitlements | `WebContent` process spawned? |
|---|---|
| *(none — unsandboxed)* | **yes** → renders |
| `app-sandbox` + `files.user-selected.read-write` | **no** → blank window |
| `app-sandbox` + `files.user-selected.read-write` + `cs.allow-jit` | **no** → still blank |
| `app-sandbox` + `files.user-selected.read-write` + **`network.client`** | **yes** → renders (36 MB RSS, i.e. real loaded content, vs. ~2–5 MB for an idle/failed one) |

This is a well-known, Apple-acknowledged WebKit requirement, not a
quirk of this project: `WKWebView` renders out-of-process, and under App
Sandbox that `WebContent` XPC service fails to start without the host
app holding `network.client` — **even when every asset is local to the
app bundle**. Apple has an open Feedback asking for the requirement to
be lifted (FB6993802), and Tauri's own docs repository tracks it as
mandatory for sandboxed macOS apps
([tauri-docs#3171](https://github.com/tauri-apps/tauri-docs/issues/3171)).

**App Review note**: this app declares `network.client` while genuinely
making no outbound connections. That is expected and standard for any
sandboxed `WKWebView`/Tauri/Electron-style app, but if App Review ever
queries it, the honest answer is exactly the above — the entitlement is
a WebKit process-architecture requirement, not a capability this app
exercises. The App Privacy declaration should continue to state that no
data is collected or transmitted, because none is.

**`com.apple.security.cs.allow-jit` was tested in the same A/B run and
is *not* required** — it did not fix the blank window, and the app works
without it. It stays excluded, now on empirical grounds rather than
reasoning alone.

`package_mas.py` now treats `network.client` as **required** (not merely
permitted): omitting it produces an app that launches successfully but
renders nothing, which is precisely the kind of silent break that should
fail a build rather than reach a user. `network.server` remains
forbidden.

### Network-capability audit: `network.client` is architectural, not applied

`network.client` is required by `WKWebView`'s process architecture (see
above). It must not be read as "this app does networking." A dedicated,
repository-wide audit was performed to keep that distinction concrete
rather than asserted:

| Checked for | Result |
|---|---|
| Remote HTTP/HTTPS frontend assets (`<script src="http...">`, remote fonts/images, etc.) | None — `grep` for `http://`/`https://` in `apps/desktop/src` found nothing |
| Remote navigation (`window.open`, `location.href =` to an external URL) | None |
| `fetch`/`XMLHttpRequest`/`axios`/`WebSocket`/`EventSource` anywhere in the frontend | None |
| Telemetry / crash-report upload (Sentry, analytics SDKs, custom crash uploaders) | None |
| Updater / runtime dependency download (`tauri-plugin-updater`, self-update logic) | None — not a dependency anywhere in `package.json` or any `Cargo.toml` |
| Application-owned runtime network calls in Rust (`reqwest`, `hyper`, `ureq`, raw `TcpStream`/`UdpSocket`) | None — not a dependency anywhere in the workspace |

This reconfirms, rather than merely repeats, M7A's own "no runtime
network access" finding (`docs/limitations.md`) and the earlier
Entitlements audit above: M PDF Processor itself has no intended
runtime networking, telemetry, updater, or remote dependency fetch of
any kind. `network.client` exists solely to let the sandboxed
`WKWebView` process render the app's own bundled, local frontend — a
distinction worth stating plainly for App Review or anyone else auditing
this entitlement later.

## File-access strategy

Every file path in this application flows through exactly one of two
native panels (`apps/desktop/src/lib/tauri.ts`):

- **Open**: `pickPdfToOpen()` → `@tauri-apps/plugin-dialog`'s `open()` →
  the path is passed to the `open_document` backend command, which
  reads it via `PdfDocumentSession::open` (plain `std::fs`/PDFium
  `dlopen`, no `tauri-plugin-fs` scoping layer).
- **Save**: `pickOutputDestination()` → `save()` → the path is passed to
  `start_processing`'s `outputPath`, which the core pipeline writes to.

Under App Sandbox, a path returned by the native open/save panel
("Powerbox") carries an automatically-active security-scoped extension
for the current process for the life of that access — no
`startAccessingSecurityScopedResource` call is needed for *immediate*,
same-session use (only for access that must survive an app relaunch).
`apps/desktop/src-tauri/src/state.rs` holds the open document's path
only in in-memory session state; nothing persists a path to disk, a
config file, or a "recent files" list across launches. **No security-scoped
bookmark is implemented, because nothing in this application needs
cross-launch file-access persistence** — adding one would be exactly
the "convenience" bookmark the M7B1 brief says not to add without a
real need.

### Sandboxed output-save architecture

**Status: COMPLETE.** Code implemented and unit-tested; real sandbox
enforcement demonstrated without any Apple credentials; the full
interactive open/save click-through has now been run by the owner
against a real, kernel-enforced sandboxed build, and passed — see
"Human acceptance: PASSED" below. This closes what was previously the
one open BLOCKING finding for M7B1.

#### The original risk

`crates/mpdf-core/src/pipeline.rs`'s write path used to
unconditionally create a **new temp file in the same directory** as the
destination (`.mpdf-<random>.pdf.partial`), then
`rename(2)` it onto the final path — for atomicity (see
`docs/pdf-output.md`). Correct and unaffected for M0–M7A (the CLI and
the GitHub-distributed desktop app never run under App Sandbox at all),
but Apple's own documented behavior (Apple Developer Forums, on
`NSSavePanel`-granted sandbox access) says: *"When the save panel gives
you back a URL it extends your sandbox so that you can access **exactly
that URL**. You are not allowed to change the URL in any way..."* — the
grant is scoped to the one path the user selected, not sibling paths in
the same directory. Creating the `.partial` temp file **next to** the
selected output path is therefore not obviously covered, under a real
sandboxed build.

#### First, whether this could be tested locally at all — it can

Before deciding whether to change any code, this milestone tested
whether real, kernel-enforced App Sandbox could be exercised locally
*without* Apple Developer credentials, rather than assuming Apple
credentials were required. They are not, for this purpose:

```
codesign --force --deep --sign - \
  --entitlements entitlements.local-sandbox-test.plist \
  "M PDF Processor.app"
```

with `entitlements.local-sandbox-test.plist` containing only
`com.apple.security.app-sandbox` and
`com.apple.security.files.user-selected.read-write` — **no**
`com.apple.application-identifier`/`com.apple.developer.team-identifier`
(no Team ID was fabricated to make this work, per the M7B1 brief; those
identity-binding keys were simply omitted, and the ad-hoc signature
still succeeded and still carried the sandbox entitlement). Launching
this ad-hoc-signed build produced, immediately and reproducibly:

```
~/Library/Containers/me.mpdf.processor/Data/{Documents,Desktop,Downloads,Library,...}
~/Library/Containers/me.mpdf.processor/.com.apple.containermanagerd.metadata.plist
```

— a real sandbox container, created by `containermanagerd`, which only
happens for a process the kernel is actually sandboxing. This is
conclusive, machine-checkable evidence that `com.apple.security.app-sandbox`
enforcement itself does not require a Developer ID, an Apple Distribution
certificate, a Team ID, or an App ID — only a code signature (ad-hoc is
sufficient) that carries the entitlement. Identity-binding entitlements
(`application-identifier`/`developer.team-identifier`) exist for App
Store *provisioning-profile validation*, not for the kernel's sandbox
enforcement decision.

This is a materially better position than the M7B1 draft assumed: local
sandbox *enforcement* testing needs no owner credentials at all. What
still needs a human is described below.

#### The fix actually made

`OutputWriteStrategy` (`pipeline.rs`) now has two variants:

- `AtomicSameDirectoryRename` — M0–M7A's exact existing behavior,
  unchanged, the default for every existing caller (CLI, GitHub desktop
  build).
- `DirectWriteToDestination` — Mac App Store build only, selected at
  **compile time** via the `mas-sandbox` Cargo feature
  (`apps/desktop/src-tauri/Cargo.toml`), set only by
  `scripts/distribution/package_mas.py`'s `tauri build --features
  mas-sandbox`. Validates in the system/container temp directory (always
  writable, sandboxed or not, no entitlement needed), then writes the
  already-validated bytes **straight to the exact granted destination
  path** — never touching a second path near it.

Properties, analyzed against the current pipeline before writing any
code (not assumed):

| Property | `AtomicSameDirectoryRename` (unchanged) | `DirectWriteToDestination` (MAS) |
|---|---|---|
| New destination | Temp-in-same-dir, validate, rename | Temp-in-container, validate, direct write |
| Existing destination / overwrite | Atomic `rename(2)` replace (Unix); unlink-then-rename (Windows, documented gap) | Direct `File::create` truncate-and-write at the exact granted path — the textbook `NSSavePanel` overwrite case |
| Cancellation | Checked before `persist`; temp dropped, destination untouched | Identical — checked before `persist` in both strategies; the destination is never reached on cancellation either way |
| Validation failure | Destination untouched (validated before any destination write) | Identical — validation happens in the container temp file, before the destination is ever touched |
| Cross-volume output | N/A (temp always shares the destination's volume, by construction) | No rename at all, so no cross-volume `EXDEV` failure mode exists for this strategy either |
| Crash *during* the final commit | **Atomic** — destination is always either the complete old file or the complete new file, never partial (`rename(2)`) | **Not atomic** — a crash mid-write can leave the destination holding a partial file. This is a real, deliberate, documented reduction in guarantee, not papered over. |
| Cleanup | `NamedTempFile`'s `Drop` deletes the temp file on every path | Identical mechanism, temp file just lives in the container instead |
| Large PDFs / memory | Whole output already held in memory (`bytes: Vec<u8>`) before any write, for both strategies — unchanged | One extra full write pass (temp copy, then destination) instead of a cheap rename; a minor I/O cost, not a memory-behavior change |

The crash-mid-write gap was evaluated against the alternative
(`NSFileCoordinator`/`FileManager.replaceItemAt`, Apple's own
sandbox-safe-save API) and deliberately not implemented this pass: it
would require new Objective-C/Foundation FFI (or a native helper
process — itself disfavored under sandbox and for Store review) that
cannot be verified end-to-end without the same live interactive test
this pass could not complete either (see below), so it would add real,
unverified surface area rather than a provably-working fix. If the
crash-window gap proves unacceptable once real testing is possible,
that FFI integration is the identified upgrade path, not a hypothetical
one.

**Regression tests** (`crates/mpdf-core/src/pipeline.rs`):
`direct_write_strategy_writes_bytes_straight_to_the_destination`,
`direct_write_strategy_overwrite_replaces_the_destination_contents`,
`direct_write_strategy_failure_before_persistence_leaves_the_old_destination_intact`
— mirroring the exact three properties the pre-existing
`AtomicSameDirectoryRename` tests already proved, for the new strategy.
`apps/desktop/src-tauri/src/worker.rs`'s
`output_write_strategy_tests::ordinary_build_keeps_the_atomic_same_directory_rename_default`
proves the GitHub build's default is unaffected; `cargo check -p
mpdf-desktop --features mas-sandbox` (run this session)
independently proves the other branch compiles and is reachable.

#### How the interactive test was actually run

This session's own tooling cannot drive a real `NSOpenPanel`/`NSSavePanel`
(`osascript`'s System Events UI scripting needs a one-time Accessibility
permission grant only a human can approve — a precisely-identified
tooling boundary, not a credentials one). Rather than leave that
unresolved, the exact credential-free local build was handed to the
owner to run at the keyboard:

```bash
APPLE_TEAM_ID=LOCALTEST01 python3 scripts/distribution/package_mas.py \
  --target-triple aarch64-apple-darwin --version 0.1.0 \
  --out-dir /tmp/mas-local-test

# entitlements.local-sandbox-test.plist: app-sandbox +
# files.user-selected.read-write + network.client only — no Team
# ID/App ID, nothing faked.
codesign --force --deep --sign - \
  --entitlements entitlements.local-sandbox-test.plist \
  "target/aarch64-apple-darwin/release/bundle/macos/M PDF Processor.app"

open "target/aarch64-apple-darwin/release/bundle/macos/M PDF Processor.app"
```

This exercises real kernel sandbox enforcement and the real
`DirectWriteToDestination` code path. The only things this specific
local test cannot exercise are Developer ID identity/provisioning
validation and notarization/Store review themselves, neither of which
affects whether the open/save/convert flow works under sandbox.

#### Human acceptance: PASSED

The owner ran the full checklist against this build and reported every
step passing:

| Step | Result |
|---|---|
| Sandboxed launch via Finder | PASS |
| UI/WebView rendering | PASS (after adding `network.client` — see "Entitlements audit") |
| Open external PDF via native panel | PASS |
| Preview | PASS |
| Estimate | PASS |
| Choose external output destination via native panel | PASS |
| Convert | PASS |
| Output PDF validity | PASS |
| Cancel mid-conversion | PASS |
| Reconvert after cancellation | PASS |
| Overwrite an existing destination | PASS |
| Quit / relaunch | PASS |
| Open and convert a second PDF after relaunch | PASS |
| Bundled PDFium, `MPDF_PDFIUM_LIBRARY` unset | PASS |

This directly exercises and confirms `DirectWriteToDestination`: new
output, overwrite of an existing destination, and cancel-then-reconvert
all completed correctly under real sandbox enforcement — the exact
scenarios analyzed in the property table above, no longer only analyzed
but observed.

#### Sandbox log review

The owner captured the unified log (`/usr/bin/log show` — note the
plain `log` command is shadowed by an unrelated shell function on the
test machine and silently returns nothing; the absolute path is
required) across the full session above, spanning three launches. It
was reviewed and the raw log was not committed (it is host-specific and
irrelevant once summarized); the relevant findings:

- **Zero file-write sandbox denials.** No `deny(1) file-write-*` naming
  this application, across opening an external PDF, converting,
  cancelling, reconverting, and overwriting an existing destination.
- **File-read records relevant to the app bundle are `allow`, not
  `deny`** (e.g. other system processes reading the bundle's own
  executable/resources — expected, harmless).
- **The only denials attributable to this app** are two mach-service
  lookups at startup, on every launch: `com.apple.Safari.SafeBrowsing.Service`
  and `com.apple.visualintelligence.visual-action-prediction`. Both are
  routine `WKWebView`/system-framework initialization probes (Safari's
  safe-browsing check, macOS's Visual Intelligence service) — this app
  navigates to no URLs and has no visual-intelligence integration, so
  neither denial is exercising an actual feature, and neither impaired
  any tested workflow.
- A handful of unrelated system processes were separately denied access
  *to query this app's process info* (`process-info-pidinfo`) — that is
  the sandbox correctly protecting this process from another one, not
  this app being denied anything.
- No crash reports for this app were found for the session.

**Deliberately not "fixed"**: none of the above denials are addressed
with a temporary-exception or private mach-service entitlement. They are
framework/system noise that did not impair the tested workflow, and
Apple's own App Review guidance disfavors exactly that class of broad or
narrow-but-unusual entitlement without a demonstrated functional need —
there is none here.

## PDFium strategy

Identical trust model to M7A, reused rather than re-implemented:
`mpdf_core::pdfium_backend::resolve_library`'s precedence
(explicit path → `MPDF_PDFIUM_LIBRARY` env var → packaged resource →
executable-adjacent → dev-tree opt-in → system if allowed) is unchanged
— zero diff to `pdfium_backend.rs`. The desktop app's own
`worker::pdfium_config` (also unchanged) resolves the bundled resource
path once at startup via Tauri's `BaseDirectory::Resource` API and uses
it explicitly, with the `MPDF_PDFIUM_LIBRARY` override still winning
when set (unchanged developer/support behavior, `worker.rs`).

No MAS-specific resolver code was written, because none is needed: the
`tauri.mas.conf.json` overlay stages the same `resources/pdfium/*` glob
M7A's `tauri.dist.conf.json` already uses, so a MAS build's bundled
PDFium ends up at the identical `Contents/Resources/libpdfium.dylib`
path the existing resolution code already looks for. Under App Sandbox,
loading a library from inside the app's own signed bundle needs no
special entitlement (unlike loading one from outside the bundle) — this
was true in the M7A audit and remains true here; it has not been
independently re-verified against a real sandboxed process this
session.

**Developer/test override preserved, not weakened**: `MPDF_PDFIUM_LIBRARY`
still exists in code, for local development builds — this is
unconditional and unchanged. In a *real, entitled, sandboxed* MAS
production build, App Sandbox itself neutralizes any attempt to point
that variable at a path outside the container (the OS denies the read),
so no additional code-level restriction was added specifically for
MAS; the sandbox already enforces the boundary the M7B1 brief asks for.

## Signing/provisioning architecture

Distinguishing implementation from completion, as required:

| Step | Repository-side integration | Owner-side completion |
|---|---|---|
| App code signing | `package_mas.py --sign` requires `APPLE_SIGNING_IDENTITY` in the environment (an **Apple Distribution** certificate identity — the current unified Apple terminology that replaced the older separate "3rd Party Mac Developer Application" cert name; confirmed against current third-party Tauri/Apple documentation, not assumed from memory) already imported into the local keychain. Tauri's own bundler picks this up and signs the `.app` with the rendered entitlements as part of its `tauri build` step — not a separate re-signing pass afterward, avoiding the exact "bundler re-bundles and discards a prior signature" failure mode M7A hit (`docs/desktop-testing.md`). | Owner must enroll in the Apple Developer Program, generate/download an Apple Distribution certificate, and have it in the build machine's keychain. |
| Installer (`.pkg`) signing | `package_mas.py --package-pkg` requires `APPLE_INSTALLER_SIGNING_IDENTITY` (a **Mac Installer Distribution** certificate — a separate cert type from Apple Distribution, specifically for `productbuild`/`pkgbuild`) and calls `xcrun productbuild --sign ...`, then verifies with `pkgutil --check-signature`. | Owner must generate a Mac Installer Distribution certificate separately from the app-signing one. |
| Provisioning profile | Not embedded by any script in this milestone — `tauri.mas.conf.json` deliberately does not reference `bundle.macOS.files.embedded.provisionprofile` for a file that does not exist in this repository. | Owner must register the App ID (with App Sandbox capability enabled) in the Apple Developer portal, create a Mac App Store provisioning profile, download it to `apps/desktop/src-tauri/embedded.provisionprofile` (gitignored — see `.gitignore`), and add the `files` mapping to `tauri.mas.conf.json` (or a further local-only overlay) before attempting a real submission build. |
| Team ID / App ID / bundle identifier consistency | `render_mas_entitlements.py` reads the bundle identifier from `tauri.conf.json` (single source of truth, cannot drift) and reads the Team ID from `APPLE_TEAM_ID` — both are cross-checked into the rendered entitlements' `com.apple.application-identifier` (`$TEAM_ID.$IDENTIFIER`). | Owner must register the App ID `me.mpdf.processor` under their own Team ID in the Apple Developer portal, with App Sandbox capability enabled, before either matters at build time. |

**No certificate, private key, Team ID, Apple ID, password, or
provisioning-profile UUID is committed anywhere in this repository.**
Every credential is read from an environment variable or a gitignored
local file, by name only.

**No artifact produced by this milestone is signed with a real Apple
Distribution or Mac Installer Distribution identity** — no such
credentials were available to this implementation. `package_mas.py`'s
default (credential-free) mode was run locally and produces a real,
inspectable, unsigned `.app` with the MAS config/entitlements/PDFium
bundling structurally correct; this is honestly reported as a
structural build, not a signed one.

## Build artifact type

`.app` (via `tauri build --bundles app`) is the base artifact, matching
what Tauri's own current Mac App Store distribution guidance builds
first; `.pkg` (via `xcrun productbuild`, a separate Apple-documented
step, not a Tauri built-in bundle target) is the format App Store
Connect actually accepts for upload. This was determined from current
Tauri distribution documentation for this exact scenario, not assumed.
Neither this milestone nor `package_mas.py` uploads anything — `xcrun
altool`/`xcrun notarytool`-style upload commands do not appear anywhere
in this repository.

## PENDING VALIDATION

Everything below genuinely requires the owner's real Apple Developer
credentials — the sandbox itself is no longer in this list (see "Human
acceptance: PASSED" above):

- An actual `APPLE_SIGNING_IDENTITY`-signed `.app`, and
  `codesign --verify --deep --strict` plus an embedded-entitlements
  inspection (`package_mas.py --sign` implements both, unexercised with
  real credentials).
- A signed `.pkg` via `productbuild`/`APPLE_INSTALLER_SIGNING_IDENTITY`.
- App Store Connect upload, TestFlight, or App Review — none attempted,
  none of the tooling for it added. Note the resulting `.app` from a
  purely local/ad-hoc build has no `_MASReceipt` (the App Store
  purchase-receipt file real Store distribution embeds) — expected for
  this test build, and not a blocker for anything validated here.

The interactive human acceptance checklist that used to live here has
**passed** — see "Human acceptance: PASSED" under "Sandboxed
output-save architecture" above for the full per-step results and the
sandbox log conclusion.

## Owner action required

Everything below is exclusively the owner's to do — none of it is code,
and none of it exists in this repository:

- Apple Developer Program membership (individual or organization).
- Apple Distribution certificate + Mac Installer Distribution
  certificate, generated and kept in a build keychain.
- App ID registration for `me.mpdf.processor` with App
  Sandbox capability enabled, under the owner's Team ID.
- Mac App Store provisioning profile for that App ID.
- App Store Connect app record creation.
- Paid Apps Agreement acceptance, banking, and tax information in App
  Store Connect (required before *any* paid app, including
  one-time-purchase, can be submitted).
- Pricing tier selection (no price appears in this repository).
- Store listing metadata: screenshots, description, keywords, support
  URL, privacy policy URL.
- App Privacy declarations in App Store Connect (what data, if any, is
  collected — this application collects none at runtime today, but the
  declaration itself is an App Store Connect form, not a code artifact).
  If App Review asks about the `network.client` entitlement, see
  "Network-capability audit" under "Entitlements audit" above — it is a
  `WKWebView` process-architecture requirement, not application
  networking, and the privacy declaration should say so.
- The actual signed build, `.pkg` creation, upload, and submission for
  review — all deliberately left undone by this milestone.

## What M7B1 explicitly did not do

- No public release, git tag, or GitHub Release.
- No submission to App Store Connect, no upload, no TestFlight.
- No pricing, paywall, subscription, DRM, license key, or activation
  server — in code or in this document.
- No M7B2 work started.
- No Apple App ID created, no App Store Connect record created, nothing
  submitted to Apple — the identifier finalization below is a
  repository-side rename only, ahead of any Apple-side registration, so
  registration happens against the correct, final name.

## Bundle identifier

**Finalized this milestone, owner-approved**: `me.mpdf.processor`
(previously `org.museionproject.binarize`, M7A's original value). The
owner explicitly approved this permanent identifier and the
`me.mpdf.<product>` namespace convention for future PDF tools, and
asked for the change to happen now, before any Apple App ID or App
Store Connect identity is created against the old name. See "READY /
IMPLEMENTED" above for the single-source-of-truth mechanism and the
regression tests that enforce it.
