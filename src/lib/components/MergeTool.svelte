<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtPages, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked, runnable, outName, nameValid, nameError, startRun, cancelRun, move, processAnother } from "../merge";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import DoneCard from "./DoneCard.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";
  import { pointerReorder, FLIP_MS } from "../reorder.svelte";
  import { flip } from "svelte/animate";

  const m = app.merge;
  const run = m.run;

  const runFiles = $derived(runnable());
  const signedNames = $derived(runFiles.filter((f) => f.signed).map((f) => f.name));
  const totalPages = $derived(app.tool.files.reduce((s, f) => s + (f.damaged ? 0 : (f.pages ?? 0)), 0));
  const lockedCount = $derived(app.tool.files.filter((f) => !f.damaged && isLocked(f)).length);
  const lockedNote = $derived(
    lockedCount === 0 ? "" : lockedCount === 1 ? t("mergeLockedSkip1") : t("mergeLockedSkip", { n: lockedCount })
  );
  const name = $derived(outName());
  const nameBad = $derived(!nameValid());
  const nameErrKey = $derived(nameError() || "nameErr");
  let signedOpen = $state(false);

  function onrun() {
    if (signedNames.length) signedOpen = true;
    else void startRun();
  }

  async function addFiles() {
    const p = await chooseFiles(true);
    if (p.length) await addFilesToTool(p); // appends and de-duplicates by path
  }
  function remove(path: string) {
    app.tool.files = app.tool.files.filter((f) => f.path !== path);
    if (!app.tool.files.length) processAnother();
  }

  const drag = pointerReorder((from, to) => move(from, to));

  // progress: merging fills the first half when compressing follows, compressing the second.
  const p = $derived(run.progress);
  const frac = $derived.by(() => {
    if (!p || !p.pages) return 0;
    const f = p.page / p.pages;
    if (!run.compress) return f;
    return p.stage === "merging" ? f / 2 : 0.5 + f / 2;
  });
  const percent = $derived(Math.min(99, Math.floor(frac * 100)));
  const status = $derived(
    !p
      ? t("waiting")
      : p.stage === "compressing"
        ? t("mergeCompressing", { a: fmtNum(p.page), b: fmtNum(p.pages) })
        : t("compressing", { a: fmtNum(p.page), b: fmtNum(p.pages) })
  );

  const headBtn =
    "display:inline-flex;align-items:center;gap:6px;min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;white-space:nowrap;flex-shrink:0";
  const iconBtn =
    "width:36px;height:36px;border-radius:9px;border:0;background:none;color:var(--text2);display:grid;place-items:center;flex-shrink:0";
</script>

{#if app.tool.phase === "running"}
  <ProcessingCard name="{run.name}.pdf" {percent} {status} oncancel={cancelRun} />
{:else if app.tool.phase === "done" && run.result}
  <DoneCard
    subtitle={t("doneSubMerge")}
    output={run.result.output}
    meta={t("mergeDoneMeta", { n: run.result.files, p: fmtNum(run.result.pages), s: fmtSize(run.result.size) })}
    onanother={processAnother}
  />
{:else if app.tool.phase === "error"}
  <ErrorCard file={run.name + ".pdf"} details={run.error} onretry={processAnother} />
{:else}
  <ToolFrame
    files={runFiles}
    canRun={runFiles.length >= 2 && !nameBad}
    saveAs={nameBad ? undefined : name.trim() + ".pdf"}
    {onrun}
    note={lockedNote}
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
  <div style="display:flex;align-items:flex-start;justify-content:space-between;gap:12px">
    <div style="display:flex;flex-direction:column;gap:2px">
      <h1 style="margin:0;font-size:17px;font-weight:600">{t("mergeTotal", { n: app.tool.files.length, p: fmtNum(totalPages) })}</h1>
      <div style="color:var(--text2)">{t("mergeHint")}</div>
    </div>
    <button onclick={addFiles} style={headBtn}><i class="ph ph-plus" style="font-size:16px"></i>{t("addFiles")}</button>
  </div>

  <ol style="list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
    {#each app.tool.files as f, i (f.path)}
      {@const locked = isLocked(f)}
      {@const last = i === app.tool.files.length - 1}
      {@const lifted = drag.from === i}
      <li animate:flip={{ duration: FLIP_MS }} style="position:relative;z-index:{lifted ? 2 : 'auto'}">
        <div
          data-reorder-idx={i}
          style="display:flex;align-items:center;gap:12px;padding:10px 10px 10px 8px;border-radius:14px;border:1px solid {lifted
            ? 'var(--accent)'
            : 'var(--line)'};background:var(--surface);box-shadow:{lifted
            ? '0 10px 28px rgba(0,0,0,.22)'
            : 'var(--shadow)'};transform:scale({lifted ? 1.015 : 1});transition:transform 120ms,box-shadow 120ms"
        >
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            onpointerdown={(e) => drag.down(e, i)}
            aria-hidden="true"
            title={t("dragHandle")}
            style="display:grid;place-items:center;align-self:stretch;padding:0 2px;touch-action:none;color:var(--text3);cursor:{drag.from === null ? 'grab' : 'grabbing'};flex-shrink:0"
          ><i class="ph ph-dots-six-vertical" style="font-size:20px"></i></span>
          <span style="width:18px;text-align:center;font-weight:700;color:var(--text2);font-variant-numeric:tabular-nums;flex-shrink:0">{i + 1}</span>
          {#if locked}
            <span style="width:34px;height:44px;flex-shrink:0;border-radius:5px;display:grid;place-items:center;background:var(--warn-soft);color:var(--warn)"
              ><i class="ph ph-lock-simple" style="font-size:18px"></i></span
            >
          {:else if f.damaged}
            <div class="page-thumb" style="width:34px;height:44px;flex-shrink:0;border-radius:3px"></div>
          {:else}
            <Thumb path={f.path} password={app.compress.passwords[f.path] ?? null} width={34} height={44} />
          {/if}
          <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:2px">
            <div style="font-weight:600;overflow-wrap:anywhere">{f.name}</div>
            <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;font-size:14px;color:{f.damaged ? 'var(--err)' : 'var(--text2)'}">
              {#if f.damaged}{t("damagedMeta")}
              {:else if locked}{t("lockedMeta")}
              {:else}{f.pages ? `${fmtPages(f.pages)} · ` : ""}{fmtSize(f.size_bytes)}{/if}
              {#if f.signed && !f.damaged}
                <span
                  style="display:inline-flex;align-items:center;gap:4px;padding:1px 8px;border-radius:999px;background:var(--warn-soft);color:var(--warn);font-size:13px;font-weight:600"
                  ><i class="ph ph-seal-check" style="font-size:14px"></i>{t("signed")}</span
                >
              {/if}
            </div>
          </div>
          <div style="display:flex;flex-shrink:0">
            <button
              onclick={() => move(i, i - 1)}
              disabled={i === 0}
              aria-label="{t('moveUp')} {f.name}"
              style="{iconBtn};opacity:{i === 0 ? 0.35 : 1};cursor:{i === 0 ? 'default' : 'pointer'}"><i class="ph ph-caret-up" style="font-size:18px"></i></button
            >
            <button
              onclick={() => move(i, i + 1)}
              disabled={last}
              aria-label="{t('moveDown')} {f.name}"
              style="{iconBtn};opacity:{last ? 0.35 : 1};cursor:{last ? 'default' : 'pointer'}"><i class="ph ph-caret-down" style="font-size:18px"></i></button
            >
            <button onclick={() => remove(f.path)} aria-label="{t('remove')} {f.name}" style="{iconBtn};cursor:pointer"
              ><i class="ph ph-x" style="font-size:18px"></i></button
            >
          </div>
        </div>
        {#if locked && !f.damaged}
          <div style="margin-top:8px">
            <PasswordForm file={f} onskip={() => remove(f.path)} />
          </div>
        {/if}
      </li>
    {/each}
  </ol>
{/snippet}

{#snippet options()}
  <div style="display:flex;flex-direction:column;gap:8px">
    <label for="merge-name" style="font-weight:600">{t("fileName")}</label>
    <input
      id="merge-name"
      value={name}
      oninput={(e) => {
        m.name = e.currentTarget.value;
        m.nameEdited = true;
      }}
      aria-invalid={nameBad}
      autocomplete="off"
      style="height:44px;padding:0 12px;border-radius:10px;border:1px solid {nameBad ? 'var(--err)' : 'var(--line2)'};background:var(--surface);color:var(--text);font-size:15px"
    />
    {#if nameBad}<div role="alert" style="color:var(--err);font-size:14px">{t(nameErrKey)}</div>{/if}
  </div>

  <button
    role="switch"
    aria-checked={m.compress}
    onclick={() => (m.compress = !m.compress)}
    style="display:flex;align-items:flex-start;justify-content:space-between;gap:12px;padding:0;border:0;background:none;text-align:left;color:var(--text);cursor:pointer"
  >
    <span style="display:flex;flex-direction:column;gap:2px">
      <span style="font-weight:600">{t("compressResult")}</span>
      <span style="font-size:14px;color:var(--text2)">{t("compressResultHint")}</span>
    </span>
    <span style="position:relative;width:42px;height:24px;flex-shrink:0;border-radius:12px;background:{m.compress ? 'var(--accent)' : 'var(--line2)'}">
      <span style="position:absolute;top:3px;left:{m.compress ? 21 : 3}px;width:18px;height:18px;border-radius:50%;background:#fff;transition:left .15s"></span>
    </span>
  </button>
{/snippet}
