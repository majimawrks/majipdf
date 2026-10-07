<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked } from "../compress";
  import { startRun, cancelRun, processAnother, retry } from "../ocr";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";

  const run = app.ocr.run;
  const SHOWN = 6;

  const file = $derived(app.tool.files[0]);
  const locked = $derived(!!file && !file.damaged && isLocked(file));
  const pw = $derived(file ? (app.compress.passwords[file.path] ?? null) : null);
  const pages = $derived(file?.pages ?? 0);
  let signedOpen = $state(false);

  const p = $derived(run.progress);
  const percent = $derived(p?.stage === "reading" && p.total ? Math.min(99, Math.floor((p.done / p.total) * 100)) : 0);
  const status = $derived(
    p?.stage === "reading" ? t("ocrReading", { n: Math.min(p.done + 1, p.total), total: p.total }) : p?.stage === "saving" ? t("ocrSave") : t("ocrPrep")
  );

  function onrun() {
    if (file.signed) signedOpen = true;
    else void startRun();
  }

  async function chooseOther() {
    const p = await chooseFiles(false);
    if (p.length) await addFilesToTool(p);
  }
</script>

{#if app.tool.phase === "running"}
  <ProcessingCard name={run.name} {percent} {status} oncancel={cancelRun} />
{:else if app.tool.phase === "done" && run.result}
  <DoneCard
    subtitle={t("doneSubOcr")}
    output={run.result.output}
    meta={t("ocrDone", { n: fmtNum(run.result.ocr_pages), total: fmtNum(run.result.pages), s: fmtSize(run.result.size), sec: run.result.seconds })}
    icon="file-pdf"
    onanother={processAnother}
  />
{:else if app.tool.phase === "error" || file?.damaged}
  {#if file?.damaged}
    <ErrorCard file={file.name} details={t("damagedMeta")} onretry={processAnother} />
  {:else}
    <ErrorCard
      file={run.name}
      details={run.error}
      title={t("ocrErrTitle")}
      body={t("ocrErrBody", { f: run.name })}
      retryLabel={t("tryAgain")}
      onretry={retry}
    />
  {/if}
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={!!file && !locked && !!pages}
    runLabel={t("ocrRun")}
    {onrun}
    {main}
    {options}
  />
{/if}

{#if signedOpen}
  <SignedDialog
    names={[file.name]}
    oncancel={() => (signedOpen = false)}
    oncontinue={() => {
      signedOpen = false;
      void startRun();
    }}
  />
{/if}

{#snippet main()}
  {#if file}
    {#if run.noScan}
      <div role="note" style="display:flex;gap:12px;padding:14px 16px;border-radius:14px;background:var(--accent-soft);color:var(--accent-text)">
        <i class="ph ph-info" style="font-size:22px;flex-shrink:0;margin-top:1px"></i>
        <div style="font-size:15px">{t("ocrNoScan")}</div>
      </div>
    {/if}
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
        <div style="color:var(--text2)">{locked ? t("lockedMeta") : `${pages ? t("pagesN", { n: fmtNum(pages) }) : ""} · ${fmtSize(file.size_bytes)}`}</div>
      </div>
      <button
        onclick={chooseOther}
        style="display:inline-flex;align-items:center;gap:6px;min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;white-space:nowrap;flex-shrink:0"
        ><i class="ph ph-file-plus" style="font-size:16px"></i>{t("otherFile")}</button
      >
    </div>

    {#if locked}
      <PasswordForm {file} onskip={processAnother} />
    {:else if pages}
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:12px">
        {#each { length: Math.min(pages, SHOWN) } as _, i}
          <div style="padding:8px;border-radius:12px;border:1px solid var(--line);display:flex;flex-direction:column;align-items:center;gap:6px">
            <Thumb path={file.path} password={pw} page={i} width={120} height={156} />
            <span style="font-size:13px;color:var(--text2);font-variant-numeric:tabular-nums">{i + 1}</span>
          </div>
        {/each}
      </div>
      {#if pages > SHOWN}
        <div style="color:var(--text2)">{pages - SHOWN === 1 ? t("moreP1") : t("moreP", { n: fmtNum(pages - SHOWN) })}</div>
      {/if}
    {/if}
  {/if}
{/snippet}

{#snippet options()}
  <div style="color:var(--text2)">{t("ocrNote")}</div>
{/snippet}
