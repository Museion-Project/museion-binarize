import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  prepareApiPlan: vi.fn(),
  runApiTask: vi.fn(),
  cancelApiTask: vi.fn(),
}));
vi.mock("../lib/tauri", () => api);
import { ApiRoutingPanel } from "./ApiRoutingPanel";

describe("API OCR workflow", () => {
  it("requires explicit confirmation and exposes progress, cancel, retention and fallback", () => {
    const confirm = vi.fn(); const cancel = vi.fn();
    render(<ApiRoutingPanel summary={{ origin: "https://fixture.test", provider: "fixture", model: "ocr", sourceDigest: "a".repeat(64), sourceBytes: 4, pageCount: 1, budgetMicros: 100, retention: "delete_after_result" }} onConfirm={confirm} onCancel={cancel} progress="running" costMicros={2} fallbackReason="service unavailable" />);
    fireEvent.click(screen.getByRole("button", { name: "Cloud enhanced" }));
    expect(screen.getByText(/fixture.test/)).toBeInTheDocument();
    expect(screen.getByText(/running/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox")); expect(confirm).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Cancel task" })); expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("prepares a digest-bound plan and runs only after affirmative consent", async () => {
    api.prepareApiPlan.mockResolvedValue({ planDigest: "b".repeat(64), origin: "https://api.test", provider: "mpdf-api", model: "ocr-1", sourceDigest: "a".repeat(64), sourceBytes: 20, pageCount: 2, budgetMicros: 1_000_000, currency: "USD", retention: "delete_after_result" });
    api.runApiTask.mockResolvedValue({ taskId: "task", state: "result_installed", usedCostMicros: 10, budgetMicros: 1_000_000, retention: "acknowledged", artifactPath: "/artifact", fallbackReason: null });
    render(<ApiRoutingPanel documentId="doc-1" />);
    fireEvent.click(screen.getByRole("button", { name: "Cloud then local" }));
    fireEvent.change(screen.getByLabelText("API endpoint"), { target: { value: "https://api.test" } });
    fireEvent.click(screen.getByRole("button", { name: "Review upload plan" }));
    await screen.findByText(/api.test/);
    expect(api.runApiTask).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Start remote OCR" }));
    await waitFor(() => expect(api.runApiTask).toHaveBeenCalledWith(expect.objectContaining({ consent: "b".repeat(64), route: "api_then_local" })));
    expect(await screen.findByText(/result_installed/)).toBeInTheDocument();
  });
});
