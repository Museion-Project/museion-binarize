import { useState } from "react";
import type { ApiPlanRequest, ApiRoute } from "../app/types";
import { cancelApiTask, prepareApiPlan, runApiTask } from "../lib/tauri";

export interface ApiConsentSummary {
  origin: string; provider: string; model: string; sourceDigest: string;
  sourceBytes: number; pageCount: number; budgetMicros: number; retention: string;
  planDigest?: string; currency?: string;
}
interface Props {
  documentId?: string;
  disabled?: boolean; onChange?: (route: ApiRoute) => void;
  summary?: ApiConsentSummary; confirmed?: boolean; onConfirm?: () => void;
  onCancel?: () => void; progress?: string; costMicros?: number; fallbackReason?: string;
}

/** M6 route choice is explicit and defaults to the fully offline path. */
export function ApiRoutingPanel({ documentId, disabled = false, onChange, summary, confirmed = false, onConfirm, onCancel, progress, costMicros = 0, fallbackReason }: Props) {
  const [route, setRoute] = useState<ApiRoute>("local");
  const [endpoint, setEndpoint] = useState("");
  const [profileId, setProfileId] = useState("default");
  const [model, setModel] = useState("ocr-1");
  const [managedSummary, setManagedSummary] = useState<ApiConsentSummary>();
  const [managedConfirmed, setManagedConfirmed] = useState(false);
  const [managedProgress, setManagedProgress] = useState<string>();
  const [managedCost, setManagedCost] = useState(0);
  const [managedRetention, setManagedRetention] = useState<string>();
  const [managedFallback, setManagedFallback] = useState<string>();
  const [managedArtifact, setManagedArtifact] = useState<string>();
  const [error, setError] = useState<string>();
  const activeSummary = summary ?? managedSummary;
  const activeConfirmed = onConfirm ? confirmed : managedConfirmed;
  const activeProgress = progress ?? managedProgress;
  const activeCost = progress ? costMicros : managedCost;
  const choose = (next: ApiRoute) => { setRoute(next); setManagedSummary(undefined); setManagedConfirmed(false); onChange?.(next); };
  const planRequest = (): ApiPlanRequest => ({ documentId: documentId ?? "", endpoint, provider: "mpdf-api", model, budgetMicros: 1_000_000, currency: "USD", retention: "delete_after_result" });
  const prepare = async () => {
    setError(undefined);
    try { setManagedSummary(await prepareApiPlan(planRequest())); }
    catch (reason) { setError(String(reason)); }
  };
  const run = async () => {
    if (!managedSummary?.planDigest) return;
    setManagedProgress("running"); setError(undefined);
    try {
      const result = await runApiTask({ plan: planRequest(), consent: managedSummary.planDigest, profileId, route });
      setManagedProgress(result.state); setManagedCost(result.usedCostMicros);
      setManagedRetention(result.retention); setManagedFallback(result.fallbackReason ?? undefined); setManagedArtifact(result.artifactPath);
    } catch (reason) { setManagedProgress(undefined); setError(String(reason)); }
  };
  const cancel = async () => { if (onCancel) onCancel(); else await cancelApiTask(); };
  return <section className="api-routing-panel" aria-label="OCR route">
    <h2>OCR route</h2>
    <div role="group" aria-label="OCR route choices">
      {(["local", "api", "api_then_local"] as ApiRoute[]).map((value) => <button key={value} type="button" disabled={disabled} aria-pressed={route === value} onClick={() => choose(value)}>
        {value === "local" ? "Local" : value === "api" ? "Cloud enhanced" : "Cloud then local"}
      </button>)}
    </div>
    <p className="api-routing-note">{route === "local" ? "Offline processing; no network request." : "An explicit consent summary is required before upload."}</p>
    {documentId && route !== "local" && !summary && <div className="api-route-configuration">
      <label>API endpoint<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} placeholder="https://api.example.com/" /></label>
      <label>Credential profile<input value={profileId} onChange={(event) => setProfileId(event.target.value)} /></label>
      <label>OCR model<input value={model} onChange={(event) => setModel(event.target.value)} /></label>
      <button type="button" onClick={prepare} disabled={disabled || !endpoint}>Review upload plan</button>
    </div>}
    {activeSummary && route !== "local" && <div className="api-consent-summary" aria-label="API consent summary">
      <p>{activeSummary.origin} · {activeSummary.provider} / {activeSummary.model}</p>
      <p>Source {activeSummary.sourceDigest.slice(0, 12)}… · {activeSummary.sourceBytes} bytes · {activeSummary.pageCount} pages</p>
      <p>Budget {activeSummary.budgetMicros} micros · retention: {activeSummary.retention}</p>
      <label><input type="checkbox" checked={activeConfirmed} onChange={() => onConfirm ? onConfirm() : setManagedConfirmed((value) => !value)} disabled={disabled} /> Confirm this upload plan</label>
      {!summary && <button type="button" onClick={run} disabled={disabled || !activeConfirmed || !!activeProgress}>Start remote OCR</button>}
      {activeProgress && <p role="status">{activeProgress} · cost {activeCost} micros</p>}
      {managedRetention && <p>Provider retention: {managedRetention}</p>}
      {managedArtifact && <p>Durable result: {managedArtifact}</p>}
      {(fallbackReason || managedFallback) && <p role="alert">Fallback: {fallbackReason ?? managedFallback}</p>}
      {(onCancel || managedProgress === "running") && <button type="button" onClick={cancel} disabled={disabled}>Cancel task</button>}
    </div>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
