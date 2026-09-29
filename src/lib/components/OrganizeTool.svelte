<script lang="ts">
  import { untrack } from "svelte";
  import { flip } from "svelte/animate";
  import { app, type FileInfo, type OrgPage } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles, fileInfo } from "../tauri";
  import { isLocked } from "../compress";
  import {
    rotate, remove, moveBy, moveOne, rangeIds, changeCounts, initPages, undoAll, insertFile, usedSources,
    confirmReplace, startRun, cancelRun, processAnother,
  } from "../organize";
  import { addFilesToTool } from "../files";
  import { pointerReorder, FLIP_MS } from "../reorder.svelte";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";

  const o = app.organize;
  const run = o.run;

  const file = $derived(app.tool.files[0]);
  const locked = $derived(!!file && !file.damaged && isLocked(file));
  const pw = $derived(file ? (app.compress.passwords[file.path] ?? null) : null);
  const ready = $derived(!!file && !file.damaged && !locked && o.file === file.path);
  const counts = $derived(changeCounts(o.pages, o.total));
  const stem = $derived(file ? file.name.replace(/\.pdf$/i, "") : "");
  let signedOpen = $state(false);

  // New file (or newly unlocked one) -> fresh page list. Cancel and running keep it, since the path is unchanged.
  $effect(() => {
    if (file && !file.damaged && !locked && file.pages && o.file !== file.path) untrack(() => initPages(file, pw));
  });

  const signedNames = $derived(usedSources().filter((s) => s.signed).map((s) => s.name));
  function onrun() {
    if (signedNames.length) signedOpen = true;
    else void startRun();
  }

  async function chooseOther() {
    const p = await chooseFiles(false);
    if (p.length) await addFilesToTool(p); // replaceFile: asks first when there are unsaved edits
  }

  // ---- selection ----------------------------------------------------------
  const sel = $derived(o.selection);
  const noSel = $derived(sel.size === 0);

  function click(e: MouseEvent, p: OrgPage) {
    if (dragged) {
      dragged = false; // this click is the tail of a drag
      return;
    }
    const on = sel.has(p.id);
    if (e.shiftKey && o.anchor !== null) {
      o.selection = rangeIds(o.pages, o.anchor, p.id);
      return; // anchor stays where the range started
    }
    if (e.ctrlKey || e.metaKey || e.detail === 0) {
      // detail 0 = keyboard Space/Enter: toggles, so keyboard users can build a multi-selection
      const next = new Set(sel);
      if (on) next.delete(p.id);
      else next.add(p.id);
      o.selection = next;
    } else o.selection = on && sel.size === 1 ? new Set() : new Set([p.id]);
    o.anchor = p.id;
  }

  // ---- drag reorder -------------------------------------------------------
  // ponytail: dragging a tile that is selected together with others moves just that tile, not the whole selection
  // (the toolbar's Move earlier/later moves the whole selection). Upgrade if users ask for block drags.
  const drag = pointerReorder((from, to) => (o.pages = moveOne(o.pages, from, to)));
  let dragged = false;

  // The drag only starts once the pointer has moved more than 5px, so a plain click still selects.
  function down(e: PointerEvent, i: number) {
    if (e.button !== 0) return;
    dragged = false;
    const x0 = e.clientX;
    const y0 = e.clientY;
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
    const move = (ev: PointerEvent) => {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) <= 5) return;
      stop();
      dragged = true;
      drag.down(e, i);
      // replay this move for the reorder listeners that were just attached (a fast flick may be the only one)
      window.dispatchEvent(new PointerEvent("pointermove", { clientX: ev.clientX, clientY: ev.clientY }));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
  }

  // ---- toolbar ------------------------------------------------------------
  const tools = $derived([
    { icon: "arrow-counter-clockwise", label: t("rotL"), go: () => (o.pages = rotate(o.pages, sel, -90)) },
    { icon: "arrow-clockwise", label: t("rotR"), go: () => (o.pages = rotate(o.pages, sel, 90)) },
    { icon: "caret-left", label: t("moveEarlier"), go: () => (o.pages = moveBy(o.pages, sel, -1)) },
    { icon: "caret-right", label: t("moveLater"), go: () => (o.pages = moveBy(o.pages, sel, 1)) },
    {
      icon: "trash",
      label: t("del"),
      danger: true,
      go: () => {
        o.pages = remove(o.pages, sel);
        o.selection = new Set();
        o.anchor = null;
      },
    },
  ]);

  // ---- insert from PDF ----------------------------------------------------
  let ins = $state<FileInfo | null>(null); // locked or damaged file waiting in the inline panel

  async function pickInsert() {
    const p = await chooseFiles(false);
    if (!p.length) return;
    ins = null;
    const f = (await fileInfo(p)).find((x) => x.is_pdf);
    if (!f) {
      app.notice = "notPdf";
      return;
    }
    app.notice = "";
    if (f.damaged || f.encrypted) ins = f; // encrypted: always ask, the page count is only known after unlocking
    else insertFile(f, null);
  }

  function unlocked(f: FileInfo, r: { pages: number | null; signed: boolean }) {
    ins = null;
    insertFile({ ...f, pages: r.pages, signed: r.signed }, app.compress.passwords[f.path] ?? null);
  }

  // ---- progress -----------------------------------------------------------
  const p = $derived(run.progress);
  const percent = $derived(p && p.pages ? Math.min(99, Math.floor((p.page / p.pages) * 100)) : 0);
  const status = $derived(p ? t("compressing", { a: fmtNum(p.page), b: fmtNum(p.pages) }) : t("waiting"));

  const changeRows = $derived(
    [
      counts.rotated && { icon: "arrow-clockwise", text: t("rotated", { n: counts.rotated }) },
      counts.deleted && { icon: "trash", text: t("deleted", { n: counts.deleted }) },
      counts.added && { icon: "file-plus", text: t("orgAdded", { n: counts.added }) },
      counts.moved && { icon: "arrows-down-up", text: t("orgMoved") },
    ].filter((x): x is { icon: string; text: string } => !!x)
  );

  const btn =
    "display:inline-flex;align-items:center;gap:6px;min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;white-space:nowrap;flex-shrink:0";
</script>

{#if app.tool.phase === "running"}
  <ProcessingCard name={run.name} {percent} {status} oncancel={cancelRun} />
{:else if app.tool.phase === "done" && run.result}
  <DoneCard
    subtitle={t("doneSubOrg")}
    output={run.result.output}
    meta={t("orgDoneMeta", { n: fmtNum(run.result.pages), s: fmtSize(run.result.size) })}
    onanother={processAnother}
  />
{:else if app.tool.phase === "error" || file?.damaged}
  <ErrorCard file={file?.damaged ? file.name : run.name} details={run.error || t("damagedMeta")} onretry={processAnother} />
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={ready && o.pages.length > 0 && counts.any}
    runLabel={t("orgRun", { n: o.pages.length })}
    saveAs="{stem}_organized.pdf"
    {onrun}
    {main}
    {options}
  />
{/if}

{#if signedOpen}
  <SignedDialog
    names={signedNames}
    oncancel={() => (signedOpen = false)}
    oncontinue={() => {
      signedOpen = false;
      void startRun();
    }}
  />
{/if}

{#snippet main()}
  {#if file}
    <div style="display:flex;align-items:center;gap:14px">
      <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:4px">
        <h1 style="margin:0;font-size:17px;font-weight:600;overflow-wrap:anywhere">{file.name}</h1>
        <div style="color:var(--text2)">{locked ? t("lockedMeta") : t("orgHint")}</div>
      </div>
      <button onclick={chooseOther} style={btn}><i class="ph ph-file-plus" style="font-size:16px"></i>{t("otherFile")}</button>
    </div>

    {#if o.pendingFile}
      <div role="alert" style="padding:12px 14px;border-radius:12px;background:var(--warn-soft);display:flex;align-items:center;gap:10px;flex-wrap:wrap">
        <span style="flex:1;min-width:200px;font-weight:600">{t("orgReplaceQ")}</span>
        <button onclick={() => (o.pendingFile = null)} style={btn}>{t("orgReplaceNo")}</button>
        <button
          onclick={confirmReplace}
          style="min-height:40px;padding:0 16px;border-radius:10px;border:0;background:var(--accent);color:var(--accent-ink);font-size:15px;font-weight:600;cursor:pointer">{t("orgReplaceYes")}</button
        >
      </div>
    {/if}

    {#if locked}
      <PasswordForm {file} onskip={processAnother} />
    {:else if ready}
      <div
        role="toolbar"
        aria-label={t("organizeName")}
        style="position:sticky;top:-22px;z-index:3;display:flex;align-items:center;gap:6px;flex-wrap:wrap;padding:8px;border-radius:14px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow)"
      >
        <span aria-live="polite" style="padding:0 8px;font-weight:600;min-width:96px;color:{noSel ? 'var(--text2)' : 'var(--accent-text)'}">
          {noSel ? t("selNone") : t("selN", { n: sel.size })}
        </span>
        {#each tools as b}
          <button
            onclick={b.go}
            disabled={noSel}
            title={b.label}
            class="tb"
            style="display:inline-flex;align-items:center;gap:6px;min-height:38px;padding:0 11px;border-radius:9px;border:0;background:none;color:{noSel ? 'var(--text3)' : b.danger ? 'var(--err)' : 'var(--text)'};font-size:14.5px;cursor:{noSel ? 'default' : 'pointer'};white-space:nowrap"
            ><i class="ph ph-{b.icon}" style="font-size:18px"></i>{b.label}</button
          >
        {/each}
        <div style="flex:1"></div>
        <button
          onclick={pickInsert}
          style="display:inline-flex;align-items:center;gap:6px;min-height:38px;padding:0 12px;border-radius:9px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:14.5px;cursor:pointer;white-space:nowrap"
          ><i class="ph ph-file-plus" style="font-size:18px"></i>{t("insert")}</button
        >
      </div>

      {#if ins}
        <div style="display:flex;flex-direction:column;gap:8px">
          <div style="font-weight:600;overflow-wrap:anywhere">{ins.name}</div>
          {#if ins.damaged}
            <div role="alert" style="display:flex;align-items:center;gap:10px;color:var(--err)">
              {t("damagedMeta")}
              <button onclick={() => (ins = null)} style="border:0;background:none;color:var(--text2);font-size:15px;cursor:pointer;min-height:36px">{t("close")}</button>
            </div>
          {:else}
            <div style="margin-left:-46px">
              <PasswordForm file={ins} onskip={() => (ins = null)} onunlocked={(r) => unlocked(ins!, r)} />
            </div>
          {/if}
        </div>
      {/if}

      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(118px,1fr));gap:12px;user-select:none">
        {#each o.pages as pg, i (pg.id)}
          {@const on = sel.has(pg.id)}
          {@const lifted = drag.from === i}
          {@const s = o.sources[pg.src]}
          <div animate:flip={{ duration: FLIP_MS }} style="position:relative;z-index:{lifted ? 2 : 'auto'}">
            <button
              data-reorder-idx={i}
              onpointerdown={(e) => down(e, i)}
              ondragstart={(e) => e.preventDefault()}
              onclick={(e) => click(e, pg)}
              aria-pressed={on}
              aria-label={t("pageAria", { n: i + 1 })}
              style="position:relative;width:100%;display:flex;flex-direction:column;gap:6px;padding:8px 8px 6px;border-radius:12px;border:2px solid {on || lifted ? 'var(--accent)' : 'transparent'};background:{on ? 'var(--accent-soft)' : 'var(--surface)'};color:var(--text);cursor:{drag.from === null ? 'pointer' : 'grabbing'};box-shadow:{lifted ? '0 10px 28px rgba(0,0,0,.22)' : 'none'};transform:scale({lifted ? 1.04 : 1});transition:transform 120ms,box-shadow 120ms"
            >
              <span style="width:100%;aspect-ratio:1;display:grid;place-items:center">
                <span style="width:74%;aspect-ratio:3/4;display:block;transform:rotate({pg.rotation}deg);transition:transform .2s">
                  <Thumb path={s.path} password={s.password} page={pg.page} width={90} height={120} radius={4} fill />
                </span>
              </span>
              <span style="display:flex;justify-content:space-between;align-items:center;min-height:22px;gap:4px">
                <span style="font-size:13.5px;font-weight:600;color:var(--text2);font-variant-numeric:tabular-nums">{i + 1}</span>
                {#if pg.isNew}
                  <span style="font-size:12px;font-weight:700;padding:1px 6px;border-radius:5px;background:var(--accent-soft);color:var(--accent-text)">{t("newPage")}</span>
                {/if}
              </span>
              {#if on}
                <span style="position:absolute;top:6px;right:6px;width:22px;height:22px;border-radius:50%;background:var(--accent);color:var(--accent-ink);display:grid;place-items:center">
                  <i class="ph ph-check" style="font-size:13px"></i>
                </span>
              {/if}
            </button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
{/snippet}

{#snippet options()}
  {#if ready}
    <div style="display:flex;flex-direction:column;gap:10px">
      <div style="font-weight:600">{t("orgChanges")}</div>
      {#if !changeRows.length}
        <div style="color:var(--text2)">{t("noChanges")}</div>
      {/if}
      {#each changeRows as c}
        <div style="display:flex;align-items:center;gap:10px">
          <i class="ph ph-{c.icon}" style="font-size:18px;color:var(--accent-text)"></i><span>{c.text}</span>
        </div>
      {/each}
      {#if counts.any}
        <button
          onclick={undoAll}
          class="undo"
          style="align-self:flex-start;min-height:38px;padding:0 12px;margin-left:-12px;border-radius:9px;border:0;background:none;color:var(--accent-text);font-size:15px;font-weight:600;cursor:pointer">{t("reset")}</button
        >
      {/if}
    </div>
  {/if}
{/snippet}

<style>
  .tb:not(:disabled):hover {
    background: var(--surface2);
  }
  .undo:hover {
    background: var(--accent-soft);
  }
</style>
