# Museion PDF

English · [中文](README.zh-CN.md)

Local PDF black-and-white processing and editable contents bookmarks, with a Chinese/English interface. This Beta is for **Apple Silicon Macs**.

## Download and install

[Download 0.2.0-beta.2](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.2.0-beta.2). Open the DMG and drag **Museion PDF.app** into Applications. The release is Developer ID signed and Apple notarized. No separate Python or Homebrew installation is needed.

## Use

1. Open a PDF, select all pages or a page range for black-and-white processing, and inspect the preview.
2. For bookmarks, select contents pages, generate entries, then edit titles, hierarchy and destination pages.
3. Review and confirm the contents, then save a new PDF. The original is preserved.

Body OCR is disabled. Apple Vision can read selected contents pages and page-number margins locally; documents are not uploaded. Apple image hierarchy suggestions require macOS 27 and an available system model. Otherwise basic hierarchy remains for manual review. No model download is initiated.

## Beta limitations

- macOS arm64 only. Mandatory binaries target macOS 13 or later; actual machine testing was on macOS 27, with older systems not separately tested.
- Black-and-white conversion replaces selected pages with images, removes their text layers and interactive annotations, and may increase file size.
- Generated hierarchy and destinations require review before saving.
- Bookmark-only writing does not support encrypted or digitally signed PDFs.
- Gatekeeper enforcement was already disabled on the test host. Independent Apple distribution checks are retained; downloading and launching on a separate clean Mac has not been tested.

[Report an issue](https://github.com/Museion-Project/museion-binarize/issues) with your OS version, steps and a non-sensitive example.

## Development and licenses

[Local build and release](docs/macos-local-release.md) · [Changelog](CHANGELOG.md) · [Third-party licenses](THIRD_PARTY_LICENSES.md). Earlier RC and experimental pipeline descriptions are retained in the [historical documentation](docs/legacy/README.md); those features are not the Beta feature list.

Created and maintained by Pei Haoran under Museion Project. Source is dual-licensed MIT OR Apache-2.0.
