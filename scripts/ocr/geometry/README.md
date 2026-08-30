# GeometryProvider clean-native control

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
