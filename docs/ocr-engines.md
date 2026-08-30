# OCR engines, models, and the gold evaluation

This document records **why** the production OCR default is what it is, what
was measured, and what a future engine must clear before it can replace it.
Nothing here is aspirational: every number in the bake-off table was produced
by running the candidate through the same argv/stdin protocol the application
uses, over a corpus whose ground truth is committed in
`scripts/ocr/gold/corpus.py`.

## The order that makes the rest of this possible

```
original colour/grayscale PDF   (the only authoritative input)
  -> render the ORIGINAL pages
  -> OCR those renders
  -> typed OcrPage/OcrBlock/OcrLine/OcrWord evidence
  -> derived text layer + logical lines
  -> automatic bookmarks (+ review where needed)
  -> freeze the text layer and the bookmark tree
  -> binarize the VISIBLE pages only
  -> assemble ONE final PDF: binarized pixels, original OCR coordinates,
     confirmed outline
  -> reopen and verify independently
```

Binarized pages are **never** OCR'd. `mpdf_core::orchestrator` is the only
implementation of this order; the CLI (`mpdf run`) and the desktop app both
call it, and neither has its own copy.

## What was actually wrong

The previous local provider was `scripts/ocr/rapidocr_sidecar.py` driving
`rapidocr_onnxruntime` 1.4.4 with the `ch_PP-OCRv4_rec` recognizer. That is a
**Chinese/English** model. Its character inventory contains no polytonic
Greek and inadequate German coverage, which produced exactly the reported
failures:

| Printed | Recognized | What the metric calls it |
|---|---|---|
| `ὀξέων` | `ogeov` | diacritic error + Greek→Latin script confusion |
| `für` | `fir` | diacritic error |
| `Ü ä ü` | `U a u` | diacritic error |

This is a **model-selection** defect. No threshold, no preprocessing, and no
dictionary post-correction can recover a codepoint the recognizer cannot
emit — and this project does not attempt dictionary or LLM "repair" in any
case (see "Rules", below).

The same sidecar also published **one detector rectangle as a block, a line,
and a word at once**. On a printed contents page that strands every page
number on its own "line", so no row can be assembled. That defect is fixed
separately by `mpdf_core::logical_lines`; see "Logical lines".

## Gold evaluation

* Corpus: `scripts/ocr/gold/corpus.py` — 15 samples, 8 of them must-pass.
  Ground truth is text in the file; page images are **rendered on demand** by
  `scripts/ocr/gold/fixtures.py` from the OFL-licensed Noto Sans already
  vendored at `crates/mpdf-core/assets/fonts/`. No third-party page image is
  committed, and rendering is byte-deterministic.
* Coverage: polytonic Ancient Greek prose and isolated codepoints
  (`ὀξέων ἄνθρωπος ῥ ᾶ ΐ ΰ ᾳ ῃ ῳ`), German (`Ä Ö Ü ä ö ü ß für Über`),
  Greek/German/English/Latin mixed lines, numerals, footnotes, printed
  contents pages with leader dots and right-flush page numbers (Arabic and
  Roman), a wrapped contents title, one and two columns, 200/300/400 dpi,
  90° rotation, and light speckle.
* Metrics (`scripts/ocr/gold/metrics.py`): CER, WER, exact-word rate,
  **diacritic error rate**, **Greek↔Latin lookalike/script-confusion rate**,
  NFC violations, and printed-contents row assembly.
* Run it:

  ```sh
  python3 scripts/ocr/gold/run_eval.py --work-dir /tmp/gold \
      --candidate "tess-best-auto:tesseract:auto:/path/to/models" \
      --json /tmp/gold/report.json
  ```

**A provider's own confidence is not accuracy.** Confidence is recorded on
the evidence and used for reporting and review routing, but it is never a
substitute for a gold measurement and never on its own opens an automatic
gate.

### Bake-off results (2026-08-28, macOS arm64, Tesseract 5.5.3)

Overall figures across the whole corpus. `gate` is pass only when every
must-pass sample succeeded.

| candidate | engine / model | license | gate | CER | WER | exact word | diacritic err | script confusion | NFC viol. | s/page |
|---|---|---|---|---|---|---|---|---|---|---|
| **tess-best-auto** | Tesseract 5.5.3 LSTM, tessdata_best 4.1.0 `grc+deu+eng` | Apache-2.0 | **pass** | **0.103** | **0.442** | **0.558** | **0.036** | **0.001** | 0 | 0.59 |
| tess-std-auto | Tesseract 5.5.3 LSTM, tessdata 4.1.0 `grc+deu+eng` | Apache-2.0 | pass | 0.113 | 0.429 | 0.576 | 0.042 | 0.001 | 0 | 0.53 |
| tess-best-latdeueng | tessdata_best `lat+deu+eng` | Apache-2.0 | fail | 0.336 | 0.608 | 0.398 | 0.653 | 0.056 | 0 | 0.72 |
| tess-best-deu | tessdata_best `deu` | Apache-2.0 | fail | 0.364 | 0.610 | 0.396 | 0.719 | 0.027 | 0 | 0.42 |
| tess-best-eng | tessdata_best `eng` | Apache-2.0 | fail | 0.371 | 0.721 | 0.284 | 1.000 | 0.055 | 0 | 0.41 |
| paddle-v5-el | PaddleOCR 3.3.2, PP-OCRv5 `el` (Modern Greek) | Apache-2.0 code; weights unpinned | fail | 0.298 | 0.757 | 0.257 | 0.645 | 0.005 | 0 | 13.2 |
| paddle-v5-en | PaddleOCR 3.3.2, PP-OCRv5 `en` | Apache-2.0 code; weights unpinned | fail | 0.314 | 0.468 | 0.543 | 0.581 | 0.095 | 0 | 13.2 |
| tess-best-ell | tessdata_best `ell` (Modern Greek) | Apache-2.0 | fail | 0.597 | 0.790 | 0.225 | 0.311 | 0.057 | 0 | 0.47 |
| tess-best-grc | tessdata_best `grc` alone | Apache-2.0 | fail | 0.608 | 0.768 | 0.238 | 0.305 | 0.048 | 0 | 0.33 |
| rapidocr `ch_PP-OCRv4` | RapidOCR 1.4.4 + Chinese recognizer | Apache-2.0 code; model unpinned | **not run** — package not installable offline on this host; superseded upstream. Its failure mode is reproduced exactly by `tess-best-eng` (Latin-only model on Greek): diacritic error 1.000, script confusion 0.055. |
| Kraken | multi-script segmentation + per-script models | Apache-2.0 code; **per-model licenses unconfirmed** | **not evaluated** — see below. |

Per-sample figures for the adopted candidate:

| sample | CER | diacritic err | notes |
|---|---|---|---|
| `grc-prose-heraclitus` | 0.000 | 0.000 | |
| `grc-prose-plato` | 0.000 | 0.000 | |
| `grc-codepoints` | 0.131 | 0.263 | isolated accented vowels with no word context |
| `deu-prose-umlauts` | 0.000 | 0.000 | |
| `deu-codepoints` | 0.000 | 0.000 | |
| `mixed-greek-german-english` | 0.029 | 0.000 | |
| `mixed-latin-numerals` | 0.000 | 0.000 | |
| `mixed-two-column` | 0.035 | 0.000 | |
| `grc-…-200dpi` / `-400dpi` | 0.000 | 0.000 | |
| `deu-…-noise` | 0.000 | 0.000 | |
| `deu-…-rot90` | 0.000 | 0.000 | orientation pass, boxes mapped back |
| `toc-single-column` | 0.248 | 0.200 | 5/5 rows assembled, 5/5 numbers exact |
| `toc-roman-front-matter` | 0.345 | 0.000 | 4/4 rows assembled, 2/4 numerals exact |
| `toc-wrapped-title` | 0.211 | 0.000 | 1/2 rows; wrapped titles are a known limit |

CER on the `toc-*` samples is dominated by leader-dot runs being recognized
with a different number of dots. That is cosmetic; the decision-relevant
number is row assembly, which is scored separately.

### Decisions

**Adopted: Tesseract 5 LSTM (`--oem 1`) with `tessdata_best` 4.1.0, profile
`auto` = `grc+deu+eng`.** It is the only candidate that passes the gate. It
is also *smaller* than the standard `tessdata` alternative for this profile
(≈39.8 MiB vs ≈56.9 MiB including `osd`), and ~22× faster than PaddleOCR on
this host. Both the engine and the trained data are Apache-2.0, and every
file is pinned by URL, size and SHA-256 in
`distribution/ocr-models/manifest.toml`.

**Rejected: RapidOCR + `ch_PP-OCRv4`.** Wrong script inventory; the direct
cause of the reported failures. It remains available behind
`scripts/ocr/rapidocr_sidecar.py` for reproducing historical runs only, and
that file now says so at the top.

**Rejected as a default: PaddleOCR PP-OCRv5.** Measured, not assumed. Its
`el` recognizer targets **Modern** Greek, which is monotonic: on polytonic
input it returns the right letters with the marks removed
(`ῥ`→`ρ`, `ᾶ`→`α`, `ἄ`→dropped), giving a diacritic error rate of 0.645. It
also fetches weights at first use, which violates the no-runtime-download
rule, and is ~13 s/page here. Usable as a comparator behind
`--engine paddleocr`; never a production default.

**Not evaluated: Kraken.** Its multi-script segmentation plus per-script
recognition is a plausible fit, and Kraken itself is Apache-2.0, but each
published model carries its own license. A future evaluation must, per model:
confirm the license permits redistribution, pin a version and SHA-256, record
both in `distribution/ocr-models/manifest.toml`, and clear the gold gate. A
model whose license is missing or non-redistributable may be a development
candidate only — it never enters a bundle and is never fetched automatically.

## Language profiles

"Greek" is not one language. The profiles are:

| profile | Tesseract languages | for |
|---|---|---|
| `auto` (default) | `grc+deu+eng` | this project's corpus, one combined pass |
| `greek-ancient` | `grc` | polytonic Ancient Greek only |
| `greek-ancient-german-english` | `grc+deu+eng` | explicit form of `auto` |
| `greek-modern` | `ell` | monotonic Modern Greek |
| `german` / `german-english` | `deu`, `deu+eng` | |
| `english` | `eng` | |
| `latin-german-english` | `lat+deu+eng` | |

A mixed Greek/German/English *line* is read in one pass by the combined model
rather than being pre-split, which is why `auto` beats every single-language
profile on the mixed samples. Whole *regions* of one script are a different
question — see block-level script routing below.

## Block-level script routing

### Why the combined pass alone is not enough

The synthetic gold set reports a Greek→Latin script-confusion rate of
`0.0010` for `auto`. On real pages it measures `0.0096` — ten times worse,
and in exactly the failure the profile exists to prevent:

| printed | recognized by `auto` |
|---|---|
| ἐκεῖνο | `EXELVO` |
| ἀνηρτημένου | `AVYNOTNLEVOU` |
| νοῦς | `vole` |
| τὸ | `TO` (12×) |
| καὶ | `nol` / `Kal` / `Kol` |

The cause is not the model set. With both Greek and Latin models loaded, the
Latin ones win inside Greek words. Recognizing the *same four pages* with
`grc` alone drops script confusions from 138 to 5.

`grc` alone is not the answer either: it destroys the Latin around the Greek.
The running head `I. Metaphysische Kausalität` came back as
`Ι. Μειαρῃγϑιβοῆε Καιβα τᾶῖ`. Neither single profile is correct for a page
that is Greek body text under a German running head.

### What the router does

The combined `grc+deu+eng` pass always runs and stays authoritative for page
orientation, block geometry, reading order, and every region the router
declines to touch. On top of it:

1. Lines are grouped into maximal runs of one leaning script. Runs, not
   Tesseract blocks: with `--psm 6` the engine reports a whole page as one
   block, so block-level routing would in practice be page-level routing —
   measured, that destroyed the German running head.
2. A run is re-recognized with `grc` only if it is unambiguously Greek:
   ≥80% of its letters Greek, ≥12 Greek letters outright, and ≤40 Latin
   letters. Latin-dominant runs, genuinely mixed runs, captions, page
   numbers and short fragments all keep the combined result.
3. The crop spans the parent block's full width and the run's own vertical
   extent, padded so high detached Greek accents are not clipped.
4. The candidate replaces the combined text only if it stays inside the crop,
   keeps the region's character count within 0.60–1.60×, retains ≥80% of the
   Greek and ≥50% of any substantial Latin, clears a confidence floor, and is
   NFC. Any failure keeps the combined result, so a router failure degrades
   to today's behaviour rather than to missing text.
5. At most 12 re-recognition subprocesses and 45 s of extra wall clock per
   page; exceeding either deterministically keeps the combined result.

Every decision is recorded in `routing_decisions` on the provider response —
region geometry, Greek/Latin counts, the selected profile, coverage,
confidence, and the replacement or fallback reason. It carries **no candidate
text**: a rejected Greek reading must not be recoverable from the evidence.
The response `parameters` additionally carry `routing_mode`,
`routing_thresholds_version`, and the segment counts.

`--routing off` reproduces the single combined pass exactly, which is what
makes the ablation below a like-for-like comparison rather than a claim.

### Threshold calibration

Swept on the CGPG dev split and on real Greek and German/mixed pages. The
curve is not monotonic in either direction, which is the whole difficulty:

| Greek ratio | real Greek pages: CER / script conf. | German+mixed pages: CER |
|---|---|---|
| combined (off) | 0.0639 / 138 | 0.0765 |
| 0.80 | 0.0558 / 20 | 0.0859 |
| 0.85 | 0.0512 / 32 | 0.0827 |
| 0.90 | 0.0512 / 32 | 0.0799 |
| 0.95 | 0.0525 / 41 | 0.0773 |

Lowering the ratio catches more corrupted Greek but starts rerouting the
mixed footnote lines where a classical edition puts German or Latin *inside*
a Greek quotation. `0.80` was adopted because it is the setting that met the
script-confusion rate target on real pages; `0.90` gave a better aggregate
CER but missed it. The Latin ceiling was calibrated the same way: tightening
it to 20 protected the German but also blocked the long Greek body regions,
which legitimately carry 25–30 Latin characters of footnote markers.

Bump `ROUTING_THRESHOLDS_VERSION` whenever any of these change, so evidence
produced under different calibrations is never silently compared.

### Results, and why routing now defaults to off

**Greek script routing is disabled by default.** It is kept, flagged, and
measured, but it did not earn a default.

The honest sequence matters here. A first version replaced whole Greek regions
and reported a large win: real Greek-dense pages went from 138 Greek→Latin
confusions to 20, and a 16-page slice of the CGPG holdout went from 139 to 0.
Two things were wrong with that number.

First, it was partly bought from the Latin. The German and mixed pages
regressed — CER 0.076471 → 0.085930, WER 0.202409 → 0.219546 — because a Greek
column routinely carries a running head, a reference, or an apparatus line
inside it, and the Greek-only model cannot emit Latin at all. Guarding that
properly (line-level geometric alignment, a reachable Latin-retention floor,
and leaving alone any region the combined pass already read confidently)
brings the German and mixed pages back to **exactly** the combined baseline:
CER 0.076471, WER 0.202409, a 0.00% change.

Second, once those guards are in place the Greek benefit largely disappears,
and the 16-page holdout slice turns out not to have been representative:

| CGPG holdout | 16 pages | **full 63 pages** |
|---|---|---|
| combined, script confusions | 139 | 951 |
| routed, script confusions | 0 | **933** (−1.9%) |
| combined CER | 0.1494 | 0.1766 |
| routed CER | 0.1357 | **0.1797** (worse) |
| regions rerouted | 28 / 41 | **18 / 197** |

The guards that stop the router breaking German also stop it firing: 9% of
regions, for a 1.16× cost and a slightly worse CER. On the four real
Greek-dense pages it still helps (138 → 108 confusions, CER 0.0639 → 0.0617,
diacritic errors 81 → 74) but misses every acceptance target set for it.

So the earlier headline was mostly the damage, not the repair. Routing stays
behind `--routing block-script-v1` until it can earn a default on a corpus of
that size.

### What is still open for Greek routing

Not met, at the German-safe calibration that ships:

| target | actual |
|---|---|
| 4 Greek pages: script confusions ≤ 14 | 108 |
| 4 Greek pages: CER ≤ 0.057 | 0.0617 |
| 4 Greek pages: diacritic errors ≤ 53 | 74 |

Met: German/Latin spans take no new errors (0.00% CER and WER change), the
running head survives on every page, the Latin-retention floor is reachable
and tested, and the 45 s budget is a real subprocess timeout.

## Small-type Latin regions

Separate from Greek routing, separately measured, and **off by default**
(`--small-type-latin on` to enable).

It is off for the same reason Greek routing is: the evidence is one page. It
helps on that page, misses its target, and repairs half the known failures —
enough to keep and keep measuring, not enough to change what every document
gets. Both passes are passed to the sidecar explicitly by the Rust runner and
are part of the OCR job fingerprint, so toggling either invalidates existing
checkpoints instead of letting a resumed run mix evidence from both settings.

German umlauts survive 11 pt body text perfectly and fail in 8 pt notes. On a
real scanned page, measured against a hand transcription of that page:

| region | umlaut/ß errors, combined | with small-type pass |
|---|---|---|
| body, 11 pt | 0 / 24 | **0 / 24** |
| footnote, 8 pt | 5 / 16 | **3 / 16** |

The fix is not resolution. Re-recognizing the footnote *as its own region*
corrects most of it at every scale tested, including 1.0×; the errors come
from the note competing with body text for one page-wide set of layout
assumptions, not from missing pixels. (Raising whole-page DPI was already
shown to make Greek worse: 400 → 600 dpi moved CER 0.176 → 0.207.)

Detection is geometric and script-based, never lexical: a run of contiguous
lines whose **line pitch** is below 0.92× the block median — pitch, because
body and footnote glyph heights overlap (0.97–1.03 against 0.86–1.03 of the
median) while their pitches separate cleanly, about 53 px against 44 px — that
is also Latin-dominant, at least 3 lines, and carries at least 40 Latin
letters. The crop is re-recognized with the profile's Latin languages at 2×,
and the candidate must pass the same containment, coverage, retention,
confidence and NFC checks as a Greek candidate.

Cost on a typical page: 8.3 s → 11.8 s (1.42×), inside the same shared
subprocess and wall-clock budget as Greek routing.

Why it stays off by default:

- **One page of real material.** Pitch thresholds calibrated on a single
  scanned volume are not evidence about scanned books in general.
- **3/16 footnote errors against a ≤ 2/16 target.**
- **Two of four known failing words repaired** (`Zwölfzahl`, `gegründete`)
  against a ≥ 3/4 target; `diesbezüglichen` and `Stählin` still lose their
  diaeresis.

`704 c → 704 0` is a digit/letter confusion, tracked separately and not
counted in the umlaut figures.

## External benchmark: CGPG## Detached Greek accent bands

Separate from the two passes above, separately measured, and **off by default**
(`--detached-greek-accents on` to enable).

In Teubner-set Greek the breathings and accents sit high and clear of the
letter body. Tesseract's line finder treats that band as a line of its own and
publishes it as marks — `\ »" / 3 \ 3 \ »" 3 \ RA 3 / N /`. On four real
pages, 16 such lines put 394 characters of pure noise into the text layer.
Raising page DPI is not the remedy: 400 → 600 dpi measured *worse*, because
the accents separate further.

Detection is geometric and script-based, never lexical: a line is a candidate
when it is at most 0.55× the page's median line height, its **minimum** word
confidence is at most 0.55 (the mean is high — the engine is quite sure a
backslash is a backslash — while something in the band is always unreadable),
it carries fewer than four Greek letters and at most one substantive token,
and its box sits inside the top of a Greek-dominant base line it overlaps
horizontally. The characters such a band happens to emit are corroboration at
most; keying on them would delete real punctuation, page numbers, apparatus
sigla and mathematical symbols.

Band and base line are cropped together and re-recognized as one line with
`grc`. The ablation:

| variant | raw CER | diacritic errors | Greek→Latin |
|---|---|---|---|
| baseline combined | 0.1756 | 81 | 137 |
| suppress band only | 0.1177 | 81 | 137 |
| union crop, PSM 7, 1.0× | 0.1104 | 64 | 90 |
| **union crop, PSM 13, 1.0×** | **0.1086** | **61** | **87** |
| union crop, PSM 7, 1.5× | 0.1114 | 69 | 88 |
| union crop, PSM 7, 2.0× | 0.1123 | 71 | 88 |
| union crop, PSM 6, 1.0× | 0.1557 | 76 | 90 |

PSM 13 wins; PSM 6 is far worse because it re-runs layout analysis on a crop
that is a single line by construction, and upscaling measured slightly worse.

**Suppression and recovery are different gains, and are reported separately.**
Deleting the band removes noise and recovers nothing — measured alone it moves
raw CER 0.1756 → 0.1177 while diacritic errors stay at 81 and script
confusions at 137. Only re-recognition recovers. As shipped, with the safety
gates at full strength:

| | combined | detached pass |
|---|---|---|
| artifact lines | 16 | **0** |
| artifact characters | 394 | **0** |
| raw CER | 0.1756 | **0.1171** |
| folded CER | 0.1649 | **0.1065** |
| diacritic errors | 81 / 1331 | **78 / 1331** |
| Greek→Latin confusions | 137 | **135** |
| NFC violations | 0 | 0 |

Of 20 bands, 8 were replaced and 12 suppressed. The unconstrained ablation
reaches 61 diacritic errors, but only by accepting candidates that drop Latin:
most rejections are `latin-lost`, where the base line's "Latin" is itself
corrupted Greek. The strict gate was kept and the smaller recovery reported
rather than relaxing a safety condition. A page with no candidates comes back
byte-identical, which is the control.

Folded CER folds `ϑ/θ` and `ϕ/φ` **for evaluation only**; the OCR output is
never rewritten.

Why it stays off by default:

- One real Teubner-set source. A single typeface is not evidence about Greek
  editions in general.
- On the full CGPG holdout the pass finds no candidates at all and changes
  nothing — no regression, but no gain.
- Most of the measured gain is artifact suppression, not diacritic recovery.

All three optional passes share one subprocess and wall-clock budget, are
passed to the sidecar explicitly, and are part of the OCR job fingerprint:
all eight on/off combinations produce distinct fingerprints, so toggling any
of them invalidates existing checkpoints rather than mixing evidence.

## External benchmark: CGPG (Patrologia Graeca)

`scripts/ocr/gold/cgpg.py` loads the Patrologia Graeca OCR ground truth
(DOI `10.5281/zenodo.20008699`, `data-v2.zip` SHA-256
`2ee5d79f…04e1f7`, CC BY 4.0; 304 image/PAGE XML pairs, 266 with line
transcriptions). It is an *external* check: everything else in
`scripts/ocr/gold` is rendered by this repository and shares its blind spots.

The corpus is provisioned by hand into a temporary directory. Nothing
downloads it, no image or transcription enters the repository, and the
application has no code path that reaches it. The loader refuses DOCTYPEs and
entity declarations, image filenames that escape the corpus directory,
polygons outside the page, and images whose pixel dimensions disagree with
the transcription.

Two properties of the corpus are worth knowing before reading any number
from it:

- **Its script labels and its transcriptions never coincide.** 109
  `MainText_ColGreek` and 70 `MainText_ColLatin` regions exist, but all of
  them are on the 38 layout-only pages; every transcribed region is typed
  generically as `text`. A scorer must therefore derive the script from the
  human transcription itself, which `cgpg.script_of_text` does — reading the
  reference, never the OCR output.
- **Every transcribed region is Greek.** CGPG can demonstrate that the Greek
  improves; it cannot demonstrate that the Latin is unharmed. That evidence
  has to come from the German and mixed pages instead.

`dev`/`holdout` are assigned by a hash of the page name, so the split is
stable when pages are added or reordered and cannot be widened by re-sorting.
Thresholds were tuned on `dev` only.

    python3 scripts/ocr/gold/run_cgpg.py \
        --corpus /tmp/cgpg/data --models /path/to/tessdata_best \
        --split holdout --limit 16 --json out.json

Full holdout — 63 of the 77 holdout pages carry transcriptions:

| | combined | routed |
|---|---|---|
| CER | 0.1766 | 0.1797 |
| WER | 0.4530 | 0.4568 |
| diacritic error rate | 0.0913 | 0.0909 |
| Greek→Latin confusions | 951 | 933 |
| missing / extra lines | 0 / 148 | 0 / 159 |
| regions rerouted | — | 18 / 197 |
| seconds per page | 8.04 | 9.35 |

A 16-page slice of the same holdout previously reported 139 → 0 confusions and
a CER improvement. The full split does not reproduce it. Report the full
split.

The public package is **not** the 30-page test split used in the paper that
accompanies the corpus. No result here may be compared with that paper's
headline accuracy, and none is claimed. This holdout has also been observed in
several earlier rounds, so it is no longer a pristine blind test.

Full holdout with the detached-accent pass (63 transcribed pages of 77):
identical on and off — CER 0.1766, WER 0.4530, diacritic error rate 0.0913,
951 script confusions, 0 missing lines, 148 extra lines in both. The pass
finds no candidates in this typeface.

## Orientation

Orientation detection and recognition are **separate passes**. Tesseract's
`--psm 1` folds them together and, on a printed contents page, drops the
right-flush page number off the end of every row. The sidecar therefore runs
`--psm 0` (OSD) first, rotates the raster upright only when the orientation
confidence clears a threshold, recognizes with `--psm 6`, and then maps every
box back into the original image's coordinate space. The applied rotation and
its confidence are recorded in the page's provider parameters. `osd` is
verified against the manifest on every page for exactly this reason.

## Logical lines

`mpdf_core::logical_lines` rebuilds printed rows from provider segments using
only measured geometry: baseline and vertical overlap, character height,
horizontal gap, and column membership. It

* merges a title, its leader run, and a right-flush Arabic or Roman page
  number into one logical line, recording `page_number_recovered`;
* refuses to merge across a detected column boundary;
* refuses to merge segments whose heights differ by more than 1.8×, so a
  superscript footnote marker is never glued onto body text;
* requires a *wide* gap before claiming a numeric token is a contents page
  number, so an ordinary number in a sentence is not absorbed;
* leaves a provider that already emits real lines untouched;
* leaves native-text pages untouched, because their boxes are synthesized
  from extracted runs and carry no measured layout;
* records, per line, how many segments were merged, which column it belongs
  to, and the reason for each merge.

The typed OCR records on disk keep the provider's own segmentation exactly as
it arrived, so the raw evidence stays auditable; assembly happens when the
derived document is built.

## Rules this layer will not break

* **No dictionary or LLM repair of local output.** Text recognized by this
  layer is emitted as the engine produced it, NFC-normalized and nothing else.
  Combining marks, breathings, accents, diaereses, and iota subscripts are
  never stripped.

  A cloud model is not an exception to this rule; it is a *different
  recognizer*, selected explicitly, whose output replaces a line only when it
  aligns to a line this layer measured — never a post-hoc "correction" of
  local text. See [`ocr-providers.md`](ocr-providers.md) and
  [ADR 0010](adr/0010-provider-neutral-ocr-and-cloud-modes.md). The local
  reading is kept for every line the alignment could not place, and a model's
  confidence in its own output is never recorded as accuracy.
* **No silent transliteration.** A Greek profile returns Greek Unicode or
  fails. Latin lookalikes are measured (`script_confusion_rate`), never
  accepted as a fallback.
* **No runtime downloads.** Models are provisioned out of band by
  `scripts/ocr/provision_models.py` or shipped in a release bundle. The
  sidecar never fetches anything.
* **No text in logs.** The sidecar prints a fixed string to stderr on
  failure. Page content never reaches a log or the parent process's
  diagnostics.
* **Bounded execution.** The provider is invoked by argv with a timeout, a
  single OMP thread, and caps on stdout and on the recorded artifact.
* **Provenance is part of the evidence.** Engine, model, version, language
  profile, model-set name/version, per-file SHA-256, and license are recorded
  on every page — and, for a cloud page, the provider mode, execution
  location, prompt digest, geometry source, alignment version and any
  fallback reason as well. Changing a model changes the evidence fingerprint, which
  invalidates prior checkpoints and candidate digests by design rather than
  silently reinterpreting them.

## Provisioning models

```sh
# Verify an existing directory and write the sidecar manifest:
python3 scripts/ocr/provision_models.py --target-dir /path/to/models

# Developer/packaging convenience: fetch the pinned files first.
python3 scripts/ocr/provision_models.py --target-dir /path/to/models --download
```

Both modes verify size and SHA-256 against
`distribution/ocr-models/manifest.toml` and fail on any mismatch. `--download`
is refused for any model set not marked `redistributable`.
