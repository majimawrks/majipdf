<script lang="ts">
  import { untrack } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { fmtSize, fmtNum } from "../format";
  import { chooseFiles } from "../tauri";
  import { addFilesToTool } from "../files";
  import { isLocked } from "../compress";
  import { groupsFor, defaultRanges, startRun, cancelRun, processAnother } from "../split";
  import ToolFrame from "./ToolFrame.svelte";
  import PasswordForm from "./PasswordForm.svelte";
  import SignedDialog from "./SignedDialog.svelte";
  import ProcessingCard from "./ProcessingCard.svelte";
  import SplitDone from "./SplitDone.svelte";
  import ErrorCard from "./ErrorCard.svelte";
  import Thumb from "./Thumb.svelte";

  const s = app.split;
  const run = s.run;

  const file = $derived(app.tool.files[0]);
  const locked = $derived(!!file && !file.damaged && isLocked(file));
  const pw = $derived(file ? (app.compress.passwords[file.path] ?? null) : null);
  const pages = $derived(file?.pages ?? 0);
  let signedOpen = $state(false);

  // New file (or newly unlocked page count) -> default options; cancel keeps them since neither changes.
  $effect(() => {
    void file?.path, file?.pages;
    untrack(() => {
      s.mode = "ranges";
      s.ranges = defaultRanges(pages || 10);
      s.n = "2";
    });
  });

  const parsed = $derived(pages ? groupsFor(pages) : { groups: [] });
  const groups = $derived("groups" in parsed ? parsed.groups : []);
  const err = $derived("error" in parsed ? t(parsed.error.key, parsed.error.params) : "");
  const ok = $derived(!err && groups.length > 0);

  // Page -> first group covering it (tint), and which pages open a group (label).
  const owner = $derived.by(() => {
    const o = new Int32Array(pages + 1).fill(-1);
    groups.forEach((g, gi) => {
      for (let p = g.from; p <= g.to; p++) if (o[p] < 0) o[p] = gi;
    });
    return o;
  });
  const covered = $derived(owner.slice(1).every((x) => x >= 0));
  const starts = $derived(new Set(groups.map((g) => g.from)));

  const stem = $derived(file ? file.name.replace(/\.pdf$/i, "") : "");
  const saveAs = $derived(ok ? `${stem}_part1.pdf${groups.length > 1 ? " …" : ""}` : undefined);

  function onrun() {
    if (!ok) return;
    if (file.signed) signedOpen = true;
    else void startRun(groups);
  }

  async function chooseOther() {
    const p = await chooseFiles(false);
    if (p.length) await addFilesToTool(p);
  }

  const p = $derived(run.progress);
  const percent = $derived(p ? Math.min(99, Math.floor((p.index / p.total) * 100)) : 0);
  const status = $derived(p ? t("splitProgress", { a: Math.min(p.index + 1, p.total), b: p.total }) : t("waiting"));

  const MODES = [
    { id: "every", label: "splitEvery", hint: "splitEveryHint" },
    { id: "ranges", label: "splitRanges", hint: "splitRangesHint" },
    { id: "n", label: "splitN", hint: "splitNHint" },
  ] as const;

  const input = (bad: boolean) =>
    `height:44px;padding:0 12px;border-radius:10px;border:1px solid ${bad ? "var(--err)" : "var(--line2)"};background:var(--surface);color:var(--text);font-size:15px`;
</script>

{#if app.tool.phase === "running"}
  <ProcessingCard name={run.name} {percent} {status} oncancel={cancelRun} />
{:else if app.tool.phase === "done" && run.result}
  <SplitDone result={run.result} onanother={processAnother} />
{:else if app.tool.phase === "error" || file?.damaged}
  <ErrorCard file={file?.damaged ? file.name : run.name} details={run.error || t("damagedMeta")} onretry={processAnother} />
{:else}
  <ToolFrame
    files={file ? [file] : []}
    canRun={ok}
    runLabel={t("splitRun", { n: groups.length })}
    {saveAs}
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
      void startRun(groups);
    }}
  />
{/if}

{#snippet main()}
  {#if file}
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
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(112px,1fr));gap:12px">
        {#each { length: pages } as _, i}
          {@const n = i + 1}
          {@const gi = owner[n]}
          {@const g = gi >= 0 ? gi % 5 : -1}
          <div
            style="padding:8px;border-radius:12px;background:{g >= 0 ? `var(--g${g})` : 'transparent'};border:1px solid {g >= 0 ? 'transparent' : 'var(--line)'};opacity:{g >= 0 || err ? 1 : 0.4};display:flex;flex-direction:column;align-items:center;gap:6px"
          >
            <Thumb path={file.path} password={pw} page={i} width={92} height={120} />
            <div style="display:flex;flex-direction:column;align-items:center;min-height:36px;font-size:13px;color:var(--text2);font-variant-numeric:tabular-nums">
              <span>{n}</span>
              {#if g >= 0 && starts.has(n) && groups[gi].from === n}
                <span style="font-weight:700;color:var(--gi{g})">{t("fileN", { n: gi + 1 })}</span>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
{/snippet}

{#snippet options()}
  {#if file && !locked && pages}
    <div style="display:flex;flex-direction:column;gap:10px">
      <div style="font-weight:600">{t("splitHow")}</div>
      <div role="radiogroup" aria-label={t("splitHow")} style="display:flex;flex-direction:column;gap:8px">
        {#each MODES as m}
          {@const sel = s.mode === m.id}
          <button
            role="radio"
            aria-checked={sel}
            onclick={() => (s.mode = m.id)}
            style="display:flex;gap:10px;text-align:left;padding:{sel ? '11px 13px' : '12px 14px'};border-radius:12px;border:{sel ? '2px solid var(--accent)' : '1px solid var(--line2)'};background:{sel ? 'var(--accent-soft)' : 'var(--surface)'};color:var(--text);cursor:pointer"
          >
            <span style="width:18px;height:18px;flex-shrink:0;margin-top:2px;border-radius:50%;border:2px solid {sel ? 'var(--accent)' : 'var(--line2)'};display:grid;place-items:center">
              {#if sel}<span style="width:8px;height:8px;border-radius:50%;background:var(--accent)"></span>{/if}
            </span>
            <span style="display:flex;flex-direction:column;gap:2px">
              <span style="font-weight:600">{t(m.label)}</span>
              <span style="font-size:14px;color:var(--text2)">{t(m.hint)}</span>
            </span>
          </button>
        {/each}
      </div>
    </div>

    {#if s.mode === "ranges"}
      <div style="display:flex;flex-direction:column;gap:8px">
        <label for="split-ranges" style="font-weight:600">{t("rangesLabel")}</label>
        <input id="split-ranges" bind:value={s.ranges} aria-invalid={!!err} autocomplete="off" spellcheck="false" style={input(!!err)} />
        {#if err}<div role="alert" style="color:var(--err);font-size:14px">{err}</div>{/if}
        <div style="font-size:14px;color:var(--text2)">{t("rangesHint")}</div>
      </div>
    {:else if s.mode === "n"}
      <div style="display:flex;flex-direction:column;gap:8px">
        <label for="split-n" style="font-weight:600">{t("nLabel")}</label>
        <input id="split-n" type="number" min="1" max={pages} step="1" bind:value={s.n} aria-invalid={!!err} style="{input(!!err)};width:120px" />
        {#if err}<div role="alert" style="color:var(--err);font-size:14px">{t("everyNErr", { max: pages })}</div>{/if}
      </div>
    {/if}

    {#if ok}
      <div style="padding:12px 14px;border-radius:12px;background:var(--surface2);display:flex;flex-direction:column;gap:4px">
        <div style="font-weight:600">{t("splitCreates", { n: fmtNum(groups.length) })}</div>
        {#if s.mode === "ranges" && !covered}<div style="font-size:14px;color:var(--text2)">{t("splitNotIncluded")}</div>{/if}
      </div>
    {/if}
  {/if}
{/snippet}
