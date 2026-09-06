# GeometryProvider development comparisons

## Current finalist refinement (round 2)

`intent.md` §8.4 excludes Tesseract and authorizes the Paddle/Surya adapters in
`finalist_adapters.py`. Both emit `museion-geometry-evidence/1`, retaining full
provider payloads, source polygons, fragments, logical grouping, geometric
region containment, column bands, margin split evidence and provenance.
No production Rust schema or semantic D output is changed.

The [round-2 report](../../../docs/evidence/geometry-finalists-r2-2026-09-06/report.md)
contains all 25 per-page scores, strict IoU, catastrophic cases, specialty
slices, column/order diagnostics, determinism and the Surya recommendation.
Paddle's Brisson grouping regression remains an explicit failure.

```sh
PYTHONPATH=/private/tmp/mpdf-geometry-dev-2026-09-06/surya-compat \
  python3 scripts/ocr/geometry/run_finalist_bakeoff.py surya
python3 scripts/ocr/geometry/run_finalist_bakeoff.py paddle
python3 scripts/ocr/geometry/normalize_finalist_passes.py
python3 scripts/ocr/geometry/score_finalists.py
python3 scripts/ocr/geometry/build_finalist_viewer.py
python3 -B -m pytest -p no:cacheprovider \
  scripts/ocr/geometry/test_finalist_adapters.py \
  scripts/ocr/geometry/test_finalist_metrics.py \
  scripts/ocr/geometry/test_geometry_bakeoff.py -q
```

These scripts use explicit recorded dev/model paths and refuse to overwrite
existing run manifests. Reproduction needs a fresh evidence directory and
matching model/runtime paths. Scoring validates the final implementation
snapshot. The recorded run retained both actual detector passes before the
pre-score r2.2 polygon representation fix; both raw passes were separately
normalized again. See its runtime-diagnostics note for the exact evidence path.
Column/specialty labels are evaluation-only existing dev hints, not human D
Gold, and never enter an adapter.

## Earlier 25-page development bake-off (round 1)

The current task is the geometry-only comparison in `intent.md` §8.3, using the
25 verified development references. Its independent runner is
`run_dev_geometry_bakeoff.py`; the original clean-native control below remains
historical evidence. The protocol, input manifest, raw outputs, runtime hashes,
results and report live in
[`docs/evidence/geometry-dev-bakeoff-2026-09-06`](../../../docs/evidence/geometry-dev-bakeoff-2026-09-06/).

The runner reads only the development manifest's explicit image/reference
paths. It does not open frozen holdout assets. It runs Tesseract PSM 3, Surya
local detection with geometric ordering, and Paddle local detection plus
layout ordering twice each. Models are preloaded into local caches; no Gemini
or production entrypoint is called. This is provider selection evidence only.

Run candidates sequentially, then `score`. Existing candidate run manifests
are protected against accidental overwrite. The paths/constants describe this
recorded local experiment; reproduction elsewhere needs equivalent cache paths
and a new evidence directory. Surya requires the tested Transformers 4.56.2
runtime; see the evidence's `runtime-diagnostics/README.md` for the rejected
5.8.0 model-loading attempt.

```sh
python3 scripts/ocr/geometry/run_dev_geometry_bakeoff.py tesseract
PYTHONPATH=/private/tmp/mpdf-geometry-dev-2026-09-06/surya-compat \
  python3 scripts/ocr/geometry/run_dev_geometry_bakeoff.py surya
python3 scripts/ocr/geometry/run_dev_geometry_bakeoff.py paddle
python3 scripts/ocr/geometry/run_dev_geometry_bakeoff.py score
python3 -B -m pytest -p no:cacheprovider \
  scripts/ocr/geometry/test_geometry_bakeoff.py \
  scripts/ocr/geometry/test_dev_geometry_bakeoff.py -q
```

## Earlier clean-native control

This harness measures line geometry only. It rasterizes selected pages from a
born-digital PDF and uses visible PDF text-object lines as a text-free native
reference. It does not claim that the reference is human closed-world gold and
does not clear any candidate for historical scans.

The frozen default pages exercise a contents page, chapter opening, inline
Greek, dense polytonic footnotes, and two-column indexes. Candidate outputs are
matched one-to-one by line-box IoU and scored on precision/recall/F1, mean IoU,
gold-box coverage, reading-order Kendall tau, exact repeat determinism, and
latency.

The result status is always `historical_material_not_validated`. A provider can
be selected as the current clean-control fallback, but production clearance
requires a separate frozen historical-scan geometry holdout.

## Reproduce

```bash
python3 scripts/ocr/geometry/build_native_geometry_control.py \
  /path/to/control.pdf /private/tmp/mpdf-geometry-control

swiftc scripts/ocr/geometry/apple_vision_geometry.swift \
  -o /private/tmp/apple_vision_geometry

python3 scripts/ocr/geometry/run_geometry_bakeoff.py \
  /private/tmp/mpdf-geometry-control/manifest.json \
  /private/tmp/mpdf-geometry-bakeoff.json \
  --tesseract /path/to/tesseract \
  --tessdata /path/to/tessdata_best \
  --tesseract-psm 3 \
  --paddle-model-dir /path/to/PP-OCRv5_server_det \
  --apple-vision /private/tmp/apple_vision_geometry
```

The Apple candidate is macOS-only. Paddle is optional and imports its locally
installed/cached runtime; the harness never downloads a model. Run the Python
metric contract tests with:

```bash
python3 -m pytest scripts/ocr/geometry/test_geometry_bakeoff.py -q
```

## Current milestone reference preflight

The current A/B/C/E contract is in `plan.md`; full D semantics belong to the
later structure stage. `milestone_preflight.py` checks reference readiness
only. It does not compute OCR scores, promote a provider, or consume a frozen
evaluation. It leaves all Gold records, schemas and validators unchanged.

It verifies frozen record/image byte digests without parsing held-out line
content for diagnosis, and excludes development overlap by page ID, actual
image digest, and selection-manifest source digest plus PDF page number.
Source/page bindings are provenance assertions from those manifests, not an
independent reinspection of source PDF bytes. New renderings of a protected
source page cannot become development data through a different filename.

For non-holdout records, reference eligibility requires reviewed geometry and
transcription for every line, exhaustive visible-line coverage, a named
reviewer, and no unresolved reference notes. D's structure/typography review
flags and overall full-Gold `verified` status are not required. This is not a
replacement for full Gold validation, nor proof of representative domain
coverage, decoding, model quality or clean-cache stability.
These checks describe the existing first-party annotation records, not a new
universal requirement that every development fixture be human Gold. Drafts
remain usable for diagnosis; independently generated synthetic references can
support component regression within their measured scope.

```bash
python3 -B scripts/ocr/geometry/milestone_preflight.py \
  --manifest gold-data/mixed-scholarly-v1/freeze-manifest-2026-09-05.json \
  --holdout-pages gold-data/mixed-scholarly-v1/pages \
  --holdout-images gold-data/mixed-scholarly-v1/images \
  --dev-pages gold-data/mixed-scholarly-v1/pages \
  --dev-images gold-data/mixed-scholarly-v1/images \
  --selection gold-data/mixed-scholarly-v1/selection-2026-08-30.json \
  --selection gold-data/mixed-scholarly-v1/selection-2026-08-31.json \
  --output /private/tmp/mpdf-dev-reference-preflight.json
```

Choose a fresh output path: existing evidence is never overwritten. Exit 2
means no eligible development reference, not frozen-model FAIL. Exit 0 only
means references exist and does not establish development exit criteria.
On 2026-09-05 the existing 25 non-holdout draft records provided zero eligible
full-page references; see
`docs/evidence/ocr-geometry-dev-reference-preflight-2026-09-05.json`.

```bash
python3 -B -m pytest -p no:cacheprovider \
  scripts/ocr/geometry/test_milestone_preflight.py \
  scripts/ocr/geometry/test_geometry_bakeoff.py -q
```
