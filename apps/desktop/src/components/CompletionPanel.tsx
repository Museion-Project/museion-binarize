import { useState } from "react";

import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { ProcessingCompleted } from "../app/types";
import { formatBytes, formatMicros, formatPercent } from "../lib/formatting";

interface CompletionPanelProps {
  report: ProcessingCompleted;
  onStartOver: () => void;
}

/** Primary summary + expandable advanced details — not twenty metrics at
 * once. See docs/desktop.md, "GUI completion improvements". */
export function CompletionPanel({ report, onStartOver }: CompletionPanelProps) {
  const [expanded, setExpanded] = useState(false);
  const reductionLabel =
    report.sizeReductionFraction !== null
      ? `${formatPercent(report.sizeReductionFraction)} smaller`
      : null;

  return (
    <div className="status-bar-completion" role="status" aria-live="polite">
      <span className="status-bar-completion-summary">
        Done — {formatBytes(report.outputBytes)}
        {reductionLabel && `, ${reductionLabel}`}, {formatMicros(report.elapsedUs)},{" "}
        {report.pagesProcessed} page{report.pagesProcessed === 1 ? "" : "s"}
      </span>
      <span className="status-bar-completion-path" title={report.outputPath}>
        {report.outputPath}
      </span>
      <button
        type="button"
        className="status-bar-completion-details-toggle"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        {expanded ? "Hide details" : "Details"}
      </button>
      <button type="button" onClick={() => revealItemInDir(report.outputPath).catch(() => {})}>
        Reveal in Finder
      </button>
      <button type="button" onClick={onStartOver}>
        Convert again
      </button>

      {expanded && (
        <dl className="status-bar-completion-details">
          <dt>Input size</dt>
          <dd>{formatBytes(report.originalBytes)}</dd>
          {report.inputToOutputRatio !== null && (
            <>
              <dt>Input/output ratio</dt>
              <dd>{report.inputToOutputRatio.toFixed(2)}:1</dd>
            </>
          )}
          <dt>Median page time</dt>
          <dd>{formatMicros(report.medianProcessingDurationUs)}</dd>
          <dt>Overall ink ratio</dt>
          <dd>{formatPercent(report.overallBlackPixelRatio)}</dd>
          {report.slowestPage && (
            <>
              <dt>Slowest page</dt>
              <dd>
                page {report.slowestPage.pageNumber} ({formatMicros(report.slowestPage.value)})
              </dd>
            </>
          )}
          {report.largestEncodedPage && (
            <>
              <dt>Largest encoded page</dt>
              <dd>
                page {report.largestEncodedPage.pageNumber} (
                {formatBytes(report.largestEncodedPage.value)})
              </dd>
            </>
          )}
          {report.smallestEncodedPage && (
            <>
              <dt>Smallest encoded page</dt>
              <dd>
                page {report.smallestEncodedPage.pageNumber} (
                {formatBytes(report.smallestEncodedPage.value)})
              </dd>
            </>
          )}
          {report.estimateComparison && (
            <>
              <dt>Prior estimate</dt>
              <dd>
                {formatBytes(report.estimateComparison.estimatedOutputBytes)} estimated vs.{" "}
                {formatBytes(report.estimateComparison.actualOutputBytes)} actual (
                {formatPercent(report.estimateComparison.relativeErrorFraction)} error)
              </dd>
            </>
          )}
        </dl>
      )}
    </div>
  );
}
