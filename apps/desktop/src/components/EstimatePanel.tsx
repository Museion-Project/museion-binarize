import type { EstimateState } from "../app/reducer";
import type { EstimateResult } from "../app/types";
import { formatBytes } from "../lib/formatting";

interface EstimatePanelProps {
  estimate: EstimateState;
  disabled: boolean;
  onEstimate: () => void;
}

/** Experimental output-size estimate, shown near the output controls.
 * Deliberately restrained wording — "~X MB" and a range, never an exact
 * number, always labeled Experimental. See docs/size-estimation.md. */
export function EstimatePanel({ estimate, disabled, onEstimate }: EstimatePanelProps) {
  return (
    <section className="estimate-panel" aria-label="Estimated output size">
      <div className="estimate-panel-header">
        <span className="estimate-panel-title">Estimated output</span>
        <button type="button" onClick={onEstimate} disabled={disabled || estimate.kind === "running"}>
          {estimate.kind === "idle" || estimate.kind === "failed" ? "Estimate" : "Re-estimate"}
        </button>
      </div>

      {estimate.kind === "idle" && <p className="estimate-panel-status">Not estimated</p>}

      {estimate.kind === "running" && (
        <p className="estimate-panel-status" role="status" aria-live="polite">
          Estimating…
        </p>
      )}

      {estimate.kind === "failed" && (
        <p className="estimate-panel-status estimate-panel-error" role="alert">
          Estimate failed: {estimate.error.message}
        </p>
      )}

      {(estimate.kind === "ready" || estimate.kind === "stale") && (
        <EstimateSummary
          result={estimate.kind === "ready" ? estimate.result : estimate.previous}
          stale={estimate.kind === "stale"}
        />
      )}
    </section>
  );
}

function EstimateSummary({ result, stale }: { result: EstimateResult; stale: boolean }) {
  return (
    <div className={stale ? "estimate-summary stale" : "estimate-summary"}>
      <p className="estimate-summary-central">~{formatBytes(result.estimatedOutputBytes)}</p>
      <p className="estimate-summary-range">
        Likely range {formatBytes(result.estimatedLowerBytes)}–
        {formatBytes(result.estimatedUpperBytes)}
      </p>
      <p className="estimate-summary-meta">
        Based on {result.sampledPages.length} sampled page
        {result.sampledPages.length === 1 ? "" : "s"} · Experimental
        {stale && " · Estimate outdated"}
      </p>
    </div>
  );
}
