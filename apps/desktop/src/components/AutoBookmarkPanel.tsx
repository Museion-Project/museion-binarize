import { useEffect, useRef, useState } from "react";

import type {
  AutoBookmarkResult,
  AutoBookmarkStage,
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
  const [stage, setStage] = useState<AutoBookmarkStage | null>(null);
  const [result, setResult] = useState<AutoBookmarkResult | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const finished = useRef(onFinished);
  finished.current = onFinished;

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;
    const attach = (pending: Promise<() => void>) => {
      void pending.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    };
    attach(onAutoBookmarkStage((event) => setStage(event.stage)));
    attach(
      onAutoBookmarkCompleted((event) => {
        setResult(event);
        setStage(null);
        setJobId(null);
        finished.current();
      }),
    );
    attach(
      onAutoBookmarkCancelled(() => {
        setStage(null);
        setJobId(null);
      }),
    );
    attach(
      onAutoBookmarkFailed((event) => {
        setError(event.error);
        setStage(null);
        setJobId(null);
      }),
    );
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  const running = jobId !== null;
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
    setError(null);
    setResult(null);
    try {
      const started = await startAutoBookmark({
        documentId,
        packagePath: packagePath.trim(),
        outputPath: outputPath.trim(),
        overwrite,
        regenerate,
      });
      setJobId(started.jobId);
      setStage("analyzing_toc");
    } catch (raw) {
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
        {running && (
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
