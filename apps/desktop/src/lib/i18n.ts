import { useSyncExternalStore } from "react";
import { english } from "./messages";
export type Locale = "zh" | "en";
const storageKey = "museion-binarize.locale";
export function initialLocale(): Locale {
  try { const saved = localStorage.getItem(storageKey); if (saved === "zh" || saved === "en") return saved; } catch { /* Storage can be unavailable. */ }
  return typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}
let locale = initialLocale();
const listeners = new Set<() => void>();
export function setLocale(value: Locale) {
  locale = value;
  try { localStorage.setItem(storageKey, value); } catch { /* Switching still works without persistence. */ }
  if (typeof document !== "undefined") document.documentElement.lang = value === "zh" ? "zh-CN" : "en";
  listeners.forEach(listener => listener());
}
export function useLocale() {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => locale);
}
export function t(source: string, ...values: unknown[]): string {
  const template = locale === "en" ? english[source] ?? source : source;
  return template.replace(/\{(\d+)\}/g, (token, index) => Number(index) < values.length ? String(values[Number(index)]) : token);
}

// Translate stored UI notices and known backend messages at display time.
// Never use this on file names, document text, editable titles or error details.
export function localizeMessage(message: string, depth = 0): string {
  if (Object.prototype.hasOwnProperty.call(english, message)) return t(message);
  for (const [source, translated] of Object.entries(english)) {
    if (message === translated) return locale === "en" ? translated : source;
    if (!source.includes("{0}")) continue;
    for (const template of [source, translated]) {
      const indices: number[] = [];
      const pattern = template.split(/(\{\d+\})/).map(part => {
        if (/^\{\d+\}$/.test(part)) { indices.push(Number(part.slice(1,-1))); return "([\\s\\S]*?)"; }
        return part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      }).join("");
      const match = new RegExp("^" + pattern + "$", "u").exec(message);
      if (match) { const values: string[] = []; indices.forEach((index, i) => { values[index] = depth < 3 ? localizeMessage(match[i+1], depth + 1) : match[i+1]; }); return t(source, ...values); }
    }
  }
  return message;
}
