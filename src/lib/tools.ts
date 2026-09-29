import type { ToolId } from "./state.svelte";

export const TOOL_ORDER: ToolId[] = ["compress", "merge", "split", "organize", "word", "excel"];

export const TOOL_ICON: Record<ToolId, string> = {
  compress: "ph ph-arrows-in",
  merge: "ph ph-stack",
  split: "ph ph-scissors",
  organize: "ph ph-squares-four",
  word: "ph ph-file-doc",
  excel: "ph ph-file-xls",
};

export const SOON = [
  { id: "ocr", icon: "ph ph-scan", nameKey: "ocrName", descKey: "ocrDesc" },
  { id: "edit", icon: "ph ph-signature", nameKey: "editName", descKey: "editDesc" },
] as const;
