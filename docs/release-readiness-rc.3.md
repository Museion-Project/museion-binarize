# rc.3 release-readiness record

`0.1.0-rc.3` is source under preparation. The current public download is
still `v0.1.0-rc.2`; this record is not evidence of a release.

| Gate | State | Evidence / owner action |
|---|---|---|
| Version, WiX mapping, identity freeze | pass_static | `check_version_consistency.py`, `check_identity_compatibility.py` |
| Current-state documentation | pass_static | `release_readiness.py` |
| Distribution script/SBOM/manifest checks | pass_local | `test_distribution.py`, `test_release_hardening.py` |
| PDFium integration smoke | pending (explicit evidence available) | `docs/evidence/pdfium-smoke-aarch64.json`; pass locally with `release_readiness.py --pdfium-evidence ...`; other targets remain pending |
| macOS arm64 base install/runtime | pending | The checked-in arm64 record predates the profile split and contains the historical OCR overlay, so it cannot prove the current `base` artifact. Record `docs/evidence/rc3-base-install-arm64.json` with `distribution_profile: base` and no `bundled_ocr` field. |
| Reader matrix | pending | Requires all dependencies and real reader runs; no download by this check |
| Distribution CI on rc.3 commit | not_run | Owner-triggered manual workflow |
| Developer ID signing and notarization | pending owner credentials | Real certificate/API key run required; mock tests only in this round |
| Windows/Linux install and cross-version upgrade | not_run | Owner-triggered rc.2 → rc.3 checklist; the macOS arm64 fresh-install/runtime path above does not prove upgrade behavior |
| Base artifact without OCR runtime | by design | Base desktop/CLI readiness does not require Tesseract, a sidecar, models, or OCR smoke. Native-text PDFs and non-OCR conversion work; OCR of a scanned page fails explicitly with provider unavailable. |
| Optional local OCR plugin artifact | pending | Tesseract, `mpdf_ocr_sidecar.py`, pinned `tessdata_best`, their closure/licenses/SBOM, and installed smoke are a separate `optional-local-ocr-plugin` profile, not a base gate. |
| MAS UI/runtime | pending rc.3 run | Existing MAS technical path is separate from GitHub Developer ID distribution |
| Automatic-bookmark human gold and commercial readers | pending | Synthetic fixtures are not production accuracy evidence |
| Cloud OCR product availability | blocked/disabled | Paid brokered `mpdf-credits` is unavailable because no production complete-OCR service/policy exists; Gemini BYOK is disabled. Neither blocks the base or optional local-plugin profile. |
| Privacy/accessibility/performance report | pending evidence review | Base conversion and the optional local plugin are offline; provider discovery has no available cloud OCR product path. The explicit-endpoint generic API compatibility surface is assessed separately. |

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

The historical combined-build record remains at
`docs/evidence/rc3-local-install-arm64.json` for auditability. The readiness
validator rejects it for the current `base` profile instead of treating a
bundled OCR runtime as base evidence.
