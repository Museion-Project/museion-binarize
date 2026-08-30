import type { CloudFallback, OcrProviderModeId } from "./types";

/**
 * The provider choice the pipeline panel holds and the picker edits.
 *
 * `ready` is computed by the picker rather than by the panel, because the
 * conditions that block a start — missing consent, missing key, no authorized
 * ceiling, a mode with no production service — are all things the picker
 * already knows. The backend re-checks every one of them regardless; this
 * exists so the button is honest, not so the rule lives here.
 */
export interface OcrProviderSelection {
  mode: OcrProviderModeId;
  cloudConsent: boolean;
  /** A slot *label*. No field in this app ever carries a key. */
  credentialSlot: string;
  cloudFallback: CloudFallback;
  cloudEndpoint: string;
  maxCredits: number;
  creditsPerPage: number;
  ready: boolean;
  blockedReason: string | null;
}

/** Local, no consent, no credential: the state the app starts in. */
export const DEFAULT_SELECTION: OcrProviderSelection = {
  mode: "local",
  cloudConsent: false,
  credentialSlot: "default",
  cloudFallback: "local",
  cloudEndpoint: "",
  maxCredits: 0,
  creditsPerPage: 1,
  ready: true,
  blockedReason: null,
};
