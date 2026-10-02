import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SELECTION } from "../app/ocrProvider";
import type { OcrProviderSelection } from "../app/ocrProvider";
import type { OcrProviderStatus } from "../app/types";
import { OcrProviderPicker } from "./OcrProviderPicker";

const mocks = vi.hoisted(() => ({
  ocrProviderStatus: vi.fn(),
}));

vi.mock("../lib/tauri", () => mocks);

function status(): OcrProviderStatus {
  return {
    defaultMode: "local",
    modes: [
      {
        id: "local",
        displayName: "Local OCR plugin",
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
        id: "mpdf-credits",
        displayName: "M PDF Cloud OCR",
        defaultMode: false,
        usesNetwork: true,
        requiresCredential: false,
        executionLocation: "remote:mpdf-brokered",
        productionReady: false,
        availability: "unavailable",
        blockers: ["no payment provider is integrated"],
        model: null,
        credentialPresent: false,
        structuredBboxDefaultEnabled: false,
      },
    ],
  };
}

/** Renders the picker with the selection state a real parent would hold. */
function renderPicker(initial: OcrProviderSelection = DEFAULT_SELECTION) {
  const state = { current: initial };
  const onChange = vi.fn((next: OcrProviderSelection) => {
    state.current = next;
    rerender(next);
  });
  const view = render(
    <OcrProviderPicker value={initial} onChange={onChange} disabled={false} pageCount={412} />,
  );
  function rerender(next: OcrProviderSelection) {
    view.rerender(
      <OcrProviderPicker value={next} onChange={onChange} disabled={false} pageCount={412} />,
    );
  }
  return { state, onChange };
}

describe("OcrProviderPicker", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.ocrProviderStatus.mockResolvedValue(status());
  });

  it("preselects local and says plainly that nothing leaves the machine", async () => {
    renderPicker();
    const local = await screen.findByRole("radio", { name: /Local OCR/ });
    expect(local).toBeChecked();
    expect(screen.getByText(/Native-text PDFs work in the base app/)).toBeTruthy();
    expect(screen.getByText(/optional offline OCR plugin/)).toBeTruthy();
  });

  it("has no BYOK choice or credential input", async () => {
    renderPicker();
    await screen.findByRole("radio", { name: /Local OCR/ });
    expect(screen.queryByText(/Use My Key/)).toBeNull();
    expect(screen.queryByPlaceholderText(/API key/i)).toBeNull();
    expect(document.querySelector('input[type="password"]')).toBeNull();
  });

  it("shows the brokered mode's blockers and refuses to let it start", async () => {
    renderPicker({
      ...DEFAULT_SELECTION,
      mode: "mpdf-credits",
      ready: false,
      blockedReason: "no payment provider is integrated",
    });
    const credits = await screen.findByRole("radio", { name: /M PDF Cloud OCR/ });
    expect(screen.getByText("no payment provider is integrated")).toBeTruthy();
    expect(credits).toBeDisabled();
    expect(screen.getByText(/Paid and brokered/)).toBeTruthy();
    expect(screen.getByRole("checkbox", { name: /Upload 412/ })).toBeDisabled();
    expect(screen.getByLabelText(/Never spend more than/)).toBeDisabled();
  });
});
