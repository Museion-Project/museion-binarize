import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ReviewIssue } from "../app/types";
import { ReviewWorkbench } from "./ReviewWorkbench";

const mocks = vi.hoisted(() => ({
  loadReviewQueue: vi.fn(),
  addReviewRevision: vi.fn(),
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

function loadFixture() {
  fireEvent.change(screen.getByLabelText("MDP package path"), {
    target: { value: "/tmp/book.mdp" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Load review queue" }));
}

describe("ReviewWorkbench", () => {
  beforeEach(() => {
    mocks.loadReviewQueue.mockReset().mockResolvedValue(issues);
    mocks.addReviewRevision.mockReset().mockResolvedValue(undefined);
  });

  it("renders the three columns and loads a local queue", async () => {
    render(<ReviewWorkbench />);
    expect(screen.getByRole("heading", { name: "Review workbench" })).toBeTruthy();
    expect(screen.getByLabelText("Review issues")).toBeTruthy();
    expect(screen.getByLabelText("Evidence structure")).toBeTruthy();
    expect(screen.getByLabelText("Evidence properties")).toBeTruthy();
    loadFixture();
    expect(await screen.findByText("2 issue(s)")).toBeTruthy();
    expect(mocks.loadReviewQueue).toHaveBeenCalledWith("/tmp/book.mdp");
  });

  it("filters issues by kind and shows selected source/effective evidence", async () => {
    render(<ReviewWorkbench />);
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
    render(<ReviewWorkbench />);
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
    render(<ReviewWorkbench />);
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
});
