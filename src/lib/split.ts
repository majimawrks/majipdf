// Split tool logic: turning the user's choice into page groups, and running/cancelling.
import { app, settings, openTool, settled, requestCancel, type FileInfo } from "./state.svelte";
import * as api from "./tauri";

export type Group = { from: number; to: number };
export type ParseError = { key: "rangeErr" | "rangeErrFormat"; params: Record<string, string | number> };
export type Parsed = { groups: Group[] } | { error: ParseError };

// "1-3, 5, 8-10" -> [{1,3},{5,5},{8,10}], in the order typed. 1-based, inclusive.
export function parseRanges(text: string, pages: number): Parsed {
  const bad = (x: string): Parsed => ({ error: { key: "rangeErrFormat", params: { x } } });
  if (!text.trim()) return bad("");
  const groups: Group[] = [];
  for (const raw of text.split(",")) {
    const tok = raw.trim();
    const m = /^(\d+)(?:-(\d+))?$/.exec(tok);
    if (!m) return bad(tok);
    const from = Number(m[1]);
    const to = m[2] === undefined ? from : Number(m[2]);
    if (to < from) return bad(tok);
    for (const p of [from, to]) if (p < 1 || p > pages) return { error: { key: "rangeErr", params: { p, max: pages } } };
    groups.push({ from, to });
  }
  return { groups };
}

export const everyPage = (pages: number): Group[] => Array.from({ length: pages }, (_, i) => ({ from: i + 1, to: i + 1 }));

export function everyN(pages: number, n: number): Group[] {
  const out: Group[] = [];
  for (let from = 1; from <= pages; from += n) out.push({ from, to: Math.min(pages, from + n - 1) });
  return out;
}

export function selfTest() {
  const g = (s: string, pages = 10) => JSON.stringify(parseRanges(s, pages));
  console.assert(g("1-3, 5, 8-10") === '{"groups":[{"from":1,"to":3},{"from":5,"to":5},{"from":8,"to":10}]}', "ranges");
  console.assert(g(" 2 , 4-4 ") === '{"groups":[{"from":2,"to":2},{"from":4,"to":4}]}', "spaces");
  console.assert(g("0").includes('"rangeErr"'), "page 0");
  console.assert(g("1-11").includes('"p":11'), "out of bounds");
  console.assert(g("3-1").includes('"rangeErrFormat"'), "reversed");
  console.assert(g("a").includes('"x":"a"'), "bad token");
  console.assert(g("").includes('"rangeErrFormat"'), "empty");
  console.assert(g("1,,2").includes('"rangeErrFormat"'), "empty token");
  console.assert(everyPage(3).length === 3, "everyPage");
  console.assert(JSON.stringify(everyN(5, 2)) === '[{"from":1,"to":2},{"from":3,"to":4},{"from":5,"to":5}]', "everyN");
  console.log("split selfTest passed (no assertion failures above)");
}
if (import.meta.env.DEV) selfTest();

// ---- state-backed part ----------------------------------------------------
const s = app.split;
export const defaultRanges = (pages: number) => (pages >= 10 ? "1-3, 5, 8-10" : `1-${Math.max(1, pages)}`);

// Split takes one file: a new choice replaces the current one (SplitTool resets the options when the file changes).
export function replaceFile(f: FileInfo) {
  app.tool.files = [f];
  app.tool.phase = "loaded";
}

export function groupsFor(pages: number): Parsed {
  if (s.mode === "every") return { groups: everyPage(pages) };
  if (s.mode === "n") {
    const n = Number(s.n);
    return Number.isInteger(n) && n >= 1 && n <= pages
      ? { groups: everyN(pages, n) }
      : { error: { key: "rangeErrFormat", params: { x: s.n } } };
  }
  return parseRanges(s.ranges, pages);
}

let runId = 0;

export async function startRun(groups: Group[]) {
  const f = app.tool.files[0];
  if (!f || !groups.length) return;
  const id = ++runId;
  const r = s.run;
  r.name = f.name;
  r.total = groups.length;
  r.progress = null;
  r.result = null;
  r.error = "";
  app.tool.phase = "running";
  const unlisten = await api.onSplitProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.split({
      path: f.path,
      password: app.compress.passwords[f.path] ?? null,
      groups,
      out_mode: settings.outMode,
    });
    if (settled(undefined)) return;
    if (id !== runId) return;
    r.result = res;
    app.tool.phase = "done";
  } catch (e) {
    if (settled(e) || id !== runId) return;
    r.error = String(e);
    app.tool.phase = "error";
  } finally {
    unlisten();
  }
}

export const cancelRun = () => requestCancel(api.splitCancel);

export const processAnother = () => openTool("split");
