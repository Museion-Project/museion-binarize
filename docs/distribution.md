# Distribution overview

For the longer-term Museion product, open-source, mobile, institution,
and sustainability direction (which is deliberately broader than the
features in the current release), see
[`product-strategy.zh-CN.md`](product-strategy.zh-CN.md).
That strategy document is forward-looking; this file remains the source
of truth for the distribution policy of the software that exists today.

M PDF Processor's intended long-term distribution model:

- **GitHub source remains open source** (MIT OR Apache-2.0 — unchanged
  by anything in this document; see [`limitations.md`](limitations.md)'s
  and the repository root's license files).
- **GitHub builds remain fully functional.** No feature is held back
  from the open-source build.
- **A future Mac App Store edition** may be sold as a paid convenience
  distribution — packaging and platform-integration work, not a
  separate closed-source feature tier. See
  [`mac-app-store-readiness.md`](mac-app-store-readiness.md): technical
  sandbox readiness (App Sandbox, entitlements, the sandboxed
  output-save path) is complete and human-acceptance-tested locally,
  but production Apple Developer signing/provisioning is still pending
  owner credentials, and no App Store Connect submission has been made.
- GitHub Sponsors may coexist with both later; sponsorship integration
  is out of scope for this engineering milestone.

No DRM, license keys, activation servers, feature paywalls, subscription
logic, or artificial differences between a GitHub build and a future
Store build exist anywhere in this repository, and none are planned as
part of this milestone.

## Current distribution policy

This is the project's distribution policy for rc.3 source while
`v0.1.0-rc.2` remains the current public release — an evolving plan, not an irreversible promise about
every hypothetical future product:

1. Source code is open on GitHub (MIT OR Apache-2.0).
2. Official GitHub binaries are free and fully functional — see
   [the release page](https://github.com/Museion-Project/museion-binarize/releases)
   and the root [`README.md`](../README.md)'s "Download" section.
3. GitHub Sponsors is available at
   [github.com/sponsors/pei-haoran](https://github.com/sponsors/pei-haoran).
4. A paid Mac App Store edition is planned for later, once Apple
   Developer signing/provisioning is ready — as a convenience
   installation/update channel and a way to support development, not
   as a replacement for the free GitHub build.
5. No subscription model.
6. No DRM or license activation.
7. No intentional core-feature paywall between the GitHub build and the
   future Mac App Store edition, under the current product model.

### GitHub Sponsors

`.github/FUNDING.yml` points to the maintainer's approved Sponsors
profile. Sponsorship supports continued open-source development; it does
not change the functionality or license of GitHub builds.

## What Milestone 7A actually built

A self-contained path from source checkout to production artifacts, with
no PDFium/Rust/Node/pnpm setup required by the *end user* of a packaged
artifact (a *builder* still needs the normal toolchain — see
[`releasing.md`](releasing.md)):

- [`pdfium-bundling.md`](pdfium-bundling.md) — trusted bundled PDFium
  resolution for both the desktop app and the standalone CLI, with a
  pinned, checksum-verified provenance chain
  (`distribution/pdfium/manifest.toml`).
- [`releasing.md`](releasing.md) — versioning, artifact naming,
  checksums, the release-manifest schema, the `workflow_dispatch`-only
  GitHub Actions build workflow, and the signing/notarization
  integration code; production credentials and a real rc.3 run remain
  pending.
- [`mac-app-store-readiness.md`](mac-app-store-readiness.md) — an audit
  (not implementation) of what a future Mac App Store submission would
  need.

## What Milestone 7A did not do

- **No rc.3 public release was published.** The public rc.2 release remains
  the only download referenced by the README; no rc.3 tag or GitHub Release
  was created. See
  [`releasing.md`](releasing.md), "Publication is a separate deliberate
  step."
- **No rc.3 signing run has been performed with owner credentials** — the
  fail-closed integration exists and ad-hoc builds remain structurally
  verifiable, but production Developer ID/notarization evidence is pending. See
  [`releasing.md`](releasing.md), "Signing and notarization."
- **Windows and Linux packaging is configured and expected to build**,
  but was not exercised on a real human-operated machine during this
  milestone (this environment has no Windows/Linux desktop to test on)
  — see [`desktop-testing.md`](desktop-testing.md) for the exact
  verification-state table and the human checklist for when that
  hardware becomes available.
- **No Mac App Store submission work** (StoreKit, App Sandbox migration,
  App Store Connect metadata, paid-app agreements) exists.
- **No auto-updater, telemetry, or crash-report upload** was added.
- **No cloud OCR mode is available.** ADR 0011 exposes only `local` and the
  planned paid/brokered `mpdf-credits` product modes. `mpdf-credits` has
  availability `unavailable`: no production complete-coordinate OCR backend,
  payment path, production receipt keys, or published retention/deletion
  policy exists. Legacy `gemini-byok` state remains readable but its
  availability is `disabled`; it is omitted from provider lists/pickers and
  cannot authorize a release capability. Readiness records these states:

  | Gate | State |
  |---|---|
  | `mpdf_credits_complete_ocr_backend` | `blocked_no_service` |
  | `mpdf_credits_payment_integration` | `blocked_no_service` |
  | `cloud_privacy_policy_and_deletion` | `blocked_not_published` |
  | `gemini_byok_product_mode` | `disabled` |

  Readiness is profile-gated rather than treating the report as informational.
  `--profile source` checks source/static consistency. `--profile base`
  requires the signed cross-platform deterministic converter evidence and
  deliberately does **not** require an OCR engine, sidecar, model, or OCR smoke.
  `--profile optional-local-ocr-plugin` requires all base gates plus the
  independently staged plugin and installed OCR smoke/human-gold evidence.
  Ordinary CI and `build-distribution.yml` run only `source` because they
  produce source evidence or private workflow artifacts. The separate
  `publish-release.yml` workflow does not offer `source`; it offers `base` and
  `optional-local-ocr-plugin`, and every required gate for the selected
  artifact must be an explicit pass. Any required `pending`, `not_run`, or
  `blocked_*` gate exits non-zero.

  A base artifact contains the deterministic converter and PDFium, not OCR.
  Native-text PDFs work without a plugin. If a scanned page needs OCR, an
  OCR/searchable-output request fails explicitly with provider unavailable;
  unrelated binarization still works. This is intended product behavior, not a
  missing base-release dependency.

  The checked-in macOS install record predates this profile split and contains
  the historical OCR overlay. Readiness keeps it for auditability but rejects
  it as `base` evidence; a fresh base install record must declare
  `distribution_profile: base` and omit `bundled_ocr`.

  ### Optional local OCR plugin artifact

  The local plugin is a separate per-target, self-contained artifact: a frozen
  sidecar executable (including its image dependency), a pinned Tesseract
  executable and required shared libraries, verified `tessdata_best` files,
  and licenses. It must not depend on Homebrew, system `python3`, system
  Tesseract, or a separately installed Pillow. The macOS arm64 candidate has
  local installed four-page GUI and artifact-bound OCR smoke evidence; the
  cross-platform optional-plugin profile remains pending. The release-wide
  `distribution/ocr-models/manifest.toml` flag remains
  `bundled_in_release = false`: the models are not part of the base release.

  `scripts/distribution/stage_ocr_runtime.py` remains the audited, network-free
  plugin staging boundary. It accepts only an explicitly frozen sidecar, a
  pinned Tesseract root/binary, a previously provisioned `tessdata_best`
  directory, and explicit license files. It records source hashes,
  target/architecture, model hashes, dependency observations, and loader-path
  rewrites in `runtime-manifest.json`. Model downloads remain a separate,
  explicit checksum-verified provisioning operation.

  `scripts/distribution/verify_ocr_runtime.py` remains the independent plugin
  verifier. It rejects system Python/Tesseract requirements, symlinks, unlisted
  files, missing licenses, non-executable entry points, target/release
  mismatches, and model bytes that differ from the pinned manifest. Its
  `pass_structure_only` result cannot close the optional-plugin release gate
  without a separate installed OCR run.

  A verified plugin can be staged into a plugin-bearing CLI archive with
  `package_cli.py --ocr-runtime PATH --pinned-ocr-models PATH`, or copied to the
  ignored stable Tauri resource directory with
  `stage_desktop_ocr_runtime.py` and the explicit
  `tauri.ocr-runtime.overlay.json`. Those opt-in artifacts are distinct from
  the base artifacts. Without the input/overlay, packaging remains compatible
  and makes no OCR claim. `generate_sbom.py --ocr-runtime-manifest PATH`
  expands the plugin SBOM with Tesseract, sidecar, models, staged libraries,
  and licenses; the base SBOM explicitly describes OCR as optional.

  `ocr_runtime_smoke.py` emits `mpdf-ocr-runtime-smoke-evidence` with `pass`,
  `blocked`, or `not_run` and binds the result to artifact/runtime/engine/
  sidecar/model hashes and source immutability. A structure-only verification
  is never accepted by
  `release_readiness.py --profile optional-local-ocr-plugin`.

## Verification-state discipline

Throughout this milestone's documentation, these are treated as
distinct claims, never conflated:

```
"can build" != "can package" != "runtime verified" != "signed" != "notarized" != "published"
```

A green Windows build in CI is not the same claim as "Windows is
verified." See [`desktop-testing.md`](desktop-testing.md) for the
per-platform table that keeps these separate.
