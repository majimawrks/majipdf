<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked } from "../compress";
  import { estimateSec, startRun, tryOtherMethod, cancelRun, processAnother, retry } from "../excel";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";

  const run = app.excel.run;
  const x = app.excel;
  const SHEETS = [
    { id: "per_page", label: "xlsSheetsPage", hint: "xlsSheetsPageHint" },
    { id: "one", label: "xlsSheetsOne", hint: "xlsSheetsOneHint" },
  ] as const;
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
  const own = $derived(run.engine === "own");
  const percent2 = $derived(run.progress && run.progress.pages ? Math.floor((run.progress.page / run.progress.pages) * 100) : 0);
  const status = $derived.by(() => {
    if (own) return run.progress?.pages ? t("xlsPageOf", { a: run.progress.page, b: run.progress.pages }) : t("wordStagePrep");
    const st = t("xlsStageExcel");
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
  <ProcessingCard name={run.name} percent={own ? percent2 : percent} {status} oncancel={cancelRun} extra={own ? undefined : t("xlsBgNote")} />
{:else if app.tool.phase === "done" && run.result}
  {@const res = run.v2 ?? run.result}
  <DoneCard
    subtitle={t("doneSubExcel")}
    output={res.output}
    meta={t("xlsDoneMeta", { n: fmtNum(res.pages), s: fmtSize(res.size) })}
    icon="file-xls"
    onanother={processAnother}
    extra={res.empty_pages > 0 ? emptyNote : undefined}
    after={!run.v2 && app.office.excel ? other : undefined}
  />
{:else if app.tool.phase === "error" || file?.damaged}
  {#if file?.damaged}
    <ErrorCard file={file.name} details={t("damagedMeta")} onretry={processAnother} />
  {:else}
    {@const missing = run.error === "excel_missing"}
    <ErrorCard
      file={run.name}
      details={run.error}
      title={missing ? t("xlsMissingTitle") : t("xlsErrTitle")}
      body={missing ? t("xlsMissingBody") : t("xlsErrBody", { f: run.name })}
      retryLabel={t("tryAgain")}
      onretry={retry}
    />
  {/if}
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={!!file && !locked && !!pages}
    runLabel={t("xlsRun")}
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
          <div style="font-size:14px">{t("scannedExcel")}</div>
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
  <div style="display:flex;flex-direction:column;gap:8px">
    <div style="font-weight:600">{t("sheetLabel")}</div>
    <div role="radiogroup" aria-label={t("sheetLabel")} style="display:flex;flex-direction:column;gap:8px">
      {#each SHEETS as p}
        {@const sel = x.sheetMode === p.id}
        <button
          role="radio"
          aria-checked={sel}
          onclick={() => (x.sheetMode = p.id)}
          style="display:flex;gap:10px;text-align:left;padding:{sel ? '11px 13px' : '12px 14px'};border-radius:12px;border:{sel ? '2px solid var(--accent)' : '1px solid var(--line2)'};background:{sel ? 'var(--accent-soft)' : 'var(--surface)'};color:var(--text);cursor:pointer"
        >
          <span style="width:18px;height:18px;flex-shrink:0;margin-top:2px;border-radius:50%;border:2px solid {sel ? 'var(--accent)' : 'var(--line2)'};display:grid;place-items:center">
            {#if sel}<span style="width:8px;height:8px;border-radius:50%;background:var(--accent)"></span>{/if}
          </span>
          <span style="display:flex;flex-direction:column;gap:2px">
            <span style="font-weight:600">{t(p.label)}</span>
            <span style="font-size:14px;color:var(--text2)">{t(p.hint)}</span>
          </span>
        </button>
      {/each}
    </div>
  </div>

  <div style="display:flex;flex-direction:column;gap:6px">
    <button
      role="switch"
      aria-checked={x.numbers}
      onclick={() => (x.numbers = !x.numbers)}
      style="display:flex;align-items:center;justify-content:space-between;gap:12px;padding:0;border:0;background:none;text-align:left;color:var(--text);cursor:pointer"
    >
      <span style="font-weight:600">{t("numbers")}</span>
      <span style="position:relative;width:42px;height:24px;flex-shrink:0;border-radius:12px;background:{x.numbers ? 'var(--accent)' : 'var(--line2)'}">
        <span style="position:absolute;top:3px;left:{x.numbers ? 21 : 3}px;width:18px;height:18px;border-radius:50%;background:#fff;transition:left .15s"></span>
      </span>
    </button>
    <div style="font-size:14px;color:{x.numbers ? 'var(--warn)' : 'var(--text2)'}">{x.numbers ? t("xlsNumbersOn") : t("xlsNumbersOff")}</div>
  </div>
{/snippet}

{#snippet emptyNote()}
  <div role="note" style="display:flex;gap:10px;padding:12px 14px;border-radius:12px;background:var(--warn-soft);color:var(--warn);font-size:14px">
    <i class="ph ph-warning" style="font-size:18px;flex-shrink:0"></i>{t("xlsEmptyPages", { n: run.v2 ?? run.result ? (run.v2 ?? run.result)!.empty_pages : 0 })}
  </div>
{/snippet}

{#snippet other()}
  <div style="display:flex;flex-direction:column;gap:2px;align-items:flex-start">
    <button
      onclick={() => void tryOtherMethod()}
      style="padding:0;border:0;background:none;color:var(--accent-text);font-size:15px;font-weight:600;cursor:pointer;text-decoration:underline"
      >{t("xlsOther")}</button
    >
    <span style="font-size:13.5px;color:var(--text2)">{t("xlsOtherHint")}</span>
  </div>
{/snippet}
