<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum, fmtElapsed } from "../format";
  import { cancelRun } from "../compress";

  const run = app.compress.run;
  let elapsed = $state(0);

  // Ticks every second so the screen never looks frozen, even if gs is silent for a while.
  onMount(() => {
    const start = Date.now();
    const id = setInterval(() => (elapsed = Math.floor((Date.now() - start) / 1000)), 1000);
    return () => clearInterval(id);
  });

  function frac(i: number): number {
    const p = run.progress[i];
    if (!p) return 0;
    if (p.state === "done") return 1;
    const pageFrac = p.pages ? p.page / p.pages : 0;
    if (run.mode === "target" && p.attempt) return Math.min(0.95, ((p.attempt - 1 + pageFrac) / 6));
    return Math.min(0.99, pageFrac);
  }

  function status(i: number): string {
    const p = run.progress[i];
    if (!p) return t("waiting");
    if (p.state === "done") return t("doneShort", { s: p.current_size ? fmtSize(p.current_size) : "" }).replace(/ · $/, "");
    if (run.mode === "target" && p.attempt)
      return t("attempt", { a: p.attempt, dpi: p.dpi ?? "", s: p.current_size ? fmtSize(p.current_size) : "…" });
    return t("compressing", { a: fmtNum(p.page), b: fmtNum(p.pages) });
  }

  const overall = $derived(
    Math.floor((run.files.reduce((s, _, i) => s + frac(i), 0) / Math.max(1, run.files.length)) * 100)
  );
  const large = $derived(run.files.some((f) => (f.pages ?? 0) > 300));
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
      <div style="font-size:22px;font-weight:700;color:var(--accent-text);font-variant-numeric:tabular-nums">{overall}%</div>
    </div>
    {#each run.files as f, i}
      {@const done = run.progress[i]?.state === "done"}
      <div style="display:flex;flex-direction:column;gap:6px">
        <div style="display:flex;justify-content:space-between;gap:12px">
          <span style="font-weight:600;overflow-wrap:anywhere">{f.name}</span>
          <span style="font-size:14px;font-variant-numeric:tabular-nums;white-space:nowrap;color:{done ? 'var(--ok)' : 'var(--text2)'}">{status(i)}</span>
        </div>
        <div style="height:8px;border-radius:4px;background:var(--surface2);overflow:hidden">
          <div style="height:100%;width:{frac(i) * 100}%;background:{done ? 'var(--ok)' : 'var(--accent)'};transition:width .3s linear"></div>
        </div>
      </div>
    {/each}
    {#if large}
      <div style="display:flex;gap:10px;padding:12px 14px;border-radius:12px;background:var(--accent-soft);color:var(--text)">
        <i class="ph ph-info" style="font-size:20px;color:var(--accent-text);flex-shrink:0"></i>{t("largeNote")}
      </div>
    {/if}
    <div style="display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap">
      <div style="font-size:14px;color:var(--text2);flex:1;min-width:220px">
        {t("procNote")}
        <span style="font-variant-numeric:tabular-nums;color:var(--text3)"> · {t("elapsed", { t: fmtElapsed(elapsed) })}</span>
      </div>
      <button
        onclick={cancelRun}
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
