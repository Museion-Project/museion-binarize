// Presets are convenience configurations, not quality guarantees — see
// docs/desktop.md and docs/roadmap.md. No preset claims to be "best",
// "optimal", or tuned for any specific script or language: no benchmark
// in this repository supports a claim like that yet (see
// docs/benchmarking.md). Every value below is an ordinary, deterministic
// ProcessingSettings value; nothing here is inferred from the image.

import type { PresetId, ProcessingSettings } from "../app/types";

export function defaultSettings(): ProcessingSettings {
  return {
    dpi: 400,
    method: "sauvola",
    threshold: null,
    sauvolaWindowSize: null,
    sauvolaK: null,
    contrast: 0,
    medianDenoise: false,
    backgroundNormalization: false,
    backgroundRadius: null,
    despeckle: "off",
  };
}

export const PRESETS: Record<Exclude<PresetId, "custom">, { label: string; description: string; settings: ProcessingSettings }> = {
  default: {
    label: "Default",
    description: "Sauvola thresholding at 400 DPI with no preprocessing or cleanup.",
    settings: defaultSettings(),
  },
  "fine-detail": {
    label: "Fine detail",
    description:
      "A smaller Sauvola window and higher DPI, for pages with small print or fine diacritics. No despeckling.",
    settings: {
      dpi: 600,
      method: "sauvola",
      threshold: null,
      sauvolaWindowSize: 15,
      sauvolaK: 0.2,
      contrast: 0,
      medianDenoise: false,
      backgroundNormalization: false,
      backgroundRadius: null,
      despeckle: "off",
    },
  },
  "noisy-scan": {
    label: "Noisy scan",
    description:
      "Median denoise and background normalization before thresholding, with conservative despeckling. May remove some fine marks along with noise.",
    settings: {
      dpi: 400,
      method: "sauvola",
      threshold: null,
      sauvolaWindowSize: null,
      sauvolaK: 0.3,
      contrast: 0,
      medianDenoise: true,
      backgroundNormalization: true,
      backgroundRadius: null,
      despeckle: "conservative",
    },
  },
};

/** Whether `settings` matches a known preset exactly, or is "Custom". */
export function matchingPreset(settings: ProcessingSettings): PresetId {
  for (const [id, preset] of Object.entries(PRESETS) as [
    Exclude<PresetId, "custom">,
    (typeof PRESETS)[Exclude<PresetId, "custom">],
  ][]) {
    if (settingsEqual(settings, preset.settings)) {
      return id;
    }
  }
  return "custom";
}

function settingsEqual(a: ProcessingSettings, b: ProcessingSettings): boolean {
  return (
    a.dpi === b.dpi &&
    a.method === b.method &&
    a.threshold === b.threshold &&
    a.sauvolaWindowSize === b.sauvolaWindowSize &&
    a.sauvolaK === b.sauvolaK &&
    a.contrast === b.contrast &&
    a.medianDenoise === b.medianDenoise &&
    a.backgroundNormalization === b.backgroundNormalization &&
    a.backgroundRadius === b.backgroundRadius &&
    a.despeckle === b.despeckle
  );
}

/** `<source-stem>-binarized.pdf`, matching the convention in docs/cli.md. */
export function defaultOutputFileName(sourceFileName: string): string {
  const stem = sourceFileName.replace(/\.pdf$/i, "");
  return `${stem}-binarized.pdf`;
}
