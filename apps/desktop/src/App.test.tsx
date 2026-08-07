import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({
    name: "Museion Binarize",
    phase: "Phase 1 — under development",
  }),
}));

describe("App", () => {
  it("renders the project name and phase, and disables PDF opening", async () => {
    render(<App />);

    expect(await screen.findByText("Museion Binarize")).toBeInTheDocument();
    expect(
      screen.getByText("Phase 1 — under development"),
    ).toBeInTheDocument();

    const openButton = screen.getByRole("button", {
      name: /open pdf/i,
    });
    expect(openButton).toBeDisabled();
  });
});
