<script lang="ts">
  import { app, openTool } from "../state.svelte";
  import { t } from "../i18n";
  import { TOOL_ORDER, TOOL_ICON, SOON } from "../tools";
  import { chooseFiles } from "../tauri";
  import { addFilesToHome } from "../files";
  import type { ToolId } from "../state.svelte";

  import Thumb from "./Thumb.svelte";
  import { fmtSize, fmtPages } from "../format";

  async function homeChoose() {
    const paths = await chooseFiles(false);
    if (!paths.length) return;
    await addFilesToHome(paths);
  }

  function homeClear() {
    app.home.droppedFile = null;
  }

  function openSuggested(id: ToolId) {
    const f = app.home.droppedFile;
    openTool(id, f ? [f] : []);
  }

  const suggestions: ToolId[] = ["compress", "split", "organize", "word"];
</script>

<div style="flex:1;min-height:0;overflow:auto">
  <div style="max-width:1120px;margin:0 auto;padding:26px 32px 32px;display:flex;flex-direction:column;gap:26px">
    {#if !app.home.droppedFile}
      <div
        style="display:flex;align-items:center;justify-content:space-between;gap:16px;text-align:left;height:160px;padding:18px 24px;border-radius:20px;border:2px dashed {app.dragging
          ? 'var(--accent)'
          : 'var(--line2)'};background:{app.dragging ? 'var(--accent-soft)' : 'var(--surface)'}"
      >
        <span
          style="width:44px;height:44px;flex-shrink:0;border-radius:14px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center"
        >
          <i class="ph ph-tray-arrow-down" style="font-size:22px"></i>
        </span>
        <div style="display:flex;flex-direction:column;gap:4px;flex:1;min-width:0">
          <div style="font-size:22px;font-weight:600;letter-spacing:-0.01em">
            {app.dragging ? t("dropRelease") : t("dropTitle")}
          </div>
          <div style="font-size:15.5px;color:var(--text2)">{t("dropSub")}</div>
        </div>
        <button
          onclick={homeChoose}
          style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer;flex-shrink:0"
        >
          <i class="ph ph-folder-open" style="font-size:19px"></i>{t("choose")}
        </button>
      </div>
      {#if app.notice}
        <div role="status" style="padding:10px 14px;border-radius:12px;background:var(--warn-soft);color:var(--text);font-size:14.5px">
          {t(app.notice as any)}
        </div>
      {/if}
    {:else}
      <div
        style="display:flex;flex-direction:column;gap:18px;padding:22px 24px 24px;border-radius:20px;border:1px solid var(--line);background:var(--surface);box-shadow:var(--shadow)"
      >
        <div style="display:flex;align-items:center;gap:14px;flex-wrap:wrap">
          {#if app.home.droppedFile.encrypted}
            <span style="width:40px;height:52px;flex-shrink:0;border-radius:6px;display:grid;place-items:center;background:var(--warn-soft);color:var(--warn)"
              ><i class="ph ph-lock-simple" style="font-size:22px"></i></span
            >
          {:else}
            <Thumb path={app.home.droppedFile.path} password={null} width={40} height={52} radius={4} />
          {/if}
          <div style="flex:1;min-width:220px;display:flex;flex-direction:column;gap:2px">
            <div style="font-size:17px;font-weight:600;overflow-wrap:anywhere">{app.home.droppedFile.name}</div>
            <div style="color:var(--text2)">{app.home.droppedFile.pages ? `${fmtPages(app.home.droppedFile.pages)} · ` : ""}{fmtSize(app.home.droppedFile.size_bytes)}</div>
          </div>
          <button
            onclick={homeClear}
            style="min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer"
            >{t("otherFile")}</button
          >
        </div>
        <div style="font-size:15px;font-weight:600;color:var(--text2)">{t("suggestLead")}</div>
        <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:10px">
          {#each suggestions as id (id)}
            <button
              onclick={() => openSuggested(id)}
              style="display:flex;align-items:center;gap:12px;text-align:left;min-height:60px;padding:10px 14px;border-radius:14px;border:1px solid {id ===
              'compress'
                ? 'var(--accent)'
                : 'var(--line)'};background:{id === 'compress' ? 'var(--accent-soft)' : 'var(--surface)'};color:var(--text);cursor:pointer"
            >
              <span
                style="width:36px;height:36px;flex-shrink:0;border-radius:10px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center"
              >
                <i class={TOOL_ICON[id]} style="font-size:20px"></i>
              </span>
              <span style="display:flex;flex-direction:column;gap:1px;min-width:0">
                <span style="font-weight:600;font-size:15.5px">{t(`${id}Name` as any)}</span>
                {#if id === "compress"}
                  <span style="font-size:13px;color:var(--accent-text);font-weight:600">{t("mostUsed")}</span>
                {/if}
              </span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <div style="display:flex;flex-direction:column;gap:14px">
      <h2 style="margin:0;font-size:17px;font-weight:600;color:var(--text2)">{t("toolsHeading")}</h2>
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(236px,1fr));gap:14px">
        {#each TOOL_ORDER as id (id)}
          {@const needs = id === "word" && !app.office.word ? t("needsWord") : ""}
          <button
            onclick={() => openTool(id)}
            style="display:flex;flex-direction:column;align-items:flex-start;gap:10px;text-align:left;padding:18px 18px 16px;border-radius:16px;border:1px solid var(--line);background:var(--surface);color:var(--text);cursor:pointer;box-shadow:var(--shadow)"
            class="hover-accent-border"
          >
            <span
              style="width:42px;height:42px;border-radius:12px;background:var(--accent-soft);color:var(--accent-text);display:grid;place-items:center"
            >
              <i class={TOOL_ICON[id]} style="font-size:23px"></i>
            </span>
            <span style="display:flex;flex-direction:column;gap:3px">
              <span style="font-size:17px;font-weight:600">{t(`${id}Name` as any)}</span>
              <span style="font-size:14.5px;color:var(--text2)">{t(`${id}Desc` as any)}</span>
            </span>
            {#if needs}
              <span style="display:flex;align-items:center;gap:5px;font-size:13.5px;font-weight:600;color:var(--warn)">
                <i class="ph ph-info" style="font-size:15px"></i>{needs}
              </span>
            {/if}
          </button>
        {/each}
        {#each SOON as c (c.id)}
          <div
            aria-disabled="true"
            style="display:flex;flex-direction:column;align-items:flex-start;gap:10px;padding:18px 18px 16px;border-radius:16px;border:1px dashed var(--line2);color:var(--text3)"
          >
            <span style="display:flex;width:100%;justify-content:space-between;align-items:center;gap:8px">
              <span style="width:42px;height:42px;border-radius:12px;background:var(--surface2);display:grid;place-items:center">
                <i class={c.icon} style="font-size:23px"></i>
              </span>
              <span
                style="white-space:nowrap;font-size:12.5px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;padding:3px 8px;border-radius:6px;background:var(--surface2);color:var(--text2)"
                >{t("soon")}</span
              >
            </span>
            <span style="display:flex;flex-direction:column;gap:3px">
              <span style="font-size:17px;font-weight:600;color:var(--text2)">{t(c.nameKey as any)}</span>
              <span style="font-size:14.5px;color:var(--text3)">{t(c.descKey as any)}</span>
            </span>
          </div>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .hover-accent-border:hover {
    border-color: var(--accent);
  }
</style>
