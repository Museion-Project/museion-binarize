# ADR 0003: Machine-readable Document Package 0.1

## Status

Accepted for Milestone 1; implementation is in progress pending CI.

## Decision

Add a project-owned, local-first MDP 0.1 container to `mpdf-core`. The
container uses `mpdf-document-package`/`0.1`, deterministic SHA-256-derived
document/page/asset IDs, and JSON records for manifest, source, pages, assets,
provenance and validation. The source PDF remains an external reference by
default. No OCR engine, provider SDK, network service or PDFium type appears
in the persistent schema.

The page master coordinate space is orientation-normalized, pixel-based and
top-left-origin. PDF-point source geometry is retained with an explicit affine
transform, so later OCR and outline evidence can be mapped back without
guessing. Printed page labels, existing outline evidence, typography evidence
and region evidence are typed nullable/list fields.

## Safety and compatibility

Package creation writes a temporary sibling directory and atomically renames
it into a destination that must not already exist. Reads validate relative
paths, reject parent traversal/absolute paths and symlinks, enforce page/asset
count and byte limits, verify all available resource digests, and reject
unknown major schema versions. MDP is a 0.x contract: minor additions may be
safely ignored by this reader, while incompatible major versions require an
explicit reader.

## Consequences

The first vertical slice can be created from the existing PDF inspect/session
path and is deterministic even when no OCR or rendered asset exists. Future
OCR/provider integrations must store original artifacts and provenance rather
than replacing source evidence. A package does not itself guarantee OCR
quality or bookmark correctness; those are later, reviewable layers.
