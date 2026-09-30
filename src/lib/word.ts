// PDF -> Word: state-backed run logic. Word gives no page progress, so the UI shows a time estimate.
import { app, settings, openTool, settled, requestCancel, type FileInfo } from "./state.svelte";
import * as api from "./tauri";

const w = app.word;

// Spike timings: 13-16 s for 1-6 pages, 52 s for 37 pages.
export const estimateSec = (pages: number) => 12 + 1.2 * pages;

export function replaceFile(f: FileInfo) {
  app.tool.files = [f];
  app.tool.phase = "loaded";
}

let runId = 0;

export async function startRun() {
  const f = app.tool.files[0];
  if (!f) return;
  const id = ++runId;
  const r = w.run;
  r.name = f.name;
  r.pages = f.pages ?? 0;
  r.startedAt = Date.now();
  r.progress = null;
  r.result = null;
  r.error = "";
  app.tool.phase = "running";
  const unlisten = await api.onWordProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.wordConvert({ path: f.path, password: app.compress.passwords[f.path] ?? null, out_mode: settings.outMode });
    if (settled(undefined)) return;
    if (id !== runId) return;
    r.result = res;
    app.tool.phase = "done";
  } catch (e) {
    if (settled(e) || id !== runId) return;
    if (e === "word_missing") {
      app.tool.phase = "loaded";
      app.office = await api.officeStatus();
      app.office.word = false; // App shows OfficeMissing; "Check again" re-reads office_status
      return;
    }
    r.error = String(e);
    app.tool.phase = "error";
  } finally {
    unlisten();
  }
}

export const cancelRun = () => requestCancel(api.wordCancel);

export const processAnother = () => openTool("word");
export const retry = () => (app.tool.phase = "loaded");
