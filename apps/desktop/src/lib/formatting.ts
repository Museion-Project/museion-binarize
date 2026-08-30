export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

export function formatMicros(elapsedUs: number): string {
  const seconds = elapsedUs / 1_000_000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds - minutes * 60);
  return `${minutes}m ${rest}s`;
}

export function formatPercent(fraction: number): string {
  return `${Math.round(fraction * 100)}%`;
}

/**
 * Describes an output size change in the direction it actually went.
 *
 * The core reports `sizeReductionFraction = 1 - output / input`, which is
 * *negative* when the output grew. Rendering that straight through produced
 * "-311% smaller" for a file that had in fact grown to 4.11x its input. A
 * conversion that grows is a normal outcome (a photographic scan binarized at
 * a high DPI can easily exceed a JPEG original), so it gets its own honest
 * wording rather than a negative "smaller".
 *
 * Returns `null` when the input size was unknown or zero, so the caller can
 * omit the phrase entirely instead of printing a meaningless number.
 */
export function formatSizeChange(reductionFraction: number | null): string | null {
  if (reductionFraction === null || !Number.isFinite(reductionFraction)) return null;
  if (Math.abs(reductionFraction) < 0.005) return "about the same size";
  if (reductionFraction > 0) {
    return `${Math.round(reductionFraction * 100)}% smaller`;
  }
  // output/input = 1 - fraction, so growth = -fraction.
  return `${Math.round(-reductionFraction * 100)}% larger`;
}
