// Scan to text (OCR): state-backed run logic. The backend reports done/total scan pages.
import { app, settings, openTool, settled, requestCancel, type FileInfo } from "./state.svelte";
import * as api from "./tauri";

const o = app.ocr;

export function replaceFile(f: FileInfo) {
  o.run.noScan = false;
  app.tool.files = [f];
  app.tool.phase = "loaded";
}

let runId = 0;

export async function startRun() {
  const f = app.tool.files[0];
  if (!f) return;
  const id = ++runId;
  const r = o.run;
  r.name = f.name;
  r.progress = null;
  r.result = null;
  r.noScan = false;
  r.error = "";
  app.tool.phase = "running";
  const unlisten = await api.onOcrProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.ocrRun({ path: f.path, password: app.compress.passwords[f.path] ?? null, out_mode: settings.outMode });
    if (settled(undefined)) return;
    if (id !== runId) return;
    r.result = res;
    app.tool.phase = "done";
  } catch (e) {
    if (settled(e) || id !== runId) return;
    if (e === "no_scan_pages") {
      r.noScan = true;
      app.tool.phase = "loaded";
      return;
    }
    r.error = String(e);
    app.tool.phase = "error";
  } finally {
    unlisten();
  }
}

export const cancelRun = () => requestCancel(api.ocrCancel);

export const processAnother = () => {
  o.run.noScan = false;
  openTool("ocr");
};
export const retry = () => (app.tool.phase = "loaded");
