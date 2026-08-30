# Closed-world CGPG gold v1

This directory contains human-reviewed gold records, not provider output.

The local annotator starts with the four forensic pages (`399`, `443`, `489`,
`503`). Run it with `--all-holdout` to expose the full frozen 12-page set. CGPG
PAGE XML boxes and text are annotation scaffolding only: they remain orange and
cannot pass the validator until every line is explicitly human verified. Use
Shift-drag on the page to add visible lines missing from PAGE XML.

Draft/final page records are written atomically under `pages/`. A page becomes
gold only after every line is green, the exhaustive full-page checkbox is
checked, a reviewer is recorded, and unresolved notes are empty.

```sh
python3 scripts/ocr/gold/annotate_closed_world_gold.py \
  --corpus-root /path/to/cgpg/data

python3 scripts/ocr/gold/closed_world_gold.py \
  gold-data/closed-world-cgpg-v1/pages/*.json
```

Schema: `schemas/mpdf-closed-world-ocr-gold-page-1.0.schema.json`.
