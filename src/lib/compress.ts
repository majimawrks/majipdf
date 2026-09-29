// Compress tool logic: what is runnable, running, cancelling, keep/discard.
import { app, settings, openTool, type FileInfo } from "./state.svelte";
import * as api from "./tauri";

const c = app.compress;

export const isLocked = (f: FileInfo) => f.encrypted && !c.passwords[f.path];
export const isRunnable = (f: FileInfo) => !f.damaged && !isLocked(f);
export const runnable = () => app.tool.files.filter(isRunnable);

export function targetValue(): number {
  const n = Number(c.targetMb.replace(",", "."));
  return Number.isFinite(n) ? n : 0;
}
export const targetValid = () => c.mode === "preset" || targetValue() > 0;

let runId = 0;

export async function startRun() {
  const files = runnable();
  if (!files.length || !targetValid()) return;
  const id = ++runId;
  const r = c.run;
  r.files = files;
  r.mode = c.mode;
  r.targetMb = targetValue();
  r.progress = {};
  r.results = [];
  r.keep = {};
  r.tries = {};
  r.error = "";
  r.locked = app.tool.files.filter(isLocked).map((f) => f.name);
  app.tool.phase = "running";
  const unlisten = await api.onCompressProgress((p) => {
    if (id !== runId) return;
    r.progress[p.index] = p;
    if (p.attempt) r.tries[p.index] = Math.max(r.tries[p.index] ?? 0, p.attempt);
  });
  try {
    const results = await api.compress({
      files: files.map((f) => ({ path: f.path, password: c.passwords[f.path] ?? null })),
      mode: c.mode,
      preset: c.preset,
      target_mb: r.targetMb,
      grayscale: c.grayscale,
      out_mode: settings.outMode,
    });
    if (id !== runId) return;
    r.results = results;
    app.tool.phase = "done";
  } catch (e) {
    if (id !== runId || e === "cancelled") return;
    r.error = String(e);
    app.tool.phase = "error";
  } finally {
    unlisten();
  }
}

export async function cancelRun() {
  runId++; // ignore anything the running call still reports
  app.tool.phase = "loaded"; // options and files are untouched
  try {
    await api.compressCancel();
  } catch {
    // ponytail: backend already stopped or nothing running; nothing to recover.
  }
}

export async function keep(i: number) {
  const res = c.run.results[i];
  if (!res.temp) return;
  c.run.keep[i] = "busy";
  try {
    const out = await api.keepResult(res.temp, res.path, settings.outMode);
    res.output = out;
    c.run.keep[i] = "kept";
  } catch {
    c.run.keep[i] = "failed";
  }
}

export async function discard(i: number) {
  const res = c.run.results[i];
  if (!res.temp) return;
  c.run.keep[i] = "busy";
  try {
    await api.discardResult(res.temp);
  } catch {
    // ponytail: temp cleanup failure is harmless to the user.
  }
  res.after = res.before;
  c.run.keep[i] = "discarded";
}

export function processAnother() {
  openTool("compress");
}
