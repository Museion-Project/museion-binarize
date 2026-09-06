# Surya runtime correction (before scoring)

Surya 0.17.0 with the pre-existing Transformers 5.8.0 installation returned one almost-full-page rectangle on consecutive development images. The run was interrupted, and its original JSON and geometry were moved intact to `surya-transformers-5.8.0-aborted/`. It is an invalid model-loading attempt, not a candidate accuracy result.

A direct comparison of every loaded tensor with the pinned safetensors checkpoint found 116/276 mismatches (no missing or extra tensor names): `surya-weight-load-comparison.json`. This establishes incorrect loaded weights; the exact upstream initialization mechanism was not separately traced. No reference boxes were involved in the diagnosis.

An isolated temporary dependency directory uses Transformers 4.56.2, huggingface-hub 0.35.3 and tokenizers 0.22.2. Under that runtime, all 276 tensors equal the original checkpoint after float32 conversion: `surya-compatible-weight-load.json`. The global installation, Surya package, checkpoint, image inputs, detection thresholds, orderer, matching/scoring code and original protocol are unchanged. The Surya candidate's accepted two-pass run records the corrected version in `surya-run.json`.

Command prefix for the valid run:

```sh
PYTHONPATH=/private/tmp/mpdf-geometry-dev-2026-09-06/surya-compat python3 scripts/ocr/geometry/run_dev_geometry_bakeoff.py surya
```

This is a runtime compatibility correction, not model/threshold optimization against development scores. No scores were used to choose this dependency version.
