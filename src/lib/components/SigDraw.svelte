<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { sigAddDrawn, refreshSigs, pickSig } from "../edit";

  // Modal drawing canvas, 600x240 pt (stored at 2x). Black ink on a transparent canvas, exported as a PNG data URL.
  const e = app.edit;
  const CW = 600;
  const CH = 240;
  let cv: HTMLCanvasElement;
  let name = $state(t("sigKindSig"));
  let inked = $state(false);
  let saving = $state(false);
  let err = $state("");
  let last: { x: number; y: number; mx: number; my: number } | null = null;
  let nameEl = $state<HTMLInputElement>();

  const ctx = () => cv.getContext("2d")!;
  onMount(() => {
    const c = ctx();
    c.scale(2, 2);
    c.lineCap = "round";
    c.lineJoin = "round";
    c.strokeStyle = "#000";
    c.fillStyle = "#000";
    nameEl?.select();
  });

  const pt = (ev: PointerEvent) => {
    const r = cv.getBoundingClientRect();
    return { x: ((ev.clientX - r.left) / r.width) * CW, y: ((ev.clientY - r.top) / r.height) * CH };
  };
  // Pen pressure when the device reports it; mouse/touch get a steady width.
  const width = (ev: PointerEvent) => (ev.pointerType === "pen" && ev.pressure > 0 ? 1 + ev.pressure * 3.5 : 2.4);

  function down(ev: PointerEvent) {
    ev.preventDefault();
    cv.setPointerCapture(ev.pointerId);
    const p = pt(ev);
    const c = ctx();
    c.beginPath();
    c.arc(p.x, p.y, width(ev) / 2, 0, Math.PI * 2);
    c.fill();
    last = { x: p.x, y: p.y, mx: p.x, my: p.y };
    inked = true;
  }
  // Smoothing: quadratic curve through segment midpoints, the raw point as the control.
  function move(ev: PointerEvent) {
    if (!last) return;
    const c = ctx();
    for (const sub of ev.getCoalescedEvents?.().length ? ev.getCoalescedEvents() : [ev]) {
      const p = pt(sub);
      const mx: number = (last.x + p.x) / 2;
      const my: number = (last.y + p.y) / 2;
      c.lineWidth = width(sub);
      c.beginPath();
      c.moveTo(last.mx, last.my);
      c.quadraticCurveTo(last.x, last.y, mx, my);
      c.stroke();
      last = { x: p.x, y: p.y, mx, my };
    }
  }
  const up = () => (last = null);

  function clear() {
    ctx().clearRect(0, 0, CW, CH);
    inked = false;
  }

  function close() {
    if (!saving) e.drawOpen = false;
  }
  async function done() {
    if (!inked || saving) return;
    saving = true;
    err = "";
    try {
      const it = await sigAddDrawn(cv.toDataURL("image/png"), name.trim() || t("sigKindSig"));
      await refreshSigs();
      e.drawOpen = false;
      pickSig(it); // ready to place
    } catch (x) {
      err = String(x);
    } finally {
      saving = false;
    }
  }

  function onkeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape") {
      ev.preventDefault();
      ev.stopPropagation();
      close();
    } else if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
      ev.preventDefault();
      void done();
    }
  }

  const btn = "min-height:44px;padding:0 20px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer;white-space:nowrap";
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div style="position:fixed;inset:0;z-index:50;background:var(--scrim);display:grid;place-items:center;padding:20px;overflow:auto" {onkeydown}>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="draw-title"
    style="width:100%;max-width:{CW + 52}px;box-sizing:border-box;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:22px 26px;display:flex;flex-direction:column;gap:16px"
  >
    <div id="draw-title" style="font-size:19px;font-weight:700">{t("sigDrawTitle")}</div>
    <canvas
      bind:this={cv}
      width={CW * 2}
      height={CH * 2}
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
      aria-label={t("sigDrawTitle")}
      style="width:100%;max-width:{CW}px;aspect-ratio:{CW}/{CH};touch-action:none;cursor:crosshair;border-radius:12px;border:1px solid var(--line2);background:var(--page);align-self:center"
    ></canvas>
    <label style="display:flex;flex-direction:column;gap:6px;font-weight:600">
      {t("sigName")}
      <input
        bind:this={nameEl}
        bind:value={name}
        maxlength="60"
        style="height:44px;box-sizing:border-box;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font:inherit;padding:0 12px"
      />
    </label>
    {#if err}<div role="alert" style="padding:10px 12px;border-radius:10px;background:var(--err-soft);overflow-wrap:anywhere">{err}</div>{/if}
    <div style="display:flex;gap:10px;justify-content:flex-end;flex-wrap:wrap">
      <button style={btn} onclick={clear} disabled={!inked || saving}>{t("sigClear")}</button>
      <span style="flex:1"></span>
      <button style={btn} onclick={close} disabled={saving}>{t("cancel")}</button>
      <button
        onclick={done}
        disabled={!inked || saving}
        style="min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer">{t("sigDone")}</button
      >
    </div>
  </div>
</div>

<style>
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed !important;
  }
</style>
