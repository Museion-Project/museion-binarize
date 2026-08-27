import { useState } from "react";

import type { BookmarkCandidate, ReviewIssue } from "../app/types";
import { addReviewRevision, loadReviewQueue, loadBookmarks, confirmBookmark, rejectBookmark, editBookmark, reparentBookmark } from "../lib/tauri";

export function ReviewWorkbench() {
  const [packagePath, setPackagePath] = useState("");
  const [issues, setIssues] = useState<ReviewIssue[]>([]);
  const [selected, setSelected] = useState<ReviewIssue | null>(null);
  const [text, setText] = useState("");
  const [aiSuggested, setAiSuggested] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [bookmarks, setBookmarks] = useState<BookmarkCandidate[]>([]);
  const [selectedBookmark, setSelectedBookmark] = useState<BookmarkCandidate | null>(null);
  const [bookmarkTitle, setBookmarkTitle] = useState("");
  const [bookmarkParent, setBookmarkParent] = useState("");
  const [bookmarkLevel, setBookmarkLevel] = useState(0);
  const [bookmarkFilter, setBookmarkFilter] = useState<BookmarkCandidate["status"] | "all">("all");
  const [kindFilter, setKindFilter] = useState<ReviewIssue["kind"] | "all">("all");
  const visibleIssues = kindFilter === "all" ? issues : issues.filter((issue) => issue.kind === kindFilter);
  const visibleBookmarks = bookmarkFilter === "all" ? bookmarks : bookmarks.filter((candidate) => candidate.status === bookmarkFilter);

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

  function selectBookmark(candidate: BookmarkCandidate | null) { setSelectedBookmark(candidate); setBookmarkTitle(candidate?.effectiveTitle ?? ""); setBookmarkParent(candidate?.effectiveParentId ?? ""); setBookmarkLevel(candidate?.effectiveLevel ?? 0); }
  async function refreshBookmarks() { if (!packagePath.trim()) return; try { const next = await loadBookmarks(packagePath.trim()); setBookmarks(next); selectBookmark(next.find((candidate) => candidate.candidateId === selectedBookmark?.candidateId) ?? next[0] ?? null); setMessage(null); } catch (error) { setMessage(String(error)); } }
  async function bookmarkAction(action: "confirm" | "reject") { if (!selectedBookmark) return; try { if (action === "confirm") await confirmBookmark(packagePath.trim(), selectedBookmark.candidateId); else await rejectBookmark(packagePath.trim(), selectedBookmark.candidateId); await refreshBookmarks(); } catch (error) { setMessage(String(error)); } }
  async function saveBookmarkEdit() { if (!selectedBookmark || !bookmarkTitle.trim()) return; try { await editBookmark(packagePath.trim(), selectedBookmark.candidateId, bookmarkTitle.trim()); await refreshBookmarks(); } catch (error) { setMessage(String(error)); } }
  async function saveBookmarkParent() { if (!selectedBookmark) return; try { await reparentBookmark(packagePath.trim(), selectedBookmark.candidateId, bookmarkParent.trim() || null, bookmarkLevel); await refreshBookmarks(); } catch (error) { setMessage(String(error)); } }

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
        <button type="button" onClick={() => void refreshBookmarks()}>Load bookmarks</button>
        <label>
          Filter kind
          <select value={kindFilter} onChange={(event) => setKindFilter(event.target.value as ReviewIssue["kind"] | "all")}>
            <option value="all">All</option><option value="low_confidence">Low confidence</option>
            <option value="reading_order_gap">Reading order</option><option value="unicode_normalization">Unicode</option>
            <option value="empty_region">Empty region</option>
          </select>
        </label>
      </header>
      <section aria-label="Bookmark tree" className="bookmark-review">
        <label>Bookmark status <select value={bookmarkFilter} onChange={(event) => setBookmarkFilter(event.target.value as BookmarkCandidate["status"] | "all")}><option value="all">All</option><option value="proposed">Proposed</option><option value="needs_review">Needs review</option><option value="confirmed">Confirmed</option><option value="rejected">Rejected</option></select></label>
        <strong>{visibleBookmarks.length} bookmark(s)</strong>
        <nav aria-label="Bookmark candidates">{visibleBookmarks.map((candidate) => <button type="button" key={candidate.candidateId} onClick={() => selectBookmark(candidate)}>{" ".repeat(candidate.effectiveLevel)}{candidate.effectiveTitle} — page {candidate.physicalPageIndex + 1}</button>)}</nav>
        {selectedBookmark && <div aria-label="Bookmark evidence"><h3>{selectedBookmark.effectiveTitle}</h3><p>Source title: {selectedBookmark.sourceTitle}</p><p>Page: {selectedBookmark.targetPageId} ({selectedBookmark.physicalPageIndex + 1})</p><p>BBox: {selectedBookmark.masterBbox ? `${selectedBookmark.masterBbox.x}, ${selectedBookmark.masterBbox.y}, ${selectedBookmark.masterBbox.width} × ${selectedBookmark.masterBbox.height}` : "not available"}</p><p>Confidence: {selectedBookmark.confidence}</p><p>Evidence: {selectedBookmark.evidence.length} ref(s)</p><p>Rules: {selectedBookmark.ruleTrace.join(", ") || "none"}</p><button type="button" onClick={() => void bookmarkAction("confirm")}>Confirm bookmark</button><button type="button" onClick={() => void bookmarkAction("reject")}>Reject bookmark</button><label>Bookmark title<input value={bookmarkTitle} onChange={(event) => setBookmarkTitle(event.target.value)} /></label><button type="button" onClick={() => void saveBookmarkEdit()}>Save bookmark title</button><label>Parent candidate ID<input value={bookmarkParent} onChange={(event) => setBookmarkParent(event.target.value)} /></label><label>Bookmark level<input type="number" min={0} max={64} value={bookmarkLevel} onChange={(event) => setBookmarkLevel(Number(event.target.value))} /></label><button type="button" onClick={() => void saveBookmarkParent()}>Save bookmark parent</button></div>}
      </section>
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
