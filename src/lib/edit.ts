// Edit PDF (inline text editing): contract types, command wrappers and state-backed session logic.
// See _docs/edit-contract.md. The backend keeps the session (and the job guard) from edit_open until edit_close.
import { app, settings, openTool, settled, setLeaveHook, type FileInfo, type EditTool } from "./state.svelte";
import { inTauri } from "./tauri";
import { t } from "./i18n";
import { sameStyle, textOf, mergeRuns, applyRange, styleOver, normRuns } from "./rich";

// Edit 2 (see _docs/edit2-contract.md)
export type Align = "left" | "center" | "right" | "justified";
export interface Style { font: string /* "orig" | PC family */; size: number; color: string; bold: boolean; italic: boolean; align: Align }
export type Rect = [number, number, number, number]; // display pt, top-left origin: x, y, w, h
export interface Run { text: string; style: Style }
export interface Obj { id: number; kind: "textbox" | "whiteout" | "image"; rect: Rect; rot: 0 | 90 | 180 | 270; text?: string; style?: Style; runs?: Run[]; asset?: string }
export interface ObjReq { kind: "textbox" | "whiteout" | "image" | "delete"; page: number; id: number | null; rect?: Rect; text?: string; style?: Style; runs?: Run[] | null; rot?: 0 | 90 | 180 | 270; asset?: string; cover_only?: boolean }
export type ObjRes =
  | { kind: "ok"; page: PageModel; id: number }
  | { kind: "missing_chars"; chars: string }
  | { kind: "unsupported_chars"; chars: string }
  | { kind: "whiteout_partial"; chars: number };
export interface SigItem { id: string; name: string; kind: "signature" | "stamp"; w: number; h: number; data_url: string }

export interface EditOpenReq { path: string; password: string | null }
export interface EditDoc { pages: number; sizes: [number, number][] }
export interface Para {
  id: number;
  bbox: [number, number, number, number]; // pt, top-left origin: x, y, w, h
  text: string;
  marker: string | null;
  align: "left" | "center" | "right" | "justified";
  size: number;
  pitch: number;
  first_indent: number;
  css_font: string;
  bold: boolean;
  italic: boolean;
  color: string;
  status: "direct" | "pc_font" | "locked";
  reason: null | "form" | "type3" | "rotated" | "structure" | "scan" | "invisible";
  edited: boolean;
  pc_font_used: boolean;
  rot: 0 | 90 | 180 | 270; // clockwise angle at which the text appears on the displayed page
  style: Style;
  runs: Run[];
  orig_font_name: string;
}
export interface PageModel { page: number; paras: Para[]; scan: boolean; objs: Obj[] }
export interface EditApplyReq { page: number; para: number; text: string; use_pc_font: boolean; allow_overlap: boolean; style?: Style | null; runs?: Run[] | null }
export type EditApplyRes =
  | { kind: "ok"; page: PageModel }
  | { kind: "missing_chars"; chars: string }
  | { kind: "unsupported_chars"; chars: string }
  | { kind: "overflow"; lines_over: number }
  | { kind: "cannot_push" };
export interface EditHistory { can_undo: boolean; can_redo: boolean; count: number; changed_pages: number[] }
export interface EditSaveReq { out_mode: "next" | "folder" }
export interface EditSaveRes { output: string; size: number; seconds: number }

// ---- commands -------------------------------------------------------------
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}
// Browser-dev only: a fake backend in edit.mock.ts (delete that file and these `mock` branches to drop it).
const mock = import.meta.env.DEV && !inTauri ? () => import("./edit.mock") : null;

export const editOpen = (req: EditOpenReq): Promise<EditDoc> => (mock ? mock().then((m) => m.open(req)) : invoke("edit_open", { req }));
export const editPage = (page: number): Promise<PageModel> => (mock ? mock().then((m) => m.page(page)) : invoke("edit_page", { page }));
export const editRender = (page: number, widthPx: number): Promise<string> =>
  mock ? mock().then((m) => m.render(page, widthPx)) : invoke("edit_render", { page, widthPx });
export const editApply = (req: EditApplyReq): Promise<EditApplyRes> => (mock ? mock().then((m) => m.apply(req)) : invoke("edit_apply", { req }));
export const editApplyObj = (req: ObjReq): Promise<ObjRes> => (mock ? mock().then((m) => m.applyObj(req)) : invoke("edit_apply_obj", { req }));
export const editFonts = (): Promise<string[]> => (mock ? mock().then((m) => m.fonts()) : invoke("edit_fonts"));
export const sigList = (): Promise<SigItem[]> => (mock ? mock().then((m) => m.sigList()) : invoke("sig_list"));
export const sigImport = (path: string, kind: SigItem["kind"]): Promise<SigItem> => (mock ? mock().then((m) => m.sigImport(path, kind)) : invoke("sig_import", { path, kind }));
export const sigAddDrawn = (dataUrl: string, name: string): Promise<SigItem> => (mock ? mock().then((m) => m.sigAddDrawn(dataUrl, name)) : invoke("sig_add_drawn", { dataUrl, name }));
export const sigRename = (id: string, name: string): Promise<void> => (mock ? mock().then((m) => m.sigRename(id, name)) : invoke("sig_rename", { id, name }));
export const sigDelete = (id: string): Promise<void> => (mock ? mock().then((m) => m.sigDelete(id)) : invoke("sig_delete", { id }));
export const editUndo = (): Promise<EditHistory> => (mock ? mock().then((m) => m.undo()) : invoke("edit_undo"));
export const editRedo = (): Promise<EditHistory> => (mock ? mock().then((m) => m.redo()) : invoke("edit_redo"));
export const editHistory = (): Promise<EditHistory> => (mock ? mock().then((m) => m.history()) : invoke("edit_history"));
export const editSave = (req: EditSaveReq): Promise<EditSaveRes> => (mock ? mock().then((m) => m.save(req)) : invoke("edit_save", { req }));
export const editClose = (): Promise<void> => (mock ? mock().then((m) => m.close()) : invoke("edit_close"));

// ---- session --------------------------------------------------------------
const e = app.edit;
let tok = 0; // invalidates an in-flight open when the file changes
let leaveAction: (() => void) | null = null;
let fmtTimer: ReturnType<typeof setTimeout> | 0 = 0;
const sticky = new Set<string>(); // targets that accepted "Use PC font": it is never asked again this session
let pend: Partial<Style> = {}; // format patches waiting for the debounce (selected text box, no editor)

const pwOf = (f: FileInfo) => app.compress.passwords[f.path] ?? null;

function resetState() {
  e.stage = "idle";
  e.open = false;
  e.path = "";
  e.doc = null;
  e.models = {};
  e.allGen = 0;
  e.pgen = {};
  e.hist = { can_undo: false, can_redo: false, count: 0, changed_pages: [] };
  e.dirty = false;
  e.editing = null;
  e.tool = "text";
  e.sel = null;
  e.fmt = null;
  e.armed = null;
  e.sigs = [];
  e.fonts = [];
  e.sigOpen = false;
  e.drawOpen = false;
  e.woBar = null;
  e.objError = "";
  clearTimeout(fmtTimer);
  fmtTimer = 0;
  sticky.clear();
  pend = {};
  e.leaveAsk = false;
  e.busy = false;
  e.error = "";
  e.run.result = null;
  e.run.error = "";
}

// closing = hold stage "closing" (not "idle") so the tool does not reopen the file before the caller has moved on.
export async function closeSession(closing = false) {
  tok++;
  const wasOpen = e.open;
  resetState();
  if (closing) e.stage = "closing";
  if (wasOpen) {
    try {
      await editClose();
    } catch {
      // ponytail: nothing to release if the backend already dropped the session.
    }
  }
}

// Called by the tool once the file is known and unlocked.
export async function openFor(f: FileInfo) {
  const my = ++tok;
  e.stage = "opening";
  e.error = "";
  try {
    const doc = await editOpen({ path: f.path, password: pwOf(f) });
    if (my !== tok) {
      void editClose().catch(() => {});
      return;
    }
    e.doc = doc;
    e.path = f.path;
    e.open = true;
    e.models = {};
    e.hist = await editHistory();
    e.fonts = await editFonts().catch(() => []);
    await refreshSigs();
    e.stage = "ready";
  } catch (err) {
    if (my !== tok) return;
    e.error = String(err);
    e.stage = "error";
  }
}

// Single-file tool: a new choice replaces the current one; unsaved changes ask first (same dialog as leaving).
export function replaceFile(f: FileInfo) {
  const go = () =>
    leaveTo(() => {
      app.tool.files = [f];
      app.tool.phase = "loaded";
    });
  if (e.open && e.dirty) askLeave(() => void go());
  else void go();
}

function askLeave(action: () => void) {
  leaveAction = action;
  e.leaveAsk = true;
}
export function confirmLeave() {
  const a = leaveAction;
  leaveAction = null;
  e.leaveAsk = false;
  a?.();
}
export function cancelLeave() {
  leaveAction = null;
  e.leaveAsk = false;
}

// Header back / Home / another tool while a session is open: close it first (after asking when dirty).
setLeaveHook((go) => {
  if (app.route !== "edit" || (!e.open && e.stage !== "opening")) return false;
  const run = () => void leaveTo(go);
  if (e.dirty || e.busy) askLeave(run); // busy: an apply is still in flight
  else run();
  return true;
});

async function leaveTo(go: () => void) {
  await closeSession(true);
  go();
  e.stage = "idle";
}

export const chooseAnother = () => void leaveTo(() => openTool("edit"));

async function refreshHistory() {
  e.hist = await editHistory();
}

const FALLBACK: Style = { font: "Arial", size: 11, color: "#000000", bold: false, italic: false, align: "left" };
const sameRuns = (a: Run[], b: Run[]) => a.length === b.length && a.every((r, k) => r.text === b[k].text && sameStyle(r.style, b[k].style));

// Dominant body paragraph of a page (longest text among editable ones): default style + rotation for new boxes.
function domPara(page: number): Para | undefined {
  const ps = (e.models[page]?.paras ?? []).filter((p) => p.status !== "locked");
  return ps.reduce<Para | undefined>((m, p) => (!m || p.text.length > m.text.length ? p : m), undefined);
}
export function domRot(page: number): 0 | 90 | 180 | 270 {
  const w: Record<number, number> = {};
  for (const p of e.models[page]?.paras ?? []) w[p.rot] = (w[p.rot] ?? 0) + p.text.length;
  return ([0, 90, 180, 270] as const).reduce((m, r) => ((w[r] ?? 0) > (w[m] ?? 0) ? r : m), 0 as 0 | 90 | 180 | 270);
}
function defaultStyle(page: number): Style {
  const p = domPara(page);
  if (!p) return { ...FALLBACK };
  const font = e.fonts.includes(p.css_font) ? p.css_font : "Arial";
  return { font, size: p.style.size, color: p.style.color, bold: p.style.bold, italic: p.style.italic, align: p.style.align === "justified" ? "left" : p.style.align };
}

const selObj = (): Obj | undefined => (e.sel ? e.models[e.sel.page]?.objs?.find((o) => o.id === e.sel!.id) : undefined);
const boxRuns = (o: Obj): Run[] => (o.runs?.length ? o.runs : o.text ? [{ text: o.text, style: o.style! }] : []);
// A direct paragraph keeps its own font ("orig") unless the user picks one: never send the CSS family name.
const paraRuns = (p: Para): Run[] => {
  const rs = p.runs?.length ? p.runs : [{ text: p.text, style: p.style }];
  return mergeRuns(rs.map((r) => ({ text: r.text, style: p.status === "direct" ? { ...r.style, font: "orig" } : r.style })));
};
const stickyKey = (page: number, id: number | null) => (id === null ? "" : page + ":" + id);

// Format panel target follows the open editor, else the selected text box. A pending debounced change is never overwritten.
function syncFmt() {
  if (e.editing || fmtTimer) return;
  const o = selObj();
  if (o && o.kind === "textbox" && o.style) {
    const rs = boxRuns(o);
    const sv = styleOver(rs, 0, textOf(rs).length, o.style);
    e.fmt = { kind: "box", page: e.sel!.page, id: o.id, style: { ...sv.style }, mixed: sv.mixed, origName: domPara(e.sel!.page)?.orig_font_name ?? "", origOk: o.style.font === "orig" };
  } else e.fmt = null;
}

// Panel values follow the editor selection (style at its start; mixed fields blank).
function showFmt() {
  const f = e.fmt;
  const ed = e.editing;
  if (!f || !ed) return;
  const sv = styleOver(ed.runs, ed.selS, ed.selE, ed.base);
  f.style = { ...sv.style };
  f.mixed = sv.mixed;
}
export function selChanged(s: number, en: number) {
  const ed = e.editing;
  if (!ed || (ed.selS === s && ed.selE === en)) return;
  ed.selS = s;
  ed.selE = en;
  showFmt();
}

export function openEditor(page: number, id: number, usePc = false) {
  const p = e.models[page]?.paras.find((x) => x.id === id);
  if (!p || e.busy || e.editing?.bar) return; // a visible warning bar keeps its editor
  e.sel = null;
  const runs = paraRuns(p);
  const base = runs[0]?.style ?? { ...p.style, font: p.status === "direct" ? "orig" : p.style.font };
  const n = textOf(runs).length;
  e.editing = { kind: "para", page, id, text: p.text, runs, base, rev: 0, selS: n, selE: n, usePc: usePc || sticky.has(stickyKey(page, id)), applying: false, bar: null };
  e.fmt = { kind: "para", page, id, style: { ...base }, mixed: [], origName: p.orig_font_name, origOk: p.status === "direct" };
  showFmt();
}

export function openBoxEditor(page: number, id: number | null, rect?: Rect, rot: 0 | 90 | 180 | 270 = 0) {
  if (e.busy || e.editing?.bar) return;
  const o = id === null ? undefined : e.models[page]?.objs?.find((x) => x.id === id);
  if (id !== null && o?.kind !== "textbox") return;
  e.sel = id === null ? null : { page, id };
  const runs = o ? boxRuns(o) : [];
  const base = { ...(o?.style ?? defaultStyle(page)) };
  const n = textOf(runs).length;
  e.editing = { kind: "box", page, id, rect, rot, text: textOf(runs), runs, base, rev: 0, selS: n, selE: n, usePc: false, applying: false, bar: null };
  e.fmt = { kind: "box", page, id, style: { ...base }, mixed: [], origName: domPara(page)?.orig_font_name ?? "", origOk: base.font === "orig" };
  showFmt();
}

export function closeEditor() {
  clearTimeout(fmtTimer);
  fmtTimer = 0;
  e.editing = null;
  e.fmt = null;
  syncFmt();
}

// keep = a format change: apply but leave the editor open.
export async function applyEditor(allowOverlap = false, keep = false) {
  const ed = e.editing;
  if (!ed || ed.applying) return;
  clearTimeout(fmtTimer);
  fmtTimer = 0;
  if (ed.kind === "box") return applyBox(ed, keep);
  const para = e.models[ed.page]?.paras.find((x) => x.id === ed.id);
  const runs = normRuns(ed.runs, true);
  if (!para || sameRuns(runs, paraRuns(para))) {
    if (!keep) closeEditor(); // nothing changed
    return;
  }
  ed.applying = true;
  ed.bar = null;
  e.busy = true;
  try {
    const res = await editApply({ page: ed.page, para: ed.id!, text: textOf(runs), use_pc_font: ed.usePc, allow_overlap: allowOverlap, style: null, runs });
    if (res.kind === "ok") {
      e.models[ed.page] = res.page;
      e.pgen[ed.page] = (e.pgen[ed.page] ?? 0) + 1;
      e.dirty = true;
      if (!keep) closeEditor();
      await refreshHistory();
    } else if (res.kind === "missing_chars" || res.kind === "unsupported_chars") ed.bar = { kind: res.kind === "missing_chars" ? "missing" : "unsupported", chars: res.chars };
    else if (res.kind === "overflow") ed.bar = { kind: "overflow", n: res.lines_over };
    else ed.bar = { kind: "cannot_push" };
  } catch (err) {
    ed.bar = { kind: "error", text: String(err) };
  } finally {
    ed.applying = false;
    e.busy = false;
  }
}

// Text box editor: an empty new box places nothing; an emptied existing box is deleted. Newlines are kept as typed.
async function applyBox(ed: NonNullable<typeof e.editing>, keep: boolean) {
  const o = ed.id === null ? undefined : e.models[ed.page]?.objs?.find((x) => x.id === ed.id);
  const runs = normRuns(ed.runs, false);
  const text = textOf(runs);
  if (ed.id === null ? !text : !o) {
    if (!keep) closeEditor();
    return;
  }
  if (o && sameRuns(runs, mergeRuns(boxRuns(o)))) {
    if (!keep) closeEditor();
    return;
  }
  const req: ObjReq = text
    ? { kind: "textbox", page: ed.page, id: ed.id, rect: o?.rect ?? ed.rect!, text, style: { ...runs[0].style }, runs, rot: o?.rot ?? ed.rot ?? 0 }
    : { kind: "delete", page: ed.page, id: ed.id };
  ed.applying = true;
  ed.bar = null;
  e.busy = true;
  try {
    const res = await runObj(req);
    if (res.kind === "ok") {
      if (!keep || !text) closeEditor();
      else {
        ed.id = res.id;
        ed.rect = undefined;
        if (e.fmt) e.fmt.id = res.id;
      }
    } else if (res.kind === "missing_chars" || res.kind === "unsupported_chars") ed.bar = { kind: res.kind === "missing_chars" ? "missing" : "unsupported", chars: res.chars };
  } catch (err) {
    ed.bar = { kind: "error", text: String(err) };
  } finally {
    ed.applying = false;
    e.busy = false;
  }
}

// "Use PC font" on the missing_chars bar: retry the same text with the PC font. Sticky: this target never asks again.
export function usePcFont() {
  const ed = e.editing;
  if (!ed) return;
  ed.usePc = true;
  const k = stickyKey(ed.page, ed.id);
  if (k) sticky.add(k);
  if (ed.kind === "box") {
    const pc = e.fonts.includes("Arial") ? "Arial" : (e.fonts[0] ?? "Arial");
    ed.runs = ed.runs.map((r) => (r.style.font === "orig" ? { text: r.text, style: { ...r.style, font: pc } } : r));
    if (ed.base.font === "orig") ed.base = { ...ed.base, font: pc };
    ed.rev++;
  }
  void applyEditor();
}

// ---- format panel: acts on the editor selection (caret only = whole target); each change re-applies 250 ms later ----
export function setStyle(patch: Partial<Style>) {
  const f = e.fmt;
  if (!f) return;
  const ed = e.editing;
  if (ed) {
    const whole = patch.align !== undefined; // alignment is per paragraph
    ed.runs = applyRange(ed.runs, whole ? 0 : ed.selS, whole ? 1e9 : ed.selE, patch);
    if (ed.selS === ed.selE || whole) ed.base = { ...ed.base, ...patch };
    ed.text = textOf(ed.runs);
    ed.rev++;
    showFmt();
  } else {
    pend = { ...pend, ...patch };
    f.style = { ...f.style, ...patch };
  }
  clearTimeout(fmtTimer);
  fmtTimer = setTimeout(runFmt, 250);
}
async function runFmt() {
  fmtTimer = 0;
  const f = e.fmt;
  if (!f) return;
  if (e.busy || e.editing?.applying) {
    fmtTimer = setTimeout(runFmt, 100);
    return;
  }
  if (e.editing) return void (await applyEditor(false, true));
  const o = selObj();
  if (o?.kind !== "textbox") return;
  const patch = pend;
  pend = {};
  const runs = mergeRuns(boxRuns(o).map((r) => ({ text: r.text, style: { ...r.style, ...patch } })));
  const r = await applyObj({ ...objReq(f.page, o), style: { ...(runs[0]?.style ?? f.style) }, runs });
  if (r?.kind === "missing_chars") e.objError = t("editMissing", { c: r.chars });
  else if (r?.kind === "unsupported_chars") e.objError = t("editUnsupported", { c: r.chars });
}

// ---- placed objects ----
const objReq = (page: number, o: Obj, rect: Rect = o.rect): ObjReq => ({ kind: o.kind, page, id: o.id, rect, text: o.text, style: o.style, runs: o.runs ?? null, rot: o.rot, asset: o.asset });

// Raw call + model update; the caller owns e.busy.
async function runObj(req: ObjReq): Promise<ObjRes> {
  const res = await editApplyObj(req);
  if (res.kind === "ok") {
    e.models[req.page] = res.page;
    e.pgen[req.page] = (e.pgen[req.page] ?? 0) + 1;
    e.dirty = true;
    e.woBar = null;
    e.sel = req.kind === "delete" ? null : { page: req.page, id: res.id };
    await refreshHistory();
  } else if (res.kind === "whiteout_partial") {
    e.woBar = { page: req.page, req, rect: req.rect ?? [0, 0, 0, 0] };
  }
  return res;
}

export async function applyObj(req: ObjReq): Promise<ObjRes | null> {
  if (e.busy) return null;
  e.busy = true;
  e.objError = "";
  try {
    const res = await runObj(req);
    syncFmt();
    return res;
  } catch (err) {
    e.objError = String(err);
    return null;
  } finally {
    e.busy = false;
  }
}

export function selectObj(page: number, id: number) {
  if (e.editing) return;
  e.sel = { page, id };
  syncFmt();
}
export function clearSel() {
  if (e.editing) return;
  e.sel = null;
  syncFmt();
}
export const moveObj = (page: number, o: Obj, rect: Rect) => applyObj(objReq(page, o, rect));
export function deleteSel() {
  const s = e.sel;
  if (s && !e.editing) void applyObj({ kind: "delete", page: s.page, id: s.id });
}
export const addWhiteout = (page: number, rect: Rect) => applyObj({ kind: "whiteout", page, id: null, rect });
export function coverOnly() {
  const b = e.woBar;
  if (b) void applyObj({ ...b.req, cover_only: true });
}
export const dismissWo = () => (e.woBar = null);

let nd: [number, number] = [0, 0];
let ndT: ReturnType<typeof setTimeout> | 0 = 0;
export function nudge(dx: number, dy: number) {
  if (!selObj() || e.editing) return;
  nd = [nd[0] + dx, nd[1] + dy];
  clearTimeout(ndT);
  ndT = setTimeout(flushNudge, 180);
}
function flushNudge() {
  if (e.busy) {
    ndT = setTimeout(flushNudge, 100);
    return;
  }
  const o = selObj();
  const [dx, dy] = nd;
  nd = [0, 0];
  if (o && e.sel) void moveObj(e.sel.page, o, [o.rect[0] + dx, o.rect[1] + dy, o.rect[2], o.rect[3]]);
}

// ---- tools ----
export function setTool(tl: EditTool) {
  if (e.editing?.bar) return;
  if (tl === "sig") e.sigOpen = e.tool !== "sig" || !e.sigOpen;
  else {
    e.sigOpen = false;
    e.armed = null;
  }
  e.tool = tl;
  e.woBar = null;
  clearSel();
}
export function escTool() {
  e.woBar = null;
  e.tool = "text";
  e.sigOpen = false;
  e.armed = null;
}
export function pickSig(it: SigItem) {
  e.armed = it;
  e.tool = "sig";
  e.sigOpen = false;
}
// Click on a page with an armed item: 150 pt (signature) / 110 pt (stamp) wide, centred at the click.
export async function placeArmed(page: number, cx: number, cy: number) {
  const it = e.armed;
  const sz = e.doc?.sizes[page];
  if (!it || !sz) return;
  const w = it.kind === "stamp" ? 110 : 150;
  const h = (w * it.h) / it.w;
  const rect: Rect = [Math.max(0, Math.min(cx - w / 2, sz[0] - w)), Math.max(0, Math.min(cy - h / 2, sz[1] - h)), w, h];
  const r = await applyObj({ kind: "image", page, id: null, rect, rot: 0, asset: it.id });
  if (r?.kind === "ok") escTool();
}

// ---- signature library ----
export async function refreshSigs() {
  e.sigs = await sigList().catch(() => []);
}

async function step(fn: () => Promise<EditHistory>) {
  if (e.busy || e.editing) return;
  e.busy = true;
  try {
    e.hist = await fn();
    e.dirty = true;
    e.allGen++;
    e.sel = null;
    e.woBar = null;
    syncFmt();
  } catch {
    // ponytail: nothing to undo/redo (stale buttons); the next history call corrects them.
    await refreshHistory().catch(() => {});
  } finally {
    e.busy = false;
  }
}
export const undo = () => step(editUndo);
export const redo = () => step(editRedo);

export async function startSave() {
  const f = app.tool.files[0];
  if (!f || e.editing?.bar) return; // an open warning keeps the typed text; resolve it first
  const r = e.run;
  r.name = f.name;
  r.result = null;
  r.error = "";
  e.editing = null;
  app.tool.phase = "running";
  try {
    const res = await editSave({ out_mode: settings.outMode });
    if (settled(undefined)) return;
    r.result = res;
    e.dirty = false;
    app.tool.phase = "done";
  } catch (err) {
    if (settled(err)) return;
    r.error = String(err);
    app.tool.phase = "error";
  }
}

export const keepEditing = () => (app.tool.phase = "loaded");

// "Add text" click: a 200 pt wide box (up to the right margin) at the click, in the page's dominant text direction.
export function startBox(page: number, x: number, y: number) {
  const sz = e.doc?.sizes[page];
  if (!sz) return;
  const rot = domRot(page);
  const side = rot === 90 || rot === 270;
  const th = defaultStyle(page).size * 1.15;
  const len = Math.max(60, Math.min(200, (side ? sz[1] : sz[0]) - 36 - (side ? y : x)));
  const rect: Rect = side ? [Math.min(x, sz[0] - th - 4), Math.min(y, sz[1] - len - 4), th, len] : [Math.min(x, sz[0] - len - 4), Math.min(y, sz[1] - th - 4), len, th];
  openBoxEditor(page, null, rect, rot);
}
