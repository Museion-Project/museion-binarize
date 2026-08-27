import { useState } from "react";

import type { ReviewIssue } from "../app/types";
import { addReviewRevision, loadReviewQueue } from "../lib/tauri";

export function ReviewWorkbench() {
  const [packagePath, setPackagePath] = useState("");
  const [issues, setIssues] = useState<ReviewIssue[]>([]);
  const [selected, setSelected] = useState<ReviewIssue | null>(null);
  const [text, setText] = useState("");
  const [aiSuggested, setAiSuggested] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<ReviewIssue["kind"] | "all">("all");
  const visibleIssues = kindFilter === "all" ? issues : issues.filter((issue) => issue.kind === kindFilter);

  async function refresh() {
    if (!packagePath.trim()) return;
    try {
      const next = await loadReviewQueue(packagePath.trim());
      setIssues(next);
      setSelected(next[0] ?? null);
      setMessage(null);
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function submitRevision() {
    if (!selected || !text) return;
    try {
      await addReviewRevision({
        packagePath: packagePath.trim(),
        targetRef: selected.targetRef,
        baseEvidenceDigest: selected.baseEvidenceDigest,
        text,
        aiSuggested,
      });
      setMessage("Revision saved; original OCR evidence remains unchanged.");
      setText("");
    } catch (error) {
      setMessage(String(error));
    }
  }

  return (
    <section className="review-workbench" aria-label="Review workbench">
      <header className="review-workbench-header">
        <h2>Review workbench</h2>
        <p>Local evidence only. Page and bbox references are shown; no fake image geometry is rendered.</p>
        <label>
          MDP package path
          <input value={packagePath} onChange={(event) => setPackagePath(event.target.value)} placeholder="/path/to/package" />
        </label>
        <button type="button" onClick={() => void refresh()}>Load review queue</button>
        <label>
          Filter kind
          <select value={kindFilter} onChange={(event) => setKindFilter(event.target.value as ReviewIssue["kind"] | "all")}>
            <option value="all">All</option><option value="low_confidence">Low confidence</option>
            <option value="reading_order_gap">Reading order</option><option value="unicode_normalization">Unicode</option>
            <option value="empty_region">Empty region</option>
          </select>
        </label>
      </header>
      <div className="review-workbench-columns">
        <aside className="review-issues" aria-label="Review issues">
          <strong>{visibleIssues.length} issue(s)</strong>
          {visibleIssues.map((issue) => (
            <button key={issue.issueId} type="button" className={selected?.issueId === issue.issueId ? "selected" : ""} onClick={() => setSelected(issue)}>
              Page {issue.pageIndex + 1}: {issue.kind}
            </button>
          ))}
        </aside>
        <section className="review-structure" aria-label="Evidence structure">
          {selected ? <><h3>{selected.kind}</h3><p>{selected.reason}</p><p>Source: {selected.sourceText ?? "not available"}</p><p>Effective: {selected.effectiveText ?? "not available"}</p><p>Confidence: {selected.confidence ?? "not available"}</p><code>{selected.targetRef}</code></> : <p>Select an issue to inspect its evidence reference.</p>}
        </section>
        <section className="review-properties" aria-label="Evidence properties">
          {selected && <><p>Page: {selected.pageId}</p><p>Coordinate space: {selected.coordinateSpace ?? "master"}</p><p>BBox: {selected.bbox.x}, {selected.bbox.y}, {selected.bbox.width} × {selected.bbox.height}</p><p>Base digest: {selected.baseEvidenceDigest}</p><textarea value={text} onChange={(event) => setText(event.target.value)} placeholder="Revision text" /><label><input type="checkbox" checked={aiSuggested} onChange={(event) => setAiSuggested(event.target.checked)} /> AI suggestion (never applied by default)</label><button type="button" onClick={() => void submitRevision()}>Save revision</button></>}
        </section>
      </div>
      {message && <p role="status">{message}</p>}
    </section>
  );
}
