<script lang="ts">
  import { settings } from "../state.svelte";
  import { t } from "../i18n";
  import type { Snippet } from "svelte";
  import { openPath, revealPath } from "../tauri";

  // Generic single-output done card (Merge now; Split/Organize/Word/Excel later).
  let {
    subtitle,
    output,
    meta,
    icon = "file-pdf",
    onanother,
    extra,
    after,
  }: { subtitle: string; output: string; meta: string; icon?: string; onanother: () => void; extra?: Snippet; after?: Snippet } = $props();

  const name = $derived(output.split(/[\\/]/).pop() ?? output);
  const dir = $derived(output.replace(/[\\/][^\\/]*$/, "").split(/[\\/]/).join(" › "));
  const location = $derived(dir ? t("savedIn", { dir }) : settings.outMode === "folder" ? t("savedFolder") : t("savedNextTo"));

  const btn = "display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 20px;border-radius:12px;font-size:16px;cursor:pointer;";
  const primary = btn + "border:0;background:var(--accent);color:var(--accent-ink);font-weight:600";
  const secondary = btn + "border:1px solid var(--line2);background:var(--surface);color:var(--text)";
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
        <div style="font-size:24px;font-weight:700">{t("done")}</div>
        <div style="color:var(--text2)">{subtitle}</div>
      </div>
    </div>

    <div style="display:flex;align-items:center;gap:14px;padding:14px 16px;border-radius:14px;background:var(--surface2)">
      <span style="width:40px;height:40px;flex-shrink:0;border-radius:10px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center">
        <i class="ph ph-{icon}" style="font-size:22px"></i>
      </span>
      <div style="min-width:0">
        <div style="font-weight:600;overflow-wrap:anywhere">{name}</div>
        <div style="font-size:14px;color:var(--text2)">{meta}</div>
      </div>
    </div>

    {@render extra?.()}

    <div style="display:flex;align-items:center;gap:8px;color:var(--text2);overflow-wrap:anywhere">
      <i class="ph ph-folder-simple" style="font-size:18px;flex-shrink:0"></i>{location}
    </div>

    <div style="display:flex;gap:10px;flex-wrap:wrap">
      <button onclick={() => openPath(output)} style={primary}><i class="ph ph-{icon}" style="font-size:19px"></i>{t("openFile")}</button>
      <button onclick={() => revealPath(output)} style={secondary}><i class="ph ph-folder-open" style="font-size:19px"></i>{t("showFolder")}</button>
      <button onclick={onanother} style={quiet}><i class="ph ph-arrow-counter-clockwise" style="font-size:19px"></i>{t("another")}</button>
    </div>
    {@render after?.()}
  </div>
</div>
