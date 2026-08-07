import { formatPercent } from "../lib/formatting";
import type { JobProgress } from "../app/reducer";

interface ProcessingProgressProps {
  job: JobProgress;
  cancelling: boolean;
  elapsedSeconds: number;
}

const STAGE_LABELS: Record<string, string> = {
  queued: "Queued",
  opening: "Opening",
  rendering: "Rendering",
  binarizing: "Binarizing",
  encoding: "Encoding",
  writing: "Writing",
  validating: "Validating",
};

export function ProcessingProgress({ job, cancelling, elapsedSeconds }: ProcessingProgressProps) {
  const stageLabel = STAGE_LABELS[job.stage] ?? job.stage;
  return (
    <div className="status-bar-progress" role="status" aria-live="polite">
      <progress value={job.fraction} max={1} aria-label="Conversion progress" />
      <span>
        {cancelling ? "Cancelling…" : stageLabel}
        {job.pageNumber !== null && ` — page ${job.pageNumber} of ${job.pageCount}`}
        {" · "}
        {formatPercent(job.fraction)}
        {" · "}
        {elapsedSeconds.toFixed(0)}s elapsed
      </span>
    </div>
  );
}
