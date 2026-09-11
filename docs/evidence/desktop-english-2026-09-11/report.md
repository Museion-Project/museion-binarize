# English desktop adaptation — 2026-09-11

Implemented against intent.md §8.44 EN1–EN3. Alignment: **aligned** for bilingual UI, local preference persistence and preserving document state. Existing local/Apple-only processing and explicit review requirements remain unchanged.

- Added a 中文 / English selector. A saved preference takes precedence over system language; otherwise zh uses Chinese and other languages use English. Storage failure does not prevent switching. HTML language follows the selected locale.
- Localized the active workspace, black-and-white settings, contents editor, preview, password prompt, progress, validation, and known local-backend messages. Native macOS menus/dialog controls follow macOS language. Unknown technical diagnostics remain verbatim rather than being guessed or suppressed.
- No translation is applied to document text, filenames, editable bookmark titles, target pages or diagnostic detail. Switching preserves drafts, review state and processing settings. Future messages belong in src/lib/messages.ts; no DOM text replacement is used.
- Adjusted wrapping for longer English labels and the unavailable OCR explanation.

## Validation

- TypeScript, ESLint, frontend production build and local release app build passed.
- 65 tests across 9 files passed, including English generate/edit/review/save IPC flow, language switching without title/target/review mutation, persistence/default selection, storage failure and nested backend-message translation. Existing Chinese workflow tests remain explicit Chinese tests.
- Actual macOS release window: English home rendered; native language selector switched to Chinese and the workspace labels updated. No processing was triggered by language switching. Initial English OCR description overflow was found and fixed in the final CSS.
- Native PDF open smoke remained incomplete: the file panel displayed the selected existing fixture but Open remained disabled. No root cause established. The complete English save flow is therefore frontend integration evidence with mocked IPC, not a real PDF-processing/save claim.
- No cloud calls, source-PDF writes, bookmark approval, publication or persistent background task.

The separate local bilingual build is target/release/bundle/macos/Museion Binarize English.app. Existing named application bundles were not overwritten. Evidence: source-sha256.json and build/test logs alongside this report.

Final packaged UI rechecked: the saved Chinese preference survived application restart, switching back to English updated the actual window, and the OCR explanation now wraps within the sidebar. The final app is left open in English.
