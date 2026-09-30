// Merge tool logic: what is runnable, running, cancelling, reordering.
import { app, settings, openTool, settled, requestCancel } from "./state.svelte";
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
// "" when fine, else the i18n key of the problem.
export const nameError = (): "" | "nameErr" | "nameReserved" | "nameLong" => {
  const n = outName().trim();
  if (!n || /[\\/:*?"<>|]/.test(n) || /^[. ]+$/.test(n)) return "nameErr";
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(n.split(".")[0].trim())) return "nameReserved"; // with or without an extension
  if (stem(n).length > 150) return "nameLong";
  return "";
};
export const nameValid = () => nameError() === "";

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

export const cancelRun = () => requestCancel(api.mergeCancel);

export function move(from: number, to: number) {
  const f = app.tool.files;
  if (from === to || to < 0 || to >= f.length) return;
  const [x] = f.splice(from, 1);
  f.splice(to, 0, x);
}

export const processAnother = () => openTool("merge");
