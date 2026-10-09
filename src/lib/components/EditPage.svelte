<script lang="ts">
  import { onMount, tick } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n";
  import { editPage, editRender, openEditor, closeEditor, applyEditor, usePcFont, type Para } from "../edit";

  // One page: lazy-rendered image, paragraph overlay and (when open here) the in-place editor. All text is plain; never {@html}.
  let { i, scale }: { i: number; scale: number } = $props();

  const e = app.edit;
  const size = $derived(e.doc!.sizes[i]);
  const dispW = $derived(size[0] * scale);
  const dispH = $derived(size[1] * scale);
  const model = $derived(e.models[i]);
  const ed = $derived(e.editing && e.editing.page === i ? e.editing : null);
  const edPara = $derived(ed ? model?.paras.find((p) => p.id === ed.id) : undefined);

  const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
  const rw = $derived(Math.min(2400, Math.max(200, Math.ceil((dispW * dpr) / 100) * 100)));
  const gen = $derived(e.allGen + (e.pgen[i] ?? 0));

  let host: HTMLDivElement;
  let near = $state(false);
  let img = $state("");
  let loadedKey = "";
  let ta = $state<HTMLTextAreaElement>();
  let barEl = $state<HTMLDivElement>();
  let barPos = $state({ left: 0, top: 0, ready: false });
  let askRevert = $state(false);
  const barW = 340;

  onMount(() => {
    const io = new IntersectionObserver((es) => (near = es.some((x) => x.isIntersecting)), { rootMargin: "1200px 0px" });
    io.observe(host);
    return () => io.disconnect();
  });

  // Load (and reload after an edit / undo / redo / zoom) only while near the viewport; far pages drop their image.
  $effect(() => {
    if (!near) {
      img = "";
      loadedKey = "";
      return;
    }
    const key = `${gen}|${rw}`;
    if (key === loadedKey) return;
    let dead = false;
    Promise.all([editPage(i), editRender(i, rw)])
      .then(([m, url]) => {
        if (dead) return;
        e.models[i] = m;
        img = url;
        loadedKey = key;
      })
      .catch(() => {}); // placeholder stays
    return () => (dead = true);
  });

  // Editor height follows its content.
  $effect(() => {
    if (!ed || !ta) return;
    ed.text;
    scale;
    ta.style.height = "auto";
    ta.style.height = ta.scrollHeight + "px";
  });

  // A new bar (or none) always starts without the revert question.
  $effect(() => {
    ed?.bar;
    askRevert = false;
  });

  // Place the bar on the text's "below" side (rot 0 below, 270 right, 90 left, 180 above), flip to the opposite side when it
  // does not fit, else overlay it on the box; always clamped into the visible scroll area.
  function place() {
    if (!barEl || !ed?.bar || !edPara) return;
    const h = host.getBoundingClientRect();
    const r0 = ta?.getBoundingClientRect();
    const sc = host.parentElement?.getBoundingClientRect();
    const R = r0 && r0.width > 0 ? { l: r0.left, t: r0.top, r: r0.right, b: r0.bottom } : { l: h.left + edPara.bbox[0] * scale, t: h.top + edPara.bbox[1] * scale, r: h.left + (edPara.bbox[0] + edPara.bbox[2]) * scale, b: h.top + (edPara.bbox[1] + edPara.bbox[3]) * scale };
    const V = { l: Math.max(sc?.left ?? 0, 0) + 8, t: Math.max(sc?.top ?? 0, 0) + 8, r: Math.min(sc?.right ?? innerWidth, innerWidth) - 8, b: Math.min(sc?.bottom ?? innerHeight, innerHeight) - 8 };
    const w = barEl.offsetWidth;
    const hh = barEl.offsetHeight;
    const clampX = (x: number) => Math.max(V.l, Math.min(x, V.r - w));
    const clampY = (y: number) => Math.max(V.t, Math.min(y, V.b - hh));
    const spots: Record<string, { left: number; top: number; ok: boolean }> = {
      below: { left: clampX(R.l), top: R.b + 8, ok: R.b + 8 + hh <= V.b },
      above: { left: clampX(R.l), top: R.t - 8 - hh, ok: R.t - 8 - hh >= V.t },
      right: { left: R.r + 8, top: clampY(R.t), ok: R.r + 8 + w <= V.r },
      left: { left: R.l - 8 - w, top: clampY(R.t), ok: R.l - 8 - w >= V.l },
    };
    const rot = edPara.rot;
    const [pref, opp] = rot === 270 ? ["right", "left"] : rot === 90 ? ["left", "right"] : rot === 180 ? ["above", "below"] : ["below", "above"];
    const pick = spots[pref].ok ? spots[pref] : spots[opp].ok ? spots[opp] : { left: clampX(R.l), top: clampY(R.t), ok: true };
    barPos = { left: clampX(pick.left), top: clampY(pick.top), ready: true };
  }
  $effect(() => {
    // re-place after every change that moves the box or resizes the bar
    ed?.bar;
    ed?.text;
    askRevert;
    scale;
    edPara;
    if (!ed?.bar) {
      barPos.ready = false;
      return;
    }
    void tick().then(place);
  });
  $effect(() => {
    if (!ed?.bar) return;
    const sc = host.parentElement;
    sc?.addEventListener("scroll", place, { passive: true });
    window.addEventListener("resize", place);
    return () => {
      sc?.removeEventListener("scroll", place);
      window.removeEventListener("resize", place);
    };
  });

  const reasonKey = (p: Para) => (p.reason === "form" ? "editLockForm" : p.reason === "scan" ? "editLockScan" : "editLockOther");
  const tipUp = (p: Para) => p.bbox[1] + p.bbox[3] > size[1] - 150;
  const tipRight = (p: Para) => p.bbox[0] > size[0] / 2;
  const fontOf = (p: Para) => `"${p.css_font.replace(/"/g, "")}", sans-serif`;

  function focusEnd(node: HTMLTextAreaElement) {
    node.focus();
    node.setSelectionRange(node.value.length, node.value.length);
  }

  function onkeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape") {
      ev.preventDefault();
      ev.stopPropagation();
      closeEditor();
    } else if (ev.key === "Enter") {
      ev.preventDefault(); // no new paragraphs in Edit 1
      if (ev.ctrlKey || ev.metaKey) void applyEditor();
    }
  }

  function oninput(ev: Event) {
    const el = ev.currentTarget as HTMLTextAreaElement;
    if (!e.editing) return;
    e.editing.text = el.value.replace(/\s*[\r\n]+\s*/g, " ");
    e.editing.bar = null;
  }

  const btn = "min-height:32px;padding:0 12px;border-radius:8px;border:1px solid var(--line2);background:var(--surface);color:var(--text);font-size:14px;cursor:pointer;white-space:nowrap";
  const btnPrimary = "min-height:32px;padding:0 12px;border-radius:8px;border:0;background:var(--accent);color:var(--accent-ink);font-size:14px;font-weight:600;cursor:pointer;white-space:nowrap";
</script>

<div
  bind:this={host}
  id="ep-{i}"
  role="group"
  aria-label={t("pageAria", { n: i + 1 })}
  style="position:relative;flex-shrink:0;margin-inline:auto;width:{dispW}px;height:{dispH}px;background:var(--page);box-shadow:0 0 0 1px var(--pageedge),var(--shadow)"
>
  {#if img}<img src={img} alt="" draggable="false" style="position:absolute;inset:0;width:100%;height:100%;display:block;user-select:none" />{/if}

  {#each model?.paras ?? [] as p (p.id)}
    {@const box = `left:${p.bbox[0] * scale}px;top:${p.bbox[1] * scale}px;width:${p.bbox[2] * scale}px;height:${p.bbox[3] * scale}px`}
    {@const tip = `${tipUp(p) ? "bottom:100%" : "top:100%"};${tipRight(p) ? "right:0" : "left:0"}`}
    {#if p.status === "direct"}
      <button class="para direct" class:edited={p.edited} class:hide={ed?.id === p.id} style={box} aria-label={p.text} onclick={() => openEditor(i, p.id)}></button>
    {:else if p.status === "pc_font"}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (keyboard focus reveals the tooltip and its button) -->
      <div class="para lock" class:edited={p.edited} class:hide={ed?.id === p.id} style={box} role="note" tabindex="0" aria-label={t("editPcHint")}>
        <i class="ph ph-lock-simple badge"></i>
        <div class="tip" style={tip}>
          <div class="bubble">
            <div>{t("editPcHint")}</div>
            <button style={btnPrimary} onclick={() => openEditor(i, p.id, true)}>{t("editPcBtn")}</button>
          </div>
        </div>
      </div>
    {:else}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (keyboard focus reveals the tooltip) -->
      <div class="para lock" style={box} role="note" tabindex="0" aria-label={t(reasonKey(p))}>
        <i class="ph ph-lock-simple badge"></i>
        <div class="tip" style={tip}><div class="bubble">{t(reasonKey(p))}</div></div>
      </div>
    {/if}
  {/each}

  {#if ed && edPara}
    {@const p = edPara}
    {@const side = p.rot === 90 || p.rot === 270}
    {@const tw = (side ? p.bbox[3] : p.bbox[2]) * scale}
    {@const th = (side ? p.bbox[2] : p.bbox[3]) * scale}
    <div
      data-editor
      style="position:absolute;z-index:5;left:{p.bbox[0] * scale}px;top:{p.bbox[1] * scale}px;width:{Math.max(p.bbox[2] * scale, 300)}px"
    >
      <!-- sideways text: the editor is laid out in the text's own frame (line length x stacking extent) and turned over the box -->
      <div
        style={p.rot
          ? `position:absolute;left:${(p.bbox[2] * scale - tw) / 2}px;top:${(p.bbox[3] * scale - th) / 2}px;width:${tw}px;transform:rotate(${p.rot}deg);transform-origin:${tw / 2}px ${th / 2}px`
          : ""}
      >
      <textarea
        bind:this={ta}
        use:focusEnd
        value={ed.text}
        {oninput}
        {onkeydown}
        readonly={ed.applying}
        rows="1"
        spellcheck="false"
        aria-label={t("editName")}
        style="display:block;box-sizing:border-box;margin:0;padding:0;border:0;resize:none;overflow:hidden;outline:2px solid var(--accent);outline-offset:2px;background:var(--surface);color:var(--text);width:{tw}px;min-height:{th}px;font-family:{fontOf(p)};font-weight:{p.bold ? 700 : 400};font-style:{p.italic ? 'italic' : 'normal'};font-size:{p.size * scale}px;line-height:{p.pitch * scale}px;text-align:{p.align === 'justified' ? 'justify' : p.align};text-indent:{p.first_indent * scale}px;opacity:{ed.applying ? 0.7 : 1}"
      ></textarea>
      </div>
    </div>
  {/if}
  {#if ed && edPara && ed.bar}
    {@const b = ed.bar}
    <!-- Fixed layer: never clipped by the page or the scroller; placed beside the text's "below" side, flipped / clamped in place() -->
    <div
      data-editor
      bind:this={barEl}
      role="alert"
      style="position:fixed;z-index:30;left:{barPos.left}px;top:{barPos.top}px;visibility:{barPos.ready ? 'visible' : 'hidden'};box-sizing:border-box;width:{barW}px;max-width:calc(100vw - 16px);padding:10px 12px;border-radius:10px;background:{b.kind === 'missing' || b.kind === 'cannot_push' || b.kind === 'overflow' ? 'var(--warn-soft)' : 'var(--err-soft)'};color:var(--text);font-size:14px;line-height:1.4;box-shadow:var(--shadow);display:flex;flex-direction:column;gap:8px;overflow-wrap:anywhere;text-align:left;text-indent:0;font-weight:400;font-style:normal;text-align-last:auto"
    >
      <div>
        {#if b.kind === "missing"}{t("editMissing", { c: b.chars })}
        {:else if b.kind === "unsupported"}{t("editUnsupported", { c: b.chars })}
        {:else if b.kind === "overflow"}{t("editOverflow", { n: b.n })}
        {:else if b.kind === "cannot_push"}{t("editCannotPush")}
        {:else}{b.text}{/if}
      </div>
      {#if askRevert}
        <div>{t("editRevertAsk")}</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          <button style={btnPrimary} onclick={closeEditor}>{t("editRevert")}</button>
          <button style={btn} onclick={() => (askRevert = false)}>{t("cancel")}</button>
        </div>
      {:else}
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          {#if b.kind === "missing"}
            <button style={btnPrimary} onclick={usePcFont}>{t("editUsePc")}</button>
          {:else if b.kind === "cannot_push" || b.kind === "overflow"}
            <button style={btnPrimary} onclick={() => applyEditor(true)}>{t("editKeepOverlap")}</button>
          {/if}
          {#if b.kind !== "unsupported" && b.kind !== "error"}
            <button
              style={btn}
              onclick={async () => {
                if (e.editing) e.editing.bar = null;
                await tick();
                ta?.focus();
              }}>{t("editKeep")}</button
            >
          {/if}
          <button style={btn} onclick={() => (askRevert = true)}>{t("editRevert")}</button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .para {
    position: absolute;
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
    border-radius: 2px;
    font: inherit;
  }
  .para.hide {
    visibility: hidden;
  }
  .direct {
    cursor: text;
  }
  .direct:hover,
  .direct:focus-visible,
  .lock:hover,
  .lock:focus-within {
    outline: 1.5px dashed var(--accent);
    outline-offset: 2px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .lock {
    cursor: not-allowed;
  }
  .edited::before {
    content: "";
    position: absolute;
    left: -8px;
    top: 0;
    bottom: 0;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }
  .badge {
    display: none;
    position: absolute;
    top: -10px;
    right: -10px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    place-items: center;
    background: var(--surface);
    color: var(--text2);
    border: 1px solid var(--line2);
    font-size: 12px;
  }
  .lock:hover .badge,
  .lock:focus-within .badge {
    display: grid;
  }
  .tip {
    display: none;
    position: absolute;
    z-index: 6;
    padding: 6px 0;
    width: 270px;
    max-width: 70vw;
  }
  .lock:hover .tip,
  .lock:focus-within .tip {
    display: block;
  }
  .bubble {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line2);
    box-shadow: var(--shadow);
    font-size: 14px;
    line-height: 1.4;
    cursor: default;
    text-align: left;
    font-weight: 400;
  }
</style>
