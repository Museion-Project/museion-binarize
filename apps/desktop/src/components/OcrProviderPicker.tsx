import { useEffect, useState } from "react";

import type { OcrProviderStatus } from "../app/types";
import type { OcrProviderSelection } from "../app/ocrProvider";
import { ocrProviderStatus } from "../lib/tauri";

export interface OcrProviderPickerProps {
  value: OcrProviderSelection;
  onChange: (next: OcrProviderSelection) => void;
  disabled: boolean;
  /** Pages that would be uploaded, so consent names a real number. */
  pageCount: number;
}

/**
 * Chooses where text recognition runs.
 *
 * Three rules shape this component, and all three are about not misleading
 * someone into an upload they did not intend:
 *
 * 1. **Local is preselected and stays preselected.** Nothing here changes the
 *    mode on its own, and no error path falls forward into a cloud mode.
 * 2. **Consent names the actual document.** "Upload 412 page images to paid
 *    brokered OCR" is a decision; "enable cloud OCR" is a shrug.
 * 3. **There is no BYOK path.** The only cloud product mode is paid,
 *    brokered M PDF Credits, which remains unavailable until a production
 *    complete-coordinate OCR backend exists.
 */
export function OcrProviderPicker({
  value,
  onChange,
  disabled,
  pageCount,
}: OcrProviderPickerProps) {
  const [status, setStatus] = useState<OcrProviderStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    ocrProviderStatus()
      .then((next) => {
        if (cancelled) return;
        setStatus(next);
      })
      .catch(() => {
        if (!cancelled) setStatus(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const selected = status?.modes.find((mode) => mode.id === value.mode) ?? null;

  function update(patch: Partial<OcrProviderSelection>) {
    const next = { ...value, ...patch };
    const mode = status?.modes.find((item) => item.id === next.mode) ?? null;
    let blocked: string | null = null;
    if (next.mode !== "local") {
      if (mode?.availability === "unavailable") {
        blocked = mode.blockers.join("; ");
      } else if (!next.cloudConsent) {
        blocked = "confirm that page images will be uploaded";
      } else if (next.mode === "mpdf-credits" && next.maxCredits <= 0) {
        blocked = "authorize a credit ceiling first";
      }
    }
    onChange({ ...next, ready: blocked === null, blockedReason: blocked });
  }

  return (
    <fieldset className="ocr-provider-picker">
      <legend>Where text recognition runs</legend>
      {(status?.modes ?? []).map((mode) => (
        <div key={mode.id} className="ocr-provider-mode">
          <label>
            <input
              type="radio"
              name="ocr-provider-mode"
              value={mode.id}
              checked={value.mode === mode.id}
              disabled={disabled || mode.availability === "unavailable"}
              onChange={() => update({ mode: mode.id, cloudConsent: false })}
            />{" "}
            {mode.displayName}
            {mode.defaultMode && <> (default)</>}
            {mode.availability === "beta" && <> (Beta)</>}
          </label>
          <p className="ocr-provider-detail">
            {mode.id === "local"
              ? "Native-text PDFs work in the base app. Scanned pages require the optional offline OCR plugin; nothing is uploaded."
              : mode.usesNetwork
              ? `Page images are uploaded. Runs at ${mode.executionLocation}.`
              : "Nothing leaves this machine."}
            {mode.model && <> Model: {mode.model}.</>}
            {mode.id === "mpdf-credits" && (
              <> Paid and brokered; explicit consent and a hard cost limit are required.</>
            )}
          </p>
          {mode.availability !== "stable" && (
            <ul role="note" className="ocr-provider-blockers">
              {mode.blockers.map((blocker) => (
                <li key={blocker}>{blocker}</li>
              ))}
            </ul>
          )}
        </div>
      ))}

      {selected?.usesNetwork && (
        <div className="ocr-provider-cloud">
          <label>
            <input
              type="checkbox"
              checked={value.cloudConsent}
              disabled={disabled || selected.availability === "unavailable"}
              onChange={() => update({ cloudConsent: !value.cloudConsent })}
            />{" "}
            Upload {pageCount} rendered page image{pageCount === 1 ? "" : "s"} of
            this document to {selected.displayName}.
          </label>

          {selected.id === "mpdf-credits" && (
            <div className="ocr-provider-credits">
              <label>
                Never spend more than
                <input
                  type="number"
                  min={0}
                  value={value.maxCredits}
                  disabled={disabled || selected.availability === "unavailable"}
                  onChange={(event) => update({ maxCredits: Number(event.target.value) || 0 })}
                />
              </label>
              <p className="ocr-provider-hint">
                Pricing is not shown until a production complete-coordinate OCR
                service and auditable reservation policy exist.
              </p>
            </div>
          )}
        </div>
      )}

      {!value.ready && value.blockedReason && (
        <p role="status" className="ocr-provider-blocked">
          Cannot start: {value.blockedReason}
        </p>
      )}
    </fieldset>
  );
}
