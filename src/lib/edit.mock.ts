// DEV-ONLY fake backend for the Edit PDF tool in a plain browser (no Tauri). Imported lazily from edit.ts
// behind `import.meta.env.DEV && !inTauri`; delete this file plus the `mock` branches in edit.ts to remove it.
// All text is made up. Magic inputs: "@" -> missing_chars, "Ω" -> unsupported_chars, "OVERFLOW" -> overflow, "PUSH" -> cannot_push.
import type { EditOpenReq, EditApplyReq, EditApplyRes, EditDoc, EditHistory, EditSaveReq, EditSaveRes, PageModel, Para } from "./edit";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const W = 595;
const H = 842;
const LOREM =
  "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.";

const para = (id: number, y: number, text: string, o: Partial<Para> = {}): Para => ({
  id, bbox: [72, y, 451, 14], text, marker: null, align: "left", size: 11, pitch: 14, first_indent: 0, css_font: "Arial",
  bold: false, italic: false, color: "#000000", status: "direct", reason: null, edited: false, pc_font_used: false, rot: 0, ...o,
});
const fit = (p: Para) => {
  if (p.rot === 90 || p.rot === 270) return p; // sideways: the box is fixed in this mock
  const perLine = Math.max(8, Math.floor(p.bbox[2] / (p.size * 0.5)));
  p.bbox[3] = Math.max(1, Math.ceil(p.text.length / perLine)) * p.pitch;
  return p;
};
const initial = (): PageModel[] => [
  {
    page: 0, scan: false,
    paras: [
      para(0, 80, "Laporan Contoh Kegiatan Tahunan", { align: "center", size: 18, pitch: 22, bold: true, bbox: [72, 80, 451, 22] }),
      para(1, 130, LOREM, { align: "justified", css_font: "Times New Roman" }),
      para(2, 215, LOREM.slice(40) + " " + LOREM.slice(0, 120), { align: "justified", css_font: "Times New Roman" }),
      para(3, 300, "Paragraf ini memakai font yang tidak bisa ditulisi, contoh untuk tombol font komputer.", { status: "pc_font", css_font: "Calibri" }),
      para(4, 345, "Teks miring tidak bisa disunting (rotated).", { status: "locked", reason: "rotated", italic: true }),
      para(6, 120, "Teks vertikal contoh yang dibaca dari bawah ke atas", { rot: 90, bbox: [566, 120, 26, 220], css_font: "Arial" }),
      para(7, 6, "Kotak di tepi atas", { bbox: [72, 6, 200, 14] }),
      para(8, 420, "Kotak di tepi kanan", { bbox: [440, 420, 150, 14], align: "right" }),
      para(9, 450, "Kotak di tepi kiri", { bbox: [2, 450, 150, 14] }),
      para(10, 400, "Vertikal tepi kiri dibaca dari atas ke bawah", { rot: 270, bbox: [3, 480, 26, 220] }),
      para(5, 790, "Halaman 1 dari 2 (footer berulang)", { status: "locked", reason: "form", size: 9, pitch: 11, align: "center" }),
    ].map(fit),
  },
  {
    page: 1, scan: false,
    paras: [
      para(0, 80, "Lampiran", { size: 16, pitch: 20, bold: true, bbox: [72, 80, 451, 20] }),
      para(1, 120, LOREM.slice(0, 160), { marker: "1.", bbox: [96, 120, 427, 14] }),
      para(2, 175, LOREM.slice(60, 230), { marker: "2.", bbox: [96, 175, 427, 14] }),
      para(3, 240, "Struktur halaman ini tidak dikenali.", { status: "locked", reason: "structure" }),
      para(5, 818, "Kotak di tepi bawah halaman terakhir", { bbox: [72, 822, 300, 14] }),
      para(6, 600, "Vertikal tepi bawah kanan", { rot: 90, bbox: [566, 560, 26, 270] }),
      para(4, 790, "Halaman 2 dari 2 (footer berulang)", { status: "locked", reason: "form", size: 9, pitch: 11, align: "center" }),
    ].map(fit),
  },
];

let pages: PageModel[] = initial();
type Op = { page: number; id: number; before: string; after: string };
let done: Op[] = [];
let undone: Op[] = [];

const hist = (): EditHistory => ({ can_undo: done.length > 0, can_redo: undone.length > 0, count: done.length, changed_pages: [...new Set(done.map((o) => o.page))] });
const setText = (page: number, id: number, text: string, edited: boolean) => {
  const p = pages[page].paras.find((x) => x.id === id)!;
  p.text = text;
  p.edited = edited;
  fit(p);
};

export async function open(_r: EditOpenReq): Promise<EditDoc> {
  await sleep(500);
  pages = initial();
  done = [];
  undone = [];
  return { pages: 2, sizes: [[W, H], [W, H]] };
}
export async function page(i: number): Promise<PageModel> {
  await sleep(60);
  return structuredClone(pages[i]);
}
export async function render(i: number, widthPx: number): Promise<string> {
  await sleep(250);
  const k = widthPx / W;
  const bars = pages[i].paras
    .map((p) => {
      const lines = Math.round(p.bbox[3] / p.pitch);
      return Array.from({ length: lines }, (_, l) => {
        const w = l === lines - 1 ? p.bbox[2] * 0.55 : p.bbox[2];
        const x = p.align === "center" ? p.bbox[0] + (p.bbox[2] - w) / 2 : p.bbox[0];
        return `<rect x="${x}" y="${p.bbox[1] + l * p.pitch + p.pitch * 0.25}" width="${w}" height="${p.size * 0.55}" fill="${p.status === "locked" ? "#c9c9cf" : "#9a9aa3"}"/>`;
      }).join("");
    })
    .join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${widthPx}" height="${Math.round(H * k)}" viewBox="0 0 ${W} ${H}"><rect width="100%" height="100%" fill="#fff"/>${bars}</svg>`;
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
}
export async function apply(r: EditApplyReq): Promise<EditApplyRes> {
  await sleep(300);
  if (r.text.includes("@") && !r.use_pc_font) return { kind: "missing_chars", chars: "@" };
  if (r.text.includes("Ω")) return { kind: "unsupported_chars", chars: "Ω" };
  if (r.text.includes("OVERFLOW") && !r.allow_overlap) return { kind: "overflow", lines_over: 3 };
  if (r.text.includes("PUSH") && !r.allow_overlap) return { kind: "cannot_push" };
  const before = pages[r.page].paras.find((x) => x.id === r.para)!.text;
  setText(r.page, r.para, r.text, true);
  done.push({ page: r.page, id: r.para, before, after: r.text });
  undone = [];
  return { kind: "ok", page: structuredClone(pages[r.page]) };
}
export async function undo(): Promise<EditHistory> {
  await sleep(100);
  const o = done.pop();
  if (o) {
    setText(o.page, o.id, o.before, done.some((d) => d.page === o.page && d.id === o.id));
    undone.push(o);
  }
  return hist();
}
export async function redo(): Promise<EditHistory> {
  await sleep(100);
  const o = undone.pop();
  if (o) {
    setText(o.page, o.id, o.after, true);
    done.push(o);
  }
  return hist();
}
export const history = async (): Promise<EditHistory> => hist();
export async function save(_r: EditSaveReq): Promise<EditSaveRes> {
  await sleep(1500);
  return { output: "C:/mock/Laporan_Contoh_edited.pdf", size: 210 * 1024, seconds: 2 };
}
export async function close(): Promise<void> {}
