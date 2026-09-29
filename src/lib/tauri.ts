// Thin wrapper so the UI still renders in a plain browser (`npm run dev`, no Tauri runtime).
// In the browser, file_info / dialogs / compress are simulated so the whole flow can be clicked through.
import type { CompressProgress, CompressRequest, CompressResult, FileInfo, MergeProgress, MergeRequest, MergeResult, OrganizeProgress, OrganizeRequest, OrganizeResult, OutMode, ExcelProgress, ExcelRequest, ExcelResult, SplitProgress, SplitRequest, SplitResult, WordProgress, WordRequest, WordResult } from "./state.svelte";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

// ---- browser-dev mocks ----------------------------------------------------
const MB = 1024 * 1024;
const mk = (name: string, size_bytes: number, o: Partial<FileInfo> = {}): FileInfo => ({
  path: `C:/mock/${name}`, name, size_bytes, is_pdf: true, pages: 10, encrypted: false, signed: false, damaged: false, scanned: false, ...o,
});
const MOCK: FileInfo[] = [
  mk("Laporan_Realisasi_Anggaran_2026.pdf", 8.4 * MB, { pages: 38 }),
  mk("Scan_Kuitansi_Perjalanan_Dinas.pdf", 14 * MB, { pages: 12, signed: true, scanned: true }),
  mk("Daftar_Hadir_Rapat.pdf", 184 * 1024, { pages: 2 }),
  mk("Kontrak_Pengadaan_2026.pdf", 3.1 * MB, { pages: null, encrypted: true }),
];
const MOCK_FOLDER: FileInfo[] = [
  mk("Lampiran_Rusak.pdf", 90 * 1024, { pages: null, damaged: true }),
  mk("Arsip_Buku_Besar.pdf", 120 * MB, { pages: 1240 }),
];
const mockDb = () => [...MOCK, ...MOCK_FOLDER];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const mockListeners = new Set<(p: CompressProgress) => void>();
let mockCancel = false;
let mockPick = 0;
// ---------------------------------------------------------------------------

export async function fileInfo(paths: string[]): Promise<FileInfo[]> {
  if (!inTauri) {
    const out = mockDb().filter((f) => paths.includes(f.path));
    if (paths.includes("C:/mock/folder")) out.push(...MOCK_FOLDER);
    return out;
  }
  return invoke<FileInfo[]>("file_info", { paths });
}

export async function officeStatus(): Promise<{ word: boolean; excel: boolean }> {
  if (!inTauri) return { word: true, excel: true }; // browser-dev: assume installed
  return invoke<{ word: boolean; excel: boolean }>("office_status");
}

/** Flash the taskbar button when a job finishes while the user is in another window. */
export async function flashIfUnfocused() {
  if (!inTauri) return;
  const { getCurrentWindow, UserAttentionType } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (!(await w.isFocused())) await w.requestUserAttention(UserAttentionType.Informational);
}

export async function chooseFiles(multiple: boolean): Promise<string[]> {
  // browser-dev: single-file picks cycle through the mock files (plain, signed, plain, locked) so every case can be reached
  if (!inTauri) return multiple ? MOCK.map((f) => f.path) : [MOCK[mockPick++ % MOCK.length].path];
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({
    multiple,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!result) return [];
  return Array.isArray(result) ? result : [result];
}

export async function chooseFolder(): Promise<string[]> {
  if (!inTauri) return ["C:/mock/folder"];
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({ directory: true });
  return result ? [result as string] : [];
}

export async function unlock(path: string, password: string): Promise<{ ok: boolean; pages: number | null; signed: boolean; scanned: boolean }> {
  if (!inTauri) {
    await sleep(400);
    return password === "secret" ? { ok: true, pages: 7, signed: false, scanned: false } : { ok: false, pages: null, signed: false, scanned: false };
  }
  return invoke("unlock", { path, password });
}

export async function onCompressProgress(cb: (p: CompressProgress) => void): Promise<() => void> {
  if (!inTauri) {
    mockListeners.add(cb);
    return () => mockListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<CompressProgress>("compress-progress", (e) => cb(e.payload));
}

export async function compress(req: CompressRequest): Promise<CompressResult[]> {
  if (!inTauri) return mockCompress(req);
  return invoke<CompressResult[]>("compress", { req });
}

export async function compressCancel(): Promise<void> {
  if (!inTauri) {
    mockCancel = true;
    return;
  }
  return invoke("compress_cancel");
}

export async function keepResult(temp: string, original: string, outMode: OutMode): Promise<string> {
  if (!inTauri) return original.replace(/\.pdf$/i, "_compressed.pdf");
  return invoke<string>("keep_result", { temp, original, outMode });
}

export async function discardResult(temp: string): Promise<void> {
  if (!inTauri) return;
  return invoke("discard_result", { temp });
}

export async function openPath(path: string): Promise<void> {
  if (!inTauri) return;
  return invoke("open_path", { path });
}

export async function revealPath(path: string): Promise<void> {
  if (!inTauri) return;
  return invoke("reveal_path", { path });
}

async function mockCompress(req: CompressRequest): Promise<CompressResult[]> {
  mockCancel = false;
  const emit = (p: CompressProgress) => mockListeners.forEach((l) => l(p));
  const db = mockDb();
  const results: CompressResult[] = [];
  const perFile = 3000 / req.files.length;
  for (let index = 0; index < req.files.length; index++) {
    const f = db.find((x) => x.path === req.files[index].path) ?? db[0];
    const pages = f.pages ?? 7;
    const before = f.size_bytes;
    const steps = 8;
    const attempts = req.mode === "target" ? 3 : 1;
    const dpis = [100, 72, 72];
    const noRed = f.name.startsWith("Daftar");
    const miss = req.mode === "target" && f.name.startsWith("Scan");
    for (let a = 1; a <= attempts; a++) {
      for (let s = 1; s <= steps; s++) {
        await sleep(perFile / (steps * attempts));
        if (mockCancel) throw "cancelled";
        emit({
          index, state: "working", page: Math.ceil((pages * s) / steps), pages,
          attempt: req.mode === "target" ? a : null,
          dpi: req.mode === "target" ? dpis[a - 1] : null,
          current_size: req.mode === "target" && a > 1 ? Math.round(before / (2 + a)) : null,
        });
      }
    }
    emit({ index, state: "done", page: pages, pages, attempt: null, dpi: null, current_size: noRed ? before : Math.round(before * 0.3) });
    const path = f.path;
    if (req.mode === "target" && noRed) results.push({ path, before, after: before, output: null, status: "under_target", temp: null, error: null });
    else if (noRed) results.push({ path, before, after: before, output: null, status: "no_reduction", temp: null, error: null });
    else if (miss) results.push({ path, before, after: Math.round(before * 0.09), output: null, status: "target_missed", temp: path + ".tmp", error: null });
    else results.push({ path, before, after: Math.round(before * 0.3), output: path.replace(/\.pdf$/i, "_compressed.pdf"), status: "done", temp: null, error: null });
  }
  return results;
}

// ---- thumbnails + merge ---------------------------------------------------
export async function thumbnail(path: string, password: string | null, page: number, width: number): Promise<string> {
  if (!inTauri) {
    await sleep(150);
    const h = Math.round(width * 1.3);
    const lines = Array.from({ length: 7 }, (_, i) => `<rect x="${width * 0.15}" y="${h * (0.3 + i * 0.09)}" width="${width * (i % 3 === 2 ? 0.4 : 0.7)}" height="${h * 0.03}" fill="#c8ccd6"/>`).join("");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${h}"><rect width="100%" height="100%" fill="#fff"/><rect x="${width * 0.15}" y="${h * 0.12}" width="${width * 0.5}" height="${h * 0.08}" fill="#7c55c4"/>${lines}</svg>`;
    return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
  }
  return invoke<string>("thumbnail", { path, password, page, width });
}

const mergeListeners = new Set<(p: MergeProgress) => void>();
let mockMergeCancel = false;

export async function onMergeProgress(cb: (p: MergeProgress) => void): Promise<() => void> {
  if (!inTauri) {
    mergeListeners.add(cb);
    return () => mergeListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<MergeProgress>("merge-progress", (e) => cb(e.payload));
}

export async function merge(req: MergeRequest): Promise<MergeResult> {
  if (!inTauri) return mockMerge(req);
  return invoke<MergeResult>("merge", { req });
}

export async function mergeCancel(): Promise<void> {
  if (!inTauri) {
    mockMergeCancel = true;
    return;
  }
  return invoke("merge_cancel");
}

async function mockMerge(req: MergeRequest): Promise<MergeResult> {
  mockMergeCancel = false;
  const emit = (p: MergeProgress) => mergeListeners.forEach((l) => l(p));
  const db = mockDb();
  const infos = req.files.map((f) => db.find((x) => x.path === f.path));
  const pages = infos.reduce((s, f) => s + (f?.pages ?? 7), 0);
  const size = infos.reduce((s, f) => s + (f?.size_bytes ?? 0), 0);
  const stages: MergeProgress["stage"][] = req.compress ? ["merging", "compressing"] : ["merging"];
  for (const stage of stages) {
    const steps = 10;
    for (let s = 1; s <= steps; s++) {
      await sleep(2000 / (steps * stages.length));
      if (mockMergeCancel) throw "cancelled";
      emit({ stage, page: Math.ceil((pages * s) / steps), pages });
    }
  }
  const first = req.files[0].path;
  const dir = first.replace(/[\/][^\/]*$/, "");
  return { output: `${dir}/${req.output_name}.pdf`, pages, size: Math.round(size * (req.compress ? 0.4 : 1)), files: req.files.length };
}

// ---- split ----------------------------------------------------------------
const splitListeners = new Set<(p: SplitProgress) => void>();
let mockSplitCancel = false;

export async function onSplitProgress(cb: (p: SplitProgress) => void): Promise<() => void> {
  if (!inTauri) {
    splitListeners.add(cb);
    return () => splitListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<SplitProgress>("split-progress", (e) => cb(e.payload));
}

export async function split(req: SplitRequest): Promise<SplitResult> {
  if (!inTauri) return mockSplit(req);
  return invoke<SplitResult>("split", { req });
}

export async function splitCancel(): Promise<void> {
  if (!inTauri) {
    mockSplitCancel = true;
    return;
  }
  return invoke("split_cancel");
}

async function mockSplit(req: SplitRequest): Promise<SplitResult> {
  mockSplitCancel = false;
  const total = req.groups.length;
  for (let index = 1; index <= total; index++) {
    await sleep(1500 / total);
    if (mockSplitCancel) throw "cancelled";
    splitListeners.forEach((l) => l({ index, total }));
  }
  const dir = req.path.replace(/[\\/][^\\/]*$/, "");
  const stem = req.path.split(/[\\/]/).pop()!.replace(/\.pdf$/i, "");
  return {
    folder: dir,
    outputs: req.groups.map((g, i) => ({ path: `${dir}/${stem}_part${i + 1}.pdf`, from: g.from, to: g.to, size: (g.to - g.from + 1) * 90 * 1024 })),
  };
}

// ---- organize -------------------------------------------------------------
const organizeListeners = new Set<(p: OrganizeProgress) => void>();
let mockOrganizeCancel = false;

export async function onOrganizeProgress(cb: (p: OrganizeProgress) => void): Promise<() => void> {
  if (!inTauri) {
    organizeListeners.add(cb);
    return () => organizeListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<OrganizeProgress>("organize-progress", (e) => cb(e.payload));
}

export async function organize(req: OrganizeRequest): Promise<OrganizeResult> {
  if (!inTauri) return mockOrganize(req);
  return invoke<OrganizeResult>("organize", { req });
}

export async function organizeCancel(): Promise<void> {
  if (!inTauri) {
    mockOrganizeCancel = true;
    return;
  }
  return invoke("organize_cancel");
}

async function mockOrganize(req: OrganizeRequest): Promise<OrganizeResult> {
  mockOrganizeCancel = false;
  const pages = req.pages.length;
  const steps = 10;
  for (let s = 1; s <= steps; s++) {
    await sleep(1500 / steps);
    if (mockOrganizeCancel) throw "cancelled";
    organizeListeners.forEach((l) => l({ page: Math.ceil((pages * s) / steps), pages }));
  }
  const first = req.sources[0].path;
  const dir = first.replace(/[\\/][^\\/]*$/, "");
  const stem = first.split(/[\\/]/).pop()!.replace(/\.pdf$/i, "");
  return { output: `${dir}/${stem}_organized.pdf`, pages, size: pages * 90 * 1024 };
}

// ---- word -----------------------------------------------------------------
const wordListeners = new Set<(p: WordProgress) => void>();
let mockWordCancel = false;

export async function onWordProgress(cb: (p: WordProgress) => void): Promise<() => void> {
  if (!inTauri) {
    wordListeners.add(cb);
    return () => wordListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<WordProgress>("word-progress", (e) => cb(e.payload));
}

export async function wordConvert(req: WordRequest): Promise<WordResult> {
  if (!inTauri) return mockWord(req);
  return invoke<WordResult>("word_convert", { req });
}

export async function wordCancel(): Promise<void> {
  if (!inTauri) {
    mockWordCancel = true;
    return;
  }
  return invoke("word_cancel");
}

async function mockWord(req: WordRequest): Promise<WordResult> {
  mockWordCancel = false;
  const stages: WordProgress["stage"][] = ["preparing", "converting", "saving"];
  const t0 = Date.now();
  for (const [i, stage] of stages.entries()) {
    wordListeners.forEach((l) => l({ stage }));
    await sleep(i === 1 ? 2000 : 1000);
    if (mockWordCancel) throw "cancelled";
  }
  if (req.path.includes("Hadir")) throw "word_missing"; // mock: force the Word-not-installed card
  if (req.path.includes("Arsip")) throw "COM error 0x800706BA"; // mock: generic failure
  const dir = req.path.replace(/[\\/][^\\/]*$/, "");
  const stem = req.path.split(/[\\/]/).pop()!.replace(/\.pdf$/i, "");
  return { output: `${dir}/${stem}.docx`, size: 210 * 1024, seconds: Math.round((Date.now() - t0) / 1000) };
}

// ---- excel ----------------------------------------------------------------
const excelListeners = new Set<(p: ExcelProgress) => void>();
let mockExcelCancel = false;

export async function onExcelProgress(cb: (p: ExcelProgress) => void): Promise<() => void> {
  if (!inTauri) {
    excelListeners.add(cb);
    return () => excelListeners.delete(cb);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<ExcelProgress>("excel-progress", (e) => cb(e.payload));
}

export async function excelConvert(req: ExcelRequest): Promise<ExcelResult> {
  if (!inTauri) return mockExcel(req);
  return invoke<ExcelResult>("excel_convert", { req });
}

export async function excelCancel(): Promise<void> {
  if (!inTauri) {
    mockExcelCancel = true;
    return;
  }
  return invoke("excel_cancel");
}

async function mockExcel(req: ExcelRequest): Promise<ExcelResult> {
  mockExcelCancel = false;
  const pages = mockDb().find((f) => f.path === req.path)?.pages ?? 7;
  const t0 = Date.now();
  const emit = (p: ExcelProgress) => excelListeners.forEach((l) => l(p));
  if (req.engine === "own") {
    const steps = 10;
    for (let s = 1; s <= steps; s++) {
      emit({ stage: s < steps ? "reading" : "writing", page: Math.ceil((pages * s) / steps), pages });
      await sleep(100);
      if (mockExcelCancel) throw "cancelled";
    }
  } else {
    emit({ stage: "excel", page: 0, pages: 0 });
    for (let i = 0; i < 8; i++) {
      await sleep(500);
      if (mockExcelCancel) throw "cancelled";
    }
    if (req.path.includes("Hadir")) throw "excel_missing"; // mock: force the friendly missing-Excel error
  }
  if (req.path.includes("Arsip")) throw "pdfium: failed to load page 3"; // mock: generic failure
  const dir = req.path.replace(/[\\/][^\\/]*$/, "");
  const stem = req.path.split(/[\\/]/).pop()!.replace(/\.pdf$/i, "");
  return {
    output: `${dir}/${stem}${req.engine === "excel" ? "_v2" : ""}.xlsx`,
    size: 64 * 1024,
    seconds: Math.round((Date.now() - t0) / 1000),
    pages,
    empty_pages: req.path.includes("Scan") ? 2 : 0,
  };
}

// Real Tauri clipboard goes through the webview API; falls back to nothing if denied.
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
