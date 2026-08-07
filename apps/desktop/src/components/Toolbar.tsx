import { formatBytes } from "../lib/formatting";

interface ToolbarProps {
  fileName: string | null;
  sourceBytes: number | null;
  pageCount: number | null;
  outputPath: string | null;
  canOpen: boolean;
  canChooseOutput: boolean;
  canConvert: boolean;
  isProcessing: boolean;
  isCancelling: boolean;
  onOpen: () => void;
  onChooseOutput: () => void;
  onConvert: () => void;
  onCancel: () => void;
}

export function Toolbar(props: ToolbarProps) {
  return (
    <header className="toolbar" role="toolbar" aria-label="Document actions">
      <button type="button" onClick={props.onOpen} disabled={!props.canOpen}>
        Open PDF
      </button>

      {props.fileName && (
        <span className="toolbar-file" title={props.fileName}>
          {props.fileName}
          {props.sourceBytes !== null && (
            <span className="toolbar-file-size"> · {formatBytes(props.sourceBytes)}</span>
          )}
          {props.pageCount !== null && (
            <span className="toolbar-file-size">
              {" "}
              · {props.pageCount} {props.pageCount === 1 ? "page" : "pages"}
            </span>
          )}
        </span>
      )}

      <div className="toolbar-spacer" />

      <button type="button" onClick={props.onChooseOutput} disabled={!props.canChooseOutput}>
        {props.outputPath ? "Change output…" : "Choose output…"}
      </button>
      {props.outputPath && (
        <span className="toolbar-output" title={props.outputPath}>
          {props.outputPath}
        </span>
      )}

      {props.isProcessing ? (
        <button
          type="button"
          className="danger"
          onClick={props.onCancel}
          disabled={props.isCancelling}
        >
          {props.isCancelling ? "Cancelling…" : "Cancel"}
        </button>
      ) : (
        <button
          type="button"
          className="primary"
          onClick={props.onConvert}
          disabled={!props.canConvert}
          title="Convert (Cmd/Ctrl+Enter)"
        >
          Convert
        </button>
      )}
    </header>
  );
}
