<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import logo from "../../../src-tauri/icons/icon.svg";

  let closeBtn: HTMLButtonElement;
  const close = () => (app.aboutOpen = false);

  $effect(() => closeBtn?.focus());
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div onclick={close} style="position:fixed;inset:0;z-index:40;background:var(--scrim);display:grid;place-items:center;padding:16px">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="about-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    style="position:relative;width:320px;max-width:100%;padding:30px 24px 24px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:0 24px 60px rgba(0,0,0,.3);display:flex;flex-direction:column;align-items:center;text-align:center"
  >
    <button
      bind:this={closeBtn}
      onclick={close}
      aria-label={t("close")}
      style="position:absolute;top:10px;right:10px;width:34px;height:34px;border-radius:9px;border:0;background:none;color:var(--text2);cursor:pointer;display:grid;place-items:center"
    >
      <i class="ph ph-x" style="font-size:18px"></i>
    </button>
    <img src={logo} alt="" width="72" height="72" style="display:block" />
    <h2 id="about-title" style="margin:14px 0 2px;font-size:22px;font-weight:700;letter-spacing:-0.01em">
      <span style="color:var(--logo-gold)">maji</span><span style="color:var(--accent-text)">pdf</span>
    </h2>
    <div style="color:var(--text2);font-size:14.5px;font-variant-numeric:tabular-nums">{t("aboutVersion", { v: __APP_VERSION__ })}</div>
    <div style="margin-top:18px;line-height:1.5">
      <div style="font-weight:600">{t("aboutBuiltBy")}</div>
      <div style="color:var(--text2);font-size:14.5px">{t("aboutAi")}</div>
    </div>
    <div style="margin-top:18px;color:var(--text3);font-size:13.5px">© majimawrks 2026</div>
  </div>
</div>
