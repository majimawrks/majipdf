<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtPages } from "../format";
  import { chooseFiles, chooseFolder } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked, runnable, startRun, targetValid, processAnother } from "../compress";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import CompressProcessing from "./CompressProcessing.svelte";
  import CompressDone from "./CompressDone.svelte";
  import ErrorCard from "./ErrorCard.svelte";

  const c = app.compress;
  const GRID = "minmax(0,1fr) 104px 40px";

  const runFiles = $derived(runnable());
  const signedNames = $derived(runFiles.filter((f) => f.signed).map((f) => f.name));
  const sizeBad = $derived(c.mode === "target" && !targetValid());
  let signedOpen = $state(false);
  const lockedCount = $derived(app.tool.files.filter((f) => !f.damaged && isLocked(f)).length);
  const lockedNote = $derived(lockedCount === 0 ? "" : lockedCount === 1 ? t("lockedSkip1") : t("lockedSkip", { n: lockedCount }));

  function onrun() {
    if (signedNames.length) signedOpen = true;
    else void startRun();
  }

  async function addFiles() {
    const p = await chooseFiles(true);
    if (p.length) await addFilesToTool(p);
  }
  async function addFolder() {
    const p = await chooseFolder();
    if (p.length) await addFilesToTool(p);
  }
  function remove(path: string) {
    app.tool.files = app.tool.files.filter((f) => f.path !== path);
    if (!app.tool.files.length) processAnother();
  }

  const PRESETS = [
    { id: "smallest", label: "smallest", hint: "smallestHint" },
    { id: "balanced", label: "balanced", hint: "balancedHint" },
    { id: "print", label: "print", hint: "printHint" },
  ] as const;

  const headBtn =
    "display:inline-flex;align-items:center;gap:6px;min-height:40px;padding:0 14px;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:15px;cursor:pointer;white-space:nowrap";
</script>

{#if app.tool.phase === "running"}
  <CompressProcessing />
{:else if app.tool.phase === "done"}
  <CompressDone />
{:else if app.tool.phase === "error"}
  <ErrorCard file={c.run.files[0]?.name ?? ""} details={c.run.error} onretry={processAnother} />
{:else}
  <ToolFrame files={runFiles} canRun={runFiles.length > 0 && !sizeBad} onrun={onrun} note={lockedNote} {main} {options} />
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
  <div style="display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:nowrap">
    <h1 style="margin:0;font-size:20px;font-weight:700">{t("filesReady", { n: app.tool.files.length })}</h1>
    <div style="display:flex;gap:10px;flex-shrink:0">
      <button onclick={addFiles} style={headBtn}><i class="ph ph-plus" style="font-size:16px"></i>{t("addFiles")}</button>
      <button onclick={addFolder} style={headBtn}><i class="ph ph-file-zip" style="font-size:16px"></i>{t("addFolder")}</button>
    </div>
  </div>

  <div style="border:1px solid var(--line);border-radius:16px;background:var(--surface);overflow:hidden">
    <div
      style="display:grid;grid-template-columns:{GRID};gap:10px;padding:10px 16px;background:var(--surface2);font-size:13px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:var(--text3)"
    >
      <span>{t("colFile")}</span><span style="text-align:right">{t("colBefore")}</span><span></span>
    </div>
    {#each app.tool.files as f (f.path)}
      {@const locked = isLocked(f)}
      {@const skipped = c.skipped.includes(f.path)}
      <div style="border-top:1px solid var(--line)">
        <div style="display:grid;grid-template-columns:{GRID};gap:10px;align-items:center;padding:10px 16px">
          <div style="display:flex;align-items:center;gap:12px;min-width:0">
            <span
              style="width:34px;height:34px;flex-shrink:0;border-radius:9px;display:grid;place-items:center;background:{locked || f.damaged
                ? 'var(--warn-soft)'
                : 'var(--accent-soft)'};color:{locked || f.damaged ? 'var(--warn)' : 'var(--accent-text)'}"
            >
              <i class="ph ph-{f.damaged ? 'file-x' : locked ? 'lock-simple' : 'file-pdf'}" style="font-size:19px"></i>
            </span>
            <div style="min-width:0">
              <div style="font-weight:600;overflow-wrap:anywhere">{f.name}</div>
              <div style="font-size:14px;color:{f.damaged ? 'var(--err)' : 'var(--text2)'}">
                {#if f.damaged}{t("damagedMeta")}
                {:else if locked}{skipped ? t("skippedPw") : t("lockedMeta")}
                {:else if f.pages}{fmtPages(f.pages)}{/if}
              </div>
            </div>
          </div>
          <span style="text-align:right;color:var(--text2);font-variant-numeric:tabular-nums">{fmtSize(f.size_bytes)}</span>
          <button
            onclick={() => remove(f.path)}
            aria-label="{t('remove')} {f.name}"
            style="width:36px;height:36px;border-radius:9px;border:0;background:none;color:var(--text2);cursor:pointer;display:grid;place-items:center"
          >
            <i class="ph ph-x" style="font-size:18px"></i>
          </button>
        </div>
        {#if locked && !skipped}
          <PasswordForm file={f} />
        {/if}
      </div>
    {/each}
  </div>
{/snippet}

{#snippet options()}
  <div style="display:flex;flex-direction:column;gap:10px">
    <div style="font-weight:600">{t("howSmall")}</div>
    <div style="display:flex;padding:3px;border-radius:10px;background:var(--surface2);gap:2px">
      {#each [["preset", "modePreset"], ["target", "modeTarget"]] as const as [m, label]}
        <button
          onclick={() => (c.mode = m)}
          aria-pressed={c.mode === m}
          style="flex:1;height:36px;border-radius:7px;border:0;cursor:pointer;font-size:15px;font-weight:600;background:{c.mode === m ? 'var(--surface)' : 'transparent'};color:{c.mode === m ? 'var(--text)' : 'var(--text2)'};box-shadow:{c.mode === m ? '0 1px 2px rgba(0,0,0,.12)' : 'none'}"
          >{t(label)}</button
        >
      {/each}
    </div>
  </div>

  {#if c.mode === "preset"}
    <div role="radiogroup" aria-label={t("howSmall")} style="display:flex;flex-direction:column;gap:8px">
      {#each PRESETS as p}
        {@const sel = c.preset === p.id}
        <button
          role="radio"
          aria-checked={sel}
          onclick={() => (c.preset = p.id)}
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
  {:else}
    <div style="display:flex;flex-direction:column;gap:8px">
      <label for="target-mb" style="font-weight:600">{t("maxSize")}</label>
      <div style="display:flex;width:160px;height:44px;border-radius:10px;border:1px solid {sizeBad ? 'var(--err)' : 'var(--line2)'};overflow:hidden;background:var(--surface)">
        <input
          id="target-mb"
          bind:value={c.targetMb}
          inputmode="decimal"
          aria-invalid={sizeBad}
          style="flex:1;min-width:0;border:0;padding:0 12px;font-size:18px;font-weight:600;background:transparent;color:var(--text)"
        />
        <span style="display:grid;place-items:center;padding:0 12px;background:var(--surface2);color:var(--text2);font-weight:600">MB</span>
      </div>
      {#if sizeBad}<div role="alert" style="color:var(--err);font-size:14px">{t("sizeErr")}</div>{/if}
      <div style="font-size:14px;color:var(--text2)">{t("targetHint")}</div>
    </div>
  {/if}

  <button
    role="switch"
    aria-checked={c.grayscale}
    onclick={() => (c.grayscale = !c.grayscale)}
    style="display:flex;align-items:flex-start;justify-content:space-between;gap:12px;padding:0;border:0;background:none;text-align:left;color:var(--text);cursor:pointer"
  >
    <span style="display:flex;flex-direction:column;gap:2px">
      <span style="font-weight:600">{t("gray")}</span>
      <span style="font-size:14px;color:var(--text2)">{t("grayHint")}</span>
    </span>
    <span style="position:relative;width:42px;height:24px;flex-shrink:0;border-radius:12px;background:{c.grayscale ? 'var(--accent)' : 'var(--line2)'}">
      <span style="position:absolute;top:3px;left:{c.grayscale ? 21 : 3}px;width:18px;height:18px;border-radius:50%;background:#fff;transition:left .15s"></span>
    </span>
  </button>
{/snippet}
