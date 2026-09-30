<script lang="ts">
  import { app, settings, goHome, saveSettings, isRunning } from "../state.svelte";
  import { t } from "../i18n";
  import logo from "../../../src-tauri/icons/icon.svg"; // single source for app + header icon

  const inTool = $derived(app.route !== "home");
  const toolName = $derived(inTool ? t(`${app.route}Name` as any) : "");

  function setLang(l: "en" | "id") {
    settings.lang = l;
    saveSettings();
  }
</script>

<header
  style="display:flex;align-items:center;gap:12px;padding:0 16px 0 14px;height:56px;border-bottom:1px solid var(--line);background:var(--surface);flex-shrink:0;flex-wrap:nowrap;min-width:0"
>
  <button
    onclick={goHome}
    aria-disabled={isRunning()}
    aria-label="majipdf — home"
    style="display:flex;align-items:center;gap:9px;background:none;border:0;padding:6px 8px;border-radius:10px;cursor:pointer;color:var(--text);flex-shrink:0;opacity:{isRunning() ? 0.5 : 1}"
  >
    <img src={logo} alt="" width="30" height="30" style="display:block" />
    <span style="font-weight:700;font-size:18px;letter-spacing:-0.01em;white-space:nowrap"
      >maji<span style="color:var(--accent-text)">pdf</span></span
    >
  </button>

  {#if inTool}
    <span style="width:1px;height:22px;background:var(--line);flex-shrink:0"></span>
    <button
      onclick={goHome}
      aria-disabled={isRunning()}
      style="opacity:{isRunning() ? 0.5 : 1};display:flex;align-items:center;gap:6px;min-height:36px;padding:0 10px;border-radius:9px;border:0;background:none;color:var(--text2);font-size:15px;cursor:pointer;flex-shrink:0;white-space:nowrap"
      class:hover-surface2={!isRunning()}
    >
      <i class="ph ph-arrow-left" style="font-size:16px"></i>{t("allTools")}
    </button>
    <span style="font-weight:600;font-size:16px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;min-width:0">
      {toolName}
    </span>
  {/if}

  <div style="flex:1;min-width:8px"></div>

  <div
    role="group"
    aria-label={t("langLabel")}
    style="display:flex;padding:3px;border-radius:10px;background:var(--surface2);gap:2px;flex-shrink:0"
  >
    <button
      onclick={() => setLang("en")}
      aria-pressed={settings.lang === "en"}
      style="min-width:40px;height:30px;border-radius:7px;border:0;cursor:pointer;font-size:13.5px;font-weight:700;letter-spacing:.03em;background:{settings.lang ===
      'en'
        ? 'var(--surface)'
        : 'transparent'};color:{settings.lang === 'en' ? 'var(--text)' : 'var(--text2)'};box-shadow:{settings.lang ===
      'en'
        ? '0 1px 2px rgba(0,0,0,.12)'
        : 'none'}">EN</button
    >
    <button
      onclick={() => setLang("id")}
      aria-pressed={settings.lang === "id"}
      style="min-width:40px;height:30px;border-radius:7px;border:0;cursor:pointer;font-size:13.5px;font-weight:700;letter-spacing:.03em;background:{settings.lang ===
      'id'
        ? 'var(--surface)'
        : 'transparent'};color:{settings.lang === 'id' ? 'var(--text)' : 'var(--text2)'};box-shadow:{settings.lang ===
      'id'
        ? '0 1px 2px rgba(0,0,0,.12)'
        : 'none'}">ID</button
    >
  </div>

  <button
    onclick={() => ((app.settingsOpen = false), (app.aboutOpen = true))}
    aria-label={t("about")}
    title={t("about")}
    style="width:38px;height:38px;border-radius:10px;border:0;background:none;color:var(--text2);cursor:pointer;display:grid;place-items:center;flex-shrink:0"
    class="hover-surface2"
  >
    <i class="ph ph-info" style="font-size:21px"></i>
  </button>

  <button
    data-settings-toggle
    onclick={() => (app.settingsOpen = !app.settingsOpen)}
    aria-label={t("settings")}
    aria-expanded={app.settingsOpen}
    style="width:38px;height:38px;border-radius:10px;border:0;background:none;color:var(--text2);cursor:pointer;display:grid;place-items:center;flex-shrink:0"
    class="hover-surface2"
  >
    <i class="ph ph-gear-six" style="font-size:21px"></i>
  </button>
</header>

<style>
  .hover-surface2:hover {
    background: var(--surface2);
    color: var(--text);
  }
</style>
