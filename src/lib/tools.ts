import { isMac, type ToolId } from "./state.svelte";

// Word for Mac can't import PDFs, so PDF to Word isn't offered there.
export const TOOL_ORDER: ToolId[] = (["compress", "merge", "split", "organize", "word", "excel", "ocr", "edit"] as ToolId[]).filter((id) => !(isMac && id === "word"));

export const TOOL_ICON: Record<ToolId, string> = {
  compress: "ph ph-arrows-in",
  merge: "ph ph-stack",
  split: "ph ph-scissors",
  organize: "ph ph-squares-four",
  word: "ph ph-file-doc",
  excel: "ph ph-file-xls",
  ocr: "ph ph-scan",
  edit: "ph ph-pencil-simple-line",
};

// Nothing is "coming soon" right now; Home still renders the (empty) list.
export const SOON: { id: string; icon: string; nameKey: string; descKey: string }[] = [];
