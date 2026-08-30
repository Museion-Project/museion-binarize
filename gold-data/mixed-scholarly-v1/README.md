# Mixed scholarly closed-world OCR gold

This is the first-party evaluation set for pages that may mix German, French,
English, Latin, and polytonic Greek. It starts from blank human annotation; no
OCR provider output is treated as gold or silently used as annotation truth.

Launch the local annotator and select exported page images:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py
```

Or pass images explicitly:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py \
  --images /path/to/page-001.png /path/to/page-002.png
```

Draft JSON is written under `pages/` by default. A draft may contain zero lines
while annotation is beginning. A verified page must contain every visible text
line with an original-image pixel box, exact NFC transcription, language,
content class, and explicit reading order. The page-level exhaustive-coverage
gate must also pass.

CGPG scaffolding remains available only through the explicit legacy mode:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py \
  --corpus-root /path/to/cgpg/data
```

Existing CGPG drafts use schema 1.0 and are neither migrated nor overwritten by
the mixed-gold tool. New first-party drafts use schema 1.1.
