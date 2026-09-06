# Geometry provider development bake-off — 2026-09-06

Scope: intent.md §8.3, user request in this task on 2026-09-06. Compare the 25 verified development pages (1,378 reference lines). No frozen holdout assets, Gemini, production wiring, provider freeze, or production acceptance. Reference manifest is read-only and its image/reference hashes are checked before and after inference.

This protocol is written before running/scoring these candidates on this development set. Earlier native-control experiments and development annotation provenance exist; this is not a blind test.

## Candidates and fixed adapters

1. Tesseract 5 PSM 3, OEM 1, pinned grc+eng traineddata; TSV level-4 boxes and native block/paragraph/line traversal. Recognition is executed internally to obtain TSV, but text is discarded and never evaluated.
2. Surya 0.17.0 local DetectionPredictor, cached text_detection/2025_05_07 checkpoint; CPU float32, batch size 1; shipped detector defaults; polygon envelopes as line boxes. Generic deterministic XY-cut order below.
3. PaddleOCR 3.3.2 PP-OCRv5_server_det plus PP-DocLayout_plus-L; local CPU, MKLDNN disabled, 4 threads; shipped prediction defaults. Detector polygons supply all line boxes; layout does not discard any detector line. Assign each line to the smallest layout region containing its center with at least 50% line overlap, otherwise an independent region. XY-cut orders regions, then lines inside regions. Labels are retained as raw diagnostics only, not assessed as semantic D.

All inputs are identical original PNG pixels, with each model's default internal preprocessing. No reference text, boxes or reading order enters inference or ordering. No parameter tuning on results. All engines use at most 4 main CPU threads; candidates run sequentially to make latency comparisons useful. Surya auxiliary postprocessing uses its default behavior.

XY-cut: recursively prefer a vertical whitespace gap of at least 1.5 median line heights (left-to-right columns), otherwise choose the largest horizontal gap of at least 0.5 median line heights (top-to-bottom bands); otherwise sort by top then left. Gap ties use coordinate order. For region ordering the scale remains median detected line height. This intentionally simple common adapter is an implementation decision, not a validated semantic order model. Diagnostic common-XY order is also scored for all three without changing their boxes.

## Measurement and decision rule

Use the existing geometry harness's greedy descending-IoU one-to-one matcher with IoU >= 0.30. Report micro precision/recall/F1, matched-line mean IoU, matched-line reference-area coverage, and coverage averaged over **all** reference lines (unmatched=0). Add IoU>=0.50 F1 as a sensitivity diagnostic, not a changed primary criterion. All 25 pages remain in denominators, including failures.

Reading-order accuracy = concordant matched pairs / all comparable matched pairs, pooled over pages; report comparable/all-reference-pair coverage separately, plus page-level results. Pages with fewer than two matches have null order accuracy, not 100%. No cross-page pairs. Order cannot rescue missing lines.

Run two complete passes per candidate in the same loaded process. Report exact ordered-box repeat identity and identity rounded to 0.001 pixel, per-page failures, inference median/p95/max latency (includes image loading/preprocessing/postprocessing and ordering, excludes initialization), initialization separately. Same-process repeatability is not cross-device or cross-version determinism.

Select the development winner among candidates completing all 25 pages on both passes, by first-pass micro line F1, then matched-line mean IoU, then pooled reading-order accuracy, then median inference latency. Also identify the order winner if different and expose material tradeoffs. No statistical significance or unseen-data generalization is inferred from 25 development pages. This selection does not claim A/B/C/E milestone thresholds or production readiness.

## Provenance and limits

Record versions, model-file SHA256, protocol and runner SHA256, manifest SHA256, raw geometry from each pass, metrics and failures. Recheck development inputs only; never open the 16 frozen holdout images or records. The reference includes machine-assisted preannotations corrected by the user/visual review; existing candidate provenance may favor some detectors. Discuss this in the report.
