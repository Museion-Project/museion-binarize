# Mixed-script OCR gold and geometry-provider research

Date: 2026-08-30

## Question

Is there a reusable, public OCR gold set that covers the target domain on the
same pages: German, French, English, and polytonic Greek, with exhaustive line
transcription, line geometry, and reading order? Which local, deterministic
geometry engines are credible candidates to connect to the provider contract
without selecting a winner before the project gold exists?

## Search strategy

Primary-source searches targeted dataset repositories/cards, engine
documentation, model repositories, and released papers. Search terms combined
`historical OCR`, `classical commentary`, `Greek Latin script`, `German French
English`, `PAGE XML`, `line coordinates`, `reading order`, and `layout analysis`.

## Dataset result

No single public dataset was found that satisfies the complete target contract.

| Dataset | Useful coverage | Missing from target contract | Intended use |
|---|---|---|---|
| GT4HistComment / AJMC | Polytonic Greek mixed with German, English, or Latin; 3,356 verified lines; CC-BY-4.0 | No French; line pairs plus region-layout coordinates are not an exhaustive page-level line/read-order gold | Mixed-script transcription control and seed examples |
| FineBooks BHL IMPACT GT | 2,165 page images; English, French, German, Latin; PAGE XML with polygons, Unicode and reading order; CC-BY-3.0 | No Greek | Latin-script layout and reading-order control |
| CGPG | Greek recognition gold and Greek/Latin layout annotations | Recognition annotations cover ancient Greek lines rather than all visible mixed-language text | Greek-only transcription stress test |

Conclusion: M PDF needs its own closed-world page gold sampled from the actual
German/French/English/polytonic-Greek scholarly-document distribution. AJMC and
BHL-IMPACT remain external controls; CGPG remains a Greek specialist set.

## Geometry-provider shortlist (no winner selected)

| Candidate | Why keep it | Main risk to test on project gold |
|---|---|---|
| Tesseract 5 line geometry | Apache-2.0; existing sidecar; TSV/hOCR line and word coordinates; easy deterministic control | Segmentation is coupled to recognition configuration and may fail on complex commentary layouts |
| Orli via Kraken 7 | Apache-2.0 code; directly detects baselines in reading order; PAGE XML; trained for historical documents | Base model requires bfloat16; Apple-Silicon/runtime and model redistribution must be verified |
| Kraken segmentation | Mature historical-document line/baseline segmentation and PAGE output | Runtime/model footprint and every selected model's license/provenance must be pinned |
| PaddleOCR text detection | Apache-2.0 project; detection-only API returns quadrilaterals and scores; ONNX Runtime path | No authoritative reading order; needs a deterministic core-owned ordering stage and historical-page validation |
| Apple Vision | On-device macOS API with line/word observations and normalized bounding regions | OS/model revision variability, portability, and polytonic-Greek behavior; exploratory control only |

The contract is deliberately candidate-neutral. The geometry provider owns
only line identity, coordinates, and order. Gemini owns transcription keyed by
those immutable line identifiers and cannot return geometry.

## Primary sources

- https://github.com/AjaxMultiCommentary/GT-commentaries-OCR
- https://huggingface.co/datasets/finebooks/bhl-impact-gt
- https://zenodo.org/records/20008699
- https://github.com/mittagessen/orli
- https://github.com/OCR-D/ocrd_kraken
- https://github.com/PaddlePaddle/PaddleOCR/blob/main/docs/version3.x/module_usage/text_detection.en.md
- https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html
- https://developer.apple.com/documentation/vision/recognizedtextobservation

