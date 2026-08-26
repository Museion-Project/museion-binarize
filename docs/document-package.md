# Machine-readable Document Package (MDP)

MDP 0.1 is the local, provider-neutral evidence container introduced in
Milestone 1. It records source identity, page geometry, stable IDs, assets,
and provenance. It intentionally does not perform OCR and does not contain a
cloud/provider dependency or a product display name.

## Layout

```text
book.mdp/
├── manifest.json
├── source.json
├── source/                   # optional packaged source files
├── pages/p000001.json
├── assets.json
├── assets/                  # optional referenced files
├── provenance.json
└── validation.json
```

The source PDF is external by default: `source.json` stores its byte length,
SHA-256 and a basename reference, but does not copy it into the package.
Optional assets under `assets/` are referenced by package-relative POSIX paths and are checked by
length and SHA-256 when validating.

## Coordinates and extension points

Each page has a normalized, top-left-origin master space (pixels), a PDF-point
source space (bottom-left origin), and an explicit six-value affine transform
between them. Rotation is normalized to 0/90/180/270 degrees and visible
dimensions come from the existing PDF inspection session. Printed page labels,
existing outline evidence, typography evidence, and region evidence are typed
fields and may be empty. Future OCR/outline runs can add provenance and assets
without changing the source evidence.

The stable schema identifier is `mpdf-document-package` with version `0.1`.
Unknown major versions are rejected; unknown minor versions in the 0.x line
are safely ignored (unknown fields are not retained by the current serde
reader). See the auditable
manifest schema at
[`schemas/mpdf-document-package-0.1.schema.json`](../schemas/mpdf-document-package-0.1.schema.json),
which covers each persistent MDP record type.

## Safety boundary

`mpdf package create input.pdf --output book.mdp` creates a new directory
atomically and refuses to overwrite an existing destination. `mpdf package
validate book.mdp` rejects absolute/parent-traversing paths, symlinked
resources, missing files, duplicate or inconsistent IDs, invalid page order,
non-finite coordinates/matrices, digest mismatches, unsupported major versions,
and resources over the documented count/size limits. JSON metadata is bounded
before parsing. Validation never follows a path outside the package root.

The existing `process` command does not implicitly emit an MDP directory in
0.1. Keeping package creation explicit avoids coupling a converted PDF's
overwrite/validation transaction to a second directory transaction; callers
can create the evidence package first with the same inspected source.

MDP 0.1 is intentionally a 0.x contract: compatible additions may arrive in
minor versions, while a future incompatible major version requires an
explicit reader. OCR text, provider artifacts, reading order and generated
bookmarks remain future layers; this milestone only preserves the evidence
needed to add them reproducibly.
