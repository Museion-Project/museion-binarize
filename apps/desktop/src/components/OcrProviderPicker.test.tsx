import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SELECTION } from "../app/ocrProvider";
import type { OcrProviderSelection } from "../app/ocrProvider";
import type { OcrProviderStatus } from "../app/types";
import { OcrProviderPicker } from "./OcrProviderPicker";

const mocks = vi.hoisted(() => ({
  ocrProviderStatus: vi.fn(),
  storeModelProviderCredential: vi.fn(),
  modelProviderCredentialStatus: vi.fn(),
  deleteModelProviderCredential: vi.fn(),
  testOcrProvider: vi.fn(),
}));

vi.mock("../lib/tauri", () => mocks);

function status(credentialPresent: boolean): OcrProviderStatus {
  return {
    defaultMode: "local",
    modes: [
      {
        id: "local",
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
        id: "gemini-byok",
        displayName: "Gemini API — Use My Key",
        defaultMode: false,
        usesNetwork: true,
        requiresCredential: true,
        executionLocation: "remote:user-key",
        productionReady: false,
        availability: "beta",
        blockers: ["live validation pending"],
        model: "gemini-3.7-flash",
        credentialPresent,
        structuredBboxDefaultEnabled: false,
      },
      {
        id: "mpdf-credits",
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
    mocks.ocrProviderStatus.mockResolvedValue(status(false));
  });

  it("preselects local and says plainly that nothing leaves the machine", async () => {
    renderPicker();
    const local = await screen.findByRole("radio", { name: /Local OCR/ });
    expect(local).toBeChecked();
    expect(screen.getByText(/Nothing leaves this machine/)).toBeTruthy();
  });

  it("names the real number of pages that would be uploaded", async () => {
    const { state } = renderPicker();
    await screen.findByRole("radio", { name: /Use My Key/ });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /Use My Key/ }));
    });
    expect(state.current.mode).toBe("gemini-byok");
    expect(
      screen.getByText(/Upload 412 rendered page images of this document/),
    ).toBeTruthy();
  });

  it("blocks a cloud run until consent is given and a key is stored", async () => {
    const { state } = renderPicker();
    await screen.findByRole("radio", { name: /Use My Key/ });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /Use My Key/ }));
    });
    expect(state.current.ready).toBe(false);
    expect(state.current.blockedReason).toMatch(/page images will be uploaded/);

    await act(async () => {
      fireEvent.click(screen.getByRole("checkbox", { name: /Upload 412/ }));
    });
    // Consent alone is not enough: there is still no key.
    expect(state.current.cloudConsent).toBe(true);
    expect(state.current.ready).toBe(false);
    expect(state.current.blockedReason).toMatch(/no key is stored/);
  });

  it("never renders a stored key and clears the field once it is saved", async () => {
    mocks.storeModelProviderCredential.mockResolvedValue({
      slot: "default",
      present: true,
      masked: "****",
    });
    const { state } = renderPicker();
    await screen.findByRole("radio", { name: /Use My Key/ });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /Use My Key/ }));
    });
    const field = screen.getByPlaceholderText("paste your API key") as HTMLInputElement;
    // A password field, so the value is not shoulder-readable either.
    expect(field.type).toBe("password");
    await act(async () => {
      fireEvent.change(field, { target: { value: "AIzaSyCANARY-0123456789" } });
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("checkbox", { name: /Upload 412/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /keychain/ }));
    });
    await waitFor(() => expect(mocks.storeModelProviderCredential).toHaveBeenCalled());
    expect(
      (screen.getByPlaceholderText("paste your API key") as HTMLInputElement).value,
    ).toBe("");
    expect(state.current.ready).toBe(true);
    expect(state.current.blockedReason).toBeNull();
    // The key must not survive anywhere in the rendered document.
    expect(document.body.innerHTML).not.toContain("AIzaSyCANARY");
  });

  it("returns to local when the key is removed", async () => {
    mocks.ocrProviderStatus.mockResolvedValue(status(true));
    mocks.deleteModelProviderCredential.mockResolvedValue({
      slot: "default",
      present: false,
      masked: "****",
    });
    const { state } = renderPicker();
    await screen.findByRole("radio", { name: /Use My Key/ });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /Use My Key/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Remove the key/ }));
    });
    await waitFor(() => expect(state.current.mode).toBe("local"));
    expect(state.current.cloudConsent).toBe(false);
  });

  it("shows the brokered mode's blockers and refuses to let it start", async () => {
    const { state } = renderPicker();
    await screen.findByRole("radio", { name: /M PDF Cloud OCR/ });
    expect(screen.getByText("no payment provider is integrated")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /M PDF Cloud OCR/ }));
    });
    expect(state.current.ready).toBe(false);
    expect(state.current.blockedReason).toMatch(/payment provider/);
    // Its consent box is disabled too: there is nothing to consent to.
    expect(screen.getByRole("checkbox", { name: /Upload 412/ })).toBeDisabled();
  });

  it("defaults the failure policy to keeping a usable local result", async () => {
    renderPicker();
    await screen.findByRole("radio", { name: /Use My Key/ });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: /Use My Key/ }));
    });
    const select = screen.getByLabelText(/If a page fails/) as HTMLSelectElement;
    expect(select.value).toBe("local");
  });
});
