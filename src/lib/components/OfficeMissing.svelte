<script lang="ts">
  import { app, goHome } from "../state.svelte";
  import { t } from "../i18n";
  import { TOOL_ICON } from "../tools";
  import { officeStatus } from "../tauri";
  import type { ToolId } from "../state.svelte";

  const id = $derived(app.route as ToolId);
  const titleKey = $derived((id === "word" ? "officeWordTitle" : "officeExcelTitle") as any);
  const appName = $derived(id === "word" ? "Microsoft Word" : "Microsoft Excel");

  async function checkAgain() {
    app.office = await officeStatus();
  }

  async function copyMessage() {
    const msg = `${t(titleKey)}\n${t("officeBody", { app: appName })}`;
    try {
      await navigator.clipboard.writeText(msg);
    } catch {
      // ponytail: clipboard can be denied by the OS; no fallback UI for a skeleton.
    }
  }
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div
    style="margin:auto;width:100%;max-width:600px;display:flex;flex-direction:column;gap:16px;padding:28px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow)"
  >
    <span
      style="width:52px;height:52px;border-radius:16px;background:var(--warn-soft);color:var(--warn);display:grid;place-items:center"
    >
      <i class={TOOL_ICON[id]} style="font-size:28px"></i>
    </span>
    <h1 style="margin:0;font-size:22px;font-weight:700">{t(titleKey)}</h1>
    <p style="margin:0;font-size:16px;color:var(--text2)">{t("officeBody", { app: appName })}</p>
    <p style="margin:0;font-size:15px;color:var(--text2);display:flex;gap:8px">
      <i class="ph ph-check-circle" style="font-size:18px;color:var(--ok);flex-shrink:0;margin-top:1px"></i>{t("officeMeanwhile")}
    </p>
    <div style="display:flex;gap:10px;flex-wrap:wrap;padding-top:4px">
      <button
        onclick={checkAgain}
        style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 20px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer"
      >
        <i class="ph ph-arrow-clockwise" style="font-size:18px"></i>{t("checkAgain")}
      </button>
      <button
        onclick={copyMessage}
        style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 18px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer"
      >
        <i class="ph ph-copy" style="font-size:18px"></i>{t("copyIT")}
      </button>
      <button
        onclick={goHome}
        style="min-height:44px;padding:0 14px;border-radius:12px;border:0;background:none;color:var(--accent-text);font-size:16px;font-weight:600;cursor:pointer"
        >{t("allTools")}</button
      >
    </div>
  </div>
</div>
