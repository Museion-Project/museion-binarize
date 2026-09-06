# Local Surya → Gemini broker → core compositor

This is the local development path authorized in `intent.md` §8.7. It uses the
frozen Surya 0.17.0 detector and unchanged r3.0 adapter, sends immutable geometry
to a loopback broker, and composes the response with the existing Rust
`GeometryTranscriptionPageProvider`. It does not use the historical whole-page
text/fuzzy-alignment transport or enable the production Credits mode.

## Your Gemini API key

The prepared location is:

`~/.config/museion-binarize/broker/gemini-api-key.txt`

Paste **only the key**, one line, and save. The directory is mode 0700 and the
file is mode 0600. It is outside the repository. Never paste it into chat or a
shell command. The broker reads it at request time; the client never reads it.
A separate `client.token` authenticates local clients and is also mode 0600.
Do not put the Gemini key into that token file. Neither file is logged.

For another machine, create empty private configuration without overwriting
existing files:

```sh
python scripts/ocr/broker/server.py --init
```

## Local runtime

The current machine already has the runtime and dependency overlay prepared.
The frozen `surya-ocr==0.17.0`, Torch 2.8.0 and cached detector must be present.
The entrypoint checks their versions, model hashes and adapter hash, then
verifies all loaded detector tensors. It refuses a mismatched installation.
The development overlay for Transformers 4.56.2 / Hub 0.35.3 / tokenizers 0.22.2
is under the Git-ignored `.runtime/surya-compat`; no temporary-directory
runtime path is required. To prepare the overlay on this checkout:

```sh
sh scripts/ocr/broker/setup_runtime.sh
```

This is a developer environment overlay, not a standalone packaged runtime.

## Start the broker

Commands run from the repository root. The server stays in the foreground;
stop it with Ctrl-C. Nothing is scheduled or started automatically.

```sh
python scripts/ocr/broker/server.py
```

This listens on `127.0.0.1:8766` **with paid requests disabled**. Filling the
key file alone does not enable Gemini calls. When you intentionally want to
send one page to Gemini, start it explicitly with:

```sh
python scripts/ocr/broker/server.py --allow-paid --max-pages 1
```

The cap is the cumulative number of unique attempted model calls stored in
this broker's private `requests.sqlite3`, including failed/uncertain calls;
it is not a dollar estimate or a commercial Credits balance. To authorize
one more call after the first, explicitly raise this ceiling to 2. A single
call has a 16,384 output-token cap. No automatic upstream retries occur.
Successful identical requests replay the saved response without another
model call; changed content under the same identity or an uncertain previous
call is refused. The SQLite state contains returned transcription text but
no source images or API keys; keep this private local file as sensitive data.

## Run a page

With the broker started, run a rendered page PNG through real Surya and the
core compositor:

```sh
python scripts/ocr/broker/run_page.py /absolute/page.png /absolute/output/page.ocr.json --page-index 0
```

Outputs are `page.ocr.geometry.json` (complete immutable evidence plus core
geometry) and `page.ocr.json` (validated `OcrPage`). Existing outputs are never
overwritten. The OcrPage also embeds the original evidence JSON verbatim,
including raw detector boxes, fragments, split provenance and D-ready fields.
No text or word boxes are invented to fill missing responses: wrong line IDs,
order, count, digest, geometry or response model identity fail closed. Gemini
is pinned to the project's `gemini-3.7-flash`; there is no model fallback.

If broker/Gemini fails after geometry detection, the geometry file remains for
inspection. The lower-level `surya_broker_page` Rust example can reuse it;
uncertain upstream requests must not be retried under a fresh ID to evade the
broker's duplicate-call protection.

The model key travels only in the broker's `x-goog-api-key` header to the fixed
Google HTTPS endpoint; redirects and environment proxies are disabled. API
shape follows [generateContent](https://ai.google.dev/api/generate-content).
Raw upstream error bodies are discarded. The broker accepts no browser Origin
and no unauthenticated requests, and has bounded request/response sizes.

## Evidence and limits

Local tests use a temporary **test** key and a simulated Gemini response. The
actual Surya → authenticated HTTP broker → Rust compositor test covers Burnet
1100, all 40 logical lines, original D-ready JSON retention, no raw-number
roundtrip loss, duplicate replay and refusal of altered geometry. The services
are closed when the test ends. No real key is read and no paid API is called.

```sh
python -m pytest -q scripts/ocr/broker/test_broker.py
cargo test -p mpdf-api-client --lib
MPDF_RUN_LOCAL_SURYA_CHAIN=1 python -m pytest -q scripts/ocr/broker/test_local_chain.py
```

Real Gemini model availability, text quality and account permissions remain
unverified until the user configures the key and authorizes an actual call.
The normal desktop/CLI `mpdf-credits` mode remains unavailable; production
broker deployment, payment, packaging and document-wide product integration
are separate work. This page does not claim those gates passed.
