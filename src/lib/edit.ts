// Edit PDF (inline text editing): contract types, command wrappers and state-backed session logic.
// See _docs/edit-contract.md. The backend keeps the session (and the job guard) from edit_open until edit_close.
import { app, settings, openTool, settled, setLeaveHook, type FileInfo } from "./state.svelte";
import { inTauri } from "./tauri";

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
}
export interface PageModel { page: number; paras: Para[]; scan: boolean }
export interface EditApplyReq { page: number; para: number; text: string; use_pc_font: boolean; allow_overlap: boolean }
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
export const editUndo = (): Promise<EditHistory> => (mock ? mock().then((m) => m.undo()) : invoke("edit_undo"));
export const editRedo = (): Promise<EditHistory> => (mock ? mock().then((m) => m.redo()) : invoke("edit_redo"));
export const editHistory = (): Promise<EditHistory> => (mock ? mock().then((m) => m.history()) : invoke("edit_history"));
export const editSave = (req: EditSaveReq): Promise<EditSaveRes> => (mock ? mock().then((m) => m.save(req)) : invoke("edit_save", { req }));
export const editClose = (): Promise<void> => (mock ? mock().then((m) => m.close()) : invoke("edit_close"));

// ---- session --------------------------------------------------------------
const e = app.edit;
let tok = 0; // invalidates an in-flight open when the file changes
let leaveAction: (() => void) | null = null;

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

export function openEditor(page: number, id: number, usePc = false) {
  const p = e.models[page]?.paras.find((x) => x.id === id);
  if (!p || e.busy || e.editing?.bar) return; // a visible warning bar keeps its editor
  e.editing = { page, id, text: p.text, usePc, applying: false, bar: null };
}

export function closeEditor() {
  e.editing = null;
}

export async function applyEditor(allowOverlap = false) {
  const ed = e.editing;
  if (!ed || ed.applying) return;
  const para = e.models[ed.page]?.paras.find((x) => x.id === ed.id);
  const text = ed.text.replace(/\s+/g, " ").trim();
  if (!para || text === para.text) {
    e.editing = null; // nothing changed
    return;
  }
  ed.applying = true;
  ed.bar = null;
  e.busy = true;
  try {
    const res = await editApply({ page: ed.page, para: ed.id, text, use_pc_font: ed.usePc, allow_overlap: allowOverlap });
    if (res.kind === "ok") {
      e.models[ed.page] = res.page;
      e.pgen[ed.page] = (e.pgen[ed.page] ?? 0) + 1;
      e.dirty = true;
      e.editing = null;
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

// "Use PC font" on the missing_chars bar: retry the same text with the PC font.
export function usePcFont() {
  if (!e.editing) return;
  e.editing.usePc = true;
  void applyEditor();
}

async function step(fn: () => Promise<EditHistory>) {
  if (e.busy || e.editing) return;
  e.busy = true;
  try {
    e.hist = await fn();
    e.dirty = true;
    e.allGen++;
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
