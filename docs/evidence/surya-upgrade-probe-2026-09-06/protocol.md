# Surya upgrade compatibility probe

Authorized by the user's 2026-09-06 message: commit current state, formally select Surya, and before freeze compare `Surya 0.17.0 + 2025_05_07 detector` against `Surya 0.22.1 + current default detector` on only the existing 25 pages, without adapter/rule changes.

Checkpoint: `2f73d03`. Adapter: the committed r3.0-apparatus-repair, identical bytes for both arms. No threshold or reference edits, no holdout assets, no transcription/layout inference. One actual detector pass of 25 pages per arm, CPU float32, four threads, batch size 1. Both arms use the same images. Scoring occurs only after inference and uses the existing scorer unchanged.

The 0.22.1 wheel identifies its current default as `s3://text_detection/2025_05_07`; verify runtime setting and every loaded checkpoint tensor. Use its official `DetectionPredictor.local()` to avoid the newly default shared server. Preserve full raw payload and full normalized geometry. Compare raw coordinates/polygons separately from confidence and metadata, and compare final geometry excluding version-specific provenance/digests. Report raw and logical line metrics, per-page changes, TOC/apparatus/marginalia and column/order diagnostics. No extra tuning round.

Provider selection is Surya. Runtime version decision follows the results: keep the already validated 0.17.0 baseline unless the upgrade supplies an evidenced benefit or a necessary compatibility improvement without regression. This is a bounded implementation decision, not a change to the product requirements. Selection/freeze is separate from production wiring, packaging, independent holdout and semantic D acceptance.

0.22.1 package is extracted in a temporary isolated import directory, with Transformers 5.12.1 and Hugging Face Hub >=1.5 as required by its published metadata. Other detector dependencies are shared with the existing local environment. This is a detector-path compatibility probe, not a complete Surya OCR installation test.
