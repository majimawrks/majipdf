<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { setStyle, type Align } from "../edit";

  // Format section of the options panel: edits e.fmt.style; edit.ts debounces the apply (250 ms).
  const e = app.edit;
  const f = $derived(e.fmt!);
  const mx = (k: string) => f.mixed.includes(k); // different across the selection: shown blank
  const SIZES = [8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72];
  const COLORS = ["#000000", "#1F3864", "#0070C0", "#C00000"];
  const ALIGNS: [Align, string][] = [
    ["left", "text-align-left"],
    ["center", "text-align-center"],
    ["right", "text-align-right"],
    ["justified", "text-align-justify"],
  ];
  const alignName = (a: Align) => t(({ left: "fmtAlignL", center: "fmtAlignC", right: "fmtAlignR", justified: "fmtAlignJ" } as const)[a]);
  // The family list always contains the current font so the select never shows a blank.
  const families = $derived(f.style.font === "orig" || e.fonts.includes(f.style.font) ? e.fonts : [f.style.font, ...e.fonts]);

  function setSize(ev: Event) {
    const v = parseFloat((ev.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(v) && v >= 6 && v <= 72) setStyle({ size: Math.round(v * 2) / 2 });
  }

  const lbl = "font-size:13px;font-weight:700;color:var(--text3);letter-spacing:.04em;text-transform:uppercase";
  const field = "height:40px;box-sizing:border-box;border-radius:10px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font:inherit;padding:0 10px;min-width:0";
  const tog = (on: boolean) =>
    `flex:1;min-height:36px;border-radius:7px;border:0;cursor:pointer;display:inline-flex;align-items:center;justify-content:center;gap:6px;font:inherit;font-weight:600;color:${on ? "var(--accent-text)" : "var(--text2)"};background:${on ? "var(--surface)" : "transparent"};box-shadow:${on ? "0 1px 2px rgba(0,0,0,.12)" : "none"}`;
  const track = "display:flex;padding:3px;border-radius:10px;background:var(--surface2);gap:2px";
</script>

<!-- data-editor: clicking here must not apply / close an open text editor -->
<!-- Buttons keep the editor focus and its text selection (mousedown default prevented) -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  data-editor
  aria-label={t("fmtTitle")}
  style="display:flex;flex-direction:column;gap:14px"
  onmousedown={(ev) => (ev.target as Element).closest("button") && ev.preventDefault()}
>
  <div style={lbl}>{t("fmtTitle")}</div>

  <label style="display:flex;flex-direction:column;gap:6px;font-weight:600">
    {t("fmtFont")}
    <select style={field} value={mx("font") ? "" : f.style.font} onchange={(ev) => setStyle({ font: ev.currentTarget.value })}>
      {#if mx("font")}<option value="" disabled>—</option>{/if}
      {#if f.origOk || f.style.font === "orig"}<option value="orig">{t("fmtOriginal", { n: f.origName || "—" })}</option>{/if}
      {#each families as fam (fam)}<option value={fam}>{fam}</option>{/each}
    </select>
  </label>

  <div style="display:flex;gap:12px">
    <label style="display:flex;flex-direction:column;gap:6px;font-weight:600;flex:1;min-width:0">
      {t("fmtSize")}
      <input type="number" min="6" max="72" step="0.5" list="fmt-sizes" style={field} value={mx("size") ? "" : f.style.size} placeholder="—" oninput={setSize} />
      <datalist id="fmt-sizes">{#each SIZES as s (s)}<option value={s}></option>{/each}</datalist>
    </label>
    <div style="display:flex;flex-direction:column;gap:6px;font-weight:600;flex:1;min-width:0">
      {t("fmtBold")} / {t("fmtItalic")}
      <div style={track + ";height:40px;box-sizing:border-box"}>
        <button style={tog(f.style.bold && !mx("bold"))} aria-pressed={f.style.bold && !mx("bold")} aria-label={t("fmtBold")} title={t("fmtBold")} onclick={() => setStyle({ bold: !f.style.bold })}
          ><i class="ph ph-text-b" style="font-size:18px"></i></button
        >
        <button style={tog(f.style.italic && !mx("italic"))} aria-pressed={f.style.italic && !mx("italic")} aria-label={t("fmtItalic")} title={t("fmtItalic")} onclick={() => setStyle({ italic: !f.style.italic })}
          ><i class="ph ph-text-italic" style="font-size:18px"></i></button
        >
      </div>
    </div>
  </div>

  <div style="display:flex;flex-direction:column;gap:6px">
    <span style="font-weight:600" id="fmt-color">{t("fmtColor")}</span>
    <div role="group" aria-labelledby="fmt-color" style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">
      {#each COLORS as c (c)}
        {@const on = !mx("color") && f.style.color.toLowerCase() === c.toLowerCase()}
        <button
          aria-label={c}
          title={c}
          aria-pressed={on}
          onclick={() => setStyle({ color: c })}
          style="width:32px;height:32px;border-radius:50%;cursor:pointer;background:{c};border:2px solid var(--surface);box-shadow:0 0 0 {on ? '2px var(--accent)' : '1px var(--line2)'}"
        ></button>
      {/each}
      <input
        type="color"
        aria-label={t("fmtColor")}
        value={f.style.color}
        oninput={(ev) => setStyle({ color: ev.currentTarget.value.toUpperCase() })}
        style="width:40px;height:32px;padding:0;border:1px solid var(--line2);border-radius:8px;background:var(--surface);cursor:pointer"
      />
    </div>
  </div>

  <div style="display:flex;flex-direction:column;gap:6px">
    <span style="font-weight:600" id="fmt-align">{t("fmtAlign")}</span>
    <div role="group" aria-labelledby="fmt-align" style={track}>
      {#each ALIGNS as [a, ic] (a)}
        <button style={tog(f.style.align === a)} aria-pressed={f.style.align === a} aria-label={alignName(a)} title={alignName(a)} onclick={() => setStyle({ align: a })}
          ><i class="ph ph-{ic}" style="font-size:18px"></i></button
        >
      {/each}
    </div>
  </div>
</section>
