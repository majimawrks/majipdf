<script lang="ts">
  import { app, settings } from "../state.svelte";
  import { t } from "../i18n";
  import type { ToolId } from "../state.svelte";
  import { fmtSize } from "../format";
  import type { Snippet } from "svelte";

  // Optional per-tool content; without them the frame shows the plain file list (skeleton tools).
  let {
    main,
    options,
    files = app.tool.files,
    canRun = true,
    onrun,
    note = "",
    saveAs,
    runLabel: runLabelProp,
  }: { runLabel?: string; saveAs?: string; main?: Snippet; options?: Snippet; files?: { name: string }[]; canRun?: boolean; onrun?: () => void; note?: string } = $props();

  const id = $derived(app.route as ToolId);

  const SUFFIX: Record<ToolId, string> = {
    compress: "_compressed.pdf",
    merge: "_merged.pdf",
    split: "_part1….pdf",
    organize: "_organized.pdf",
    word: ".docx",
    excel: ".xlsx",
    ocr: "_ocr.pdf",
  };

  const saveName = $derived.by(() => {
    if (saveAs) return saveAs;
    const first = files[0];
    const stem = first ? first.name.replace(/\.pdf$/i, "") : "file";
    return stem + SUFFIX[id] + (id === "compress" && files.length > 1 ? " …" : "");
  });

  const saveWhere = $derived(settings.outMode === "folder" ? t("inFolder") : t("nextTo"));

  const runLabel = $derived.by(() => {
    if (runLabelProp) return runLabelProp;
    const n = files.length;
    switch (id) {
      case "compress":
        return n === 1 ? t("compressRun1") : t("compressRun", { n });
      case "merge":
        return t("mergeRun", { n });
      case "split":
        return t("splitRun", { n });
      case "organize":
        return t("orgRun", { n });
      case "word":
        return t("wordRun");
      case "ocr":
        return t("ocrRun");
      case "excel":
        return n === 1 ? t("excelRun1") : t("excelRun", { n });
    }
  });
</script>

<div style="flex:1;min-height:0;display:flex">
  <section style="flex:1;min-width:0;overflow:auto;padding:22px 26px 28px;display:flex;flex-direction:column;gap:16px">
    {#if main}{@render main()}{:else}
    <ol style="list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px">
      {#each app.tool.files as f (f.path)}
        <li
          style="display:flex;align-items:center;gap:12px;padding:10px 10px 10px 8px;border-radius:14px;border:1px solid var(--line);background:var(--surface);box-shadow:var(--shadow)"
        >
          <div class="page-thumb" style="width:34px;height:44px;flex-shrink:0;border-radius:3px"></div>
          <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:2px">
            <div style="font-weight:600;overflow-wrap:anywhere">{f.name}</div>
            <div style="color:var(--text2);font-size:14px">{fmtSize(f.size_bytes)}</div>
          </div>
        </li>
      {/each}
    </ol>
    {/if}
  </section>

  <aside
    aria-label={t("options")}
    style="width:336px;flex-shrink:0;border-left:1px solid var(--line);background:var(--surface);display:flex;flex-direction:column;min-height:0"
  >
    <div style="padding:18px 22px 6px;font-size:13px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:var(--text3)">
      {t("options")}
    </div>
    <div style="flex:1;min-height:0;overflow:auto;padding:6px 22px 20px;display:flex;flex-direction:column;gap:20px">
      {@render options?.()}
    </div>
    <div style="border-top:1px solid var(--line);padding:14px 22px 18px;display:flex;flex-direction:column;gap:12px">
      <div style="font-size:14px;color:var(--text2);overflow-wrap:anywhere">
        {t("savesAs")} <strong style="color:var(--text);font-weight:600">{saveName}</strong>
        {saveWhere}
      </div>
      {#if note}
        <div style="display:flex;gap:6px;font-size:14px;font-weight:600;color:var(--warn)">
          <i class="ph ph-lock-simple" style="font-size:16px;margin-top:2px"></i>{note}
        </div>
      {/if}
      <button
        disabled={!onrun || !canRun}
        onclick={onrun}
        style="display:flex;align-items:center;justify-content:center;gap:8px;min-height:48px;padding:6px 16px;border-radius:12px;border:0;background:{onrun && canRun ? 'var(--accent)' : 'var(--surface2)'};color:{onrun && canRun ? 'var(--accent-ink)' : 'var(--text3)'};font-size:16.5px;font-weight:600;cursor:{onrun && canRun ? 'pointer' : 'not-allowed'};line-height:1.2"
      >
        {runLabel}<i class="ph ph-arrow-right" style="font-size:18px"></i>
      </button>
    </div>
  </aside>
</div>
