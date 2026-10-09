import { clearThumbCache } from "./thumbcache";
import type { EditDoc, EditHistory, EditSaveRes, PageModel, Style, Run, ObjReq, SigItem } from "./edit";

export const isMac = /Mac/.test(navigator.userAgent);

export type Lang = "en" | "id";
export type Theme = "auto" | "light" | "dark";
export type OutMode = "next" | "folder";
export type ToolId = "compress" | "merge" | "split" | "organize" | "word" | "excel" | "ocr" | "edit";

export interface FileInfo {
  path: string;
  name: string;
  size_bytes: number;
  is_pdf: boolean;
  pages: number | null;
  encrypted: boolean;
  signed: boolean;
  damaged: boolean;
  scanned: boolean;
}

// Compress contract types (see _docs/compress-contract.md)
export interface CompressRequest {
  files: { path: string; password: string | null }[];
  mode: "preset" | "target";
  preset: "smallest" | "balanced" | "print";
  target_mb: number;
  grayscale: boolean;
  out_mode: OutMode;
}
export interface CompressProgress {
  index: number;
  state: "working" | "done";
  page: number;
  pages: number;
  attempt: number | null;
  dpi: number | null;
  current_size: number | null;
}
export interface CompressResult {
  path: string;
  before: number;
  after: number | null;
  output: string | null;
  status: "done" | "no_reduction" | "under_target" | "target_missed" | "failed";
  temp: string | null;
  error: string | null;
}

// Merge contract types (see _docs/merge-contract.md)
export interface MergeRequest {
  files: { path: string; password: string | null }[];
  output_name: string;
  compress: boolean;
  out_mode: OutMode;
}
export interface MergeProgress {
  stage: "merging" | "compressing";
  page: number;
  pages: number;
}
export interface MergeResult {
  output: string;
  pages: number;
  size: number;
  files: number;
}

// Split contract types (see _docs/split-contract.md)
export interface SplitRequest {
  path: string;
  password: string | null;
  groups: { from: number; to: number }[];
  out_mode: OutMode;
}
export interface SplitProgress {
  index: number;
  total: number;
}
export interface SplitResult {
  folder: string;
  outputs: { path: string; from: number; to: number; size: number }[];
}

// Word contract types (see _docs/word-contract.md)
export interface WordRequest {
  path: string;
  password: string | null;
  out_mode: OutMode;
}
export interface WordProgress {
  stage: "preparing" | "converting" | "saving";
}
export interface WordResult {
  output: string;
  size: number;
  seconds: number;
}

// Excel contract types (see _docs/excel-contract.md)
export interface ExcelRequest {
  path: string;
  password: string | null;
  engine: "own" | "excel";
  sheet_mode: "per_page" | "one";
  numbers: boolean;
  out_mode: OutMode;
}
export interface ExcelProgress {
  stage: "reading" | "writing" | "excel";
  page: number;
  pages: number;
}
export interface ExcelResult {
  output: string;
  size: number;
  seconds: number;
  pages: number;
  empty_pages: number;
}

// OCR contract types (see _docs/ocr-contract.md)
export interface OcrRequest {
  path: string;
  password: string | null;
  out_mode: OutMode;
}
export interface OcrProgress {
  stage: "preparing" | "reading" | "saving";
  done: number;
  total: number;
}
export interface OcrResult {
  output: string;
  size: number;
  seconds: number;
  ocr_pages: number;
  pages: number;
}

// Edit PDF: the open in-place editor (contract types live in edit.ts)
export type EditTool = "text" | "add" | "wo" | "sig";
// Format panel target: a paragraph whose editor is open, or a text box (selected or being edited)
export interface Fmt {
  kind: "para" | "box";
  page: number;
  id: number | null; // null = a new text box that is still a draft
  style: Style; // shown in the panel: style at the editor selection start
  mixed: string[]; // fields that differ across the selection (shown blank)
  origName: string; // shown in "Original (name)"
  origOk: boolean; // "Original" is offered
}
export interface Editing {
  kind: "para" | "box";
  page: number;
  id: number | null; // null = new text box (draft, placed on first apply)
  rect?: [number, number, number, number]; // draft box rect, display pt
  rot?: 0 | 90 | 180 | 270; // draft box rotation
  text: string; // concatenation of runs
  runs: Run[];
  base: Style; // style for text typed into an empty editor
  rev: number; // bumped when the editor DOM must be rebuilt from runs
  selS: number; // selection in the editor, as text offsets
  selE: number;
  usePc: boolean;
  applying: boolean;
  bar: null | { kind: "missing"; chars: string } | { kind: "unsupported"; chars: string } | { kind: "overflow"; n: number } | { kind: "cannot_push" } | { kind: "error"; text: string };
}

// Organize contract types (see _docs/organize-contract.md)
export interface OrganizeRequest {
  sources: { path: string; password: string | null }[];
  pages: { src: number; page: number; rotation: 0 | 90 | 180 | 270 }[];
  out_mode: OutMode;
}
export interface OrganizeProgress {
  page: number;
  pages: number;
}
export interface OrganizeResult {
  output: string;
  pages: number;
  size: number;
}
// rotation is cumulative (any multiple of 90) so the CSS transition never spins backwards; normalised on send.
export interface OrgPage {
  id: number;
  src: number;
  page: number;
  rotation: number;
  isNew: boolean;
}
export interface OrgSource {
  path: string;
  password: string | null;
  name: string;
  signed: boolean;
}
const orgInit = () => ({
  file: "", // path the page list belongs to; a different file re-initialises it
  total: 0, // page count of the original file
  nextId: 0,
  pages: [] as OrgPage[],
  sources: [] as OrgSource[],
  selection: new Set<number>(), // always replaced, never mutated (plain Set is not reactive)
  anchor: null as number | null, // last clicked page id, start of a Shift range
  pendingFile: null as FileInfo | null, // a new file waiting for "replace? unsaved changes"
  run: {
    name: "",
    progress: null as OrganizeProgress | null,
    result: null as OrganizeResult | null,
    error: "",
  },
});

interface Settings {
  lang: Lang;
  theme: Theme;
  outMode: OutMode;
  outFolder: string;
}

const SETTINGS_KEY = "majipdf.settings";
const DEFAULT_SETTINGS: Settings = { lang: "en", theme: "auto", outMode: "next", outFolder: isMac ? "Documents/majipdf" : "Documents\\majipdf" };

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const parsed = JSON.parse(raw);
    return { ...DEFAULT_SETTINGS, ...parsed };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export const settings = $state<Settings>(loadSettings());

export function saveSettings() {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // ponytail: storage may be unavailable (private mode); settings just won't persist.
  }
}

export const app = $state({
  route: "home" as "home" | ToolId,
  settingsOpen: false,
  aboutOpen: false,
  dragging: false,
  home: {
    droppedFile: null as FileInfo | null,
  },
  tool: {
    phase: "empty" as "empty" | "loaded" | "running" | "done" | "error",
    cancelling: false, // Cancel pressed; stays until the running call settles (the backend may still be cleaning up)
    files: [] as FileInfo[],
  },
  compress: {
    mode: "preset" as "preset" | "target",
    preset: "smallest" as "smallest" | "balanced" | "print",
    targetMb: "1",
    grayscale: false,
    passwords: {} as Record<string, string>, // path -> unlocked password
    skipped: [] as string[], // locked files the user chose to skip
    run: {
      files: [] as FileInfo[], // snapshot of the files sent, same order as the request
      mode: "preset" as "preset" | "target",
      targetMb: 1,
      progress: {} as Record<number, CompressProgress>,
      results: [] as CompressResult[],
      keep: {} as Record<number, "busy" | "kept" | "discarded" | "failed">,
      keepErr: {} as Record<number, string>, // error text of a failed Keep, for "Copy details"
      tries: {} as Record<number, number>, // highest size-limit attempt seen per file
      locked: [] as string[], // names of locked files left out of the run
      error: "",
    },
  },
  merge: {
    name: "", // used once nameEdited; until then the name follows the first file
    nameEdited: false,
    compress: false,
    run: {
      name: "", // output stem sent
      compress: false,
      files: 0,
      progress: null as MergeProgress | null,
      result: null as MergeResult | null,
      error: "",
    },
  },
  split: {
    mode: "ranges" as "every" | "ranges" | "n",
    ranges: "1-3, 5, 8-10",
    n: "2",
    run: {
      name: "",
      total: 0,
      progress: null as SplitProgress | null,
      result: null as SplitResult | null,
      error: "",
    },
  },
  word: {
    run: {
      name: "",
      pages: 0,
      startedAt: 0,
      progress: null as WordProgress | null,
      result: null as WordResult | null,
      error: "",
    },
  },
  ocr: {
    run: {
      name: "",
      progress: null as OcrProgress | null,
      result: null as OcrResult | null,
      noScan: false, // backend said every page already has text: info note on the loaded view
      error: "",
    },
  },
  edit: {
    stage: "idle" as "idle" | "opening" | "ready" | "error" | "closing", // idle = waiting for edit.ts openFor
    open: false, // backend session exists (job guard held) until edit_close
    path: "",
    doc: null as EditDoc | null,
    models: {} as Record<number, PageModel>,
    allGen: 0, // bumped by undo/redo: every page reloads
    pgen: {} as Record<number, number>, // bumped by an apply: that page reloads
    hist: { can_undo: false, can_redo: false, count: 0, changed_pages: [] } as EditHistory,
    dirty: false, // unsaved changes since open / last save
    editing: null as Editing | null,
    tool: "text" as EditTool,
    sel: null as { page: number; id: number } | null, // selected placed object
    fmt: null as Fmt | null,
    armed: null as SigItem | null, // library item waiting to be placed
    sigs: [] as SigItem[],
    fonts: [] as string[],
    sigOpen: false,
    drawOpen: false,
    woBar: null as { page: number; req: ObjReq; rect: [number, number, number, number] } | null, // whiteout_partial warning (request not applied)
    objError: "",
    leaveAsk: false,
    busy: false, // an undo/redo/apply call is in flight
    error: "",
    run: { name: "", result: null as EditSaveRes | null, error: "" },
  },
  excel: {
    sheetMode: "per_page" as "per_page" | "one",
    numbers: false,
    run: {
      name: "",
      pages: 0,
      startedAt: 0,
      engine: "own" as "own" | "excel",
      progress: null as ExcelProgress | null,
      result: null as ExcelResult | null, // own-engine result (first Done card)
      v2: null as ExcelResult | null, // Excel-engine result, shown instead when present
      error: "",
    },
  },
  organize: orgInit(),
  office: { word: true, excel: true },
  notice: "" as string,
});

export const isRunning = () => app.tool.phase === "running";

// Called by every tool when its run call settles. True = the outcome was a cancel or a backend "busy"
// and the phase has been put back (to `back`), so the caller returns without showing a result or error.
export function settled(err: unknown, back: "loaded" | "done" = "loaded"): boolean {
  if (!app.tool.cancelling && err !== "cancelled" && err !== "busy") return false;
  if (err === "busy") app.notice = "busy";
  app.tool.cancelling = false;
  app.tool.phase = back;
  return true;
}

// Cancel: show "Cancelling…" and ask the backend; the run call settling (see settled) ends it.
export async function requestCancel(cancel: () => Promise<void>) {
  if (app.tool.cancelling) return;
  app.tool.cancelling = true;
  try {
    await cancel();
  } catch {
    // ponytail: backend already stopped or nothing running; the run call still settles.
  }
}

// Edit PDF holds a session (and the backend job guard): leaving asks first and closes it. The hook returns true when it took over `go`.
let leaveHook: ((go: () => void) => boolean) | null = null;
export const setLeaveHook = (h: (go: () => void) => boolean) => (leaveHook = h);

export function goHome() {
  if (isRunning() || leaveHook?.(goHomeNow)) return;
  goHomeNow();
}

function goHomeNow() {
  clearThumbCache();
  app.route = "home";
  app.settingsOpen = false;
  app.notice = "";
}

export function openTool(id: ToolId, files: FileInfo[] = []) {
  if (isRunning() || leaveHook?.(() => openToolNow(id, files))) return;
  openToolNow(id, files);
}

function openToolNow(id: ToolId, files: FileInfo[]) {
  clearThumbCache();
  app.route = id;
  app.tool = { phase: files.length ? "loaded" : "empty", cancelling: false, files };
  app.compress.passwords = {};
  app.compress.skipped = [];
  app.merge.name = "";
  app.merge.nameEdited = false;
  app.merge.compress = false;
  const { run: _keep, ...fresh } = orgInit(); // mutate in place: tools hold `app.organize` / `.run` by reference
  Object.assign(app.organize, fresh);
  app.settingsOpen = false;
  app.notice = "";
}
