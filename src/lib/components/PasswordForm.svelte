<script lang="ts">
  import { app, type FileInfo } from "../state.svelte";
  import { t } from "../i18n";
  import { unlock } from "../tauri";

  let { file, onskip, onunlocked }: { file: FileInfo; onskip?: () => void; onunlocked?: (r: { pages: number | null; signed: boolean; scanned: boolean }) => void } = $props();
  let pw = $state("");
  let wrong = $state(false);
  let busy = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    if (!pw || busy) return;
    busy = true;
    wrong = false;
    try {
      const r = await unlock(file.path, pw);
      if (r.ok) {
        app.compress.passwords[file.path] = pw;
        const i = app.tool.files.findIndex((f) => f.path === file.path);
        if (i >= 0) app.tool.files[i] = { ...app.tool.files[i], pages: r.pages, signed: r.signed, scanned: r.scanned };
        onunlocked?.(r);
      } else wrong = true;
    } catch {
      wrong = true;
    } finally {
      busy = false;
    }
  }
</script>

<form
  onsubmit={submit}
  style="margin:0 0 12px 46px;padding:12px 14px;border-radius:12px;background:var(--warn-soft);display:flex;flex-direction:column;gap:8px"
>
  <label for="pw-{file.path}" style="display:flex;align-items:center;gap:6px;font-weight:600;color:var(--text)">
    <i class="ph ph-lock-simple" style="font-size:16px"></i>{t("locked")}
  </label>
  <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap">
    <input
      id="pw-{file.path}"
      type="password"
      bind:value={pw}
      placeholder={t("pwPlaceholder")}
      aria-invalid={wrong}
      autocomplete="off"
      style="flex:1;min-width:160px;height:44px;border-radius:10px;border:1px solid {wrong ? 'var(--err)' : 'var(--line2)'};background:var(--surface);color:var(--text);padding:0 12px;font-size:16px"
    />
    <button
      type="submit"
      disabled={!pw || busy}
      style="min-height:44px;padding:0 20px;border-radius:12px;border:0;background:{pw ? 'var(--accent)' : 'var(--surface2)'};color:{pw ? 'var(--accent-ink)' : 'var(--text3)'};font-size:16px;font-weight:600;cursor:{pw ? 'pointer' : 'not-allowed'}"
      >{t("unlock")}</button
    >
    <button
      type="button"
      onclick={() => (onskip ? onskip() : app.compress.skipped.push(file.path))}
      style="min-height:44px;padding:0 10px;border:0;background:none;color:var(--text2);font-size:15.5px;cursor:pointer">{t("skip")}</button
    >
  </div>
  {#if wrong}
    <div role="alert" style="color:var(--err);font-size:14.5px">{t("pwWrong")}</div>
  {/if}
</form>
