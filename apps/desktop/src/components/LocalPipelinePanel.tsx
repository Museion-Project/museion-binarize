import { useEffect, useRef, useState } from "react";

import type {
  LocalOcrReadiness,
  LocalPipelineCompleted,
  LocalPipelineStageEvent,
  PipelineStage,
  ProcessingSettings,
  ReviewPolicy,
  UiError,
} from "../app/types";
import { PIPELINE_STAGE_LABELS } from "../app/types";
import { DEFAULT_SELECTION } from "../app/ocrProvider";
import type { OcrProviderSelection } from "../app/ocrProvider";
import { OcrProviderPicker } from "./OcrProviderPicker";
import {
  cancelLocalPipeline,
  localOcrReadiness,
  onLocalPipelineCancelled,
  onLocalPipelineCompleted,
  onLocalPipelineFailed,
  onLocalPipelineStage,
  pickOutputDestination,
  pickPackageDirectory,
  startLocalPipeline,
} from "../lib/tauri";

export interface LocalPipelinePanelProps {
  documentId: string;
  settings: ProcessingSettings;
  /** Suggested output name, derived from the open document. */
  defaultOutputName: string;
  /** Pages in the open document, so cloud consent can name a real number. */
  pageCount?: number;
}

/**
 * The one main button: **OCR → bookmarks → binarize**.
 *
 * A person using this chooses the finished file's destination and presses
 * Start. They are never asked which OCR provider to use, which model, what a
 * TOC offset is, or what threshold to pick — those live behind "Advanced",
 * and every default is the one the gold evaluation cleared.
 *
 * The order is the backend's, not this component's: the stage list arrives
 * from `start_local_pipeline` so the UI cannot drift out of step with the
 * core orchestrator. Text is always recognized from the *original* page
 * images; the binarized pages are produced afterwards and never re-read.
 */
export function LocalPipelinePanel({
  documentId,
  settings,
  defaultOutputName,
  pageCount = 0,
}: LocalPipelinePanelProps) {
  const [outputPath, setOutputPath] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [languageProfile, setLanguageProfile] = useState("auto");
  const [sidecarPath, setSidecarPath] = useState("");
  const [modelDir, setModelDir] = useState("");
  const [onReview, setOnReview] = useState<ReviewPolicy>("pause");
  const [overwrite, setOverwrite] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [provider, setProvider] = useState<OcrProviderSelection>(DEFAULT_SELECTION);

  const [readiness, setReadiness] = useState<LocalOcrReadiness | null>(null);
  const [stages, setStages] = useState<PipelineStage[]>([]);
  const [stage, setStage] = useState<PipelineStage | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [resumed, setResumed] = useState(false);
  const [starting, setStarting] = useState(false);
  const [result, setResult] = useState<LocalPipelineCompleted | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const activeJob = useRef<string | null>(null);

  const running = jobId !== null;

  // Optional-plugin readiness is re-checked whenever its profile or explicit
  // paths change. It informs scanned-page handling; it does not block a base
  // run because reliable native-text PDFs need no OCR plugin.
  useEffect(() => {
    let cancelled = false;
    localOcrReadiness({
      languageProfile,
      providerExecutable: sidecarPath || null,
      modelDir: modelDir || null,
    })
      .then((next) => {
        if (!cancelled) setReadiness(next);
      })
      .catch(() => {
        if (!cancelled) setReadiness(null);
      });
    return () => {
      cancelled = true;
    };
  }, [languageProfile, sidecarPath, modelDir]);

  useEffect(() => {
    const unlisteners: Array<Promise<() => void>> = [
      onLocalPipelineStage((payload: LocalPipelineStageEvent) => {
        if (payload.jobId !== activeJob.current) return;
        setStage(payload.stage);
      }),
      onLocalPipelineCompleted((payload) => {
        if (payload.jobId !== activeJob.current) return;
        activeJob.current = null;
        setJobId(null);
        setStage(null);
        setResult(payload);
      }),
      onLocalPipelineCancelled((payload) => {
        if (payload.jobId !== activeJob.current) return;
        activeJob.current = null;
        setJobId(null);
        setStage("cancelled");
      }),
      onLocalPipelineFailed((payload) => {
        if (payload.jobId !== activeJob.current) return;
        activeJob.current = null;
        setJobId(null);
        setStage(null);
        setError(payload.error);
      }),
    ];
    return () => {
      unlisteners.forEach((pending) => {
        pending.then((off) => off()).catch(() => {});
      });
    };
  }, []);

  async function chooseOutput() {
    const chosen = await pickOutputDestination(defaultOutputName);
    if (chosen) setOutputPath(chosen);
  }

  async function chooseWorkspace() {
    const packageName = defaultOutputName.replace(/\.pdf$/i, ".mdp");
    const chosen = await pickPackageDirectory(packageName);
    if (chosen) setWorkspacePath(chosen);
  }

  async function start(policy: ReviewPolicy) {
    if (!outputPath || !workspacePath) return;
    setStarting(true);
    setError(null);
    setResult(null);
    try {
      const started = await startLocalPipeline({
        documentId,
        outputPath,
        workspacePath,
        settings,
        languageProfile,
        providerExecutable: sidecarPath || null,
        modelDir: modelDir || null,
        onReview: policy,
        overwrite,
        ocrProviderMode: provider.mode,
        cloudConsent: provider.cloudConsent,
        maxCredits: provider.maxCredits,
      });
      activeJob.current = started.jobId;
      setJobId(started.jobId);
      setStages(started.stages);
      setResumed(started.resumed);
      setStage(started.stages[0] ?? null);
    } catch (reason) {
      const failure = reason as { error?: UiError };
      setError(
        failure.error ?? {
          code: "internal_error",
          message: String(reason),
          hint: null,
          detail: null,
        },
      );
    } finally {
      setStarting(false);
    }
  }

  async function cancel() {
    try {
      await cancelLocalPipeline();
    } catch {
      // Already finished between render and click; the events settle state.
    }
  }

  const canStart =
    !running &&
    !starting &&
    provider.ready &&
    Boolean(outputPath) &&
    Boolean(workspacePath);

  return (
    <section className="local-pipeline-panel" aria-label="Convert this document">
      <h2>OCR → bookmarks → binarize</h2>
      <p className="local-pipeline-note">
        Text is recognized from the original pages first, then bookmarks are
        compiled from that text, and only then are the pages binarized. The
        finished PDF is black-and-white, searchable, and bookmarked. By
        default everything runs on this machine.
      </p>

      <OcrProviderPicker
        value={provider}
        onChange={setProvider}
        disabled={running}
        pageCount={pageCount}
      />

      <div className="local-pipeline-destination">
        <button type="button" onClick={chooseOutput} disabled={running}>
          Choose the finished PDF…
        </button>
        <span title={outputPath}>{outputPath || "No destination chosen"}</span>
      </div>
      <div className="local-pipeline-destination">
        <button type="button" onClick={chooseWorkspace} disabled={running}>
          Choose a working folder…
        </button>
        <span title={workspacePath}>
          {workspacePath || "No working folder chosen"}
        </span>
      </div>
      <p className="local-pipeline-hint">
        The working folder keeps the recognized text and the bookmark decisions,
        so a cancelled or interrupted run resumes instead of starting over.
      </p>

      <label>
        <input
          type="checkbox"
          checked={overwrite}
          onChange={() => setOverwrite((value) => !value)}
          disabled={running}
        />{" "}
        Replace the destination if it already exists
      </label>

      {readiness && !readiness.ready && (
        <p role="status" className="local-pipeline-readiness">
          Optional local OCR plugin is not ready: {readiness.diagnostic}
          {readiness.missingFiles.length > 0 && (
            <> Missing: {readiness.missingFiles.join(", ")}.</>
          )}
          {" "}Native-text PDFs can still start; a scanned page will stop with
          an installation error rather than producing an empty text layer.
        </p>
      )}
      {readiness?.ready && (
        <p className="local-pipeline-readiness">
          Ready · {readiness.languageProfile} · {readiness.modelSet ?? "model set unknown"}
          {readiness.modelLicense && <> · {readiness.modelLicense}</>}
        </p>
      )}

      <div className="local-pipeline-actions">
        <button type="button" onClick={() => start(onReview)} disabled={!canStart}>
          {resumed ? "Resume" : "Start"}
        </button>
        <button type="button" onClick={cancel} disabled={!running}>
          Cancel
        </button>
      </div>

      {running && (
        <ol className="local-pipeline-stages" aria-label="Progress">
          {stages
            .filter((item) => item !== "awaiting_review" || onReview === "pause")
            .map((item) => (
              <li
                key={item}
                aria-current={item === stage ? "step" : undefined}
                data-state={
                  item === stage
                    ? "active"
                    : stages.indexOf(item) < stages.indexOf(stage ?? item)
                      ? "done"
                      : "pending"
                }
              >
                {PIPELINE_STAGE_LABELS[item]}
              </li>
            ))}
        </ol>
      )}
      {!running && stage === "cancelled" && (
        <p role="status">
          Cancelled. Nothing was written, and the recognized pages were kept —
          pressing Resume continues from where it stopped.
        </p>
      )}

      {result?.status === "awaiting_review" && (
        <div className="local-pipeline-review" role="status">
          <p>
            {result.bookmarksNeedingReview} bookmark
            {result.bookmarksNeedingReview === 1 ? "" : "s"} need your decision
            before the final PDF is written. Nothing has been written yet.
          </p>
          <p>
            Review them below, then continue. You can also write only the
            entries that were already confirmed.
          </p>
          <button type="button" onClick={() => start("reviewed")} disabled={running}>
            I have reviewed — continue
          </button>
          <button type="button" onClick={() => start("confirmed")} disabled={running}>
            Continue with confirmed entries only
          </button>
        </div>
      )}

      {result?.status === "completed" && (
        <div className="local-pipeline-result" role="status">
          <p>
            Done — {result.pageCount} page{result.pageCount === 1 ? "" : "s"},{" "}
            {result.textLayerWords} searchable word
            {result.textLayerWords === 1 ? "" : "s"},{" "}
            {result.bookmarksWritten} bookmark
            {result.bookmarksWritten === 1 ? "" : "s"}.
          </p>
          <p title={result.outputPath ?? ""}>{result.outputPath}</p>
          <p>Recognized with: {result.providerMode}</p>
          {result.cloudFallbackPages.length > 0 && (
            <p role="alert" className="local-pipeline-fallback">
              {result.cloudFallbackPages.length} page
              {result.cloudFallbackPages.length === 1 ? "" : "s"} were recognized
              on this machine instead of by the cloud provider:{" "}
              {result.cloudFallbackPages.join(", ")}.
            </p>
          )}
          {result.creditsReserved > 0 && (
            <p>
              Credits: {result.creditsCharged} charged, {result.creditsReleased}{" "}
              released, {result.creditsRefunded} refunded
              {result.creditsSettled ? "" : " — not settled"}.
            </p>
          )}
          {(result.cloudInputTokens > 0 || result.cloudOutputTokens > 0) && (
            <p>
              Provider usage: {result.cloudInputTokens} input tokens,{" "}
              {result.cloudOutputTokens} output tokens.
            </p>
          )}
          {result.cloudUsageMayBeIncomplete && (
            <p role="alert">
              At least one cloud request ended without a confirmed response. Token and
              cost totals are lower bounds and may be incomplete.
            </p>
          )}
          {result.notProductionReadyReason && (
            <p role="alert">
              This provider mode is not production ready:{" "}
              {result.notProductionReadyReason}
            </p>
          )}
          {/* A warning, not an error: the converted PDF above exists and was
              verified. Only the outline was left empty rather than invented. */}
          {result.bookmarkStatus === "safe_refusal" && (
            <p role="status" className="local-pipeline-refusal">
              No reliable structure was found, so no bookmark was invented. The
              document was still converted and is searchable.{" "}
              {result.refusalReason}
            </p>
          )}
          {result.ocrErrors > 0 && (
            <p role="alert">
              {result.ocrErrors} page{result.ocrErrors === 1 ? "" : "s"} could not
              be recognized and have no searchable text.
            </p>
          )}
        </div>
      )}

      {error && (
        <p role="alert" className="local-pipeline-error">
          {error.message}
          {error.hint && <> {error.hint}</>}
        </p>
      )}

      <details
        open={advanced}
        onToggle={(event) => setAdvanced((event.target as HTMLDetailsElement).open)}
      >
        <summary>Advanced</summary>
        <label>
          Languages on the page
          <select
            value={languageProfile}
            onChange={(event) => setLanguageProfile(event.target.value)}
            disabled={running}
          >
            {(readiness?.availableProfiles ?? ["auto"]).map((profile) => (
              <option key={profile} value={profile}>
                {profile}
              </option>
            ))}
          </select>
        </label>
        <label>
          Text recognition sidecar
          <input
            value={sidecarPath}
            onChange={(event) => setSidecarPath(event.target.value)}
            placeholder="scripts/ocr/mpdf_ocr_sidecar.py"
            disabled={running}
          />
        </label>
        <label>
          Model folder
          <input
            value={modelDir}
            onChange={(event) => setModelDir(event.target.value)}
            placeholder="a folder provisioned by scripts/ocr/provision_models.py"
            disabled={running}
          />
        </label>
        <label>
          When entries need review
          <select
            value={onReview}
            onChange={(event) => setOnReview(event.target.value as ReviewPolicy)}
            disabled={running}
          >
            <option value="pause">Stop and let me decide</option>
            <option value="confirmed">Write only what is already confirmed</option>
            <option value="reviewed">Use the decisions I have already made</option>
          </select>
        </label>
        <p className="local-pipeline-hint">
          Models are never downloaded automatically. Provision them once with{" "}
          <code>scripts/ocr/provision_models.py</code>.
        </p>
      </details>
    </section>
  );
}
