// Organize tool logic: pure page-list operations, then the state-backed load/insert/run part.
import { app, settings, openTool, settled, requestCancel, type FileInfo, type OrgPage } from "./state.svelte";
import * as api from "./tauri";

type Sel = ReadonlySet<number>;

export const norm = (deg: number) => ((deg % 360) + 360) % 360;

export const rotate = (pages: OrgPage[], sel: Sel, deg: number): OrgPage[] =>
  pages.map((p) => (sel.has(p.id) ? { ...p, rotation: p.rotation + deg } : p));

export const remove = (pages: OrgPage[], sel: Sel): OrgPage[] => pages.filter((p) => !sel.has(p.id));

// Shift every selected page one step (-1 earlier, +1 later); a selected block stops at the edge, others slide past it.
export function moveBy(pages: OrgPage[], sel: Sel, d: -1 | 1): OrgPage[] {
  const a = [...pages];
  const n = a.length;
  for (let k = 0; k < n; k++) {
    const i = d < 0 ? k : n - 1 - k;
    const j = i + d;
    if (j >= 0 && j < n && sel.has(a[i].id) && !sel.has(a[j].id)) [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

// New pages go after the last selected page, or at the end when nothing is selected.
export function insertAfter(pages: OrgPage[], sel: Sel, add: OrgPage[]): OrgPage[] {
  let at = pages.length;
  if (sel.size) at = pages.reduce((m, p, i) => (sel.has(p.id) ? i + 1 : m), 0);
  return [...pages.slice(0, at), ...add, ...pages.slice(at)];
}

export function moveOne(pages: OrgPage[], from: number, to: number): OrgPage[] {
  if (from === to || from < 0 || to < 0 || from >= pages.length || to >= pages.length) return pages;
  const a = [...pages];
  a.splice(to, 0, a.splice(from, 1)[0]);
  return a;
}

// ids from a to b inclusive, whichever comes first
export function rangeIds(pages: OrgPage[], a: number, b: number): Set<number> {
  const i = pages.findIndex((p) => p.id === a);
  const j = pages.findIndex((p) => p.id === b);
  if (i < 0 || j < 0) return new Set(j < 0 ? [] : [b]);
  return new Set(pages.slice(Math.min(i, j), Math.max(i, j) + 1).map((p) => p.id));
}

export const original = (total: number): OrgPage[] =>
  Array.from({ length: total }, (_, i) => ({ id: i, src: 0, page: i, rotation: 0, isNew: false }));

export function changeCounts(pages: OrgPage[], total: number) {
  const kept = pages.filter((p) => !p.isNew);
  const rotated = pages.filter((p) => norm(p.rotation) !== 0).length;
  const deleted = total - kept.length;
  const added = pages.length - kept.length;
  // ponytail: "reordered" = the surviving original pages are no longer in ascending order (a delete alone is not a reorder).
  const moved = kept.some((p, i) => i > 0 && p.page < kept[i - 1].page);
  return { rotated, deleted, added, moved, any: rotated + deleted + added > 0 || moved };
}

export function selfTest() {
  const ids = (a: OrgPage[]) => a.map((p) => p.id).join(",");
  const S = (...x: number[]) => new Set(x);
  const base = original(5);
  console.assert(ids(moveBy(base, S(2), -1)) === "0,2,1,3,4", "earlier one");
  console.assert(ids(moveBy(base, S(0), -1)) === "0,1,2,3,4", "earlier stops at edge");
  console.assert(ids(moveBy(base, S(0, 1), -1)) === "0,1,2,3,4", "earlier block at edge");
  console.assert(ids(moveBy(base, S(1, 3), -1)) === "1,0,3,2,4", "earlier two apart");
  console.assert(ids(moveBy(base, S(1, 2), 1)) === "0,3,1,2,4", "later block");
  console.assert(ids(moveBy(base, S(4), 1)) === "0,1,2,3,4", "later stops at edge");
  console.assert(ids(moveBy(base, S(3, 4), 1)) === "0,1,2,3,4", "later block at edge");
  const r = rotate(base, S(1), -90);
  console.assert(r[1].rotation === -90 && norm(r[1].rotation) === 270 && r[0].rotation === 0, "rotate left");
  console.assert(norm(rotate(r, S(1), 90)[1].rotation) === 0, "rotate back");
  console.assert(ids(remove(base, S(0, 4))) === "1,2,3", "delete");
  const add = [{ id: 9, src: 1, page: 0, rotation: 0, isNew: true }, { id: 10, src: 1, page: 1, rotation: 0, isNew: true }];
  console.assert(ids(insertAfter(base, S(1, 3), add)) === "0,1,2,3,9,10,4", "insert after last selected");
  console.assert(ids(insertAfter(base, S(), add)) === "0,1,2,3,4,9,10", "insert at end");
  console.assert(ids(moveOne(base, 0, 3)) === "1,2,3,0,4", "moveOne");
  console.assert(ids(moveOne(base, 3, 1)) === "0,3,1,2,4", "moveOne back");
  console.assert([...rangeIds(base, 3, 1)].join() === "1,2,3", "range reversed");
  console.assert(!changeCounts(base, 5).any, "no changes at start");
  console.assert(changeCounts(remove(base, S(1)), 5).deleted === 1 && !changeCounts(remove(base, S(1)), 5).moved, "delete is not a move");
  console.assert(changeCounts(moveBy(base, S(2), -1), 5).moved, "move counted");
  const c = changeCounts(insertAfter(rotate(base, S(0, 1), 90), S(), add), 5);
  console.assert(c.rotated === 2 && c.added === 2 && c.deleted === 0 && c.any, "counts");
  console.assert(changeCounts(rotate(rotate(base, S(0), 90), S(0), -90), 5).rotated === 0, "rotate cancels");
  console.log("organize selfTest passed (no assertion failures above)");
}
if (import.meta.env.DEV) selfTest();

// ---- state-backed part ----------------------------------------------------
const o = app.organize;

function setFile(f: FileInfo) {
  delete app.compress.passwords[f.path]; // an encrypted file arrives without a page count: ask for the password again
  app.tool.files = [f];
  app.tool.phase = "loaded";
}

// Single-file tool: a new choice replaces the current one; with unsaved edits it waits for confirmation in the tool.
export function replaceFile(f: FileInfo) {
  if (o.pages.length && changeCounts(o.pages, o.total).any && app.tool.files[0] && app.tool.phase === "loaded") {
    o.pendingFile = f;
    return;
  }
  o.pendingFile = null;
  setFile(f);
}

export function confirmReplace() {
  const f = o.pendingFile;
  o.pendingFile = null;
  if (!f) return;
  o.file = ""; // even the same path starts over
  setFile(f);
}

// Called once the file's page count is known (immediately, or after unlocking).
export function initPages(f: FileInfo, password: string | null) {
  o.file = f.path;
  o.total = f.pages ?? 0;
  o.nextId = o.total;
  o.pages = original(o.total);
  o.sources = [{ path: f.path, password, name: f.name, signed: f.signed }];
  o.selection = new Set();
  o.anchor = null;
  o.pendingFile = null;
}

export function undoAll() {
  o.pages = original(o.total);
  o.nextId = o.total;
  o.sources = o.sources.slice(0, 1);
  o.selection = new Set();
  o.anchor = null;
}

export function insertFile(f: FileInfo, password: string | null) {
  if (!f.pages) return;
  let src = o.sources.findIndex((s) => s.path === f.path);
  if (src < 0) src = o.sources.push({ path: f.path, password, name: f.name, signed: f.signed }) - 1;
  const add: OrgPage[] = Array.from({ length: f.pages }, (_, i) => ({ id: o.nextId++, src, page: i, rotation: 0, isNew: true }));
  o.pages = insertAfter(o.pages, o.selection, add);
}

export const usedSources = () => [...new Set(o.pages.map((p) => p.src))].sort((a, b) => a - b).map((i) => o.sources[i]);

let runId = 0;

export async function startRun() {
  if (!o.pages.length || !changeCounts(o.pages, o.total).any) return;
  const id = ++runId;
  const r = o.run;
  r.name = o.sources[0].name.replace(/\.pdf$/i, "") + "_organized.pdf";
  r.progress = null;
  r.result = null;
  r.error = "";
  app.tool.phase = "running";
  const unlisten = await api.onOrganizeProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.organize({
      sources: o.sources.map((s) => ({ path: s.path, password: s.password })),
      pages: o.pages.map((p) => ({ src: p.src, page: p.page, rotation: norm(p.rotation) as 0 | 90 | 180 | 270 })),
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

export const cancelRun = () => requestCancel(api.organizeCancel);

export const processAnother = () => openTool("organize");
