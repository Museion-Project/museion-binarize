import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { ProcessingCompleted } from "../app/types";
import { formatBytes, formatMicros, formatReduction } from "../lib/formatting";

interface CompletionPanelProps {
  report: ProcessingCompleted;
  onStartOver: () => void;
}

export function CompletionPanel({ report, onStartOver }: CompletionPanelProps) {
  const reduction = formatReduction(report.originalBytes, report.outputBytes);
  return (
    <div className="status-bar-completion" role="status" aria-live="polite">
      <span className="status-bar-completion-summary">
        Done — {report.pagesProcessed} pages, {formatBytes(report.outputBytes)}
        {reduction && ` (${reduction})`}, {formatMicros(report.elapsedUs)}
      </span>
      <span className="status-bar-completion-path" title={report.outputPath}>
        {report.outputPath}
      </span>
      <button type="button" onClick={() => revealItemInDir(report.outputPath).catch(() => {})}>
        Reveal in Finder
      </button>
      <button type="button" onClick={onStartOver}>
        Convert again
      </button>
    </div>
  );
}
