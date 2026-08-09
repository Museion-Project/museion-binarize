# Distribution overview

Museion Binarize's intended long-term distribution model:

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

This is the project's distribution policy as of `v0.1.0-rc.1` — the
current state of an evolving plan, not an irreversible promise about
every hypothetical future product:

1. Source code is open on GitHub (MIT OR Apache-2.0).
2. Official GitHub binaries are free and fully functional — see
   [the release page](https://github.com/Museion-Project/museion-binarize/releases)
   and the root [`README.md`](../README.md)'s "Download" section.
3. GitHub Sponsors is planned, pending approval of the Sponsors
   profile. It is **not** currently available; no Sponsors link exists
   in this repository yet (see "No FUNDING.yml yet" below).
4. A paid Mac App Store edition is planned for later, once Apple
   Developer signing/provisioning is ready — as a convenience
   installation/update channel and a way to support development, not
   as a replacement for the free GitHub build.
5. No subscription model.
6. No DRM or license activation.
7. No intentional core-feature paywall between the GitHub build and the
   future Mac App Store edition, under the current product model.

### No FUNDING.yml yet

`.github/FUNDING.yml` has not been added. GitHub Sponsors is still
pending approval — adding the file (and a Sponsors link anywhere in
this repository) is deferred to a small, separate follow-up once the
Sponsors profile is actually live, so nothing here ever points at a
Sponsors page that doesn't exist yet.

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
  integration points.
- [`mac-app-store-readiness.md`](mac-app-store-readiness.md) — an audit
  (not implementation) of what a future Mac App Store submission would
  need.

## What Milestone 7A did not do

- **No public release was published.** No Git tag was created, no
  GitHub Release was drafted or made public. See
  [`releasing.md`](releasing.md), "Publication is a separate deliberate
  step."
- **No signing credentials were available**, so no artifact produced
  during this milestone is actually signed or notarized — the
  integration points exist and are documented, but the *state* is
  truthfully recorded as pending credentials, not claimed as done. See
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

## Verification-state discipline

Throughout this milestone's documentation, these are treated as
distinct claims, never conflated:

```
"can build" != "can package" != "runtime verified" != "signed" != "notarized" != "published"
```

A green Windows build in CI is not the same claim as "Windows is
verified." See [`desktop-testing.md`](desktop-testing.md) for the
per-platform table that keeps these separate.
