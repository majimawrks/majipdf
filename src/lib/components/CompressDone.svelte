<script lang="ts">
  import { app, settings } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize } from "../format";
  import { keep, discard, processAnother } from "../compress";
  import { openPath, revealPath, copyText } from "../tauri";

  const run = app.compress.run;
  const GRID = "minmax(0,1fr) 84px 84px 64px";

  // A result counts as compressed when it produced a smaller kept file.
  const isSaved = (i: number) => {
    const r = run.results[i];
    return r.status === "done" || (r.status === "target_missed" && run.keep[i] === "kept");
  };
  const savedBytes = $derived(run.results.reduce((s, r, i) => s + (isSaved(i) && r.after != null ? r.before - r.after : 0), 0));
  const count = $derived(run.results.filter((_, i) => isSaved(i)).length);
  const outputs = $derived(run.results.filter((r) => r.output).map((r) => r.output as string));

  const pct = (i: number) => {
    const r = run.results[i];
    if (r.status === "failed" || r.after == null || !r.before) return "–";
    const p = Math.round(((r.before - r.after) / r.before) * 100);
    return p > 0 ? `−${p}%` : "0%";
  };
  const tgt = () => fmtSize(run.targetMb * 1024 * 1024);

  const location = $derived.by(() => {
    if (outputs.length === 1) {
      const dir = outputs[0].replace(/[\\/][^\\/]*$/, "").split(/[\\/]/).join(" › ");
      return t("savedIn", { dir });
    }
    return settings.outMode === "folder" ? t("savedFolder") : t("savedNextTo");
  });

  const btn = "display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 20px;border-radius:12px;font-size:16px;cursor:pointer;";
  const primary = btn + "border:0;background:var(--accent-fill);color:var(--accent-ink);font-weight:600";
  const secondary = btn + "border:1px solid var(--line2);background:var(--surface);color:var(--text)";
  const quiet = btn + "border:0;background:none;color:var(--accent-text);font-weight:600";

  let copied = $state<Record<number, boolean>>({});
  async function copyDetails(i: number) {
    copied[i] = await copyText(run.results[i].error || run.keepErr[i] || "");
  }

  function noteFor(i: number): string {
    const r = run.results[i];
    const k = run.keep[i];
    if (r.status === "no_reduction") return t("noReduction");
    if (r.status === "under_target") return t("underTarget", { t: tgt() });
    if (k === "kept") return t("kept", { s: fmtSize(r.after ?? 0) });
    if (k === "discarded") return t("discarded");
    if (r.status === "failed" || k === "failed") return t("compressFailed");
    return "";
  }
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div
    style="margin:auto;width:100%;max-width:760px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:26px 28px;display:flex;flex-direction:column;gap:18px"
  >
    <div style="display:flex;align-items:center;gap:14px">
      <span style="width:48px;height:48px;border-radius:50%;background:var(--ok-soft);color:var(--ok);display:grid;place-items:center;flex-shrink:0">
        <i class="ph ph-check" style="font-size:26px"></i>
      </span>
      <div>
        <div style="font-size:24px;font-weight:700">{count === 1 ? t("compressDone1") : t("compressDone", { n: count })}</div>
        <div style="color:var(--text2)">{t("savedTotal", { s: fmtSize(savedBytes) })}</div>
      </div>
    </div>

    <div style="border:1px solid var(--line);border-radius:16px;overflow:hidden">
      <div
        style="display:grid;grid-template-columns:{GRID};gap:10px;padding:10px 16px;background:var(--surface2);font-size:13px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:var(--text3)"
      >
        <span>{t("colFile")}</span><span style="text-align:right">{t("colBefore")}</span><span style="text-align:right">{t("colAfter")}</span><span
          style="text-align:right">{t("colSaved")}</span
        >
      </div>
      {#each run.results as r, i}
        {@const name = run.files[i]?.name ?? r.path}
        {@const k = run.keep[i]}
        {@const warn = r.status === "target_missed" && (!k || k === "busy")}
        {@const bad = r.status === "failed"}
        {@const note = noteFor(i)}
        <div style="border-top:1px solid var(--line);padding:12px 16px;display:flex;flex-direction:column;gap:8px">
          <div style="display:grid;grid-template-columns:{GRID};gap:10px;align-items:center">
            <span style="display:flex;align-items:center;gap:8px;min-width:0;font-weight:600;overflow-wrap:anywhere">
              {#if bad}<i class="ph ph-x-circle" style="font-size:18px;color:var(--err);flex-shrink:0"></i>
              {:else if warn}<i class="ph ph-warning-circle" style="font-size:18px;color:var(--warn);flex-shrink:0"></i>
              {:else if r.status === "no_reduction" || r.status === "under_target" || k === "discarded"}<i
                  class="ph ph-minus-circle"
                  style="font-size:18px;color:var(--text3);flex-shrink:0"
                ></i>
              {:else}<i class="ph ph-check-circle" style="font-size:18px;color:var(--ok);flex-shrink:0"></i>{/if}
              {name}
            </span>
            <span style="text-align:right;color:var(--text2);font-variant-numeric:tabular-nums">{fmtSize(r.before)}</span>
            <span style="text-align:right;font-weight:600;font-variant-numeric:tabular-nums">{r.after != null && !bad ? fmtSize(r.after) : "–"}</span>
            <span
              style="text-align:right;font-weight:600;font-variant-numeric:tabular-nums;color:{warn ? 'var(--warn)' : pct(i).startsWith('−') ? 'var(--ok)' : 'var(--text2)'}"
              >{pct(i)}</span
            >
          </div>
          {#if warn}
            <div style="margin-left:26px;display:flex;align-items:center;gap:12px;flex-wrap:wrap;padding:10px 12px;border-radius:10px;background:var(--warn-soft)">
              <span style="flex:1;min-width:200px">{run.tries[i] ? t("triedN", { n: run.tries[i] }) + " " : ""}{t("missTarget", { t: tgt(), s: fmtSize(r.after ?? 0) })}</span>
              <button
                disabled={k === "busy"}
                onclick={() => keep(i)}
                style="min-height:38px;padding:0 16px;border-radius:10px;border:0;background:var(--accent-fill);color:var(--accent-ink);font-size:15px;font-weight:600;cursor:pointer"
                >{t("keepIt", { s: fmtSize(r.after ?? 0) })}</button
              >
              <button
                disabled={k === "busy"}
                onclick={() => discard(i)}
                style="min-height:38px;padding:0 16px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer"
                >{t("discard")}</button
              >
            </div>
          {:else if note}
            <div
              style="margin-left:26px;padding:8px 12px;border-radius:10px;background:{bad || k === 'failed' ? 'var(--err-soft)' : 'var(--surface2)'};color:var(--text2);font-size:14.5px"
            >
              <div>{note}</div>
              {#if (bad && r.error) || (k === "failed" && run.keepErr[i])}
                <button
                  onclick={() => copyDetails(i)}
                  style="margin-top:6px;display:inline-flex;align-items:center;gap:6px;min-height:32px;padding:0 12px;border-radius:9px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:14px;cursor:pointer"
                >
                  <i class="ph ph-{copied[i] ? 'check' : 'copy'}" style="font-size:16px"></i>{copied[i] ? t("copied") : t("copyDetails")}
                </button>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
      {#each run.locked as name}
        <div style="border-top:1px solid var(--line);padding:12px 16px;display:flex;flex-direction:column;gap:8px">
          <span style="display:flex;align-items:center;gap:8px;font-weight:600;overflow-wrap:anywhere"
            ><i class="ph ph-lock-simple" style="font-size:18px;color:var(--warn);flex-shrink:0"></i>{name}</span
          >
          <div style="margin-left:26px;padding:8px 12px;border-radius:10px;background:var(--warn-soft);color:var(--text);font-size:14.5px">{t("skippedPw")}</div>
        </div>
      {/each}
    </div>

    {#if outputs.length}
      <div style="display:flex;align-items:center;gap:8px;color:var(--text2);overflow-wrap:anywhere">
        <i class="ph ph-folder-simple" style="font-size:18px;flex-shrink:0"></i>{location}
      </div>
    {/if}

    <div style="display:flex;gap:10px;flex-wrap:wrap">
      {#if outputs.length === 1}
        <button onclick={() => openPath(outputs[0])} style={primary}><i class="ph ph-file-pdf" style="font-size:19px"></i>{t("openFile")}</button>
        <button onclick={() => revealPath(outputs[0])} style={secondary}><i class="ph ph-folder-open" style="font-size:19px"></i>{t("showFolder")}</button>
      {:else if outputs.length > 1}
        <button onclick={() => revealPath(outputs[0])} style={primary}><i class="ph ph-folder-open" style="font-size:19px"></i>{t("openFolder")}</button>
      {/if}
      <button onclick={processAnother} style={quiet}><i class="ph ph-arrow-counter-clockwise" style="font-size:19px"></i>{t("another")}</button>
    </div>
  </div>
</div>
