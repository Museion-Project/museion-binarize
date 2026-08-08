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
- **No M0–M7A behavior changed.** `crates/museion-binarize-core/` and
  `crates/museion-binarize-cli/` have zero diff against `main` at
  `afff0de46e47be7ec18b699606b3679003e0e098`; every M7B1 change is new
  files plus additive config, except one documentation-only correction
  (this file) and a `.gitignore` addition.

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

### Known blocking risk: the atomic write-then-rename pattern

`crates/museion-binarize-core/src/pipeline.rs`'s `write_temporary`/`persist`
writes conversion output to a **new temp file in the same directory** as
the destination (`.museion-binarize-<random>.pdf.partial`), then
`rename(2)`s it onto the final path — deliberately, for atomicity (see
`docs/pdf-output.md`). This is unaffected by, and correct for, M0–M7A
(the CLI and the GitHub-distributed desktop app never run under App
Sandbox at all).

Apple's own documented behavior (Apple Developer Forums, on
`NSSavePanel`-granted sandbox access): *"When the save panel gives you
back a URL it extends your sandbox so that you can access **exactly
that URL**. You are not allowed to change the URL in any way..."* — the
grant is scoped to the one path the user selected, not to sibling paths
in the same directory. That means creating the `.partial` temp file
**next to** the selected output path is not obviously covered by the
same sandbox extension, and could fail under a real sandboxed build.

**This was not fixed in M7B1.** Two real, credentials-independent facts
support leaving it as a documented risk rather than a blind code change:

1. It cannot be verified without an actually signed, sandboxed binary
   running under real enforcement — which requires the Apple Distribution
   credentials this milestone does not have (see "Owner action
   required").
2. It does not affect M0–M7A at all, so there is no regression risk to
   the existing, shipped GitHub build from leaving it unresolved for now.

**If real sandbox testing confirms this fails**, the fix is scoped and
known, not open-ended: write directly to the exact granted output path
for the MAS build specifically (accepting the loss of the atomic-replace
guarantee `docs/pdf-output.md` documents for the CLI/GitHub build), or
integrate Apple's `NSFileCoordinator`/`replaceItemAtURL` safe-save
pattern via a small native shim. Either is a genuine "MAS-specific
adaptation," not a core-pipeline behavior change for M0–M7A, and either
needs a real sandboxed test to confirm it actually resolves the
constraint before being called done.

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
| Team ID / App ID / bundle identifier consistency | `render_mas_entitlements.py` reads the bundle identifier from `tauri.conf.json` (single source of truth, cannot drift) and reads the Team ID from `APPLE_TEAM_ID` — both are cross-checked into the rendered entitlements' `com.apple.application-identifier` (`$TEAM_ID.$IDENTIFIER`). | Owner must register the App ID `org.museionproject.binarize` under their own Team ID in the Apple Developer portal, with App Sandbox capability enabled, before either matters at build time. |

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

Nothing below has been exercised against a real signed, sandboxed
binary — all of it requires the owner's Apple Developer credentials
first:

- An actual `APPLE_SIGNING_IDENTITY`-signed `.app`, and
  `codesign --verify --deep --strict` plus an embedded-entitlements
  inspection (`package_mas.py --sign` implements both, unexercised with
  real credentials).
- A signed `.pkg` via `productbuild`/`APPLE_INSTALLER_SIGNING_IDENTITY`.
- **Sandbox runtime acceptance** — the human checklist below, especially
  the write-then-rename risk documented above.
- App Store Connect upload, TestFlight, or App Review — none attempted,
  none of the tooling for it added.

### Human runtime acceptance checklist (to run once credentials + a provisioning profile exist)

1. Launch the MAS-configured, sandboxed, signed app.
2. Open a PDF via the picker, from a location outside any prior app
   interaction (e.g. a folder never previously touched by this app).
3. Preview it.
4. Run an estimate.
5. Convert it.
6. Cancel a conversion partway through.
7. Convert again (confirm no partial output, no leftover temp file, a
   fresh attempt succeeds).
8. Save output outside the app's own container, through the native save
   panel.
9. Quit and relaunch the app.
10. Open a different PDF.
11. Confirm no runtime `MUSEION_PDFIUM_LIBRARY` (or any other env var)
    was required at any point.
12. Check Console.app for sandbox denial (`sandboxd`/`Sandbox: deny`)
    messages during the whole sequence above — not just "the UI looked
    fine."

Until this is run, "Human runtime" for the MAS build is **pending**, in
the same sense M7A's own verification-state table used that word.

## Owner action required

Everything below is exclusively the owner's to do — none of it is code,
and none of it exists in this repository:

- Apple Developer Program membership (individual or organization).
- Apple Distribution certificate + Mac Installer Distribution
  certificate, generated and kept in a build keychain.
- App ID registration for `org.museionproject.binarize` with App
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
- No change to the permanent bundle identifier (`org.museionproject.binarize`
  is unchanged; a change did not appear necessary — see the M7B1 audit
  finding that Apple permits reusing the same identifier across
  Developer ID and Mac App Store distributions of the same app).
