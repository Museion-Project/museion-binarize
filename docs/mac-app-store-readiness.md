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

- **Permanent product identifier finalized**: `me.museion.binarize`
  (previously `org.museionproject.binarize`), owner-approved, changed
  before any Apple App ID or App Store Connect identity was created.
  Declared exactly once, in the base `tauri.conf.json`; every overlay
  (GitHub dist, MAS) inherits it rather than redeclaring it, enforced by
  `BundleIdentifierConsistencyTests` in `test_distribution.py`, including
  a repo-wide `git grep` assertion that the old identifier does not
  reappear in any active file. The GitHub-distributed macOS app and the
  future Mac App Store version share this one identity, per the intended
  `me.museion.<product>` namespace for future Museion apps.
- **Sandboxed output-save architecture**: `OutputWriteStrategy` in
  `crates/museion-binarize-core/src/pipeline.rs`, selected at compile
  time by the `mas-sandbox` Cargo feature (MAS build only). Real,
  credential-free App Sandbox enforcement was demonstrated locally
  (ad-hoc signing plus the sandbox entitlement — no Apple Developer
  Program membership required for this), and the write path was
  redesigned to work within what a sandboxed save panel's grant actually
  covers, with the resulting guarantee change (no longer crash-atomic on
  the final write, everything else unchanged) explicitly documented, not
  glossed over. See "Sandboxed output-save architecture" below and
  `docs/pdf-output.md`.
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
| Network client/server | Zero network code anywhere in `apps/desktop` or `crates/` (`grep` for `reqwest`/`http(s)://`/`TcpStream`/`fetch(` found nothing). Tauri's internal `ipc:`/`http://ipc.localhost` bridge (see `tauri.conf.json`'s CSP) is `WKWebView`'s own intra-process custom-scheme handler, not a real network socket. |
| Automation / Apple Events | No AppleScript/Scripting Bridge/`NSAppleEventDescriptor` usage. The one Finder interaction (`revealItemInDir`, the "reveal output" button) goes through `NSWorkspace`'s file-viewer API via the `tauri-plugin-opener`/`open` crate, not Apple Events. |
| Camera / microphone / location / contacts | No usage anywhere. |
| Downloads-folder entitlement | The app never targets `~/Downloads` specially — output destination is always the exact user-selected save path. |
| Temporary exceptions | None of the narrow, App-Review-discouraged temporary-exception keys are used; the ordinary `files.user-selected.read-write` entitlement covers this app's actual file-access pattern (see below). |

**Included, with evidence**:

| Entitlement | Why |
|---|---|
| `com.apple.security.app-sandbox` | Required unconditionally for Mac App Store submission. |
| `com.apple.security.files.user-selected.read-write` | The app opens PDFs and saves output exclusively through `tauri-plugin-dialog`'s native open/save panels (`apps/desktop/src/lib/tauri.ts`'s `pickPdfToOpen`/`pickOutputDestination`) — no custom file browser, no raw path entry, no `tauri-plugin-fs` (not a dependency at all). This is exactly the entitlement Apple's own documentation pairs with that access pattern. |

**One entitlement evaluated and deliberately left out despite a
plausible-sounding argument for it**: `com.apple.security.cs.allow-jit`.
Some third-party guides claim any app embedding `WKWebView` needs it for
`JavaScriptCore`. Apple's own Tauri-relevant documentation does not list
it for a sandboxed build, and `WKWebView`'s JavaScript execution runs in
a separate, Apple-signed `WebContent` XPC process with its own
entitlements — not the hosting app's process or entitlements — so the
technical basis for the claim is questionable for this
architecture specifically. It is **not** included pre-emptively without
evidence. If the human sandbox-acceptance pass (see "Pending
validation" below) shows the `WKWebView` failing to initialize or
crashing under a real signed sandboxed build, add it then, with that
observation recorded as the evidence — not before.

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

**Status: code implemented and unit-tested; real sandbox enforcement
proven possible and demonstrated without any Apple credentials; the
full interactive open/save click-through is prepared and ready but not
yet run, because this environment cannot drive native macOS dialogs —
a tooling boundary, not a credentials boundary. See "What remains
unverified" below.**

#### The original risk

`crates/museion-binarize-core/src/pipeline.rs`'s write path used to
unconditionally create a **new temp file in the same directory** as the
destination (`.museion-binarize-<random>.pdf.partial`), then
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
  "Museion Binarize.app"
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
~/Library/Containers/me.museion.binarize/Data/{Documents,Desktop,Downloads,Library,...}
~/Library/Containers/me.museion.binarize/.com.apple.containermanagerd.metadata.plist
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

**Regression tests** (`crates/museion-binarize-core/src/pipeline.rs`):
`direct_write_strategy_writes_bytes_straight_to_the_destination`,
`direct_write_strategy_overwrite_replaces_the_destination_contents`,
`direct_write_strategy_failure_before_persistence_leaves_the_old_destination_intact`
— mirroring the exact three properties the pre-existing
`AtomicSameDirectoryRename` tests already proved, for the new strategy.
`apps/desktop/src-tauri/src/worker.rs`'s
`output_write_strategy_tests::ordinary_build_keeps_the_atomic_same_directory_rename_default`
proves the GitHub build's default is unaffected; `cargo check -p
museion-binarize-desktop --features mas-sandbox` (run this session)
independently proves the other branch compiles and is reachable.

#### What remains unverified, and exactly why

The interactive open/save click-through (steps 1–17 in "Human runtime
acceptance checklist," an updated version of which follows below) was
**not run** this session. Not because of Apple credentials — those were
shown above not to be the blocker — but because driving a real
`NSOpenPanel`/`NSSavePanel` requires either a human at the keyboard or
GUI-automation tooling this environment does not have:
`osascript -e 'tell application "System Events" to ...'` (the standard
way to script native macOS UI from a shell) failed with
`execution error: "System Events" got an error: AppleEvent timed out.
(-1712)` — this requires a one-time Accessibility permission grant in
System Settings, itself a dialog only a human can click through.

This is a real, reproducible, precisely-identified boundary, reported
rather than guessed past. Everything up to that exact point — sandbox
enforcement itself, the app launching cleanly under it, the new
`DirectWriteToDestination` code path compiling and shipping in a build
that launches under real enforcement (`cargo check --features
mas-sandbox`, and an actual ad-hoc-signed sandboxed launch, both done
this session) — is real, demonstrated evidence, not assumption.

#### Ready-to-run acceptance test (no Apple credentials needed)

Because credential-free local sandbox testing works, the owner (who has
real keyboard/mouse access to this machine, unlike this session) can run
the full interactive checklist right now, without waiting for Apple
Developer Program enrollment:

```bash
APPLE_TEAM_ID=LOCALTEST01 python3 scripts/distribution/package_mas.py \
  --target-triple aarch64-apple-darwin --version 0.1.0 \
  --out-dir /tmp/mas-local-test

# entitlements.local-sandbox-test.plist: app-sandbox + files.user-selected.read-write only
codesign --force --deep --sign - \
  --entitlements entitlements.local-sandbox-test.plist \
  "target/aarch64-apple-darwin/release/bundle/macos/Museion Binarize.app"

open "target/aarch64-apple-darwin/release/bundle/macos/Museion Binarize.app"
```

then work through the checklist below. This exercises real kernel
sandbox enforcement and the real `DirectWriteToDestination` code path —
the *only* things this specific local test cannot exercise are Developer
ID identity/provisioning validation and notarization/Store review
themselves, neither of which affects whether the open/save/convert flow
works under sandbox.

## PDFium strategy

Identical trust model to M7A, reused rather than re-implemented:
`museion_binarize_core::pdfium_backend::resolve_library`'s precedence
(explicit path → `MUSEION_PDFIUM_LIBRARY` env var → packaged resource →
executable-adjacent → dev-tree opt-in → system if allowed) is unchanged
— zero diff to `pdfium_backend.rs`. The desktop app's own
`worker::pdfium_config` (also unchanged) resolves the bundled resource
path once at startup via Tauri's `BaseDirectory::Resource` API and uses
it explicitly, with the `MUSEION_PDFIUM_LIBRARY` override still winning
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

**Developer/test override preserved, not weakened**: `MUSEION_PDFIUM_LIBRARY`
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
| Team ID / App ID / bundle identifier consistency | `render_mas_entitlements.py` reads the bundle identifier from `tauri.conf.json` (single source of truth, cannot drift) and reads the Team ID from `APPLE_TEAM_ID` — both are cross-checked into the rendered entitlements' `com.apple.application-identifier` (`$TEAM_ID.$IDENTIFIER`). | Owner must register the App ID `me.museion.binarize` under their own Team ID in the Apple Developer portal, with App Sandbox capability enabled, before either matters at build time. |

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

Two genuinely different kinds of "pending" — conflating them would
overclaim one and undersell the other:

**Requires the owner's real Apple Developer credentials:**

- An actual `APPLE_SIGNING_IDENTITY`-signed `.app`, and
  `codesign --verify --deep --strict` plus an embedded-entitlements
  inspection (`package_mas.py --sign` implements both, unexercised with
  real credentials).
- A signed `.pkg` via `productbuild`/`APPLE_INSTALLER_SIGNING_IDENTITY`.
- App Store Connect upload, TestFlight, or App Review — none attempted,
  none of the tooling for it added.

**Requires only a human at this machine's keyboard — no Apple
credentials at all** (see "Sandboxed output-save architecture" above for
why, and the exact commands to run):

- The interactive acceptance checklist below, against the ad-hoc-signed
  local sandbox-test build. This was set up and demonstrated to launch
  correctly under real sandbox enforcement this session, but the
  interactive open/save clicks themselves were not driven, because this
  environment cannot script native macOS dialogs (a tooling boundary —
  `osascript`'s "System Events" UI scripting requires an Accessibility
  permission grant only a human can approve).

### Human runtime acceptance checklist

Using the ad-hoc-signed local sandbox-test build (no Apple credentials
needed) or, once available, the real `--sign`ed build — either exercises
the same code and the same sandbox enforcement:

1. Launch the sandboxed app.
2. Choose a real PDF through the native Open panel, from a location
   outside any prior app interaction.
3. Preview it.
4. Estimate it.
5. Choose an output path through the native Save panel.
6. Convert successfully.
7. Confirm the output exists at the selected location.
8. Confirm the output PDF is valid (opens, correct page count).
9. Start another conversion and cancel it partway through.
10. Confirm cancellation left no partial/false-complete output.
11. Convert again after cancellation — confirm a fresh attempt succeeds.
12. Overwrite an existing destination (the app supports `--overwrite`
    equivalent behavior — confirm it still works end to end).
13. Quit the app.
14. Relaunch it.
15. Select a different PDF.
16. Convert again.
17. Confirm no runtime `MUSEION_PDFIUM_LIBRARY` (or any other env var)
    was required at any point, and check Console.app for sandbox denial
    (`sandboxd`/`Sandbox: deny`) messages across the whole sequence —
    not just "the UI looked fine."

Until this is actually run, "Human runtime" for the MAS build stays
**pending** — not because it's blocked on anything external, but simply
because it has not happened yet.

## Owner action required

Everything below is exclusively the owner's to do — none of it is code,
and none of it exists in this repository:

- Apple Developer Program membership (individual or organization).
- Apple Distribution certificate + Mac Installer Distribution
  certificate, generated and kept in a build keychain.
- App ID registration for `me.museion.binarize` with App
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

**Finalized this milestone, owner-approved**: `me.museion.binarize`
(previously `org.museionproject.binarize`, M7A's original value). The
owner explicitly approved this permanent identifier and the
`me.museion.<product>` namespace convention for future Museion apps, and
asked for the change to happen now, before any Apple App ID or App
Store Connect identity is created against the old name. See "READY /
IMPLEMENTED" above for the single-source-of-truth mechanism and the
regression tests that enforce it.
