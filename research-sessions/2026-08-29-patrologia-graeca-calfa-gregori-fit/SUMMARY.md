# Research Query: Patrologia Graeca Corpus / Calfa–GREgORI 2026 对本项目的适配度

**Date:** 2026-08-29
**Status:** Complete
**Detailed report:** [`docs/patrologia-graeca-calfa-gregori-fit-2026.zh-CN.md`](../../docs/patrologia-graeca-calfa-gregori-fit-2026.zh-CN.md)

## Search strategy

**Keywords:** Patrologia Graeca Corpus, CGPG, Calfa, GREgORI, polytonic Greek OCR, PAGE XML, YOLO, CRNN, ground truth, license
**Sources:** LREC 2026 paper, UCLouvain project page, Zenodo APIs/releases, upstream GitHub contents, Calfa product pages, relevant runtime license
**Project evidence:** OCR sidecar/provenance contract, current gold evaluation, dataset/provenance rules, local-first distribution policy

## Results

- **Best use now:** external real-PG OCR/layout stress corpus (8.5/10).
- **Useful optional use:** attributed silver corpus for search, lemma/POS and review routing (7/10).
- **Not ready:** production OCR provider replacement (3/10).
- Zenodo v2 audit: 304 image/XML pairs; 266 XML files with 11,058 non-empty line transcriptions; 38 layout-oriented XML files; no word boxes, confidence attributes, or explicit PAGE reading order.
- The paper reports 1.05% CER / 4.69% WER for its PG-specific CRNN versus 11.57% / 39.65% for Tesseract on a 30-page real-PG test set. The published recognition model and an identifiable official test split are not available for local reproduction.
- The public repository contains a YOLOv12 layout `.pt` but no downloadable CRNN recognizer. Runtime licensing and untrusted PyTorch model loading block bundling pending clarification.

## Recommendation

Import the CC BY 4.0 PAGE corpus through a pinned external-manifest benchmark path, retest the current Tesseract profile on real PG material, and request the official split/model/license details before any provider spike. Keep raw OCR immutable; use the linguistic corpus only as an attributed, optional derived layer.

## Reproducibility notes

- Ground-truth DOI: `10.5281/zenodo.20008699`
- `data-v2.zip` Zenodo MD5: `20bf8621dfb4dfe4d9270eee26cfc90e`
- Locally computed SHA-256: `2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7`
- GitHub main commit inspected: `a415fcae253cb2d7ec69a1176cfef1008c252215`
- Layout weight SHA-256: `47a69c4eae86e765aeb907f170a227c9f64f491e4843729c97f2e6fe06cec5b0`
