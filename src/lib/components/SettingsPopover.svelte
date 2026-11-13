<script lang="ts">
  import { app, settings, saveSettings, isMac, type Theme, type OutMode, type OcrEngine } from "../state.svelte";
  import { t } from "../i18n";
  import { refreshSigs } from "../edit";

  let panel: HTMLDivElement;
  $effect(() => void refreshSigs()); // count for the "Saved signatures & stamps" row


  function close() {
    app.settingsOpen = false;
  }

  function setOutMode(mode: OutMode) {
    settings.outMode = mode;
    saveSettings();
  }

  function setOcrQuality(q: OcrEngine) {
    settings.ocrQuality = q;
    saveSettings();
  }

  function setTheme(theme: Theme) {
    settings.theme = theme;
    saveSettings();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !app.edit.drawOpen && !app.sigManageOpen) close();
  }

  function onWindowClick(e: MouseEvent) {
    const target = e.target as Element;
    // The click that opened us (gear / "Change") also bubbles here; ignore toggles.
    if (app.edit.drawOpen || app.sigManageOpen) return; // a modal above us owns the clicks
    if (panel && !panel.contains(target) && !target.closest?.("[data-settings-toggle]")) close();
  }

  const ocrOpts: { id: OcrEngine; label: string }[] = $derived([
    { id: "accurate", label: t("setOcrAcc") },
    { id: "fast", label: t("setOcrFast") },
  ]);

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

<!-- dims the window behind; a click on it closes us via onWindowClick -->
<div aria-hidden="true" style="position:fixed;inset:0;z-index:19;background:var(--scrim)"></div>

<div
  bind:this={panel}
  role="dialog"
  aria-label={t("settings")}
  style="position:absolute;top:60px;right:14px;z-index:20;width:350px;max-width:calc(100% - 28px);max-height:calc(100vh - 80px);overflow:auto;display:flex;flex-direction:column;gap:18px;padding:18px 20px 20px;border-radius:16px;background:var(--surface);border:1px solid var(--line2);box-shadow:0 16px 48px rgba(0,0,0,.35)"
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
    <legend style="padding:0 0 10px;display:flex;align-items:center;gap:8px;font-size:15.5px;font-weight:700"><i class="ph ph-folder-simple" style="font-size:19px;color:var(--accent-text)"></i>{t("setOut")}</legend>
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

  <div style="height:1px;background:var(--line);flex-shrink:0"></div>
  <fieldset style="border:0;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
    <legend style="padding:0 0 10px;display:flex;align-items:center;gap:8px;font-size:15.5px;font-weight:700"><i class="ph ph-palette" style="font-size:19px;color:var(--accent-text)"></i>{t("setAppearance")}</legend>
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

  {#if !isMac}
    <div style="height:1px;background:var(--line);flex-shrink:0"></div>
    <fieldset style="border:0;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
      <legend style="padding:0 0 10px;display:flex;align-items:center;gap:8px;font-size:15.5px;font-weight:700"><i class="ph ph-scan" style="font-size:19px;color:var(--accent-text)"></i>{t("setOcr")}</legend>
      <div role="radiogroup" style="display:grid;grid-template-columns:repeat(2,1fr);padding:3px;border-radius:11px;background:var(--surface2);gap:3px">
        {#each ocrOpts as opt (opt.id)}
          <button
            role="radio"
            aria-checked={settings.ocrQuality === opt.id}
            onclick={() => setOcrQuality(opt.id)}
            style="min-height:36px;padding:4px 6px;border-radius:8px;border:0;font-size:14px;font-weight:600;cursor:pointer;line-height:1.2;background:{settings.ocrQuality === opt.id ? 'var(--surface)' : 'transparent'};color:{settings.ocrQuality === opt.id ? 'var(--text)' : 'var(--text2)'};box-shadow:{settings.ocrQuality === opt.id ? '0 1px 2px rgba(0,0,0,.12)' : 'none'}">{opt.label}</button
          >
        {/each}
      </div>
    </fieldset>
  {/if}

  <div style="height:1px;background:var(--line);flex-shrink:0"></div>
  <fieldset style="border:0;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
    <legend style="padding:0 0 10px;display:flex;align-items:center;gap:8px;font-size:15.5px;font-weight:700"><i class="ph ph-signature" style="font-size:19px;color:var(--accent-text)"></i>{t("setSigs")}</legend>
    <div style="display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap">
      <span style="font-size:14px;color:var(--text2)">{t("setSigsCount", { n: app.edit.sigs.length })} · {t("sigSavedNote")}</span>
      <button onclick={() => (app.sigManageOpen = true)} style="min-height:36px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;font-weight:600;cursor:pointer;white-space:nowrap">{t("setSigsManage")}</button>
    </div>
  </fieldset>
</div>
