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

export function formatReduction(originalBytes: number, outputBytes: number): string | null {
  if (originalBytes === 0) return null;
  const reduction = ((originalBytes - outputBytes) / originalBytes) * 100;
  const sign = reduction >= 0 ? "-" : "+";
  return `${sign}${Math.abs(reduction).toFixed(1)}%`;
}
