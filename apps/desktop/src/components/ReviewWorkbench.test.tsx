import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BookmarkCandidate, ReviewIssue } from "../app/types";
import { ReviewWorkbench } from "./ReviewWorkbench";

const mocks = vi.hoisted(() => ({
  loadReviewQueue: vi.fn(),
  addReviewRevision: vi.fn(),
  loadBookmarkTree: vi.fn(),
  confirmBookmark: vi.fn(),
  rejectBookmark: vi.fn(),
  editBookmark: vi.fn(),
  reparentBookmark: vi.fn(),
  startAutoBookmark: vi.fn(),
  cancelAutoBookmark: vi.fn(),
  onAutoBookmarkStage: vi.fn(),
  onAutoBookmarkCompleted: vi.fn(),
  onAutoBookmarkCancelled: vi.fn(),
  onAutoBookmarkFailed: vi.fn(),
  pickPackageDirectory: vi.fn(),
  pickOutputDestination: vi.fn(),
}));

vi.mock("../lib/tauri", () => mocks);

const issues: ReviewIssue[] = [
  {
    issueId: "issue-low",
    targetRef: "word-1",
    pageId: "page-1",
    pageIndex: 0,
    bbox: { x: 1, y: 2, width: 30, height: 10 },
    baseEvidenceDigest: "a".repeat(64),
    kind: "low_confidence",
    severity: "warning",
    reason: "confidence is below threshold",
    status: "open",
    coordinateSpace: "page-1-master",
    sourceText: "ἀρχή",
    effectiveText: "ἀρχή",
    confidence: 0.42,
  },
  {
    issueId: "issue-unicode",
    targetRef: "word-2",
    pageId: "page-2",
    pageIndex: 1,
    bbox: { x: 3, y: 4, width: 20, height: 8 },
    baseEvidenceDigest: "b".repeat(64),
    kind: "unicode_normalization",
    severity: "info",
    reason: "original and normalized text differ",
    status: "open",
    coordinateSpace: "page-2-master",
    sourceText: "A\u0301",
    effectiveText: "Á",
    confidence: 0.91,
  },
];

const bookmarkCandidates: BookmarkCandidate[] = [
  {
    candidateId: "bookmark-root",
    sourceTitle: "Αρχή",
    effectiveTitle: "Ἀρχή",
    effectiveLevel: 0,
    effectiveParentId: null,
    targetPageId: "page-1",
    physicalPageIndex: 0,
    masterBbox: { x: 10, y: 20, width: 80, height: 14 },
    evidenceCount: 3,
    confidence: 0.96,
    status: "auto_confirmed",
    score: {
      titleMatch: 3900,
      pageMapping: 2000,
      numberingHierarchy: 1000,
      bodyLayout: 900,
      ocrQuality: 960,
      sequenceUniqueness: 1000,
      total: 9760,
      maximum: 10000,
    },
    alignment: {
      tocPageIndex: 1,
      bodyPageIndex: 3,
      printedLabel: "1",
      pageResidual: 0,
      mappingOffset: 2,
      runnerUpMargin: 1200,
      secondaryKeyOnly: false,
      geometryQuality: "measured",
    },
    automaticReason: "toc_body_alignment_consensus",
    reasonCodes: ["toc_body_alignment_consensus"],
  },
  {
    candidateId: "bookmark-child",
    sourceTitle: "1.1 Child",
    effectiveTitle: "1.1 Child",
    effectiveLevel: 1,
    effectiveParentId: "bookmark-root",
    targetPageId: "page-2",
    physicalPageIndex: 1,
    masterBbox: null,
    evidenceCount: 1,
    confidence: 0.55,
    status: "needs_review",
    score: null,
    alignment: null,
    automaticReason: "runner_up_margin_too_small",
    reasonCodes: ["runner_up_margin_too_small"],
  },
];

/** Handlers the component registered, in registration order:
 * stage, completed, cancelled, failed. */
const handlers: Array<(payload: unknown) => void> = [];

function loadFixture() {
  fireEvent.change(screen.getByLabelText("MDP package path (advanced)"), {
    target: { value: "/tmp/book.mdp" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Load review queue" }));
}

describe("ReviewWorkbench", () => {
  beforeEach(() => {
    mocks.loadReviewQueue.mockReset().mockResolvedValue(issues);
    mocks.addReviewRevision.mockReset().mockResolvedValue(undefined);
    mocks.loadBookmarkTree.mockReset().mockResolvedValue(bookmarkCandidates);
    mocks.startAutoBookmark.mockReset().mockResolvedValue({
      jobId: "auto-bookmark-1",
      documentId: "doc-1",
    });
    mocks.cancelAutoBookmark.mockReset().mockResolvedValue(undefined);
    mocks.pickPackageDirectory.mockReset().mockResolvedValue("/picked/book.mdp");
    mocks.pickOutputDestination.mockReset().mockResolvedValue("/picked/out.pdf");
    for (const listener of [
      mocks.onAutoBookmarkStage,
      mocks.onAutoBookmarkCompleted,
      mocks.onAutoBookmarkCancelled,
      mocks.onAutoBookmarkFailed,
    ]) {
      listener.mockReset().mockImplementation((handler: unknown) => {
        handlers.push(handler as (payload: unknown) => void);
        return Promise.resolve(() => {});
      });
    }
    handlers.length = 0;
    mocks.confirmBookmark.mockReset().mockResolvedValue(undefined);
    mocks.rejectBookmark.mockReset().mockResolvedValue(undefined);
    mocks.editBookmark.mockReset().mockResolvedValue(undefined);
    mocks.reparentBookmark.mockReset().mockResolvedValue(undefined);
  });

  it("renders the three columns and loads a local queue", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    expect(screen.getByRole("heading", { name: "Review workbench" })).toBeTruthy();
    expect(screen.getByLabelText("Review issues")).toBeTruthy();
    expect(screen.getByLabelText("Evidence structure")).toBeTruthy();
    expect(screen.getByLabelText("Evidence properties")).toBeTruthy();
    loadFixture();
    expect(await screen.findByText("2 issue(s)")).toBeTruthy();
    expect(mocks.loadReviewQueue).toHaveBeenCalledWith("/tmp/book.mdp");
  });

  it("filters issues by kind and shows selected source/effective evidence", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    loadFixture();
    await screen.findByText("2 issue(s)");
    fireEvent.change(screen.getByLabelText("Filter kind"), {
      target: { value: "low_confidence" },
    });
    expect(screen.getByText("1 issue(s)")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Page 1: low_confidence/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Page 2:/ })).toBeNull();

    fireEvent.change(screen.getByLabelText("Filter kind"), {
      target: { value: "all" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Page 2: unicode_normalization/ }));
    expect(screen.getByText("Source: A\u0301")).toBeTruthy();
    expect(screen.getByText("Effective: Á")).toBeTruthy();
    expect(screen.getByText("Confidence: 0.91")).toBeTruthy();
    expect(screen.getByText("Coordinate space: page-2-master")).toBeTruthy();
    expect(screen.getByText(`Base digest: ${"b".repeat(64)}`)).toBeTruthy();
  });

  it("submits a human revision with the selected stable target and base digest", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    loadFixture();
    await screen.findByText("2 issue(s)");
    fireEvent.change(screen.getByPlaceholderText("Revision text"), {
      target: { value: "human correction" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save revision" }));
    expect(mocks.addReviewRevision).toHaveBeenCalledWith({
      packagePath: "/tmp/book.mdp",
      targetRef: "word-1",
      baseEvidenceDigest: "a".repeat(64),
      text: "human correction",
      aiSuggested: false,
    });
  });

  it("submits an AI suggestion explicitly without changing its target metadata", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    loadFixture();
    await screen.findByText("2 issue(s)");
    fireEvent.change(screen.getByPlaceholderText("Revision text"), {
      target: { value: "ai proposal" },
    });
    fireEvent.click(screen.getByLabelText("AI suggestion (never applied by default)"));
    fireEvent.click(screen.getByRole("button", { name: "Save revision" }));
    expect(mocks.addReviewRevision).toHaveBeenCalledWith({
      packagePath: "/tmp/book.mdp",
      targetRef: "word-1",
      baseEvidenceDigest: "a".repeat(64),
      text: "ai proposal",
      aiSuggested: true,
    });
  });

  it("filters, selects, edits, confirms, rejects, and reparents durable bookmark candidates", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.change(screen.getByLabelText("MDP package path (advanced)"), {
      target: { value: "/tmp/book.mdp" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load bookmarks" }));
    expect(await screen.findByText("2 bookmark(s)")).toBeTruthy();
    expect(mocks.loadBookmarkTree).toHaveBeenCalledWith("/tmp/book.mdp");

    fireEvent.change(screen.getByLabelText("Bookmark status"), {
      target: { value: "needs_review" },
    });
    expect(screen.getByText("1 bookmark(s)")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /1.1 Child/ }));
    expect(screen.getByText("Confidence: 0.55")).toBeTruthy();
    expect(screen.getByText("BBox: not available")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Bookmark title"), {
      target: { value: "Edited child" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save bookmark title" }));
    expect(mocks.editBookmark).toHaveBeenCalledWith(
      "/tmp/book.mdp",
      "bookmark-child",
      "Edited child",
    );

    fireEvent.click(screen.getByRole("button", { name: "Confirm bookmark" }));
    expect(mocks.confirmBookmark).toHaveBeenCalledWith("/tmp/book.mdp", "bookmark-child");
    fireEvent.click(screen.getByRole("button", { name: "Reject bookmark" }));
    expect(mocks.rejectBookmark).toHaveBeenCalledWith("/tmp/book.mdp", "bookmark-child");

    fireEvent.change(screen.getByLabelText("Parent candidate ID"), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByLabelText("Bookmark level"), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save bookmark parent" }));
    expect(mocks.reparentBookmark).toHaveBeenCalledWith(
      "/tmp/book.mdp",
      "bookmark-child",
      null,
      0,
    );
  });

  it("runs the automatic path from one button and reports what was added", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.click(screen.getByRole("button", { name: "Choose folder…" }));
    await waitFor(() =>
      expect((screen.getByLabelText("MDP package folder") as HTMLInputElement).value).toBe(
        "/picked/book.mdp",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose file…" }));
    await waitFor(() =>
      expect((screen.getByLabelText("Save the new PDF as") as HTMLInputElement).value).toBe(
        "/picked/out.pdf",
      ),
    );

    const start = screen.getByRole("button", { name: "Add bookmarks automatically" });
    fireEvent.click(start);
    expect(mocks.startAutoBookmark).toHaveBeenCalledWith({
      documentId: "doc-1",
      packagePath: "/picked/book.mdp",
      outputPath: "/picked/out.pdf",
      overwrite: false,
      regenerate: false,
    });
    expect(
      await screen.findByText("Looking for a printed table of contents…"),
    ).toBeTruthy();

    const [stage, completed] = handlers;
    act(() => stage({ jobId: "auto-bookmark-1", stage: "writing_pdf" }));
    expect(screen.getByText("Writing the outlined PDF…")).toBeTruthy();
    act(() =>
      completed({
        jobId: "auto-bookmark-1",
        documentId: "doc-1",
        mode: "toc_aligned",
        status: "auto_confirmed",
        tocPageCount: 2,
        parsedEntries: 50,
        autoConfirmed: 47,
        needsReview: 3,
        skipped: 0,
        writtenBookmarks: 47,
        safeRefusalReason: null,
        reportPath: "/picked/book.mdp/bookmarks/generation-report.json",
        outputPath: "/picked/out.pdf",
      }),
    );
    expect(
      await screen.findByText(/Added 47 reliable bookmark\(s\) automatically/),
    ).toBeTruthy();
    expect(screen.getByText("Saved to /picked/out.pdf")).toBeTruthy();
    // A finished run reloads the tree without the user asking.
    expect(mocks.loadBookmarkTree).toHaveBeenCalledWith("/picked/book.mdp");
  });

  it("shows a safe refusal as a normal result, not a failure", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.change(screen.getByLabelText("MDP package folder"), {
      target: { value: "/tmp/book.mdp" },
    });
    fireEvent.change(screen.getByLabelText("Save the new PDF as"), {
      target: { value: "/tmp/out.pdf" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add bookmarks automatically" }));
    await screen.findByText("Looking for a printed table of contents…");
    const completed = handlers[1];
    act(() =>
      completed({
        jobId: "auto-bookmark-1",
        documentId: "doc-1",
        mode: "safe_refusal",
        status: "safe_refusal",
        tocPageCount: 0,
        parsedEntries: 0,
        autoConfirmed: 0,
        needsReview: 0,
        skipped: 0,
        writtenBookmarks: 0,
        safeRefusalReason: "no printed table of contents was detected",
        reportPath: "/tmp/book.mdp/bookmarks/generation-report.json",
        outputPath: null,
      }),
    );
    expect(
      await screen.findByText(/No sufficiently reliable table of contents was found/),
    ).toBeTruthy();
    expect(screen.getByText("no printed table of contents was detected")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("offers cancellation while a run is in flight and clears it afterwards", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.change(screen.getByLabelText("MDP package folder"), {
      target: { value: "/tmp/book.mdp" },
    });
    fireEvent.change(screen.getByLabelText("Save the new PDF as"), {
      target: { value: "/tmp/out.pdf" },
    });
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Add bookmarks automatically" }));
    const cancel = await screen.findByRole("button", { name: "Cancel" });
    expect(
      (screen.getByRole("button", { name: "Add bookmarks automatically" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    fireEvent.click(cancel);
    expect(mocks.cancelAutoBookmark).toHaveBeenCalledWith("auto-bookmark-1", "doc-1");
    act(() => handlers[2]({ jobId: "auto-bookmark-1", stage: "cancelled" }));
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("distinguishes an automatic entry from a human one and shows its score", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.change(screen.getByLabelText("MDP package path (advanced)"), {
      target: { value: "/tmp/book.mdp" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load bookmarks" }));
    await screen.findByText("2 bookmark(s)");

    fireEvent.change(screen.getByLabelText("Bookmark status"), {
      target: { value: "auto_confirmed" },
    });
    expect(screen.getByText("1 bookmark(s)")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Ἀρχή/ }));
    expect(screen.getByText("Status: Added automatically")).toBeTruthy();
    const breakdown = screen.getByLabelText("Bookmark score breakdown");
    expect(breakdown.textContent).toContain("3900");
    expect(breakdown.textContent).toContain("9760 of 10000");
    const alignment = screen.getByLabelText("Bookmark alignment evidence");
    expect(alignment.textContent).toContain("Contents page: 2");
    expect(alignment.textContent).toContain("Heading page: 4");
    expect(alignment.textContent).toContain("Runner-up margin: 1200");
    expect(alignment.textContent).toContain("Geometry: measured");
  });

  it("reports a backend failure as an alert without losing the panel", async () => {
    render(<ReviewWorkbench documentId="doc-1" />);
    fireEvent.change(screen.getByLabelText("MDP package folder"), {
      target: { value: "/tmp/book.mdp" },
    });
    fireEvent.change(screen.getByLabelText("Save the new PDF as"), {
      target: { value: "/tmp/out.pdf" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add bookmarks automatically" }));
    await screen.findByText("Looking for a printed table of contents…");
    act(() =>
      handlers[3]({
        jobId: "auto-bookmark-1",
        error: {
          code: "destination_conflict",
          message: "output exists or is unsafe",
          hint: "Choose a different output location.",
          detail: null,
        },
      }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("output exists or is unsafe");
    expect(alert.textContent).toContain("Choose a different output location.");
    expect(screen.getByRole("button", { name: "Add bookmarks automatically" })).toBeTruthy();
  });
});
