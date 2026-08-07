import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// vitest.config.ts does not set `test.globals`, so @testing-library/react's
// own auto-cleanup (which relies on a global `afterEach`) never registers.
// Without this, each test's rendered tree stays in the DOM for the next
// test, causing "multiple elements found" failures.
afterEach(() => {
  cleanup();
});

// jsdom does not implement IntersectionObserver, which the lazy thumbnail
// loader (PageThumbnail) depends on. A minimal stub is enough for tests:
// nothing here asserts on lazy-loading behavior itself, only that
// rendering the sidebar does not throw.
class IntersectionObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
}
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).IntersectionObserver = IntersectionObserverStub;

// jsdom does not implement scrollIntoView either (used to keep the
// selected thumbnail visible).
if (!window.HTMLElement.prototype.scrollIntoView) {
  window.HTMLElement.prototype.scrollIntoView = () => {};
}
