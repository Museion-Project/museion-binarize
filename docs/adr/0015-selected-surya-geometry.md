# ADR 0015: Selected Surya geometry provider

Status: accepted by user for provider selection; runtime freeze supported by the bounded development probe below. Production integration and independent acceptance remain pending.

## Authority and decision

The user's 2026-09-06 instruction “先 commit 当前状态，然后将provider正式确定为Surya” is recorded in `intent.md` §8.6. This supersedes the provisional Tesseract provider selection in ADR 0014. ADR 0012's deterministic geometry + geometry-bound Gemini transcription + deterministic synthesis contract remains in force.

Select **Surya** as `GeometryProvider`, with the fragment-preserving `r3.0-apparatus-repair` adapter committed in checkpoint `2f73d03`. Retain raw detector evidence, immutable source identities, derived-fragment polygons and provenance, logical lines, adaptive columns, margin lanes and low-level D-ready hints. The adapter does not assign semantic D roles or permit transcription to alter geometry.

Freeze the validated local detector configuration at **surya-ocr 0.17.0 + text_detection/2025_05_07**, CPU float32, batch size 1, four Torch threads, Transformers 4.56.2. Exact model/config/adapter hashes and observed relevant library versions are recorded in [the freeze manifest](../evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json). Do not silently upgrade or fall back to Tesseract/Paddle.

## Upgrade probe and rationale

The user requested one small comparison before freeze, on the same 25 development pages, with no adapter or rule tuning. Both releases underwent one actual detector pass. Surya 0.22.1's official release still defaults to `s3://text_detection/2025_05_07`; its official local predictor uses the same detector weights. The full comparison is in [the report](../evidence/surya-upgrade-probe-2026-09-06/report.md).

The fixed-adapter detector path supplies no geometry improvement from the upgrade. Retain the already regression-tested 0.17.0 environment rather than add the newer runtime and shared-server behavior to the frozen dependency surface. 0.22.1 remains a compatible detector-path probe candidate, not the frozen package. Complete 0.22.1 dependency installation and its default service lifecycle were not tested.

## Evidence boundaries and next integration work

Provider selection is final for this decision. The freeze identifies a reproducible **development detector/adapter configuration**, not a shipped runtime closure or quality clearance for all historical scans. The 25 pages are development data, including pages used to diagnose the bounded repair. No frozen holdout evaluation or semantic D acceptance was performed.

Existing Tesseract sidecar/plugin wiring is historical implementation and has not been replaced by this decision. Production work must explicitly wire the selected Surya geometry path, geometry-bound transcription, packaging/runtime closure and provenance, then validate that real consumer path. Until then, production alignment with the selected provider is `unverified`; the bounded development selection/probe is `aligned` with §8.6. Do not relabel the existing Tesseract path as Surya.
