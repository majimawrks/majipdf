<script lang="ts">
  import { onMount } from "svelte";
  import { app, openTool } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked } from "../compress";
  import { openFor, applyEditor, undo, redo, startSave, keepEditing, chooseAnother, confirmLeave, cancelLeave, closeEditor } from "../edit";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";
  import EditPage from "./EditPage.svelte";

  const e = app.edit;
  const PX = 96 / 72; // px per pt at 100 %

  const file = $derived(app.tool.files[0]);
  const locked = $derived(!!file && !file.damaged && isLocked(file));
  const pw = $derived(file ? (app.compress.passwords[file.path] ?? null) : null);
  const ready = $derived(e.stage === "ready" && !!e.doc);
  const pages = $derived(e.doc?.pages ?? 0);
  let signedOpen = $state(false);

  // Open the session once the file is known and unlocked.
  $effect(() => {
    if (app.route === "edit" && app.tool.phase === "loaded" && file && !file.damaged && !locked && e.stage === "idle") void openFor(file);
  });

  // ---- zoom + current page ----
  let zoom = $state<number | "fit">("fit");
  let viewW = $state(0);
  let scroller = $state<HTMLDivElement>();
  let rail = $state<HTMLElement>();
  let cur = $state(0);
  const maxW = $derived(e.doc ? Math.max(...e.doc.sizes.map((s) => s[0])) : 595);
  const fitPct = $derived(Math.max(25, Math.min(200, Math.floor((Math.max(viewW - 48, 100) / maxW / PX) * 100))));
  const pct = $derived(zoom === "fit" ? fitPct : zoom);
  const scale = $derived((pct / 100) * PX);
  const step = (d: number) => (zoom = Math.max(50, Math.min(200, Math.round((pct + d) / 10) * 10)));

  let raf = 0;
  function onscroll() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => {
      if (!scroller) return;
      const y = scroller.getBoundingClientRect().top + 60;
      let c = 0;
      for (let i = 0; i < scroller.children.length; i++) {
        if (scroller.children[i].getBoundingClientRect().top <= y) c = i;
        else break;
      }
      cur = c;
    });
  }
  $effect(() => {
    rail?.querySelector(`[data-p="${cur}"]`)?.scrollIntoView({ block: "nearest" });
  });
  const goPage = (i: number) => scroller?.querySelector(`#ep-${i}`)?.scrollIntoView({ block: "start" });

  const scanNote = $derived(!!file?.scanned || Object.values(e.models).some((m) => m.scan));
  const canSave = $derived(ready && e.hist.count > 0 && !e.busy && !e.editing?.applying);

  function onrun() {
    if (file.signed) signedOpen = true;
    else void startSave();
  }

  async function chooseOther() {
    const p = await chooseFiles(false);
    if (p.length) await addFilesToTool(p);
  }

  // Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z when no editor (or other field) has focus.
  function onkeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape" && e.editing && !e.leaveAsk && !signedOpen) {
      ev.preventDefault();
      closeEditor(); // Esc cancels even when focus sits on a bar button
      return;
    }
    if (!ready || app.tool.phase !== "loaded" || e.editing || e.leaveAsk || signedOpen) return;
    const el = ev.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    if (!(ev.ctrlKey || ev.metaKey)) return;
    const k = ev.key.toLowerCase();
    if (k === "z" && !ev.shiftKey) (ev.preventDefault(), void undo());
    else if (k === "y" || (k === "z" && ev.shiftKey)) (ev.preventDefault(), void redo());
  }

  // A click anywhere outside the open editor applies it. While a warning bar shows, a stray click does nothing
  // (only the bar's buttons or Esc act), so a misclick can never drop the typed text.
  function onpointerdown(ev: PointerEvent) {
    if (e.editing && !e.editing.bar && !(ev.target as Element).closest?.("[data-editor]")) void applyEditor();
  }

  let leaveCancel: HTMLButtonElement | undefined = $state();
  $effect(() => {
    if (e.leaveAsk) leaveCancel?.focus();
  });

  const btn =
    "display:inline-flex;align-items:center;gap:6px;min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;white-space:nowrap;flex-shrink:0";
  const tbtn =
    "display:inline-grid;place-items:center;min-width:34px;height:34px;padding:0 8px;border-radius:8px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:14.5px;cursor:pointer;white-space:nowrap";
</script>

<svelte:window {onkeydown} onpointerdowncapture={onpointerdown} />

{#if app.tool.phase === "running"}
  <ProcessingCard name={e.run.name} percent={null} status="" />
{:else if app.tool.phase === "done" && e.run.result}
  <DoneCard
    subtitle={t("doneSubEdit")}
    output={e.run.result.output}
    meta={`${fmtSize(e.run.result.size)} · ${e.run.result.seconds} s`}
    icon="file-pdf"
    anotherLabel={t("editKeep")}
    onanother={keepEditing}
  />
{:else if app.tool.phase === "error"}
  <ErrorCard file={e.run.name} details={e.run.error} title={t("editErrTitle")} body={t("editErrBody", { f: e.run.name })} retryLabel={t("tryAgain")} onretry={keepEditing} />
{:else if file?.damaged}
  <ErrorCard file={file.name} details={t("damagedMeta")} onretry={chooseAnother} />
{:else if e.stage === "error"}
  <ErrorCard
    file={file?.name ?? ""}
    details={e.error}
    body={e.error === "busy" ? t("busy") : undefined}
    retryLabel={e.error === "busy" ? t("tryAgain") : undefined}
    onretry={e.error === "busy" ? () => (e.stage = "idle") : chooseAnother}
  />
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={canSave}
    {onrun}
    {main}
    {options}
    flush={ready}
    title={t("editEditing")}
  />
{/if}

{#if signedOpen}
  <SignedDialog
    names={[file.name]}
    oncancel={() => (signedOpen = false)}
    oncontinue={() => {
      signedOpen = false;
      void startSave();
    }}
  />
{/if}

{#if e.leaveAsk}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    style="position:fixed;inset:0;z-index:50;background:var(--scrim);display:grid;place-items:center;padding:20px"
    onkeydown={(ev) => ev.key === "Escape" && (ev.preventDefault(), cancelLeave())}
  >
    <div
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="leave-title"
      style="width:100%;max-width:440px;border-radius:20px;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);padding:24px 26px;display:flex;flex-direction:column;gap:18px"
    >
      <div id="leave-title" style="font-size:19px;font-weight:700">{t("editLeave")}</div>
      <div style="display:flex;gap:10px;justify-content:flex-end">
        <button
          bind:this={leaveCancel}
          onclick={cancelLeave}
          style="min-height:44px;padding:0 20px;border-radius:12px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:16px;cursor:pointer">{t("cancel")}</button
        >
        <button
          onclick={confirmLeave}
          style="min-height:44px;padding:0 22px;border-radius:12px;border:0;background:var(--accent);color:var(--accent-ink);font-size:16px;font-weight:600;cursor:pointer">{t("editLeaveYes")}</button
        >
      </div>
    </div>
  </div>
{/if}

{#snippet main()}
  {#if ready}
    <div style="flex:1;min-height:0;display:flex">
      <nav
        bind:this={rail}
        aria-label={t("pages")}
        style="width:132px;flex-shrink:0;overflow:auto;border-right:1px solid var(--line);padding:12px 0;display:flex;flex-direction:column;align-items:center;gap:10px"
      >
        {#each { length: pages } as _, i}
          <button
            data-p={i}
            onclick={() => goPage(i)}
            aria-label={t("pageAria", { n: i + 1 })}
            aria-current={cur === i}
            style="padding:6px;border-radius:10px;border:2px solid {cur === i ? 'var(--accent)' : 'transparent'};background:{cur === i ? 'var(--accent-soft)' : 'none'};display:flex;flex-direction:column;align-items:center;gap:4px;cursor:pointer;color:var(--text2)"
          >
            <Thumb path={file.path} password={pw} page={i} width={84} height={110} />
            <span style="font-size:13px;font-variant-numeric:tabular-nums">{i + 1}</span>
          </button>
        {/each}
      </nav>

      <div style="flex:1;min-width:0;display:flex;flex-direction:column">
        <div
          role="toolbar"
          aria-label={t("editName")}
          style="flex-shrink:0;display:flex;align-items:center;gap:6px;flex-wrap:wrap;padding:8px 12px;border-bottom:1px solid var(--line);background:var(--surface)"
        >
          <button style={tbtn} onclick={() => step(-10)} disabled={pct <= 50} aria-label={t("editZoomOut")} title={t("editZoomOut")}
            ><i class="ph ph-minus" style="font-size:16px"></i></button
          >
          <span style="min-width:48px;text-align:center;font-variant-numeric:tabular-nums;font-weight:600">{pct}%</span>
          <button style={tbtn} onclick={() => step(10)} disabled={pct >= 200} aria-label={t("editZoomIn")} title={t("editZoomIn")}
            ><i class="ph ph-plus" style="font-size:16px"></i></button
          >
          <button style={tbtn} onclick={() => (zoom = "fit")} aria-pressed={zoom === "fit"}>{t("editFit")}</button>
          <span style="flex:1"></span>
          <span style="color:var(--text2);font-size:14px;font-variant-numeric:tabular-nums">{t("pageAria", { n: cur + 1 })} / {fmtNum(pages)}</span>
        </div>
        <div
          bind:this={scroller}
          bind:clientWidth={viewW}
          {onscroll}
          style="flex:1;min-height:0;overflow:auto;background:var(--surface2);padding:20px 24px 40px;display:flex;flex-direction:column;gap:18px"
        >
          {#each { length: pages } as _, i (i)}
            <EditPage {i} {scale} />
          {/each}
        </div>
      </div>
    </div>
  {:else if file}
    <div style="display:flex;align-items:center;gap:14px">
      {#if locked}
        <span style="width:44px;height:56px;flex-shrink:0;border-radius:5px;display:grid;place-items:center;background:var(--warn-soft);color:var(--warn)"
          ><i class="ph ph-lock-simple" style="font-size:22px"></i></span
        >
      {:else}
        <Thumb path={file.path} password={pw} width={44} height={56} radius={4} />
      {/if}
      <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:2px">
        <h1 style="margin:0;font-size:17px;font-weight:600;overflow-wrap:anywhere">{file.name}</h1>
        <div style="color:var(--text2)">{locked ? t("lockedMeta") : fmtSize(file.size_bytes)}</div>
      </div>
      <button onclick={chooseOther} style={btn}><i class="ph ph-file-plus" style="font-size:16px"></i>{t("otherFile")}</button>
    </div>
    {#if locked}
      <PasswordForm {file} onskip={chooseAnother} />
    {:else if e.stage === "opening"}
      <div role="status" style="display:flex;align-items:center;gap:10px;color:var(--text2)">
        <span class="spin"></span>{t("editOpening")}
      </div>
    {/if}
  {/if}
{/snippet}

{#snippet options()}
  {#if ready}
    <div style="color:var(--text2)">{t("editHint")}</div>
    <div style="display:flex;flex-direction:column;gap:10px">
      <div aria-live="polite" style="font-weight:600;color:{e.hist.count ? 'var(--accent-text)' : 'var(--text2)'}">{t("editChanges", { n: fmtNum(e.hist.count) })}</div>
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <button style={btn} onclick={undo} disabled={!e.hist.can_undo || e.busy || !!e.editing}
          ><i class="ph ph-arrow-u-up-left" style="font-size:16px"></i>{t("editUndo")}</button
        >
        <button style={btn} onclick={redo} disabled={!e.hist.can_redo || e.busy || !!e.editing}
          ><i class="ph ph-arrow-u-up-right" style="font-size:16px"></i>{t("editRedo")}</button
        >
      </div>
    </div>
    {#if scanNote}
      <div role="note" style="display:flex;gap:10px;padding:12px 14px;border-radius:12px;background:var(--accent-soft);color:var(--accent-text);font-size:14.5px">
        <i class="ph ph-info" style="font-size:20px;flex-shrink:0;margin-top:1px"></i>
        <div style="display:flex;flex-direction:column;gap:6px;align-items:flex-start">
          {t("editScanNote")}
          <button
            onclick={() => openTool("ocr", file ? [file] : [])}
            style="padding:0;border:0;background:none;color:inherit;font:inherit;font-weight:700;text-decoration:underline;cursor:pointer;text-align:left">{t("ocrName")}</button
          >
        </div>
      </div>
    {/if}
  {/if}
{/snippet}

<style>
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed !important;
  }
  .spin {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 3px solid var(--surface2);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
