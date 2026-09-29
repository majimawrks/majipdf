// PDF -> Excel: state-backed run logic. Own engine (fast, page progress) first; the Excel engine
// is only a "Not right? Try the other method" fallback from the first Done card.
import { app, settings, openTool, type FileInfo } from "./state.svelte";
import * as api from "./tauri";

const x = app.excel;

// Excel engine gives no page progress: estimate like Word does.
export const estimateSec = (pages: number) => 20 + 2 * pages;

export function replaceFile(f: FileInfo) {
  app.tool.files = [f];
  app.tool.phase = "loaded";
}

let runId = 0;

async function run(engine: "own" | "excel") {
  const f = app.tool.files[0];
  if (!f) return;
  const id = ++runId;
  const r = x.run;
  r.name = f.name;
  r.pages = f.pages ?? 0;
  r.startedAt = Date.now();
  r.engine = engine;
  r.progress = null;
  r.error = "";
  r.v2 = null;
  if (engine === "own") r.result = null;
  app.tool.phase = "running";
  const unlisten = await api.onExcelProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.excelConvert({
      path: f.path,
      password: app.compress.passwords[f.path] ?? null,
      engine,
      sheet_mode: x.sheetMode,
      numbers: x.numbers,
      out_mode: settings.outMode,
    });
    if (id !== runId) return;
    if (engine === "own") r.result = res;
    else r.v2 = res;
    app.tool.phase = "done";
  } catch (e) {
    if (id !== runId || e === "cancelled") return;
    r.error = String(e);
    app.tool.phase = "error";
  } finally {
    unlisten();
  }
}

export const startRun = () => run("own");
export const tryOtherMethod = () => run("excel");

export async function cancelRun() {
  runId++;
  app.tool.phase = x.run.engine === "excel" && x.run.result ? "done" : "loaded"; // fallback cancelled -> first Done card
  try {
    await api.excelCancel();
  } catch {
    // ponytail: backend already stopped or nothing running; nothing to recover.
  }
}

export const processAnother = () => openTool("excel");
export const retry = () => (app.tool.phase = x.run.engine === "excel" && x.run.result ? "done" : "loaded");
