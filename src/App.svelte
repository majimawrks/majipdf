<script lang="ts">
  import { onMount } from "svelte";
  import { app, settings } from "./lib/state.svelte";
  import { t } from "./lib/i18n";
  import { inTauri, officeStatus, flashIfUnfocused } from "./lib/tauri";
  import { addFilesToHome, addFilesToTool } from "./lib/files";
  import Header from "./lib/components/Header.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import SettingsPopover from "./lib/components/SettingsPopover.svelte";
  import Home from "./lib/components/Home.svelte";
  import ToolEmpty from "./lib/components/ToolEmpty.svelte";
  import CompressTool from "./lib/components/CompressTool.svelte";
  import MergeTool from "./lib/components/MergeTool.svelte";
  import SplitTool from "./lib/components/SplitTool.svelte";
  import OrganizeTool from "./lib/components/OrganizeTool.svelte";
  import WordTool from "./lib/components/WordTool.svelte";
  import ExcelTool from "./lib/components/ExcelTool.svelte";
  import ToolFrame from "./lib/components/ToolFrame.svelte";
  import OfficeMissing from "./lib/components/OfficeMissing.svelte";

  const isHome = $derived(app.route === "home");
  const officeMissing = $derived(app.route === "word" && !app.office.word); // Excel has its own engine: never gated

  onMount(() => {
    officeStatus().then((s) => {
      app.office = s;
    });

    if (!inTauri) return; // browser-dev: drops are ignored (no real paths available)

    let unlisten: (() => void) | undefined;
    (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          app.dragging = true;
        } else if (event.payload.type === "drop") {
          app.dragging = false;
          const paths = event.payload.paths;
          if (isHome) void addFilesToHome(paths);
          else void addFilesToTool(paths);
        } else {
          app.dragging = false;
        }
      });
    })();

    return () => unlisten?.();
  });

  $effect(() => {
    document.documentElement.setAttribute("lang", settings.lang);
  });

  $effect(() => {
    document.documentElement.setAttribute("data-theme", settings.theme);
  });

  // Every tool goes running → done | error; flash the taskbar then (user may be in another window).
  let prevPhase = app.tool.phase;
  $effect(() => {
    const phase = app.tool.phase;
    if (prevPhase === "running" && (phase === "done" || phase === "error")) void flashIfUnfocused();
    prevPhase = phase;
  });
</script>

<div class="app-root">
  <Header />

  {#if app.notice === "busy" && !isHome}
    <div role="alert" style="padding:10px 16px;background:var(--warn-soft);color:var(--text)">{t("busy")}</div>
  {/if}

  <main style="flex:1;min-height:0;display:flex;flex-direction:column">
    {#if isHome}
      <Home />
    {:else if officeMissing}
      <OfficeMissing />
    {:else if app.route === "compress" && app.tool.phase !== "empty"}
      <CompressTool />
    {:else if app.route === "merge" && app.tool.phase !== "empty"}
      <MergeTool />
    {:else if app.route === "split" && app.tool.phase !== "empty"}
      <SplitTool />
    {:else if app.route === "organize" && app.tool.phase !== "empty"}
      <OrganizeTool />
    {:else if app.route === "word" && app.tool.phase !== "empty"}
      <WordTool />
    {:else if app.route === "excel" && app.tool.phase !== "empty"}
      <ExcelTool />
    {:else if app.tool.phase === "loaded"}
      <ToolFrame />
    {:else}
      <ToolEmpty />
    {/if}
  </main>

  <StatusBar />

  {#if app.settingsOpen}
    <SettingsPopover />
  {/if}
</div>
