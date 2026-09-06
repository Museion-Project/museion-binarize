# Pre-score geometry representation correction

Both providers were inferred twice with adapter r2.1 attached. Before any round-2 scoring, inspection found that split child fragments inherited their source polygon even though their `bbox` had been split. Source evidence was not lost, but that polygon could misleadingly extend outside the child rectangle.

r2.2 clips the child polygon to its split interval, retains the complete original polygon as `source_polygon`, and distinguishes child geometry features from inherited `source_geometry_features`. Raw payload, source fragments, grouping, boxes, columns, split coordinates, logical IDs, reading order and detection settings are unchanged. The archived `finalist_adapters-r2.1.py` and first `implementation-snapshot.json` record the earlier implementation; original page outputs are retained under `pages/`.

The 50 independently inferred outputs per provider are each normalized with r2.2 in two complete passes, separately for inference pass 1 and pass 2. Final outputs live under `normalized/`, with separate normalization manifests and timing. This is two actual model passes plus normalization of both full raw results; it is not two model calls inferred from a single cached output. Inference time and final normalization time are reported separately/composed, not as an unchanged wall-clock process trace. No extra provider candidate or scored optimization round was introduced.

The final implementation snapshot is recorded before the first round-2 score. No reference, threshold or frozen holdout was changed.
