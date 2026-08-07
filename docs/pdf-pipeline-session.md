# The persistent PDF document session

Milestone 3 replaced `PdfRenderer` (Milestone 2) with
`crates/museion-binarize-core/src/document_session.rs`'s
`PdfDocumentSession`. This document records why, what changed, and what it
means for memory use and source-file mutation.

## The Milestone 2 problem

`PdfRenderer::render_page` reopened and reparsed the source file **on
every call**, because `PdfDocument` borrows from the `Pdfium` binding and
holding one inside `PdfRenderer` looked like it would require a
self-referential struct. This cost one full document parse per page —
expensive on a long scanned book — and left a time-of-check/time-of-use
gap: a file mutated mid-run would be picked up partway through.

## Why a persistent session is safe without `unsafe`

It turns out the self-referential concern was unnecessary. Inspecting the
pinned `pdfium-render` 0.9.3 API (not assumed from memory):

```rust
pub fn load_pdf_from_byte_vec(
    &self,
    bytes: Vec<u8>,
    password: Option<&str>,
) -> Result<PdfDocument<'_>, PdfiumError>
```

`PdfDocument<'a>`'s lifetime parameter is tied to the **`Pdfium` binding**
(the `&self` above), not to any local buffer. This crate's `Pdfium`
binding is a process-wide `&'static Pdfium` (see `pdfium_backend.rs`), so
the elided lifetime resolves to `'static`: `PdfDocument<'static>` can be
stored directly as an ordinary, movable struct field. No self-referential
struct, no raw pointers, no lifetime transmutation, and no `unsafe` block
are needed anywhere in `document_session.rs`.

`load_pdf_from_byte_vec` additionally **takes ownership of the byte
buffer**, storing it inside the document itself
(`document.set_source_byte_buffer(bytes)`). That is what makes the
snapshot policy below possible for free, rather than requiring a second
mechanism to keep a buffer alive.

## Source snapshot policy: open-bytes snapshot

`PdfDocumentSession::open` reads the entire source file into memory
**once**, then loads PDFium from that owned buffer. Every page rendered
afterwards is served from that same in-memory snapshot:

- a modification to the file at `path` made *after* `open` returns cannot
  affect an in-progress operation — there is no time-of-check/time-of-use
  gap, because there is no second read of the filesystem;
- the source is read from disk exactly **once** per operation
  (`inspect`/`analyze`/`process`/`preview`), not once per page.

This was verified against a real PDFium library, not just asserted: see
`source_mutation_after_open_does_not_affect_an_in_progress_session` in
`crates/museion-binarize-core/tests/pdf_pipeline.rs` — it opens a 3-page
document, overwrites the file on disk with an entirely different 1-page
document, and confirms every page still renders successfully from the
original snapshot.

**Output validation** (`crate::validation`) opens the *completed output*
as its own, separate `PdfDocumentSession`. That is a different file and a
different operation — not a second open of the source, and not a
violation of "one session per operation."

### Alternative considered: open-once file session

If the pinned API had not supported owning its byte buffer, the fallback
would have been to keep a file handle open for the whole operation instead
of a byte buffer — still one open, still immune to reopening on every
page, but not immune to the underlying *file's bytes* changing under an
open handle (platform-dependent). The byte-buffer policy was preferred
because it is strictly stronger and the pinned API supports it directly.

## Memory model

Because the whole source file is held in memory for the session's
lifetime, the honest bound during `process` or `analyze` is:

> source PDF bytes (held once, for the whole operation)
> + one uncompressed working raster page
> + algorithm working buffers
> + the growing compressed output PDF (`process` only)

This is **not** O(1) in either source or output size. Milestone 2's
"bounded-memory" claim described a per-page-reopen design that no longer
exists; it has been corrected here and in
[`limitations.md`](limitations.md) rather than left standing.

## The `DocumentSession` trait

The rest of the pipeline (`pipeline.rs`) depends on a project-owned
`DocumentSession` trait, not on `pdfium-render` types:

```rust
pub trait DocumentSession {
    fn info(&self) -> &PdfDocumentInfo;
    fn source_identity(&self) -> &SourceIdentity;
    fn pdfium_library_description(&self) -> String;
    fn render_page(&self, index: u32, dpi: u16) -> Result<DynamicImage>;
}
```

`process_pdf` and `analyze_pdf` each open exactly one `PdfDocumentSession`
and hand it to a generic `*_with_session` implementation. This is what
lets ordinary (non-`#[ignore]`, no-PDFium-required) tests in
`pipeline.rs` prove the single-session behaviour with a mock
`DocumentSession` that counts and records every `render_page` call — see
`process_with_session_renders_every_page_exactly_once_from_the_one_session`
and the `analyze_with_session_*` tests.

## Source identity

`SourceIdentity` (`source_identity.rs`) records what was actually opened:
canonical path, byte length, filesystem modification time, and an
**opt-in** SHA-256 of the content (off by default — hashing a large
scanned book is not free, and most operations don't need it). It never
includes a password. This underpins reproducible reporting and same-file
protection; it does not itself enforce a mutation *policy* beyond what is
described above.
