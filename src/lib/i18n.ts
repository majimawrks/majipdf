import en from "../i18n/en.json";
import id from "../i18n/id.json";
import { settings } from "./state.svelte";

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
  return s;
}
