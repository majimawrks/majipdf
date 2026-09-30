<script lang="ts">
  import { thumbnail } from "../tauri";
  import { getThumb, putThumb } from "../thumbcache";

  let {
    path,
    password = null,
    page = 0,
    width,
    height,
    radius = 3,
    fill = false,
  }: { fill?: boolean; path: string; password?: string | null; page?: number; width: number; height: number; radius?: number } = $props();

  let el: HTMLDivElement;
  let src = $state("");

  // Fetch only once the box is visible; 2x the display width keeps it sharp on high-DPI screens.
  $effect(() => {
    const w = width * 2;
    const pw = password; // tracked: unlocking a file re-fetches (the cache key has no password)
    const hit = getThumb(path, page, w);
    src = hit ?? "";
    if (hit) return;
    let dead = false;
    const io = new IntersectionObserver((es) => {
      if (!es.some((e) => e.isIntersecting)) return;
      io.disconnect();
      thumbnail(path, pw, page, w)
        .then((u) => {
          putThumb(path, page, w, u, !!pw);
          if (!dead) src = u;
        })
        .catch(() => {}); // placeholder stays
    });
    io.observe(el);
    return () => {
      dead = true;
      io.disconnect();
    };
  });
</script>

<div bind:this={el} class={src ? "" : "page-thumb"} style="width:{fill ? "100%" : width + "px"};height:{fill ? "100%" : height + "px"};flex-shrink:0;border-radius:{radius}px;overflow:hidden;display:grid;place-items:center">
  <!-- contain, not cover: landscape pages must show whole, not cropped -->
  <!-- draggable=false: a native image drag is blocked by Tauri on Windows and cancels our pointer-based reorder -->
  {#if src}<img {src} alt="" draggable="false" style="max-width:100%;max-height:100%;object-fit:contain;display:block;box-shadow:0 0 0 1px var(--pageedge)" />{/if}
</div>
