# ADR 0006: deterministic derived exports and local review overlay

Status: accepted for M4.

The canonical M4 representation is a typed Rust IR generated from MDP plus the
optional OCR extension. Presentation formats are exporters, never the source
of truth. Stable IDs include source page, structural path and evidence
position. OCR coordinates are mapped to the MDP master space and every
exported record retains page id and bbox.

JSON, JSONL, Markdown, TXT, HTML, hOCR and ALTO are deterministic: sorting,
newline policy, escaping and exporter version are fixed and timestamps/random
values are absent. An all-format export is assembled in a temporary directory
and atomically installed with an artifact manifest; no-clobber is the default.

Revisions are append-only records with target reference and base evidence
digest. Human text affects only the derived effective view; source OCR fields
are immutable. AI suggestions are retained but never selected by default.
Review issues are typed and local. Automatic bookmarks, PDF write-back and
cloud APIs remain out of scope for M4.
