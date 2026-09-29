// Merge tool logic: what is runnable, running, cancelling, reordering.
import { app, settings, openTool } from "./state.svelte";
import * as api from "./tauri";
import { isLocked, runnable } from "./compress";

const m = app.merge;
export { isLocked, runnable };

const stem = (name: string) => name.replace(/\.pdf$/i, "");
// Follows the first runnable file until the user types their own name.
export const outName = () => {
  if (m.nameEdited) return m.name;
  const first = runnable()[0] ?? app.tool.files[0];
  return first ? stem(first.name) + "_merged" : "merged";
};
export const nameValid = () => {
  const n = outName().trim();
  return n.length > 0 && !/[\\/:*?"<>|]/.test(n) && !/^[. ]+$/.test(n);
};

let runId = 0;

export async function startRun() {
  const files = runnable();
  if (files.length < 2 || !nameValid()) return;
  const id = ++runId;
  const r = m.run;
  r.name = outName().trim();
  r.compress = m.compress;
  r.files = files.length;
  r.progress = null;
  r.result = null;
  r.error = "";
  app.tool.phase = "running";
  const unlisten = await api.onMergeProgress((p) => {
    if (id === runId) r.progress = p;
  });
  try {
    const res = await api.merge({
      files: files.map((f) => ({ path: f.path, password: app.compress.passwords[f.path] ?? null })),
      output_name: r.name,
      compress: r.compress,
      out_mode: settings.outMode,
    });
    if (id !== runId) return;
    r.result = res;
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
  app.tool.phase = "loaded"; // files, order and options are untouched
  try {
    await api.mergeCancel();
  } catch {
    // ponytail: backend already stopped or nothing running; nothing to recover.
  }
}

export function move(from: number, to: number) {
  const f = app.tool.files;
  if (from === to || to < 0 || to >= f.length) return;
  const [x] = f.splice(from, 1);
  f.splice(to, 0, x);
}

export const processAnother = () => openTool("merge");
