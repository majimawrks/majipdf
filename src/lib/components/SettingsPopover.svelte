<script lang="ts">
  import { app, settings, saveSettings, type Theme, type OutMode } from "../state.svelte";
  import { t } from "../i18n";

  let panel: HTMLDivElement;

  function close() {
    app.settingsOpen = false;
  }

  function setOutMode(mode: OutMode) {
    settings.outMode = mode;
    saveSettings();
  }

  function setTheme(theme: Theme) {
    settings.theme = theme;
    saveSettings();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }

  function onWindowClick(e: MouseEvent) {
    const target = e.target as Element;
    // The click that opened us (gear / "Change") also bubbles here; ignore toggles.
    if (panel && !panel.contains(target) && !target.closest?.("[data-settings-toggle]")) close();
  }

  const themeOpts: { id: Theme; label: string }[] = $derived([
    { id: "auto", label: t("thAuto") },
    { id: "light", label: t("thLight") },
    { id: "dark", label: t("thDark") },
  ]);

  function radioStyle(on: boolean) {
    return `border:${on ? "2px solid var(--accent)" : "1px solid var(--line2)"};background:${on ? "var(--accent-soft)" : "var(--surface)"}`;
  }
  function dotStyle(on: boolean) {
    return `border:2px solid ${on ? "var(--accent)" : "var(--line2)"}`;
  }
</script>

<svelte:window onkeydown={onKeydown} onclick={onWindowClick} />

<div
  bind:this={panel}
  role="dialog"
  aria-label={t("settings")}
  style="position:absolute;top:60px;right:14px;z-index:20;width:350px;max-width:calc(100% - 28px);display:flex;flex-direction:column;gap:18px;padding:18px 20px 20px;border-radius:16px;background:var(--surface);border:1px solid var(--line);box-shadow:0 12px 40px rgba(0,0,0,.18)"
>
  <div style="display:flex;align-items:center;justify-content:space-between">
    <strong style="font-size:17px;font-weight:700">{t("settings")}</strong>
    <button
      onclick={close}
      aria-label={t("close")}
      style="width:34px;height:34px;border-radius:9px;border:0;background:none;color:var(--text2);cursor:pointer"
    >
      <i class="ph ph-x" style="font-size:18px"></i>
    </button>
  </div>

  <fieldset style="border:0;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
    <legend style="padding:0 0 8px;font-weight:600">{t("setOut")}</legend>
    <div role="radiogroup" style="display:flex;flex-direction:column;gap:8px">
      <button
        role="radio"
        aria-checked={settings.outMode === "next"}
        onclick={() => setOutMode("next")}
        style="display:flex;gap:12px;align-items:flex-start;text-align:left;padding:10px 12px;border-radius:12px;color:var(--text);cursor:pointer;{radioStyle(
          settings.outMode === 'next'
        )}"
      >
        <span
          style="width:18px;height:18px;flex-shrink:0;margin-top:2px;border-radius:50%;display:grid;place-items:center;{dotStyle(
            settings.outMode === 'next'
          )}"
        >
          <span
            style="width:8px;height:8px;border-radius:50%;background:{settings.outMode === 'next' ? 'var(--accent)' : 'transparent'}"
          ></span>
        </span>
        <span style="display:flex;flex-direction:column">
          <span style="font-weight:600">{t("setNext")}</span>
          <span style="font-size:14px;color:var(--text2);overflow-wrap:anywhere">{t("setNextHint")}</span>
        </span>
      </button>
      <button
        role="radio"
        aria-checked={settings.outMode === "folder"}
        onclick={() => setOutMode("folder")}
        style="display:flex;gap:12px;align-items:flex-start;text-align:left;padding:10px 12px;border-radius:12px;color:var(--text);cursor:pointer;{radioStyle(
          settings.outMode === 'folder'
        )}"
      >
        <span
          style="width:18px;height:18px;flex-shrink:0;margin-top:2px;border-radius:50%;display:grid;place-items:center;{dotStyle(
            settings.outMode === 'folder'
          )}"
        >
          <span
            style="width:8px;height:8px;border-radius:50%;background:{settings.outMode === 'folder' ? 'var(--accent)' : 'transparent'}"
          ></span>
        </span>
        <span style="display:flex;flex-direction:column">
          <span style="font-weight:600">{t("setFolder")}</span>
          <span style="font-size:14px;color:var(--text2);overflow-wrap:anywhere">{t("setFolderHint")}</span>
        </span>
      </button>
    </div>
  </fieldset>

  <fieldset style="border:0;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
    <legend style="padding:0 0 8px;font-weight:600">{t("setAppearance")}</legend>
    <div role="radiogroup" style="display:grid;grid-template-columns:repeat(3,1fr);padding:3px;border-radius:11px;background:var(--surface2);gap:3px">
      {#each themeOpts as opt (opt.id)}
        <button
          role="radio"
          aria-checked={settings.theme === opt.id}
          onclick={() => setTheme(opt.id)}
          style="min-height:36px;padding:4px 6px;border-radius:8px;border:0;font-size:14px;font-weight:600;cursor:pointer;line-height:1.2;background:{settings.theme ===
          opt.id
            ? 'var(--surface)'
            : 'transparent'};color:{settings.theme === opt.id ? 'var(--text)' : 'var(--text2)'};box-shadow:{settings.theme ===
          opt.id
            ? '0 1px 2px rgba(0,0,0,.12)'
            : 'none'}">{opt.label}</button
        >
      {/each}
    </div>
  </fieldset>
</div>
