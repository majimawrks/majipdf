<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n";
  import { fmtElapsed } from "../format";

  // Generic single-output processing card (Merge now; Split/Organize/Word/Excel later).
  let { name, percent, status, oncancel, extra }: { name: string; percent: number; status: string; oncancel: () => void; extra?: string } = $props();
  let elapsed = $state(0);

  // Ticks every second so the screen never looks frozen.
  onMount(() => {
    const start = Date.now();
    const id = setInterval(() => (elapsed = Math.floor((Date.now() - start) / 1000)), 1000);
    return () => clearInterval(id);
  });
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div
    role="status"
    aria-live="polite"
    style="margin:auto;width:100%;max-width:680px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:26px 28px;display:flex;flex-direction:column;gap:16px"
  >
    <div style="display:flex;align-items:center;gap:12px">
      <span class="spin"></span>
      <div style="flex:1;font-size:22px;font-weight:700">{t("processing")}</div>
      <div style="font-size:22px;font-weight:700;color:var(--accent-text);font-variant-numeric:tabular-nums">{percent}%</div>
    </div>
    <div style="display:flex;flex-direction:column;gap:6px">
      <div style="display:flex;justify-content:space-between;gap:12px">
        <span style="font-weight:600;overflow-wrap:anywhere">{name}</span>
        <span style="font-size:14px;font-variant-numeric:tabular-nums;white-space:nowrap;color:var(--text2)">{status}</span>
      </div>
      <div style="height:8px;border-radius:4px;background:var(--surface2);overflow:hidden">
        <div style="height:100%;width:{percent}%;background:var(--accent);transition:width .3s linear"></div>
      </div>
    </div>
    <div style="display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap">
      <div style="font-size:14px;color:var(--text2);flex:1;min-width:220px">
        {t("procNote")}{extra ? " " + extra : ""}
        <span style="font-variant-numeric:tabular-nums;color:var(--text3)"> · {t("elapsed", { t: fmtElapsed(elapsed) })}</span>
      </div>
      <button
        onclick={oncancel}
        style="min-height:44px;padding:0 22px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer">{t("cancel")}</button
      >
    </div>
  </div>
</div>

<style>
  .spin {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 3px solid var(--surface2);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
