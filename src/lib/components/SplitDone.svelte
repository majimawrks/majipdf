<script lang="ts">
  import { settings, type SplitResult } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtNum } from "../format";
  import { revealPath } from "../tauri";

  // Split variant of the done card: first 4 outputs, "and n more", Open folder / Process another.
  let { result, onanother }: { result: SplitResult; onanother: () => void } = $props();

  const SHOWN = 4;
  const shown = $derived(result.outputs.slice(0, SHOWN));
  const more = $derived(result.outputs.length - SHOWN);
  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const range = (o: { from: number; to: number }) =>
    o.from === o.to ? t("splitPage1", { a: o.from }) : t("splitPagesRange", { a: o.from, b: o.to });
  const dir = $derived(result.folder.split(/[\\/]/).filter(Boolean).join(" › "));
  const location = $derived(dir ? t("savedIn", { dir }) : settings.outMode === "folder" ? t("savedFolder") : t("savedNext"));

  const btn = "display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 20px;border-radius:12px;font-size:16px;cursor:pointer;";
  const primary = btn + "border:0;background:var(--accent-fill);color:var(--accent-ink);font-weight:600";
  const quiet = btn + "border:0;background:none;color:var(--accent-text);font-weight:600";
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div
    style="margin:auto;width:100%;max-width:620px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:26px 28px;display:flex;flex-direction:column;gap:18px"
  >
    <div style="display:flex;align-items:center;gap:14px">
      <span style="width:48px;height:48px;border-radius:50%;background:var(--ok-soft);color:var(--ok);display:grid;place-items:center;flex-shrink:0">
        <i class="ph ph-check" style="font-size:26px"></i>
      </span>
      <div>
        <div style="font-size:24px;font-weight:700">{t("splitDoneTitle", { n: fmtNum(result.outputs.length) })}</div>
        <div style="color:var(--text2)">{t("doneSubSplit")}</div>
      </div>
    </div>

    <ul style="list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
      {#each shown as o}
        <li style="display:flex;align-items:center;gap:14px;padding:12px 16px;border-radius:14px;background:var(--surface2)">
          <span style="width:36px;height:36px;flex-shrink:0;border-radius:10px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center">
            <i class="ph ph-file-pdf" style="font-size:20px"></i>
          </span>
          <span style="flex:1;min-width:0;font-weight:600;overflow-wrap:anywhere">{name(o.path)}</span>
          <span style="font-size:14px;color:var(--text2);white-space:nowrap">{range(o)}</span>
        </li>
      {/each}
      {#if more > 0}
        <li style="padding:2px 16px;color:var(--text2)">{t("andMore", { n: fmtNum(more) })}</li>
      {/if}
    </ul>

    <div style="display:flex;align-items:center;gap:8px;color:var(--text2);overflow-wrap:anywhere">
      <i class="ph ph-folder-simple" style="font-size:18px;flex-shrink:0"></i>{location}
    </div>

    <div style="display:flex;gap:10px;flex-wrap:wrap">
      <button onclick={() => revealPath(result.outputs[0].path)} style={primary}><i class="ph ph-folder-open" style="font-size:19px"></i>{t("openFolder")}</button>
      <button onclick={onanother} style={quiet}><i class="ph ph-arrow-counter-clockwise" style="font-size:19px"></i>{t("another")}</button>
    </div>
  </div>
</div>
