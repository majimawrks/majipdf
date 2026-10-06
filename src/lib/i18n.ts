import en from "../i18n/en.json";
import id from "../i18n/id.json";
import { isMac, settings } from "./state.svelte";

export type Lang = "en" | "id";
type Dict = typeof en;
const dicts: Record<Lang, Dict> = { en, id };

export function t(key: keyof Dict, params?: Record<string, string | number>): string {
  const dict = dicts[settings.lang] ?? dicts.en;
  let s = dict[key] ?? String(key);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.replaceAll(`{${k}}`, String(v));
    }
  }
  // macOS: path separator and modifier key (same strings serve both languages)
  return isMac ? s.replaceAll("Documents\\majipdf", "Documents/majipdf").replaceAll("Ctrl", "⌘") : s;
}
