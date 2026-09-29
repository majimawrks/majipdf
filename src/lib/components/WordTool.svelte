<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked } from "../compress";
  import { estimateSec, startRun, cancelRun, processAnother, retry } from "../word";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";

  const run = app.word.run;
  const SHOWN = 6;

  const file = $derived(app.tool.files[0]);
  const locked = $derived(!!file && !file.damaged && isLocked(file));
  const pw = $derived(file ? (app.compress.passwords[file.path] ?? null) : null);
  const pages = $derived(file?.pages ?? 0);
  const scanned = $derived(!!file?.scanned);

  // 1 Hz tick drives the estimate while running.
  let now = $state(Date.now());
  onMount(() => {
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });

  const est = $derived(estimateSec(run.pages));
  const elapsed = $derived(Math.max(0, (now - run.startedAt) / 1000));
  const percent = $derived(Math.floor(Math.min(95, (elapsed / est) * 100)));
  const stageKey = $derived(
    run.progress?.stage === "converting" ? "wordStageConv" : run.progress?.stage === "saving" ? "wordStageSave" : "wordStagePrep"
  );
  const status = $derived.by(() => {
    const st = t(stageKey);
    const left = est - elapsed;
    if (left <= 0) return t("wordAlmost", { st });
    return left >= 90 ? t("wordLeftMin", { st, m: Math.ceil(left / 60) }) : t("wordLeftSec", { st, s: Math.ceil(left) });
  });

  async function chooseOther() {
    const p = await chooseFiles(false);
    if (p.length) await addFilesToTool(p);
  }
</script>

{#if app.tool.phase === "running"}
  <ProcessingCard name={run.name} {percent} {status} oncancel={cancelRun} extra={t("wordBgNote")} />
{:else if app.tool.phase === "done" && run.result}
  <DoneCard
    subtitle={t("doneSubWord")}
    output={run.result.output}
    meta={t("wordDoneTook", { s: fmtSize(run.result.size), n: run.result.seconds })}
    icon="file-doc"
    onanother={processAnother}
  />
{:else if app.tool.phase === "error" || file?.damaged}
  {#if file?.damaged}
    <ErrorCard file={file.name} details={t("damagedMeta")} onretry={processAnother} />
  {:else}
    <ErrorCard
      file={run.name}
      details={run.error}
      title={t("wordErrTitle")}
      body={t("wordErrBody", { f: run.name })}
      retryLabel={t("tryAgain")}
      onretry={retry}
    />
  {/if}
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={!!file && !locked && !!pages}
    runLabel={scanned ? t("convertAnyway") : t("wordRun")}
    onrun={() => void startRun()}
    {main}
    {options}
  />
{/if}

{#snippet main()}
  {#if file}
    {#if scanned && !locked}
      <div role="note" style="display:flex;gap:12px;padding:14px 16px;border-radius:14px;background:var(--warn-soft);color:var(--warn)">
        <i class="ph ph-warning" style="font-size:22px;flex-shrink:0;margin-top:1px"></i>
        <div style="display:flex;flex-direction:column;gap:2px">
          <div style="font-weight:700">{t("scannedTitle")}</div>
          <div style="font-size:14px">{t("scannedWord")}</div>
        </div>
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
  <div style="color:var(--text2)">{t("wordNote")}</div>
{/snippet}
