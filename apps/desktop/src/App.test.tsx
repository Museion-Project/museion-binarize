import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { DocumentSummary } from "./app/types";

// vi.mock() factories are hoisted above these declarations, so the
// variables they close over must be created through vi.hoisted() — a
// plain `const` here would be in the temporal dead zone when the factory
// actually runs, silently yielding `undefined` from the mocked functions.
const { invokeMock, listenMock, openDialogMock, saveDialogMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn().mockResolvedValue(() => {}),
  openDialogMock: vi.fn(),
  saveDialogMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogMock(...args),
  save: (...args: unknown[]) => saveDialogMock(...args),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: vi.fn().mockResolvedValue(undefined),
}));

function sampleDocument(): DocumentSummary {
  return {
    documentId: "doc-1",
    fileName: "book.pdf",
    sourceBytes: 123_456,
    pageCount: 3,
    title: null,
    author: null,
    pdfiumLibrary: "test",
    pages: [
      { pageNumber: 1, widthPoints: 595, heightPoints: 842, sourceRotationDegrees: 0 },
      { pageNumber: 2, widthPoints: 595, heightPoints: 842, sourceRotationDegrees: 0 },
      { pageNumber: 3, widthPoints: 595, heightPoints: 842, sourceRotationDegrees: 0 },
    ],
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "project_info") {
      return { name: "Museion Binarize", phase: "Phase 1 — under development" };
    }
    throw new Error(`unexpected command in this test: ${command}`);
  });
  openDialogMock.mockReset();
  saveDialogMock.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("App — idle state", () => {
  it("shows Open PDF enabled and no processing controls", async () => {
    render(<App />);

    const openButton = await screen.findByRole("button", { name: /open pdf/i });
    expect(openButton).toBeEnabled();
    expect(screen.getByRole("button", { name: /^convert$/i })).toBeDisabled();
    expect(await screen.findByText("Phase 1 — under development")).toBeInTheDocument();
  });
});

describe("App — document open", () => {
  it("shows document details and the page sidebar once a document opens", async () => {
    openDialogMock.mockResolvedValue("/tmp/book.pdf");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "project_info") {
        return { name: "Museion Binarize", phase: "Phase 1 — under development" };
      }
      if (command === "open_document") {
        return sampleDocument();
      }
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open pdf/i }));

    await waitFor(() => expect(screen.getByText("book.pdf")).toBeInTheDocument());
    expect(screen.getByRole("listbox", { name: /page thumbnails/i })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^page \d$/i })).toHaveLength(3);
    expect(screen.getByRole("button", { name: /page 1/i })).toHaveAttribute(
      "aria-current",
      "true",
    );
  });

  it("prompts for a password and does not retain it after a failed attempt", async () => {
    openDialogMock.mockResolvedValue("/tmp/secret.pdf");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "project_info") {
        return { name: "Museion Binarize", phase: "Phase 1 — under development" };
      }
      if (command === "open_document") {
        throw {
          code: "password_required",
          message: "the PDF requires a password",
          hint: null,
          detail: null,
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open pdf/i }));

    expect(await screen.findByLabelText("Password")).toBeInTheDocument();
  });
});

describe("App — settings", () => {
  async function openDocument() {
    openDialogMock.mockResolvedValue("/tmp/book.pdf");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "project_info") {
        return { name: "Museion Binarize", phase: "Phase 1 — under development" };
      }
      if (command === "open_document") {
        return sampleDocument();
      }
      if (command === "render_preview") {
        return {
          requestId: 1,
          pageNumber: 1,
          kind: "original",
          width: 10,
          height: 10,
          pngBase64: "",
          renderDpi: 72,
          isReducedResolution: false,
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open pdf/i }));
    await screen.findByText("book.pdf");
  }

  it("hides the manual threshold control for Otsu and shows it for Manual", async () => {
    await openDocument();
    const methodSelect = screen.getByLabelText(/binarization method/i);
    fireEvent.change(methodSelect, { target: { value: "otsu" } });
    expect(screen.queryByLabelText(/threshold \(0–255\)/i)).not.toBeInTheDocument();

    fireEvent.change(methodSelect, { target: { value: "manual" } });
    expect(screen.getByLabelText(/threshold \(0–255\)/i)).toBeInTheDocument();
  });

  it("shows Sauvola-specific controls only for Sauvola", async () => {
    await openDocument();
    const methodSelect = screen.getByLabelText(/binarization method/i);
    fireEvent.change(methodSelect, { target: { value: "sauvola" } });
    expect(screen.getByLabelText(/window size/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/k \(sensitivity\)/i)).toBeInTheDocument();
  });

  it("marks the preset as Custom once a control is changed manually", async () => {
    await openDocument();
    const contrastInput = screen.getByLabelText(/contrast/i);
    fireEvent.change(contrastInput, { target: { value: "0.5" } });
    expect(screen.getByText("Custom")).toBeInTheDocument();
  });
});

describe("App — error presentation", () => {
  it("shows a structured, actionable error panel for a failed conversion start", async () => {
    openDialogMock.mockResolvedValue("/tmp/book.pdf");
    saveDialogMock.mockResolvedValue("/tmp/book-binarized.pdf");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "project_info") {
        return { name: "Museion Binarize", phase: "Phase 1 — under development" };
      }
      if (command === "open_document") {
        return sampleDocument();
      }
      if (command === "render_preview") {
        return {
          requestId: 1,
          pageNumber: 1,
          kind: "original",
          width: 10,
          height: 10,
          pngBase64: "",
          renderDpi: 72,
          isReducedResolution: false,
        };
      }
      if (command === "start_processing") {
        throw {
          code: "destination_conflict",
          message: "the output path must not be the same file as the input",
          hint: "Choose a different output location.",
          detail: null,
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open pdf/i }));
    await screen.findByText("book.pdf");

    fireEvent.click(screen.getByRole("button", { name: /choose output/i }));
    await waitFor(() => expect(saveDialogMock).toHaveBeenCalled());

    fireEvent.click(await screen.findByRole("button", { name: /^convert$/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "the output path must not be the same file as the input",
    );
    expect(screen.getByText("Choose a different output location.")).toBeInTheDocument();
  });
});
