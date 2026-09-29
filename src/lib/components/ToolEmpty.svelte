<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { TOOL_ICON } from "../tools";
  import { chooseFiles, chooseFolder } from "../tauri";
  import { addFilesToTool } from "../files";
  import type { ToolId } from "../state.svelte";

  const id = $derived(app.route as ToolId);
  const emptyKey = $derived(("empty" + id[0].toUpperCase() + id.slice(1)) as any);

  async function choose() {
    const paths = await chooseFiles(id === "merge" || id === "compress");
    if (!paths.length) return;
    await addFilesToTool(paths);
  }

  // ponytail: folder only for now (file_info expands folders); ZIP later.
  async function chooseFolderOrZip() {
    const paths = await chooseFolder();
    if (paths.length) await addFilesToTool(paths);
  }
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div style="margin:auto;width:100%;max-width:860px;display:flex;flex-direction:column;gap:18px">
    <div style="display:flex;flex-direction:column;gap:4px">
      <h1 style="margin:0;font-size:26px;font-weight:700;letter-spacing:-0.01em">{t(`${id}Name` as any)}</h1>
      <div style="font-size:16px;color:var(--text2)">{t(`${id}Desc` as any)}</div>
    </div>
    <div
      style="display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;text-align:center;min-height:300px;padding:40px 24px;border-radius:22px;border:2px dashed {app.dragging
        ? 'var(--accent)'
        : 'var(--line2)'};background:{app.dragging ? 'var(--accent-soft)' : 'var(--surface)'}"
    >
      <span
        style="width:64px;height:64px;border-radius:20px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center"
      >
        <i class={TOOL_ICON[id]} style="font-size:32px"></i>
      </span>
      <div style="font-size:21px;font-weight:600;max-width:560px">{app.dragging ? t("dropRelease") : t(emptyKey)}</div>
      <div style="display:flex;align-items:center;gap:12px;flex-wrap:wrap;justify-content:center">
        <button
          onclick={choose}
          style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer"
        >
          <i class="ph ph-folder-open" style="font-size:19px"></i>{t("choose")}
        </button>
        {#if id === "compress"}
          <button
            onclick={chooseFolderOrZip}
            style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 18px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer"
          >
            <i class="ph ph-file-zip" style="font-size:19px"></i>{t("chooseFolder")}
          </button>
        {/if}
      </div>
      <div style="font-size:14px;color:var(--text3);max-width:520px">{t("emptyHint")}</div>
    </div>
    {#if app.notice}
      <div role="status" style="padding:10px 14px;border-radius:12px;background:var(--warn-soft);color:var(--text);font-size:14.5px">
        {t(app.notice as any)}
      </div>
    {/if}
  </div>
</div>
