// Shared logic for turning dropped/chosen paths into app state, used by both
// the native drag-drop listener (App.svelte) and the "Choose files" buttons.
import { app, isRunning } from "./state.svelte";
import { fileInfo } from "./tauri";
import { replaceFile } from "./split";
import { replaceFile as replaceOrganizeFile } from "./organize";
import { replaceFile as replaceWordFile } from "./word";
import { replaceFile as replaceExcelFile } from "./excel";
import { replaceFile as replaceOcrFile } from "./ocr";
import { replaceFile as replaceEditFile } from "./edit";

export async function addFilesToHome(paths: string[]) {
  if (isRunning()) return;
  const infos = await fileInfo(paths);
  const pdfs = infos.filter((f) => f.is_pdf);
  if (!pdfs.length) {
    app.notice = "notPdf";
    return;
  }
  app.notice = "";
  app.home.droppedFile = pdfs[0];
}

export async function addFilesToTool(paths: string[]) {
  if (isRunning()) return; // no new input while a job runs
  const infos = await fileInfo(paths);
  if (isRunning()) return;
  const pdfs = infos.filter((f) => f.is_pdf);
  if (!pdfs.length) {
    app.notice = "notPdf";
    return;
  }
  app.notice = "";
  if (app.route === "split" || app.route === "organize" || app.route === "word" || app.route === "excel" || app.route === "ocr" || app.route === "edit") {
    // A fresh FileInfo for an encrypted file has no page count; a password remembered from an
    // earlier load would make the tool treat it as unlocked and never load it. Ask again.
    delete app.compress.passwords[pdfs[0].path];
  }
  if (app.route === "split") return replaceFile(pdfs[0]); // single-file tools
  if (app.route === "organize") return replaceOrganizeFile(pdfs[0]);
  if (app.route === "word") return replaceWordFile(pdfs[0]);
  if (app.route === "excel") return replaceExcelFile(pdfs[0]);
  if (app.route === "ocr") return replaceOcrFile(pdfs[0]);
  if (app.route === "edit") return replaceEditFile(pdfs[0]);
  const have = new Set(app.tool.files.map((f) => f.path));
  app.tool.files = [...app.tool.files, ...pdfs.filter((f) => !have.has(f.path))];
  app.tool.phase = "loaded";
}
