# rc.3 release-readiness record

`0.1.0-rc.3` is source under preparation. The current public download is
still `v0.1.0-rc.2`; this record is not evidence of a release.

| Gate | State | Evidence / owner action |
|---|---|---|
| Version, WiX mapping, identity freeze | pass_static | `check_version_consistency.py`, `check_identity_compatibility.py` |
| Current-state documentation | pass_static | `release_readiness.py` |
| Distribution script/SBOM/manifest checks | pass_local | `test_distribution.py`, `test_release_hardening.py` |
| PDFium integration smoke | pending (explicit evidence available) | `docs/evidence/pdfium-smoke-aarch64.json`; pass locally with `release_readiness.py --pdfium-evidence ...`; other targets remain pending |
| macOS arm64 install/runtime | pass_local | rc.3 DMG mounted, copied, launched, opened and rendered a 4-page PDF, converted it, reopened the result, and exited cleanly; verify with `release_readiness.py --macos-install-evidence docs/evidence/rc3-local-install-arm64.json` |
| Reader matrix | pending | Requires all dependencies and real reader runs; no download by this check |
| Distribution CI on rc.3 commit | not_run | Owner-triggered manual workflow |
| Developer ID signing and notarization | pending owner credentials | Real certificate/API key run required; mock tests only in this round |
| Windows/Linux install and cross-version upgrade | not_run | Owner-triggered rc.2 → rc.3 checklist; the macOS arm64 fresh-install/runtime path above does not prove upgrade behavior |
| Production OCR runtime in release artifacts | pending | Tesseract, `mpdf_ocr_sidecar.py`, and the pinned `tessdata_best` files are not staged into the current desktop/CLI bundles or represented as bundled SBOM components |
| MAS UI/runtime | pending rc.3 run | Existing MAS technical path is separate from GitHub Developer ID distribution |
| Automatic-bookmark human gold and commercial readers | pending | Synthetic fixtures are not production accuracy evidence |
| Privacy/accessibility/performance report | pending evidence review | Remote OCR is opt-in; local conversion/OCR remain offline |

## Owner-triggered upgrade checklist

1. Preserve a disposable rc.2 install and its app data. Install the rc.3
   Windows MSI and confirm `me.mpdf.processor` data/settings remain visible;
   verify uninstall/upgrade registration and rollback on a disposable VM.
2. On macOS, install the rc.3 `.app` from its `.dmg`; verify the unchanged
   `me.mpdf.processor` bundle/container identity and that source PDFs are not
   overwritten. Run Gatekeeper checks only after a genuine Developer ID and
   notarization run.
3. On Linux, install the rc.3 `.deb` and launch the AppImage from a clean
   user profile; confirm existing MDP/jobs paths and output-boundary behavior.
4. Record OS versions, package hashes, reader versions, and observed failures.

No step above has been run as part of this source hardening round.
