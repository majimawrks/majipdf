<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { chooseImage } from "../tauri";
  import { sigImport, sigRename, sigDelete, refreshSigs, pickSig, type SigItem } from "../edit";

  // Signature / stamp library. position:fixed under the toolbar button so the scroller never clips it.
  let { anchor }: { anchor: HTMLElement } = $props();
  const e = app.edit;

  let box = $state<HTMLDivElement>();
  let pos = $state({ left: 8, top: 8, maxH: 400 });
  let pendingPath = $state<string | null>(null); // image chosen, waiting for Signature / Stamp
  let renaming = $state<string | null>(null);
  let renameText = $state("");
  let askDel = $state<SigItem | null>(null);
  let busy = $state(false);
  let err = $state("");

  function place() {
    const r = anchor.getBoundingClientRect();
    const w = Math.min(380, innerWidth - 16);
    const left = Math.max(8, Math.min(r.left, innerWidth - w - 8));
    pos = { left, top: r.bottom + 6, maxH: Math.max(160, innerHeight - r.bottom - 14) };
  }
  $effect(() => {
    place();
    addEventListener("resize", place);
    const sc = anchor.closest("[role=toolbar]")?.parentElement;
    sc?.addEventListener("scroll", place, { passive: true });
    return () => {
      removeEventListener("resize", place);
      sc?.removeEventListener("scroll", place);
    };
  });

  // Outside click closes (the tool button toggles it itself).
  function onpointerdown(ev: PointerEvent) {
    const tg = ev.target as Node;
    if (box && !box.contains(tg) && !anchor.contains(tg)) e.sigOpen = false;
  }

  async function run(fn: () => Promise<void>) {
    busy = true;
    err = "";
    try {
      await fn();
    } catch (x) {
      err = String(x);
    } finally {
      busy = false;
    }
  }

  const importFile = () =>
    run(async () => {
      const p = await chooseImage();
      if (p) pendingPath = p;
    });
  const importAs = (kind: SigItem["kind"]) =>
    run(async () => {
      const it = await sigImport(pendingPath!, kind);
      pendingPath = null;
      await refreshSigs();
      pickSig(it);
    });
  const commitRename = (it: SigItem) =>
    run(async () => {
      const n = renameText.trim();
      renaming = null;
      if (n && n !== it.name) {
        await sigRename(it.id, n);
        await refreshSigs();
      }
    });
  const del = (it: SigItem) =>
    run(async () => {
      askDel = null;
      await sigDelete(it.id);
      if (e.armed?.id === it.id) e.armed = null;
      await refreshSigs();
    });
  const draw = () => {
    e.sigOpen = false;
    e.drawOpen = true;
  };
  const focus = (n: HTMLInputElement) => (n.focus(), n.select());

  const btn = "min-height:38px;padding:0 12px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;display:inline-flex;align-items:center;gap:6px;white-space:nowrap";
  const btnP = "min-height:38px;padding:0 14px;border-radius:10px;border:0;background:var(--accent);color:var(--accent-ink);font-size:15px;font-weight:600;cursor:pointer;white-space:nowrap";
</script>

<svelte:window {onpointerdown} />

<div
  bind:this={box}
  role="dialog"
  aria-label={t("editToolSig")}
  style="position:fixed;z-index:40;left:{pos.left}px;top:{pos.top}px;width:{Math.min(380, innerWidth - 16)}px;max-height:{pos.maxH}px;box-sizing:border-box;overflow:auto;border-radius:16px;background:var(--surface);border:1px solid var(--line);box-shadow:0 12px 40px rgba(0,0,0,.18);padding:14px;display:flex;flex-direction:column;gap:12px"
>
  <div style="display:flex;gap:8px;flex-wrap:wrap">
    <button style={btn} onclick={importFile} disabled={busy}><i class="ph ph-image" style="font-size:17px"></i>{t("sigImport")}</button>
    <button style={btn} onclick={draw} disabled={busy}><i class="ph ph-pencil-simple-line" style="font-size:17px"></i>{t("sigDraw")}</button>
  </div>

  {#if pendingPath}
    <div role="group" aria-label={t("sigKindAsk")} style="display:flex;flex-direction:column;gap:8px;padding:10px 12px;border-radius:12px;background:var(--accent-soft)">
      <div style="font-weight:600">{t("sigKindAsk")}</div>
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <button style={btnP} onclick={() => importAs("signature")} disabled={busy}>{t("sigKindSig")}</button>
        <button style={btn} onclick={() => importAs("stamp")} disabled={busy}>{t("sigKindStamp")}</button>
        <button style={btn} onclick={() => (pendingPath = null)} disabled={busy}>{t("cancel")}</button>
      </div>
    </div>
  {/if}

  {#if askDel}
    <div role="alertdialog" aria-label={t("sigDelete")} style="display:flex;flex-direction:column;gap:8px;padding:10px 12px;border-radius:12px;background:var(--warn-soft)">
      <div style="overflow-wrap:anywhere">{t("sigDeleteAsk", { n: askDel.name })}</div>
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <button style={btnP} onclick={() => del(askDel!)} disabled={busy}>{t("sigDelete")}</button>
        <button style={btn} onclick={() => (askDel = null)}>{t("cancel")}</button>
      </div>
    </div>
  {/if}

  {#if err}<div role="alert" style="padding:8px 10px;border-radius:10px;background:var(--err-soft);font-size:14px;overflow-wrap:anywhere">{err}</div>{/if}

  {#if e.sigs.length}
    <ul style="list-style:none;margin:0;padding:0;display:grid;grid-template-columns:repeat(auto-fill,minmax(104px,1fr));gap:8px">
      {#each e.sigs as it (it.id)}
        <li class="it" style="position:relative;border-radius:12px;border:2px solid {e.armed?.id === it.id ? 'var(--accent)' : 'var(--line)'};background:var(--surface)">
          {#if renaming === it.id}
            <div style="padding:8px">
              <input
                use:focus
                bind:value={renameText}
                maxlength="60"
                aria-label={t("sigRename")}
                onkeydown={(ev) => {
                  if (ev.key === "Enter") (ev.preventDefault(), void commitRename(it));
                  else if (ev.key === "Escape") (ev.preventDefault(), ev.stopPropagation(), (renaming = null));
                }}
                onblur={() => renaming === it.id && void commitRename(it)}
                style="width:100%;box-sizing:border-box;height:36px;border-radius:8px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font:inherit;padding:0 8px"
              />
            </div>
          {:else}
            <button
              onclick={() => pickSig(it)}
              style="display:flex;flex-direction:column;gap:4px;width:100%;padding:8px;border:0;background:none;color:var(--text);font:inherit;cursor:pointer;text-align:left"
            >
              <span style="display:grid;place-items:center;height:56px;border-radius:8px;background:var(--page);overflow:hidden"
                ><img src={it.data_url} alt="" draggable="false" style="max-width:100%;max-height:56px;object-fit:contain" /></span
              >
              <span style="font-size:13.5px;line-height:1.25;overflow-wrap:anywhere">{it.name}</span>
              <span style="font-size:12.5px;color:var(--text3)">{it.kind === "stamp" ? t("sigKindStamp") : t("sigKindSig")}</span>
            </button>
            <div class="act" style="position:absolute;top:4px;right:4px;display:flex;gap:2px">
              <button
                aria-label="{t('sigRename')}: {it.name}"
                title={t("sigRename")}
                onclick={() => ((renameText = it.name), (renaming = it.id))}
                style="width:28px;height:28px;border-radius:8px;border:1px solid var(--line2);background:var(--surface);color:var(--text2);cursor:pointer;display:grid;place-items:center"
                ><i class="ph ph-pencil-simple" style="font-size:15px"></i></button
              >
              <button
                aria-label="{t('sigDelete')}: {it.name}"
                title={t("sigDelete")}
                onclick={() => (askDel = it)}
                style="width:28px;height:28px;border-radius:8px;border:1px solid var(--line2);background:var(--surface);color:var(--err);cursor:pointer;display:grid;place-items:center"
                ><i class="ph ph-trash" style="font-size:15px"></i></button
              >
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <div style="color:var(--text2)">{t("sigEmpty")}</div>
  {/if}

  <div style="display:flex;align-items:center;gap:6px;color:var(--text3);font-size:13.5px"><i class="ph ph-lock-simple" style="font-size:15px"></i>{t("sigSavedNote")}</div>
</div>

<style>
  .act {
    opacity: 0;
  }
  .it:hover .act,
  .it:focus-within .act {
    opacity: 1;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed !important;
  }
</style>
