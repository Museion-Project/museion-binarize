import { useCallback, useEffect, useReducer, useState } from "react";

import "./App.css";
import { initialState, reducer } from "./app/reducer";
import { CompletionPanel } from "./components/CompletionPanel";
import { ApiRoutingPanel } from "./components/ApiRoutingPanel";
import { DocumentInfo } from "./components/DocumentInfo";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ErrorPanel } from "./components/ErrorPanel";
import { EstimatePanel } from "./components/EstimatePanel";
import { PageSidebar } from "./components/PageSidebar";
import { PasswordPrompt } from "./components/PasswordPrompt";
import { PreviewPane } from "./components/PreviewPane";
import { ReviewWorkbench } from "./components/ReviewWorkbench";
import { ProcessingProgress } from "./components/ProcessingProgress";
import { SettingsPanel } from "./components/SettingsPanel";
import { Toolbar } from "./components/Toolbar";
import { useEstimate } from "./hooks/useEstimate";
import { useProcessingEvents } from "./hooks/useProcessing";
import { usePreview } from "./hooks/usePreview";
import { defaultOutputFileName } from "./lib/settings";
import {
  BackendError,
  cancelProcessing,
  closeDocument,
  openDocument,
  onFileDragDrop,
  pickOutputDestination,
  pickPdfToOpen,
  projectInfo,
  startProcessing,
} from "./lib/tauri";

function App() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const [projectPhase, setProjectPhase] = useState<string | null>(null);
  const [pendingOpenPath, setPendingOpenPath] = useState<string | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [isFileDragActive, setIsFileDragActive] = useState(false);
  const [fileDropError, setFileDropError] = useState<string | null>(null);

  useEffect(() => {
    projectInfo()
      .then((info) => setProjectPhase(info.phase))
      .catch(() => setProjectPhase(null));
  }, []);

  useProcessingEvents(dispatch);

  usePreview(
    state.kind === "ready" ? state.document : null,
    state.kind === "ready" ? state.currentPage : 1,
    state.kind === "ready" ? state.settings : null,
    dispatch,
  );

  const runEstimate = useEstimate(
    state.kind === "ready" ? state.document : null,
    state.kind === "ready" ? state.settings : null,
    dispatch,
  );

  const activeJobId = state.kind === "processing" ? state.job.jobId : null;
  useEffect(() => {
    if (!activeJobId) {
      setElapsedSeconds(0);
      return;
    }
    const started = Date.now();
    const timer = window.setInterval(() => {
      setElapsedSeconds((Date.now() - started) / 1000);
    }, 500);
    return () => window.clearInterval(timer);
  }, [activeJobId]);

  const handleOpenPath = useCallback(async (path: string) => {
    if (state.kind === "processing") return;
    setFileDropError(null);
    dispatch({ type: "OPEN_STARTED" });
    try {
      const document = await openDocument(path);
      dispatch({ type: "OPEN_SUCCEEDED", document });
    } catch (error) {
      if (error instanceof BackendError && error.error.code === "password_required") {
        setPendingOpenPath(path);
        dispatch({ type: "PASSWORD_REQUIRED", path });
        return;
      }
      dispatch({
        type: "OPEN_FAILED",
        error: error instanceof BackendError ? error.error : { code: "internal_error", message: String(error), hint: null, detail: null },
      });
    }
  }, [state.kind]);

  const handleOpen = useCallback(async () => {
    if (state.kind === "processing" || state.kind === "opening") return;
    const path = await pickPdfToOpen();
    if (path) await handleOpenPath(path);
  }, [state.kind, handleOpenPath]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    onFileDragDrop((event) => {
      const canOpen = state.kind !== "processing" && state.kind !== "opening";

      if (event.type === "enter") {
        setIsFileDragActive(canOpen);
        return;
      }
      if (event.type === "leave") {
        setIsFileDragActive(false);
        return;
      }
      if (event.type !== "drop") return;

      setIsFileDragActive(false);
      if (!canOpen) return;
      if (event.paths.length !== 1 || !/\.pdf$/i.test(event.paths[0])) {
        setFileDropError("Drop a single PDF file.");
        return;
      }
      void handleOpenPath(event.paths[0]);
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => {
        // The native listener is unavailable when the frontend is run in
        // a regular browser; the Open PDF button remains fully functional.
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [state.kind, handleOpenPath]);

  const handlePasswordSubmit = useCallback(
    async (password: string) => {
      if (!pendingOpenPath) return;
      try {
        const document = await openDocument(pendingOpenPath, password);
        setPendingOpenPath(null);
        dispatch({ type: "OPEN_SUCCEEDED", document });
      } catch (error) {
        dispatch({
          type: "PASSWORD_RETRY_FAILED",
          message:
            error instanceof BackendError
              ? error.error.message
              : "The password was rejected.",
        });
      }
    },
    [pendingOpenPath],
  );

  const handlePasswordCancel = useCallback(() => {
    setPendingOpenPath(null);
    dispatch({ type: "DISMISS_ERROR" });
  }, []);

  const handleChooseOutput = useCallback(async () => {
    if (state.kind !== "ready") return;
    const suggested = defaultOutputFileName(state.document.fileName);
    const path = await pickOutputDestination(suggested);
    if (path) dispatch({ type: "SET_OUTPUT_PATH", path });
  }, [state]);

  const handleConvert = useCallback(async () => {
    if (state.kind !== "ready" || !state.outputPath) return;
    try {
      const started = await startProcessing({
        documentId: state.document.documentId,
        outputPath: state.outputPath,
        settings: state.settings,
        overwrite: true,
      });
      dispatch({
        type: "PROCESSING_STARTED",
        jobId: started.jobId,
        pageCount: started.pageCount,
        outputPath: state.outputPath,
      });
    } catch (error) {
      dispatch({
        type: "OPEN_FAILED",
        error: error instanceof BackendError ? error.error : { code: "internal_error", message: String(error), hint: null, detail: null },
      });
    }
  }, [state]);

  const handleCancel = useCallback(() => {
    if (state.kind !== "processing") return;
    dispatch({ type: "CANCEL_REQUESTED", jobId: state.job.jobId });
    cancelProcessing(state.job.jobId).catch(() => {});
  }, [state]);

  const handleStartOver = useCallback(async () => {
    dispatch({ type: "START_OVER" });
  }, []);

  const handleCloseAndDismiss = useCallback(() => {
    closeDocument().catch(() => {});
    dispatch({ type: "DISMISS_ERROR" });
  }, []);

  // Keyboard shortcuts: Cmd/Ctrl+O to open, Cmd/Ctrl+Enter to convert,
  // +/- and 0 to zoom, arrow keys to change page when the preview has
  // focus. Never intercepted while a text/number input is focused.
  useEffect(() => {
    function isTextInput(target: EventTarget | null): boolean {
      return (
        target instanceof HTMLElement &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      );
    }

    function onKeyDown(event: KeyboardEvent) {
      if (isTextInput(event.target)) return;
      const meta = event.metaKey || event.ctrlKey;

      if (meta && event.key.toLowerCase() === "o") {
        event.preventDefault();
        handleOpen();
        return;
      }
      if (meta && event.key === "Enter") {
        event.preventDefault();
        handleConvert();
        return;
      }
      if (state.kind !== "ready") return;

      if (event.key === "+" || event.key === "=") {
        dispatch({ type: "SET_ZOOM", zoom: Math.min(4, (state.zoom || 1) + 0.25) });
      } else if (event.key === "-") {
        dispatch({ type: "SET_ZOOM", zoom: Math.max(0.25, (state.zoom || 1) - 0.25) });
      } else if (event.key === "0") {
        dispatch({ type: "SET_ZOOM", zoom: 0 });
      } else if (event.key === "ArrowRight" || event.key === "ArrowDown") {
        const next = Math.min(state.document.pageCount, state.currentPage + 1);
        dispatch({ type: "SELECT_PAGE", page: next });
      } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
        const prev = Math.max(1, state.currentPage - 1);
        dispatch({ type: "SELECT_PAGE", page: prev });
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [state, handleOpen, handleConvert]);

  const isProcessing = state.kind === "processing";
  const isCancelling = state.kind === "processing" && state.cancelling;

  return (
    <ErrorBoundary>
      <main className="app">
        <Toolbar
          fileName={state.kind === "ready" || state.kind === "processing" || state.kind === "completed" || state.kind === "cancelled" ? state.document.fileName : null}
          sourceBytes={"document" in state && state.document ? state.document.sourceBytes : null}
          pageCount={"document" in state && state.document ? state.document.pageCount : null}
          outputPath={state.kind === "ready" ? state.outputPath : state.kind === "processing" ? state.outputPath : null}
          canOpen={state.kind !== "processing" && state.kind !== "opening"}
          canChooseOutput={state.kind === "ready"}
          canConvert={state.kind === "ready" && state.outputPath !== null}
          isProcessing={isProcessing}
          isCancelling={isCancelling}
          onOpen={handleOpen}
          onChooseOutput={handleChooseOutput}
          onConvert={handleConvert}
          onCancel={handleCancel}
        />

        {state.kind === "idle" && (
          <div className="empty-state">
            <p>Open or drop a scanned PDF to inspect, preview, and convert it.</p>
            <p className="empty-state-hint">
              All processing runs locally. No file is uploaded to any network service.
            </p>
            {projectPhase && <p className="empty-state-phase">{projectPhase}</p>}
          </div>
        )}

        {isFileDragActive && (
          <div className="file-drop-overlay" role="status" aria-live="polite">
            <span>Drop PDF to open</span>
          </div>
        )}

        {fileDropError && (
          <div className="file-drop-error" role="alert">
            <span>{fileDropError}</span>
            <button
              type="button"
              aria-label="Dismiss file drop error"
              onClick={() => setFileDropError(null)}
            >
              ×
            </button>
          </div>
        )}

        {state.kind === "opening" && (
          <div className="empty-state" role="status" aria-live="polite">
            <p>Opening document…</p>
          </div>
        )}

        {(state.kind === "ready" ||
          state.kind === "processing" ||
          state.kind === "completed" ||
          state.kind === "cancelled") && (
          <div className="workspace">
            <PageSidebar
              document={state.document}
              currentPage={state.kind === "ready" ? state.currentPage : 1}
              onSelect={(page) =>
                state.kind === "ready" && dispatch({ type: "SELECT_PAGE", page })
              }
            />

            <div className="workspace-main">
              <DocumentInfo
                document={state.document}
                currentPage={state.kind === "ready" ? state.currentPage : 1}
              />
              {state.kind === "ready" ? (
                <PreviewPane
                  preview={state.preview}
                  viewMode={state.viewMode}
                  zoom={state.zoom}
                  pageNumber={state.currentPage}
                  onViewModeChange={(mode) => dispatch({ type: "SET_VIEW_MODE", mode })}
                  onZoomChange={(zoom) => dispatch({ type: "SET_ZOOM", zoom })}
                />
              ) : (
                <div className="preview-pane-disabled" aria-hidden="true" />
              )}
            </div>

            {state.kind === "ready" && (
              <div className="settings-column">
                <SettingsPanel
                  settings={state.settings}
                  preset={state.preset}
                  disabled={false}
                  onChange={(settings, preset) =>
                    dispatch({ type: "SET_SETTINGS", settings, preset })
                  }
                />
                <ApiRoutingPanel documentId={state.document.documentId} disabled={false} />
                <EstimatePanel estimate={state.estimate} disabled={false} onEstimate={runEstimate} />
              </div>
            )}
          </div>
        )}

        {state.kind === "ready" && <ReviewWorkbench documentId={state.document.documentId} />}

        <footer className="status-bar">
          {state.kind === "processing" && (
            <ProcessingProgress
              job={state.job}
              cancelling={state.cancelling}
              elapsedSeconds={elapsedSeconds}
            />
          )}
          {state.kind === "completed" && (
            <CompletionPanel report={state.report} onStartOver={handleStartOver} />
          )}
          {state.kind === "cancelled" && (
            <div className="status-bar-cancelled" role="status">
              <span>Cancelled — no output was written.</span>
              <button type="button" onClick={handleStartOver}>
                Convert again
              </button>
            </div>
          )}
        </footer>

        {state.kind === "failed" && (
          <ErrorPanel error={state.error} onDismiss={handleCloseAndDismiss} />
        )}

        {state.kind === "passwordRequired" && (
          <PasswordPrompt
            fileName={state.path.split(/[/\\]/).pop() ?? state.path}
            attemptError={state.attemptError}
            onSubmit={handlePasswordSubmit}
            onCancel={handlePasswordCancel}
          />
        )}
      </main>
    </ErrorBoundary>
  );
}

export default App;
