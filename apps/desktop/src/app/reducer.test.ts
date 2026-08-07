import { describe, expect, it } from "vitest";

import { reducer, type AppState } from "./reducer";
import type { DocumentSummary, EstimateResult, ProcessingSettings } from "./types";

function sampleDocument(): DocumentSummary {
  return {
    documentId: "doc-1",
    fileName: "book.pdf",
    sourceBytes: 1000,
    pageCount: 3,
    title: null,
    author: null,
    pdfiumLibrary: "test",
    pages: [],
  };
}

function readyState(overrides: Partial<Extract<AppState, { kind: "ready" }>> = {}) {
  const base = reducer({ kind: "idle" }, { type: "OPEN_SUCCEEDED", document: sampleDocument() });
  if (base.kind !== "ready") throw new Error("expected ready state");
  return { ...base, ...overrides };
}

function sampleEstimate(): EstimateResult {
  return {
    requestId: 1,
    documentPageCount: 3,
    sampledPages: [
      {
        pageNumber: 1,
        rasterWidth: 100,
        rasterHeight: 100,
        blackPixelRatio: 0.1,
        ccittBytes: 500,
        bytesPerPixel: 0.05,
      },
    ],
    estimatedOutputBytes: 6_000_000,
    estimatedLowerBytes: 5_000_000,
    estimatedUpperBytes: 7_000_000,
    rangeMethod: "quartiles",
    dpi: 400,
    method: "sauvola",
    estimateTotalDurationUs: 1_000_000,
    experimental: true,
  };
}

function sampleSettings(): ProcessingSettings {
  return {
    dpi: 400,
    method: "sauvola",
    threshold: null,
    sauvolaWindowSize: null,
    sauvolaK: null,
    contrast: 0,
    medianDenoise: false,
    backgroundNormalization: false,
    backgroundRadius: null,
    despeckle: "off",
  };
}

describe("estimate state transitions", () => {
  it("starts idle for a freshly opened document", () => {
    const state = readyState();
    expect(state.estimate).toEqual({ kind: "idle" });
  });

  it("moves to running on ESTIMATE_REQUEST_STARTED", () => {
    const state = reducer(readyState(), { type: "ESTIMATE_REQUEST_STARTED", requestId: 1 });
    expect(state.kind).toBe("ready");
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "running" });
    expect(state.latestEstimateRequestId).toBe(1);
  });

  it("becomes ready with the result on a matching ESTIMATE_SUCCEEDED", () => {
    let state: AppState = readyState();
    state = reducer(state, { type: "ESTIMATE_REQUEST_STARTED", requestId: 1 });
    const result = sampleEstimate();
    state = reducer(state, { type: "ESTIMATE_SUCCEEDED", requestId: 1, result });
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "ready", result });
  });

  it("ignores a stale ESTIMATE_SUCCEEDED whose requestId is no longer current", () => {
    let state: AppState = readyState();
    state = reducer(state, { type: "ESTIMATE_REQUEST_STARTED", requestId: 1 });
    state = reducer(state, { type: "ESTIMATE_REQUEST_STARTED", requestId: 2 }); // supersedes
    state = reducer(state, {
      type: "ESTIMATE_SUCCEEDED",
      requestId: 1, // the old, superseded request
      result: sampleEstimate(),
    });
    if (state.kind !== "ready") throw new Error();
    // Still "running" (request 2's outcome hasn't arrived), not clobbered
    // by request 1's late response.
    expect(state.estimate).toEqual({ kind: "running" });
  });

  it("ignores a stale ESTIMATE_FAILED the same way", () => {
    let state: AppState = readyState();
    state = reducer(state, { type: "ESTIMATE_REQUEST_STARTED", requestId: 1 });
    state = reducer(state, { type: "ESTIMATE_REQUEST_STARTED", requestId: 2 });
    state = reducer(state, {
      type: "ESTIMATE_FAILED",
      requestId: 1,
      error: { code: "internal_error", message: "boom", hint: null, detail: null },
    });
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "running" });
  });

  it("marks a ready estimate stale when settings change", () => {
    const result = sampleEstimate();
    let state: AppState = readyState({ estimate: { kind: "ready", result } });
    state = reducer(state, {
      type: "SET_SETTINGS",
      settings: { ...sampleSettings(), dpi: 300 },
      preset: "custom",
    });
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "stale", previous: result });
  });

  it("does not touch a stale estimate again on a further settings change", () => {
    const result = sampleEstimate();
    let state: AppState = readyState({ estimate: { kind: "stale", previous: result } });
    state = reducer(state, {
      type: "SET_SETTINGS",
      settings: { ...sampleSettings(), dpi: 300 },
      preset: "custom",
    });
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "stale", previous: result });
  });

  it("does not invalidate the estimate on page selection or output path changes", () => {
    const result = sampleEstimate();
    const base = readyState({ estimate: { kind: "ready", result } });

    const afterPage = reducer(base, { type: "SELECT_PAGE", page: 2 });
    if (afterPage.kind !== "ready") throw new Error();
    expect(afterPage.estimate).toEqual({ kind: "ready", result });

    const afterOutput = reducer(base, { type: "SET_OUTPUT_PATH", path: "/tmp/out.pdf" });
    if (afterOutput.kind !== "ready") throw new Error();
    expect(afterOutput.estimate).toEqual({ kind: "ready", result });
  });

  it("resets estimate state to idle when starting over after completion", () => {
    let state: AppState = readyState({ estimate: { kind: "ready", result: sampleEstimate() } });
    state = reducer(state, {
      type: "PROCESSING_STARTED",
      jobId: "job-1",
      pageCount: 3,
      outputPath: "/tmp/out.pdf",
    });
    state = reducer(state, {
      type: "PROCESSING_COMPLETED",
      jobId: "job-1",
      report: {
        jobId: "job-1",
        outputPath: "/tmp/out.pdf",
        pagesProcessed: 3,
        originalBytes: 1000,
        outputBytes: 200,
        elapsedUs: 1000,
        pdfiumLibrary: "test",
        absoluteBytesSaved: 800,
        sizeReductionFraction: 0.8,
        inputToOutputRatio: 5,
        medianProcessingDurationUs: 100,
        overallBlackPixelRatio: 0.1,
        slowestPage: null,
        largestEncodedPage: null,
        smallestEncodedPage: null,
        estimateComparison: null,
      },
    });
    state = reducer(state, { type: "START_OVER" });
    if (state.kind !== "ready") throw new Error();
    expect(state.estimate).toEqual({ kind: "idle" });
  });
});
