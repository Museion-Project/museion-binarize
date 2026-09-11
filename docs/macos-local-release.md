# macOS local release candidate

This procedure implements intent §8.45 MR1–MR6 and §8.46 BP1–BP4.
The owner explicitly authorized the Museion PDF 0.2.0-beta.1 rename, rebuild,
notarization and GitHub prerelease publication. Cloud inference remains outside scope.
The existing `base`, `source`, and `optional-local-ocr-plugin` readiness profiles
retain their earlier gates. Use `macos-local` for this candidate.

## Product scope

- Apple Silicon; mandatory binary deployment targets no later than macOS 13.
  Runtime testing currently uses macOS 27. Older-OS compatibility has not been
  exercised on separate machines.
- Local black-and-white conversion and editable, reviewed contents bookmarks.
  Black-and-white conversion replaces the selected pages with images, removes
  their text layers/interactive annotations, and may increase file size.
- Chinese and English UI. File names, source text and bookmark titles are not
  translated by the language selector.
- Body OCR and all cloud/legacy OCR IPC commands are disabled. Local Apple
  Vision reading of selected contents pages and page-number margins remains.
- Apple image hierarchy suggestions require macOS 27 and a ready system model.
  The helper is compiled with SDK 27; unavailable models preserve basic
  hierarchy for explicit user review. No model download is initiated.
- Encrypted and digitally signed inputs are unsupported for bookmark-only
  writing. Original PDFs are never replaced. Targets and review remain required.

## Fixed build inputs

Use an isolated checkout of the source commit recorded in the evidence.
Install Rust/Node dependencies from their existing lockfiles. Python 3.11 or
later is sufficient to run the bootstrap; the build itself must use the
hash-pinned portable Python 3.11.16 distribution.

```sh
python3.11 scripts/distribution/prepare_local_python.py --dest /tmp/museion-python-build
# Select an already installed Xcode with SDK 27; this does not change xcode-select.
DEVELOPER_DIR=/path/to/Xcode.app/Contents/Developer \
  /tmp/museion-python-build/env/bin/python scripts/distribution/build_local_runtime.py
python3.11 scripts/distribution/stage_desktop_pdfium.py aarch64-apple-darwin
cd apps/desktop
pnpm tauri build --config src-tauri/tauri.macos-local.conf.json --bundles app --no-sign
```

The runtime builder refuses existing output directories and unsupported SDKs,
checks pinned package versions, and audits every native dependency. For a
rebuild, keep or remove only the generated `.release-runtime` and
`.release-runtime-build` directories first. These directories are not source.
No Python installation, compiler, Homebrew library, or model file is searched
for by the packaged app.

The main PDFium library is separately pinned in `distribution/pdfium/manifest.toml`.
The Python adapter uses the PDFium build matched to pypdfium2 in a separate
process. The frozen runtime contains the dependency notices. Exact wheel and
Python archive hashes are in `distribution/local-runtime/`.

## Local signing and notarization

Use an existing Developer ID Application identity with its private key in the
local Keychain. A `.cer` certificate is not a notarytool credential profile.
Do not export a private key or put passwords in source, logs or command arguments.

```sh
python3.11 scripts/distribution/sign_local_macos.py sign \
  --app '/absolute/path/Museion PDF.app' \
  --identity 'Developer ID Application: Your Name (TEAMID)' \
  --receipt /absolute/path/evidence/signing.json
python3.11 scripts/distribution/package_macos_dmg.py \
  --app-path '/absolute/path/Museion PDF.app' --version 0.2.0-beta.1 \
  --target-triple aarch64-apple-darwin --out-dir /absolute/path/artifacts
```

The DMG packaging step preserves symlinks and refuses to replace an existing
DMG. Do not rerun Tauri's bundler on a signed app: it replaces the bundle.

If credentials are not already stored, the owner runs this locally and answers
notarytool's prompts. Only the resulting profile name is needed by the agent:

```sh
xcrun notarytool store-credentials museion-release
```

Then submit the signed DMG to Apple's notarization service, staple the accepted
ticket, and verify Gatekeeper:

```sh
python3.11 scripts/distribution/sign_local_macos.py notarize \
  --dmg /absolute/path/artifacts/Museion-PDF-0.2.0-beta.1-macos-arm64.dmg \
  --profile museion-release --receipt /absolute/path/evidence/notarization.json
```

For a final offline-verifiable package, first notarize the app inside a DMG,
then staple its accepted ticket to the standalone app, run the independent
Apple distribution check, and package that app into a fresh output directory.
Sign the final DMG with the same Developer ID identity and notarize/staple it.
Do not overwrite or relabel the first submission as the final package.

```sh
xcrun stapler staple '/absolute/path/Museion PDF.app'
xcrun stapler validate '/absolute/path/Museion PDF.app'
syspolicy_check distribution --verbose --json '/absolute/path/Museion PDF.app'
# Repackage into a fresh directory, then sign the final DMG before submitting it.
codesign --timestamp --sign 'Developer ID Application: Your Name (TEAMID)' /absolute/path/final.dmg
```

A `spctl` result with `override=security disabled` does not prove Gatekeeper
acceptance. The notarization receipt records that assessment as `unverified`.
The independent `syspolicy_check distribution` result must be retained separately;
it performs Gatekeeper, XProtect and other distribution checks without changing
system settings. Record the host's disabled policy as an environmental limitation,
not as a successful clean-machine Gatekeeper launch.

Signing success is not notarization. Missing credentials leave the notarization
gate pending. A rejected submission or failed staple/assessment cannot produce
a success receipt.

## Evidence and delivery

The `macos-local` profile requires the checks listed in
`scripts/distribution/macos_local_readiness.py`. Evidence includes actual file
hashes and the source commit; source/build checks, native runtime closure,
functional checks, signed DMG installation and GUI workflows, PDFium/pypdf/
PDFKit/qpdf reader validation, candidate replacement and preferences, and local
privacy/accessibility/performance checks remain distinct.

```sh
python3.11 scripts/distribution/generate_sbom.py --target aarch64-apple-darwin \
  --version 0.2.0-beta.1 --local-runtime-manifest distribution/local-runtime/sbom-components.json \
  --out /absolute/path/evidence/sbom.spdx.json
python3.11 scripts/distribution/release_readiness.py --profile macos-local \
  --macos-local-evidence /absolute/path/evidence/macos-local.json --json
```

An altered/missing app, DMG, evidence record, or attachment fails validation.
Each evidence record must name its exact source and app digest. Remote CI is
not run by this local procedure and is not reported as passed. This profile
makes no Windows, Linux, Intel macOS, human Gold, automatic hierarchy accuracy,
or production cloud claim. Synthetic review-checkbox tests approve only their
fixture records; they cannot stand in for a user's document review.
