# Research Query: OCR + AI-ready document intermediate layers

**Date:** 2026-08-26
**Window:** 2026-02-26 through 2026-08-26
**Status:** Complete for architecture scoping

## Search strategy

Searched recent official release notes, primary project documentation, schemas and source files for local/API
document OCR systems that expose structured blocks, coordinates, reading order, hierarchy, confidence,
provenance, Markdown or RAG-oriented outputs.

Primary projects reviewed: Docling, PaddleOCR/PP-StructureV3, Marker, MinerU, Mistral OCR 4, olmOCR,
franken_ocr and Kreuzberg. Interchange baselines reviewed: PAGE XML, ALTO and IIIF/Web Annotation.

## Key findings

1. Recent systems converge on typed blocks + geometry + reading order + confidence; plain Markdown is a view.
2. No reviewed schema is safe to adopt verbatim: coordinate systems, hierarchy, score semantics and provider
   provenance vary.
3. Selective OCR (native PDF text first, OCR only when needed) is now a practical performance pattern.
4. A project-owned canonical IR should sit above local and API providers.
5. For M PDF Processor, bookmark evidence, searchable PDF text and grounded AI chunks can share the same source nodes.

## Recommended design

Use MDP 0.1 Document IR as the canonical layer. Keep source/provider observation/AI proposal/human revision
separate; normalize geometry into an integer top-left master coordinate space; persist page-sharded JSON;
run real local OCR behind a versioned sidecar protocol; derive Markdown, chunks, searchable text and outline
evidence from the canonical model.

Full synthesis and implementation proposal:
[`docs/ocr-ai-ready-layer.zh-CN.md`](../../docs/ocr-ai-ready-layer.zh-CN.md)

## Primary sources

- [Docling releases](https://github.com/docling-project/docling/releases)
- [DoclingDocument JSON schema](https://github.com/docling-project/docling-core/blob/main/docs/DoclingDocument.json)
- [PaddleOCR releases](https://github.com/PaddlePaddle/PaddleOCR/releases)
- [PP-StructureV3 documentation](https://www.paddleocr.ai/main/en/version3.x/pipeline_usage/PP-StructureV3.html)
- [Marker 2.0 release](https://github.com/datalab-to/marker/releases/tag/v2.0.0)
- [Marker output formats](https://github.com/datalab-to/marker/blob/master/README.md)
- [MinerU releases](https://github.com/opendatalab/MinerU/releases)
- [MinerU output files](https://github.com/opendatalab/MinerU/blob/master/docs/en/reference/output_files.md)
- [Mistral OCR 4 announcement](https://mistral.ai/news/ocr-4/)
- [Mistral OCR processor documentation](https://docs.mistral.ai/studio/document-processing/basic_ocr)
- [olmOCR releases](https://github.com/allenai/olmocr/releases)
- [olmOCR pipeline output source](https://github.com/allenai/olmocr/blob/main/olmocr/pipeline.py)
- [franken_ocr changelog](https://github.com/Dicklesworthstone/franken_ocr/blob/main/CHANGELOG.md)
- [Kreuzberg releases](https://github.com/kreuzberg-dev/kreuzberg/releases)
- [PAGE XML conventions](https://ocr-d.de/en/spec/page)
- [ALTO standard](https://www.loc.gov/standards/alto/)
- [IIIF Presentation API 3.0](https://iiif.io/api/presentation/3.0/)

## Scope caveat

This is an architecture-oriented project review, not a reproducible accuracy benchmark. Vendor/project
benchmark claims were used only to understand capability direction. Engine selection remains gated on the
Project corpus defined in `spec.md` and `plan.md`.
