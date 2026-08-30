import { useEffect, useState } from "react";

import type { CloudFallback, ConnectionTest, OcrProviderStatus } from "../app/types";
import type { OcrProviderSelection } from "../app/ocrProvider";
import {
  deleteModelProviderCredential,
  ocrProviderStatus,
  storeModelProviderCredential,
  testOcrProvider,
} from "../lib/tauri";

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
 * 2. **Consent names the actual document.** "Upload 412 page images to
 *    Google" is a decision; "enable cloud OCR" is a shrug.
 * 3. **A key is written, never read.** The only credential control is
 *    store/replace/remove. There is no "show" button because there is
 *    nothing to show: the app never receives the value back.
 */
export function OcrProviderPicker({
  value,
  onChange,
  disabled,
  pageCount,
}: OcrProviderPickerProps) {
  const [status, setStatus] = useState<OcrProviderStatus | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [credentialPresent, setCredentialPresent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<ConnectionTest | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    ocrProviderStatus(value.credentialSlot)
      .then((next) => {
        if (cancelled) return;
        setStatus(next);
        setCredentialPresent(
          next.modes.some((mode) => mode.id === "gemini-byok" && mode.credentialPresent),
        );
      })
      .catch(() => {
        if (!cancelled) setStatus(null);
      });
    return () => {
      cancelled = true;
    };
  }, [value.credentialSlot]);

  const selected = status?.modes.find((mode) => mode.id === value.mode) ?? null;

  function update(
    patch: Partial<OcrProviderSelection>,
    credentialPresentOverride?: boolean,
  ) {
    const next = { ...value, ...patch };
    const mode = status?.modes.find((item) => item.id === next.mode) ?? null;
    let blocked: string | null = null;
    if (next.mode !== "local") {
      if (mode?.availability === "unavailable") {
        blocked = mode.blockers.join("; ");
      } else if (!next.cloudConsent) {
        blocked = "confirm that page images will be uploaded";
      } else if (
        next.mode === "gemini-byok" &&
        !(credentialPresentOverride ?? credentialPresent)
      ) {
        blocked = "no key is stored for this slot";
      } else if (next.mode === "mpdf-credits" && next.maxCredits <= 0) {
        blocked = "authorize a credit ceiling first";
      }
    }
    onChange({ ...next, ready: blocked === null, blockedReason: blocked });
  }

  async function storeKey() {
    if (!keyInput.trim()) return;
    setBusy(true);
    setProblem(null);
    try {
      const masked = await storeModelProviderCredential(value.credentialSlot, keyInput);
      // The field is cleared immediately: there is no reason for the key to
      // stay in the renderer's memory after it reaches the credential store.
      setKeyInput("");
      setCredentialPresent(masked.present);
      update({}, masked.present);
    } catch (reason) {
      const failure = reason as { error?: { message?: string } };
      setProblem(failure.error?.message ?? "the key could not be stored");
    } finally {
      setBusy(false);
    }
  }

  async function removeKey() {
    setBusy(true);
    setProblem(null);
    try {
      await deleteModelProviderCredential(value.credentialSlot);
      setCredentialPresent(false);
      setTest(null);
      // Removing the key returns the app to a fully local configuration.
      update({ mode: "local", cloudConsent: false });
    } catch (reason) {
      const failure = reason as { error?: { message?: string } };
      setProblem(failure.error?.message ?? "the key could not be removed");
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    setBusy(true);
    setProblem(null);
    try {
      setTest(await testOcrProvider(value.mode, value.credentialSlot));
    } catch (reason) {
      const failure = reason as { error?: { message?: string } };
      setProblem(failure.error?.message ?? "the connection test failed");
    } finally {
      setBusy(false);
    }
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
            {mode.usesNetwork
              ? `Page images are uploaded. Runs at ${mode.executionLocation}.`
              : "Nothing leaves this machine."}
            {mode.model && <> Model: {mode.model}.</>}
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

          <label>
            If a page fails
            <select
              value={value.cloudFallback}
              disabled={disabled}
              onChange={(event) =>
                update({ cloudFallback: event.target.value as CloudFallback })
              }
            >
              <option value="local">
                Recognize it here instead, and tell me which pages
              </option>
              <option value="fail">Stop and write nothing</option>
            </select>
          </label>

          {selected.id === "gemini-byok" && (
            <div className="ocr-provider-credential">
              <p>
                Key slot <code>{value.credentialSlot}</code>:{" "}
                {credentialPresent ? "a key is stored (****)" : "no key is stored"}
              </p>
              <label>
                {credentialPresent ? "Replace the key" : "Store a key"}
                <input
                  type="password"
                  value={keyInput}
                  disabled={disabled || busy}
                  autoComplete="off"
                  onChange={(event) => setKeyInput(event.target.value)}
                  placeholder="paste your API key"
                />
              </label>
              <button type="button" onClick={storeKey} disabled={disabled || busy || !keyInput}>
                Save to this computer's keychain
              </button>
              <button
                type="button"
                onClick={removeKey}
                disabled={disabled || busy || !credentialPresent}
              >
                Remove the key
              </button>
              <button type="button" onClick={runTest} disabled={disabled || busy}>
                Test the connection
              </button>
              <p className="ocr-provider-hint">
                The key is stored in this computer's keychain and is never shown
                again, written to a settings file, or included in a log.
              </p>
              {test && (
                <p role="status">
                  {test.providerName} / {test.model}:{" "}
                  {test.modelAvailable ? "reachable" : "not reachable"} — {test.diagnostic}
                </p>
              )}
            </div>
          )}

          {selected.id === "mpdf-credits" && (
            <div className="ocr-provider-credits">
              <label>
                Service endpoint
                <input
                  value={value.cloudEndpoint}
                  disabled={disabled}
                  onChange={(event) => update({ cloudEndpoint: event.target.value })}
                  placeholder="https://…"
                />
              </label>
              <label>
                Credits per page
                <input
                  type="number"
                  min={0}
                  value={value.creditsPerPage}
                  disabled={disabled}
                  onChange={(event) =>
                    update({ creditsPerPage: Number(event.target.value) || 0 })
                  }
                />
              </label>
              <label>
                Never spend more than
                <input
                  type="number"
                  min={0}
                  value={value.maxCredits}
                  disabled={disabled}
                  onChange={(event) => update({ maxCredits: Number(event.target.value) || 0 })}
                />
              </label>
              <p className="ocr-provider-hint">
                Estimated for this document:{" "}
                {value.creditsPerPage * pageCount} credit
                {value.creditsPerPage * pageCount === 1 ? "" : "s"}. The estimate
                is reserved before the first page is sent; whatever is not used
                is released.
              </p>
            </div>
          )}
        </div>
      )}

      {problem && (
        <p role="alert" className="ocr-provider-error">
          {problem}
        </p>
      )}
      {!value.ready && value.blockedReason && (
        <p role="status" className="ocr-provider-blocked">
          Cannot start: {value.blockedReason}
        </p>
      )}
    </fieldset>
  );
}
