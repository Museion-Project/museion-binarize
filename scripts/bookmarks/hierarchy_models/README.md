# Apple directory hierarchy interface

The desktop product retains only Apple local Foundation Models. The status check runs when the directory controls open, when the window regains focus, and on manual refresh. It does not run inference. Apple unavailability is explicit and preserves the basic directory tree.

Generation sends original page images and measured entry geometry. The model may only propose level/parent changes; titles, membership, order, source evidence and page targets are preserved. Suggestions support up to three pages and 60 entries. Invalid or abstained results retain the baseline, and saving still requires human review. The existing document subprocess lifecycle handles cancellation.

`python3 scripts/bookmarks/hierarchy_models/manager.py status` checks Apple availability. SDK27 check/image-worker binaries are supplied by the local build overlay. No downloader, model catalog, llama.cpp launch or cloud request is part of this runtime.

MiniCPM-V 4.0 and local Qwen 3B were withdrawn by the user's 2026-09-11 instruction, recorded in `intent.md` §8.42. Their code snapshot and prior results remain in evaluation evidence; previously downloaded weights are retained under the ignored `.runtime/toc-models` directory, without a product route to use them. The one-time DeepSeek cloud diagnostic is isolated in `evaluation/toc-apple-cloud-ceiling-2026-09-11` and is not bundled or callable by the app.

Run local checks with `python3 -m unittest discover -s scripts/bookmarks/hierarchy_models -p 'test_*.py'`.
