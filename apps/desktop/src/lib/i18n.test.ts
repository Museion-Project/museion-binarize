import { describe, expect, it, vi } from "vitest";
import { initialLocale, localizeMessage, setLocale, t } from "./i18n";

describe("locale preferences and messages",()=>{
 it("uses a saved preference before the system language",()=>{
  vi.spyOn(window.navigator,"language","get").mockReturnValue("en-GB");
  localStorage.removeItem("museion-binarize.locale");expect(initialLocale()).toBe("en");
  setLocale("zh");expect(initialLocale()).toBe("zh");
  localStorage.setItem("museion-binarize.locale","invalid");expect(initialLocale()).toBe("en");
  vi.restoreAllMocks();
 });
 it("switches despite unavailable storage and preserves interpolation values",()=>{
  vi.spyOn(Storage.prototype,"setItem").mockImplementation(()=>{throw new Error("unavailable");});
  expect(()=>setLocale("en")).not.toThrow();
  expect(t("无法保存新 PDF：{0}","/tmp/中文.pdf")).toBe("Unable to save the new PDF: /tmp/中文.pdf");
  expect(localizeMessage("Apple 暂不可用，本次保留基础目录层级：系统模型尚未就绪（modelNotReady）")).toBe("Apple is unavailable. Keeping the basic contents hierarchy: The system model is not ready (modelNotReady)");
  expect(localizeMessage("处理页需在 1–12 之间。")).toBe("Pages to process must be between 1 and 12.");
  setLocale("zh");expect(localizeMessage("Pages to process must be between 1 and 12.")).toBe("处理页需在 1–12 之间。");
  expect(localizeMessage("Unknown diagnostic /tmp/中文.pdf")).toBe("Unknown diagnostic /tmp/中文.pdf");
  vi.restoreAllMocks();
 });
});
