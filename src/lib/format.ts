import { settings } from "./state.svelte";
import { t } from "./i18n";

const loc = () => (settings.lang === "id" ? "id-ID" : "en-US");

export function fmtNum(n: number, maxFrac = 0): string {
  return new Intl.NumberFormat(loc(), { maximumFractionDigits: maxFrac }).format(n);
}

// KB below 1 MB, then MB (2 decimals under 10 MB, whole numbers above), locale separators.
export function fmtSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return fmtNum(Math.max(1, Math.round(bytes / 1024))) + " KB";
  return fmtNum(mb, mb < 10 ? 2 : 0) + " MB";
}

export function fmtElapsed(sec: number): string {
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
}

export function fmtPages(n: number): string {
  return n === 1 ? t("pages1") : t("pagesN", { n: fmtNum(n) });
}
