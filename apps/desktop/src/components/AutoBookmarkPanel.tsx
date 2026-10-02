import { useEffect, useRef, useState } from "react";

import type {
  AutoBookmarkResult,
  AutoBookmarkStage,
  AutoBookmarkFailed,
  AutoBookmarkStageEvent,
  UiError,
} from "../app/types";
import {
  cancelAutoBookmark,
  onAutoBookmarkCancelled,
  onAutoBookmarkCompleted,
  onAutoBookmarkFailed,
  onAutoBookmarkStage,
  pickOutputDestination,
  pickPackageDirectory,
  startAutoBookmark,
} from "../lib/tauri";

const STAGE_LABELS: Record<AutoBookmarkStage, string> = {
  analyzing_toc: "Looking for a printed table of contents…",
  aligning: "Matching contents entries to headings in the text…",
  writing_pdf: "Writing the outlined PDF…",
  validating: "Reopening and verifying the output…",
  cancelled: "Cancelled.",
};

export interface AutoBookmarkPanelProps {
  documentId: string;
  packagePath: string;
  onPackagePathChange: (path: string) => void;
  /** Called after a run finishes so the bookmark tree can reload. */
  onFinished: () => void;
}

/**
 * One button for the whole automatic path. The user chooses the package and
 * where to save the new PDF; everything else — provider, thresholds, page
 * offsets — is the engine's business and is never asked of them.
 *
 * A document with no reliable structure is a normal result shown in place,
 * not an error: the panel says plainly that nothing was written.
 */
export function AutoBookmarkPanel({
  documentId,
  packagePath,
  onPackagePathChange,
  onFinished,
}: AutoBookmarkPanelProps) {
  const [outputPath, setOutputPath] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [regenerate, setRegenerate] = useState(false);
  const [jobId, setJobId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [stage, setStage] = useState<AutoBookmarkStage | null>(null);
  const [result, setResult] = useState<AutoBookmarkResult | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const activeJob = useRef<string | null>(null);
  const startGeneration = useRef(0);
  const pendingTerminal = useRef<
    | { kind: "completed"; event: AutoBookmarkResult }
    | { kind: "cancelled"; event: AutoBookmarkStageEvent }
    | { kind: "failed"; event: AutoBookmarkFailed }
    | null
  >(null);
  // Deliberately unresolved until the current effect has attached every
  // listener. This prevents a click in the initial render window from
  // dispatching an IPC run into an event-registration gap.
  const listenersReady = useRef<Promise<void>>(new Promise(() => {}));
  const finished = useRef(onFinished);
  finished.current = onFinished;

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    const registrations: Array<Promise<() => void>> = [];
    let disposed = false;
    const attach = (pending: Promise<() => void>) => {
      registrations.push(pending);
      void pending.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    };
    const belongsToDocument = (event: { documentId: string; jobId: string }) =>
      event.documentId === documentId &&
      (!activeJob.current || activeJob.current === event.jobId);
    const finish = () => {
      activeJob.current = null;
      setJobId(null);
      setStage(null);
    };
    attach(onAutoBookmarkStage((event) => {
      if (!belongsToDocument(event)) return;
      if (!activeJob.current) return;
      setStage(event.stage);
    }));
    attach(
      onAutoBookmarkCompleted((event) => {
        if (!belongsToDocument(event)) return;
        if (!activeJob.current) {
          pendingTerminal.current = { kind: "completed", event };
          return;
        }
        setResult(event);
        finish();
        finished.current();
      }),
    );
    attach(
      onAutoBookmarkCancelled((event) => {
        if (!belongsToDocument(event)) return;
        if (!activeJob.current) {
          pendingTerminal.current = { kind: "cancelled", event };
          return;
        }
        finish();
      }),
    );
    attach(
      onAutoBookmarkFailed((event) => {
        if (!belongsToDocument(event)) return;
        if (!activeJob.current) {
          pendingTerminal.current = { kind: "failed", event };
          return;
        }
        setError(event.error);
        finish();
      }),
    );
    listenersReady.current = Promise.all(registrations).then(() => undefined);
    return () => {
      disposed = true;
      startGeneration.current += 1;
      activeJob.current = null;
      pendingTerminal.current = null;
      // A panel instance is reused when the user switches documents. Clear
      // the old document's visible run as well as its event bookkeeping;
      // otherwise a stale job can leave the new document looking busy.
      setJobId(null);
      setStage(null);
      setResult(null);
      setError(null);
      setStarting(false);
      for (const unlisten of unlisteners) unlisten();
    };
  }, [documentId]);

  const running = starting || jobId !== null;
  const ready = packagePath.trim() !== "" && outputPath.trim() !== "" && !running;

  async function choosePackage() {
    const selected = await pickPackageDirectory();
    if (selected) onPackagePathChange(selected);
  }

  async function chooseOutput() {
    const selected = await pickOutputDestination("bookmarked.pdf");
    if (selected) setOutputPath(selected);
  }

  async function start() {
    if (running) return;
    const generation = ++startGeneration.current;
    setStarting(true);
    activeJob.current = null;
    pendingTerminal.current = null;
    setError(null);
    setResult(null);
    setStage("analyzing_toc");
    try {
      // Tauri event registration is asynchronous. Wait until all listeners
      // are attached before dispatching the command, closing the terminal
      // event gap for very fast runs.
      await listenersReady.current;
      if (generation !== startGeneration.current) return;
      const started = await startAutoBookmark({
        documentId,
        packagePath: packagePath.trim(),
        outputPath: outputPath.trim(),
        overwrite,
        regenerate,
      });
      if (generation !== startGeneration.current) return;
      const terminal = pendingTerminal.current as
        | { kind: "completed"; event: AutoBookmarkResult }
        | { kind: "cancelled"; event: AutoBookmarkStageEvent }
        | { kind: "failed"; event: AutoBookmarkFailed }
        | null;
      // Consume the handshake slot exactly once.  A terminal event from a
      // different (stale) job must not remain queued and contaminate the next
      // run after this IPC response establishes the real job id.
      pendingTerminal.current = null;
      activeJob.current = started.jobId;
      if (
        terminal &&
        terminal.kind === "completed" &&
        terminal.event.jobId === started.jobId &&
        terminal.event.documentId === started.documentId
      ) {
        setResult(terminal.event);
        activeJob.current = null;
        setJobId(null);
        setStage(null);
        finished.current();
      } else if (
        terminal &&
        terminal.kind === "failed" &&
        terminal.event.jobId === started.jobId &&
        terminal.event.documentId === started.documentId
      ) {
        setError(terminal.event.error);
        activeJob.current = null;
        setJobId(null);
        setStage(null);
      } else if (terminal && terminal.kind === "cancelled") {
        if (
          terminal.event.jobId !== started.jobId ||
          terminal.event.documentId !== started.documentId
        ) {
          setJobId(started.jobId);
          setStarting(false);
          return;
        }
        activeJob.current = null;
        setJobId(null);
        setStage(null);
      } else {
        setJobId(started.jobId);
      }
      setStarting(false);
    } catch (raw) {
      activeJob.current = null;
      setStage(null);
      setStarting(false);
      const failure = raw as { error?: UiError };
      setError(
        failure.error ?? { code: "internal_error", message: String(raw), hint: null, detail: null },
      );
    }
  }

  async function cancel() {
    if (!jobId) return;
    try {
      await cancelAutoBookmark(jobId, documentId);
    } catch {
      // The run already finished; its own event clears the state.
    }
  }

  return (
    <section className="auto-bookmark-panel" aria-label="Automatic table of contents">
      <h3>Add a table of contents automatically</h3>
      <p>
        Uses the book’s own evidence: an existing PDF outline, or its printed contents pages matched
        against the headings in the text. Nothing is guessed.
      </p>
      <div className="auto-bookmark-paths">
        <label htmlFor="auto-bookmark-package">MDP package folder</label>
        <div className="auto-bookmark-path-row">
          <input
            id="auto-bookmark-package"
            value={packagePath}
            onChange={(event) => onPackagePathChange(event.target.value)}
            placeholder="Choose the package folder"
            disabled={running}
          />
          <button type="button" onClick={() => void choosePackage()} disabled={running}>
            Choose folder…
          </button>
        </div>
        <label htmlFor="auto-bookmark-output">Save the new PDF as</label>
        <div className="auto-bookmark-path-row">
          <input
            id="auto-bookmark-output"
            value={outputPath}
            onChange={(event) => setOutputPath(event.target.value)}
            placeholder="Choose where to save the outlined PDF"
            disabled={running}
          />
          <button type="button" onClick={() => void chooseOutput()} disabled={running}>
            Choose file…
          </button>
        </div>
      </div>
      <div className="auto-bookmark-options">
        <label>
          <input
            type="checkbox"
            checked={overwrite}
            onChange={(event) => setOverwrite(event.target.checked)}
            disabled={running}
          />
          Replace the output file if it already exists
        </label>
        <label>
          <input
            type="checkbox"
            checked={regenerate}
            onChange={(event) => setRegenerate(event.target.checked)}
            disabled={running}
          />
          Replace existing bookmark candidates (refused while reviews exist)
        </label>
      </div>
      <div className="auto-bookmark-actions">
        <button type="button" className="primary" onClick={() => void start()} disabled={!ready}>
          Add bookmarks automatically
        </button>
        {jobId !== null && (
          <button type="button" onClick={() => void cancel()}>
            Cancel
          </button>
        )}
      </div>
      {stage && (
        <p role="status" className="auto-bookmark-stage">
          {STAGE_LABELS[stage]}
        </p>
      )}
      {result && (
        <div
          className={`auto-bookmark-result ${result.writtenBookmarks > 0 ? "succeeded" : "refused"}`}
          aria-label="Automatic bookmark result"
        >
          {result.writtenBookmarks > 0 ? (
            <>
              <p role="status">
                Added {result.autoConfirmed} reliable bookmark(s) automatically; {result.needsReview}{" "}
                need review and {result.skipped} were skipped for insufficient evidence. The output
                was reopened and verified.
              </p>
              <p>Saved to {result.outputPath}</p>
            </>
          ) : (
            <>
              <p role="status">
                No sufficiently reliable table of contents was found, so no guessed bookmarks were
                written to a PDF.
              </p>
              {result.safeRefusalReason && <p>{result.safeRefusalReason}</p>}
            </>
          )}
          <dl className="auto-bookmark-summary">
            <div>
              <dt>Automatically added</dt>
              <dd>{result.autoConfirmed}</dd>
            </div>
            <div>
              <dt>Needs review</dt>
              <dd>{result.needsReview}</dd>
            </div>
            <div>
              <dt>Skipped</dt>
              <dd>{result.skipped}</dd>
            </div>
            <div>
              <dt>Contents pages found</dt>
              <dd>{result.tocPageCount}</dd>
            </div>
          </dl>
          <p className="auto-bookmark-report">Full report: {result.reportPath}</p>
        </div>
      )}
      {error && (
        <p role="alert" className="auto-bookmark-error">
          {error.message}
          {error.hint ? ` ${error.hint}` : ""}
        </p>
      )}
    </section>
  );
}
