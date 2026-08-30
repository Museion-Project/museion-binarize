import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  LocalOcrReadiness,
  LocalPipelineCompleted,
  LocalPipelineStageEvent,
  ProcessingSettings,
} from "../app/types";
import { LocalPipelinePanel } from "./LocalPipelinePanel";

const mocks = vi.hoisted(() => ({
  localOcrReadiness: vi.fn(),
  ocrProviderStatus: vi.fn(),
  storeModelProviderCredential: vi.fn(),
  modelProviderCredentialStatus: vi.fn(),
  deleteModelProviderCredential: vi.fn(),
  testOcrProvider: vi.fn(),
  startLocalPipeline: vi.fn(),
  cancelLocalPipeline: vi.fn(),
  cancelLocalPipelineJob: vi.fn(),
  onLocalPipelineStage: vi.fn(),
  onLocalPipelineCompleted: vi.fn(),
  onLocalPipelineCancelled: vi.fn(),
  onLocalPipelineFailed: vi.fn(),
  pickOutputDestination: vi.fn(),
  pickPackageDirectory: vi.fn(),
}));

vi.mock("../lib/tauri", () => mocks);

const settings: ProcessingSettings = {
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

const ready: LocalOcrReadiness = {
  ready: true,
  engine: "tesseract",
  languageProfile: "auto",
  clearedForProduction: true,
  missingFiles: [],
  modelSet: "tessdata_best 4.1.0",
  modelLicense: "Apache-2.0",
  diagnostic: "local OCR is ready; no network access is used",
  availableProfiles: ["auto", "greek-ancient", "german"],
};

const PROVIDER_STATUS = {
  defaultMode: "local" as const,
  modes: [
    {
      id: "local" as const,
      displayName: "Local OCR",
      defaultMode: true,
      usesNetwork: false,
      requiresCredential: false,
      executionLocation: "local",
      productionReady: true,
      availability: "stable",
      blockers: [],
      model: null,
      credentialPresent: false,
      structuredBboxDefaultEnabled: false,
    },
    {
      id: "gemini-byok" as const,
      displayName: "Gemini API — Use My Key",
      defaultMode: false,
      usesNetwork: true,
      requiresCredential: true,
      executionLocation: "remote:user-key",
      productionReady: false,
      availability: "beta",
      blockers: ["live validation pending"],
      model: "gemini-3.7-flash",
      credentialPresent: false,
      structuredBboxDefaultEnabled: false,
    },
    {
      id: "mpdf-credits" as const,
      displayName: "M PDF Cloud OCR",
      defaultMode: false,
      usesNetwork: true,
      requiresCredential: true,
      executionLocation: "remote:mpdf-brokered",
      productionReady: false,
      availability: "unavailable",
      blockers: ["no payment provider is integrated"],
      model: "gemini-3.7-flash",
      credentialPresent: false,
      structuredBboxDefaultEnabled: false,
    },
  ],
};

const STAGES = [
  "analyzing_source",
  "ocr_original",
  "deriving_text",
  "generating_bookmarks",
  "awaiting_review",
  "binarizing_visuals",
  "assembling_final_pdf",
  "validating",
] as const;

/** Captures the handler each `on*` subscription was given. */
function captureListeners() {
  const captured: Record<string, (payload: never) => void> = {};
  const register =
    (name: string) =>
    (handler: (payload: never) => void) => {
      captured[name] = handler;
      return Promise.resolve(() => {});
    };
  mocks.onLocalPipelineStage.mockImplementation(register("stage"));
  mocks.onLocalPipelineCompleted.mockImplementation(register("completed"));
  mocks.onLocalPipelineCancelled.mockImplementation(register("cancelled"));
  mocks.onLocalPipelineFailed.mockImplementation(register("failed"));
  return captured;
}

function renderPanel() {
  return render(
    <LocalPipelinePanel
      documentId="doc-1"
      settings={settings}
      defaultOutputName="scan-bw.pdf"
    />,
  );
}

describe("LocalPipelinePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.localOcrReadiness.mockResolvedValue(ready);
    mocks.ocrProviderStatus.mockResolvedValue(PROVIDER_STATUS);
    captureListeners();
  });

  it("names the flow in the order the core enforces", () => {
    renderPanel();
    expect(screen.getByRole("heading", { name: /OCR → bookmarks → binarize/ })).toBeTruthy();
    expect(
      screen.getByText(/Text is recognized from the original pages first/),
    ).toBeTruthy();
  });

  it("will not start until a destination and a working folder are chosen", async () => {
    renderPanel();
    const start = screen.getByRole("button", { name: "Start" });
    expect((start as HTMLButtonElement).disabled).toBe(true);

    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    expect((screen.getByRole("button", { name: "Start" }) as HTMLButtonElement).disabled).toBe(
      true,
    );

    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Start" }) as HTMLButtonElement).disabled,
      ).toBe(false),
    );
  });

  it("reports what is missing instead of offering an unrunnable start", async () => {
    mocks.localOcrReadiness.mockResolvedValue({
      ...ready,
      ready: false,
      missingFiles: ["grc.traineddata", "osd.traineddata"],
      diagnostic: "2 model file(s) are missing",
    });
    renderPanel();
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("2 model file(s) are missing"),
    );
    expect(screen.getByRole("alert").textContent).toContain("grc.traineddata");

    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    expect((screen.getByRole("button", { name: "Start" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("renders the stage list the backend sent, not one of its own", async () => {
    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    mocks.startLocalPipeline.mockResolvedValue({
      jobId: "pipeline-auto",
      documentId: "doc-1",
      workspacePath: "/out/work",
      stages: STAGES,
      resumed: false,
    });
    renderPanel();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
    });
    const list = await screen.findByRole("list", { name: "Progress" });
    const items = list.querySelectorAll("li");
    // OCR must be shown before binarization, because that is the real order.
    const labels = Array.from(items).map((item) => item.textContent ?? "");
    const ocr = labels.findIndex((label) => label.includes("Recognizing text"));
    const binarize = labels.findIndex((label) => label.includes("Binarizing"));
    expect(ocr).toBeGreaterThanOrEqual(0);
    expect(binarize).toBeGreaterThan(ocr);
  });

  it("shows a review pause as a normal outcome with a way to continue", async () => {
    const listeners = captureListeners();
    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    mocks.startLocalPipeline.mockResolvedValue({
      jobId: "pipeline-auto",
      documentId: "doc-1",
      workspacePath: "/out/work",
      stages: STAGES,
      resumed: false,
    });
    renderPanel();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
    });
    const completed: LocalPipelineCompleted = {
      jobId: "pipeline-auto",
      documentId: "doc-1",
      status: "awaiting_review",
      bookmarkStatus: "none_confirmed",
      stageReached: "awaiting_review",
      outputPath: null,
      pageCount: 21,
      ocrPages: 21,
      ocrErrors: 0,
      bookmarksWritten: 0,
      bookmarksAutoConfirmed: 0,
      bookmarksNeedingReview: 5,
      bookmarksSkipped: 0,
      textLayerWords: 610,
      refusalReason: null,
      ocrProvenance: [],
      providerMode: "local",
      cloudFallbackPages: [],
      cloudInputTokens: 0,
      cloudOutputTokens: 0,
      cloudUsageMayBeIncomplete: false,
      creditsReserved: 0,
      creditsCharged: 0,
      creditsReleased: 0,
      creditsRefunded: 0,
      creditsSettled: false,
      notProductionReadyReason: null,
    };
    await act(async () => {
      (listeners.completed as unknown as (p: LocalPipelineCompleted) => void)(completed);
    });
    expect(screen.getByText(/5 bookmarks need your decision/)).toBeTruthy();
    expect(screen.getByText(/Nothing has been written yet/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /I have reviewed/ })).toBeTruthy();
  });

  it("treats a safe refusal as a warning on a completed conversion, not an error", async () => {
    const listeners = captureListeners();
    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    mocks.startLocalPipeline.mockResolvedValue({
      jobId: "pipeline-auto",
      documentId: "doc-1",
      workspacePath: "/out/work",
      stages: STAGES,
      resumed: false,
    });
    renderPanel();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
    });
    await act(async () => {
      (listeners.completed as unknown as (p: LocalPipelineCompleted) => void)({
        jobId: "pipeline-auto",
        documentId: "doc-1",
        status: "completed",
        bookmarkStatus: "safe_refusal",
        stageReached: "validating",
        outputPath: "/out/final.pdf",
        pageCount: 4,
        ocrPages: 4,
        ocrErrors: 0,
        bookmarksWritten: 0,
        bookmarksAutoConfirmed: 0,
        bookmarksNeedingReview: 0,
        bookmarksSkipped: 2,
        textLayerWords: 68,
        refusalReason: "no printed contents list was found",
        ocrProvenance: [],
        providerMode: "local",
        cloudFallbackPages: [],
        cloudInputTokens: 0,
        cloudOutputTokens: 0,
        cloudUsageMayBeIncomplete: false,
        creditsReserved: 0,
        creditsCharged: 0,
        creditsReleased: 0,
        creditsRefunded: 0,
        creditsSettled: false,
        notProductionReadyReason: null,
      });
    });
    const status = screen.getByText(/No reliable structure was found/);
    expect(status.textContent).toContain("no printed contents list was found");
    // The conversion itself succeeded: the finished PDF is named, the normal
    // done summary is shown, and nothing is flagged as an error.
    expect(status.textContent).toContain("still converted and is searchable");
    const done = screen.getByText(/Done — 4 pages/);
    expect(done.parentElement?.textContent).toContain("/out/final.pdf");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps recognized pages after a cancel and offers to resume", async () => {
    const listeners = captureListeners();
    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    mocks.startLocalPipeline.mockResolvedValue({
      jobId: "pipeline-auto",
      documentId: "doc-1",
      workspacePath: "/out/work",
      stages: STAGES,
      resumed: true,
    });
    mocks.cancelLocalPipeline.mockResolvedValue(undefined);
    renderPanel();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    });
    expect(mocks.cancelLocalPipeline).toHaveBeenCalled();
    const event: LocalPipelineStageEvent = {
      jobId: "pipeline-auto",
      documentId: "doc-1",
      stage: "cancelled",
    };
    await act(async () => {
      (listeners.cancelled as unknown as (p: LocalPipelineStageEvent) => void)(event);
    });
    expect(screen.getByText(/pressing Resume continues from where it stopped/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("ignores events belonging to a different job", async () => {
    const listeners = captureListeners();
    mocks.pickOutputDestination.mockResolvedValue("/out/final.pdf");
    mocks.pickPackageDirectory.mockResolvedValue("/out/work");
    mocks.startLocalPipeline.mockResolvedValue({
      jobId: "pipeline-auto",
      documentId: "doc-1",
      workspacePath: "/out/work",
      stages: STAGES,
      resumed: false,
    });
    renderPanel();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose the finished PDF/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose a working folder/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
    });
    await act(async () => {
      (listeners.completed as unknown as (p: LocalPipelineCompleted) => void)({
        jobId: "some-other-job",
        documentId: "doc-1",
        status: "completed",
        bookmarkStatus: "written",
        stageReached: "validating",
        outputPath: "/elsewhere.pdf",
        pageCount: 1,
        ocrPages: 1,
        ocrErrors: 0,
        bookmarksWritten: 0,
        bookmarksAutoConfirmed: 0,
        bookmarksNeedingReview: 0,
        bookmarksSkipped: 0,
        textLayerWords: 1,
        refusalReason: null,
        ocrProvenance: [],
        providerMode: "local",
        cloudFallbackPages: [],
        cloudInputTokens: 0,
        cloudOutputTokens: 0,
        cloudUsageMayBeIncomplete: false,
        creditsReserved: 0,
        creditsCharged: 0,
        creditsReleased: 0,
        creditsRefunded: 0,
        creditsSettled: false,
        notProductionReadyReason: null,
      });
    });
    expect(screen.queryByText("/elsewhere.pdf")).toBeNull();
  });
});
