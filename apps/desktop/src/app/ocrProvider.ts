import type { OcrProviderModeId } from "./types";

/**
 * The provider choice the pipeline panel holds and the picker edits.
 *
 * `ready` is computed by the picker rather than by the panel, because the
 * conditions that block a start — missing consent, no authorized ceiling, a
 * mode with no production service — are all things the picker
 * already knows. The backend re-checks every one of them regardless; this
 * exists so the button is honest, not so the rule lives here.
 */
export interface OcrProviderSelection {
  mode: OcrProviderModeId;
  cloudConsent: boolean;
  maxCredits: number;
  ready: boolean;
  blockedReason: string | null;
}

/** Local and no cloud consent: the state the app starts in. */
export const DEFAULT_SELECTION: OcrProviderSelection = {
  mode: "local",
  cloudConsent: false,
  maxCredits: 0,
  ready: true,
  blockedReason: null,
};
