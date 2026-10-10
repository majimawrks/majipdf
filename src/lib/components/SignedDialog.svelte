<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n";

  let { names, oncancel, oncontinue }: { names: string[]; oncancel: () => void; oncontinue: () => void } = $props();

  let card: HTMLDivElement;
  let cancelBtn: HTMLButtonElement;

  onMount(() => {
    const prev = document.activeElement as HTMLElement | null;
    cancelBtn.focus();
    return () => prev?.focus();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    } else if (e.key === "Tab") {
      // focus trap: cycle inside the two buttons
      const b = [...card.querySelectorAll<HTMLElement>("button")];
      const i = b.indexOf(document.activeElement as HTMLElement);
      e.preventDefault();
      b[(i + (e.shiftKey ? -1 : 1) + b.length) % b.length].focus();
    }
  }
</script>

<svelte:window {onkeydown} />

<div style="position:fixed;inset:0;z-index:50;background:var(--scrim);display:grid;place-items:center;padding:20px">
  <div
    bind:this={card}
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="signed-title"
    aria-describedby="signed-body"
    style="width:100%;max-width:480px;max-height:100%;overflow:auto;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:24px 26px;display:flex;flex-direction:column;gap:14px"
  >
    <span style="width:48px;height:48px;border-radius:14px;background:var(--warn-soft);color:var(--warn);display:grid;place-items:center">
      <i class="ph ph-seal-warning" style="font-size:26px"></i>
    </span>
    <div id="signed-title" style="font-size:21px;font-weight:700">{t("signedTitle")}</div>
    <div style="display:flex;flex-direction:column;gap:6px;max-height:120px;overflow:auto">
      {#each names as n}
        <span style="display:flex;align-items:center;gap:8px;padding:8px 12px;border-radius:10px;background:var(--surface2);font-weight:600;overflow-wrap:anywhere">
          <i class="ph ph-seal-check" style="font-size:18px;color:var(--text2);flex-shrink:0"></i>{n}
        </span>
      {/each}
    </div>
    <div id="signed-body" style="color:var(--text2);font-size:15.5px">{t("signedBody")}</div>
    <div style="font-weight:600">{t("signedAsk")}</div>
    <div style="display:flex;gap:10px;justify-content:flex-end">
      <button
        bind:this={cancelBtn}
        onclick={oncancel}
        style="min-height:44px;padding:0 20px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer">{t("cancel")}</button
      >
      <button
        onclick={oncontinue}
        style="min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent-fill);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer">{t("continue")}</button
      >
    </div>
  </div>
</div>
