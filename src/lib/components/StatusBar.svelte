<script lang="ts">
  import { app, settings } from "../state.svelte";
  import { t } from "../i18n";

  const outLabel = $derived(settings.outMode === "folder" ? t("outFolder") : t("outNext"));
</script>

<footer
  style="display:flex;align-items:center;gap:16px;padding:0 18px;min-height:36px;border-top:1px solid var(--line);background:var(--surface);font-size:13.5px;color:var(--text2);flex-shrink:0;flex-wrap:wrap"
>
  <span style="display:flex;align-items:center;gap:7px;white-space:nowrap">
    <i class="ph-fill ph-shield-check" style="font-size:16px;color:var(--ok)"></i>{t("trust")}
  </span>
  <div style="flex:1"></div>
  <span style="display:flex;align-items:center;gap:8px;white-space:nowrap">
    <i class="ph ph-folder" style="font-size:15px"></i>{outLabel}
    <button
      data-settings-toggle
      onclick={() => (app.settingsOpen = !app.settingsOpen)}
      style="border:0;background:none;padding:2px 4px;color:var(--accent-text);font-size:13.5px;font-weight:600;cursor:pointer;text-decoration:underline;text-underline-offset:2px"
      >{t("change")}</button
    >
  </span>
</footer>
