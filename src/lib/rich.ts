// Run helpers for the rich in-place editor: pure run algebra plus the few DOM reads/writes. Text only, never HTML.
import type { Run, Style } from "./edit";

export const sameStyle = (a: Style, b: Style) => a.font === b.font && a.size === b.size && a.color.toLowerCase() === b.color.toLowerCase() && a.bold === b.bold && a.italic === b.italic && a.align === b.align;
export const textOf = (r: Run[]) => r.map((x) => x.text).join("");

export function mergeRuns(runs: Run[]): Run[] {
  const out: Run[] = [];
  for (const r of runs) {
    if (!r.text) continue;
    const l = out[out.length - 1];
    if (l && sameStyle(l.style, r.style)) l.text += r.text;
    else out.push({ text: r.text, style: { ...r.style } });
  }
  return out;
}

// Split at [s, e) and patch the runs inside. A caret (s === e) patches everything.
export function applyRange(runs: Run[], s: number, e: number, patch: Partial<Style>): Run[] {
  if (s === e) return mergeRuns(runs.map((r) => ({ text: r.text, style: { ...r.style, ...patch } })));
  const out: Run[] = [];
  let pos = 0;
  for (const r of runs) {
    const a = pos;
    const b = (pos += r.text.length);
    if (e <= a || s >= b) {
      out.push(r);
      continue;
    }
    const i0 = Math.max(s, a) - a;
    const i1 = Math.min(e, b) - a;
    if (i0 > 0) out.push({ text: r.text.slice(0, i0), style: r.style });
    out.push({ text: r.text.slice(i0, i1), style: { ...r.style, ...patch } });
    if (i1 < r.text.length) out.push({ text: r.text.slice(i1), style: r.style });
  }
  return mergeRuns(out);
}

// Style at the caret (char before it) or over a selection; fields that differ across the selection are listed in `mixed`.
export function styleOver(runs: Run[], s: number, e: number, base: Style): { style: Style; mixed: string[] } {
  const hit: Style[] = [];
  let pos = 0;
  const lo = s === e ? Math.max(0, s - 1) : s;
  const hi = s === e ? lo + 1 : e;
  for (const r of runs) {
    const a = pos;
    pos += r.text.length;
    if (hi > a && lo < pos) hit.push(r.style);
  }
  if (!hit.length) return { style: runs[0]?.style ?? base, mixed: [] };
  const mixed = (["font", "size", "color", "bold", "italic"] as const).filter((k) => hit.some((h) => String(h[k]).toLowerCase() !== String(hit[0][k]).toLowerCase()));
  return { style: hit[0], mixed };
}

// Normalise typed text: paragraphs are one line of words, boxes keep single line breaks.
export function normRuns(runs: Run[], para: boolean): Run[] {
  const out: Run[] = [];
  let prev = "\n"; // the previous kept char: strips the leading space at the very start
  for (const r of runs) {
    let t = para ? r.text.replace(/\s+/g, " ") : r.text.replace(/[ \t]+/g, " ").replace(/ ?\n ?/g, "\n");
    if (prev === " " || prev === "\n") t = t.replace(/^ +/, "");
    if (t) prev = t[t.length - 1];
    out.push({ text: t, style: r.style });
  }
  const m = mergeRuns(out);
  const l = m[m.length - 1];
  if (l) {
    l.text = l.text.replace(/\s+$/, "");
    if (!l.text) m.pop();
  }
  return m;
}

// ---- DOM ----
export function renderRuns(el: HTMLElement, runs: Run[], css: (s: Style) => string) {
  el.textContent = "";
  runs.forEach((r, k) => {
    const sp = document.createElement("span");
    sp.dataset.k = String(k);
    sp.style.cssText = css(r.style);
    sp.appendChild(document.createTextNode(r.text));
    el.appendChild(sp);
  });
}

// Read the editor back as runs: each text node takes the style of its span (by data-k into `snap`), else the previous one.
export function readRuns(el: HTMLElement, snap: Style[], base: Style): Run[] {
  const out: Run[] = [];
  let cur = snap[0] ?? base;
  const walk = (n: Node) => {
    if (n.nodeType === 3) {
      const sp = (n.parentElement as HTMLElement | null)?.closest("[data-k]") as HTMLElement | null;
      if (sp && el.contains(sp)) cur = snap[Number(sp.dataset.k)] ?? cur;
      out.push({ text: n.nodeValue ?? "", style: cur });
    } else if ((n as Element).tagName === "BR") out.push({ text: "\n", style: cur });
    else n.childNodes.forEach(walk);
  };
  el.childNodes.forEach(walk);
  return mergeRuns(out);
}

export function offsetOf(root: HTMLElement, node: Node, off: number): number {
  const r = document.createRange();
  r.selectNodeContents(root);
  r.setEnd(node, off);
  return r.toString().length;
}

export function setSel(root: HTMLElement, s: number, e: number) {
  const at = (target: number): [Node, number] => {
    let pos = 0;
    const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let last: Text | null = null;
    for (let n = w.nextNode() as Text | null; n; n = w.nextNode() as Text | null) {
      last = n;
      if (target <= pos + n.length) return [n, target - pos];
      pos += n.length;
    }
    return last ? [last, last.length] : [root, 0];
  };
  const r = document.createRange();
  r.setStart(...at(s));
  r.setEnd(...at(e));
  const sel = getSelection();
  sel?.removeAllRanges();
  sel?.addRange(r);
}
