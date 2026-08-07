//! Resource limits for untrusted benchmark manifests.
//!
//! A benchmark dataset/profile manifest is untrusted input (it may come
//! from a `--dataset`/`--profile` path a user points at anything). These
//! constants bound how much a malformed or hostile manifest can make the
//! benchmark framework allocate or iterate over, mirroring the page/pixel
//! safety limits the core PDF pipeline already enforces elsewhere.

/// Maximum number of `[[pages]]` entries a dataset manifest may declare.
pub const MAX_DATASET_PAGES: usize = 10_000;

/// Maximum width or height, in pixels, of a benchmark input or
/// ground-truth image.
pub const MAX_IMAGE_DIMENSION: u32 = 20_000;

/// Maximum total pixel count of a benchmark input or ground-truth image
/// (checked separately from the per-dimension limit, since two limits
/// under the per-dimension cap can still multiply to an enormous
/// allocation).
pub const MAX_IMAGE_PIXELS: u64 = 200_000_000;

/// Maximum number of `[[pages.rois]]` entries per page.
pub const MAX_ROIS_PER_PAGE: usize = 64;

/// Maximum number of `[[runs]]` entries a benchmark profile may declare.
pub const MAX_PROFILE_RUNS: usize = 256;
