<script lang="ts">
  import { t } from "../i18n";
  import { copyText } from "../tauri";

  let { file, details, onretry, title, body, retryLabel }: { file: string; details: string; onretry: () => void; title?: string; body?: string; retryLabel?: string } = $props();
  let copied = $state(false);

  async function copy() {
    copied = await copyText(details);
  }
</script>

<div style="flex:1;min-height:0;overflow:auto;display:flex;padding:28px 32px">
  <div
    role="alert"
    style="margin:auto;width:100%;max-width:600px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:26px 28px;display:flex;flex-direction:column;gap:16px"
  >
    <span style="width:52px;height:52px;border-radius:16px;background:var(--err-soft);color:var(--err);display:grid;place-items:center">
      <i class="ph ph-file-x" style="font-size:28px"></i>
    </span>
    <div style="display:flex;flex-direction:column;gap:4px">
      <div style="font-size:24px;font-weight:700">{title ?? t("errTitle")}</div>
      <div style="color:var(--text2);overflow-wrap:anywhere">{body ?? t("errBody", { f: file })}</div>
    </div>
    {#if !body}
    <div style="border-radius:14px;background:var(--surface2);padding:16px 18px;display:flex;flex-direction:column;gap:8px">
      <div style="font-weight:700">{t("errNext")}</div>
      <ol style="margin:0;padding-left:20px;color:var(--text2);display:flex;flex-direction:column;gap:6px">
        <li>{t("errTry1")}</li>
        <li>{t("errTry2")}</li>
      </ol>
    </div>
    {/if}
    <div style="display:flex;gap:10px;flex-wrap:wrap">
      <button
        onclick={onretry}
        style="min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer">{retryLabel ?? t("tryAnother")}</button
      >
      <button
        onclick={copy}
        style="display:inline-flex;align-items:center;gap:8px;min-height:44px;padding:0 18px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer"
      >
        <i class="ph ph-{copied ? 'check' : 'copy'}" style="font-size:18px"></i>{copied ? t("copied") : t("copyDetails")}
      </button>
    </div>
  </div>
</div>
