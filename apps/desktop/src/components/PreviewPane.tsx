import type { PreviewPaneState } from "../app/reducer";

interface PreviewPaneProps {
  preview: PreviewPaneState;
  viewMode: "original" | "processed";
  zoom: number;
  pageNumber: number;
  onViewModeChange: (mode: "original" | "processed") => void;
  onZoomChange: (zoom: number) => void;
}

const ZOOM_STEP = 0.25;
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;

export function PreviewPane({
  preview,
  viewMode,
  zoom,
  pageNumber,
  onViewModeChange,
  onZoomChange,
}: PreviewPaneProps) {
  const dataUrl = viewMode === "original" ? preview.originalDataUrl : preview.processedDataUrl;

  return (
    <section className="preview-pane" aria-label={`Page ${pageNumber} preview`}>
      <div className="preview-toolbar">
        <div className="preview-toggle" role="radiogroup" aria-label="Preview mode">
          <button
            type="button"
            role="radio"
            aria-checked={viewMode === "original"}
            className={viewMode === "original" ? "selected" : ""}
            onClick={() => onViewModeChange("original")}
          >
            Original
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={viewMode === "processed"}
            className={viewMode === "processed" ? "selected" : ""}
            onClick={() => onViewModeChange("processed")}
          >
            Processed
          </button>
        </div>

        <div className="preview-zoom">
          <button
            type="button"
            aria-label="Zoom out"
            title="Zoom out (-)"
            onClick={() => onZoomChange(Math.max(MIN_ZOOM, (zoom || 1) - ZOOM_STEP))}
          >
            −
          </button>
          <button type="button" aria-label="Fit page" title="Fit page (0)" onClick={() => onZoomChange(0)}>
            Fit
          </button>
          <button
            type="button"
            aria-label="Zoom to 100%"
            title="100% (0)"
            onClick={() => onZoomChange(1)}
          >
            100%
          </button>
          <button
            type="button"
            aria-label="Zoom in"
            title="Zoom in (+)"
            onClick={() => onZoomChange(Math.min(MAX_ZOOM, (zoom || 1) + ZOOM_STEP))}
          >
            +
          </button>
          {zoom > 0 && <span className="preview-zoom-level">{Math.round(zoom * 100)}%</span>}
        </div>
      </div>

      <div className="preview-canvas" aria-live="polite">
        {preview.error ? (
          <p className="preview-error">Preview failed: {preview.error.message}</p>
        ) : dataUrl ? (
          <img
            src={dataUrl}
            alt={`Page ${pageNumber}, ${viewMode} rendering`}
            className={zoom === 0 ? "fit" : undefined}
            style={zoom > 0 ? { width: `${zoom * 100}%` } : undefined}
          />
        ) : (
          <p className="preview-placeholder">
            {preview.loading ? "Rendering preview…" : "No preview yet"}
          </p>
        )}
      </div>
    </section>
  );
}
