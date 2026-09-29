// Pointer-based drag reorder. HTML5 DnD is swallowed by Tauri's native file-drop
// handler on Windows (cursor shows "not allowed"), so we track the pointer ourselves.
// Usage: put `data-reorder-idx={i}` on each item, call `r.down(e, i)` from a handle's
// onpointerdown; `r.from` is the dragged item's current index (for "lifted" styling).
// The list reorders live while dragging; pair it with `animate:flip={{ duration: FLIP_MS }}`
// on the keyed items so neighbours slide out of the way.
export const FLIP_MS = 160;

export function pointerReorder(onmove: (from: number, to: number) => void) {
  const s = $state({ from: null as number | null, over: null as number | null });

  function indexAt(x: number, y: number): number | null {
    const el = document.elementFromPoint(x, y)?.closest("[data-reorder-idx]");
    return el ? Number(el.getAttribute("data-reorder-idx")) : null;
  }

  function down(e: PointerEvent, i: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    s.from = i;
    s.over = i;
    // ponytail: ignore hit-tests while items are still sliding, otherwise the neighbour that is
    // animating away from under the pointer triggers a swap back (flicker). Fixed lock = FLIP_MS.
    let lockedUntil = 0;
    const move = (ev: PointerEvent) => {
      if (performance.now() < lockedUntil || s.from === null) return;
      const at = indexAt(ev.clientX, ev.clientY);
      if (at === null || at === s.from) return;
      onmove(s.from, at);
      s.from = s.over = at;
      lockedUntil = performance.now() + FLIP_MS;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      s.from = s.over = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  }

  return {
    get from() {
      return s.from;
    },
    get over() {
      return s.over;
    },
    down,
  };
}
