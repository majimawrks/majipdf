import { isMac, type ToolId } from "./state.svelte";

// Word for Mac can't import PDFs, so PDF to Word isn't offered there.
export const TOOL_ORDER: ToolId[] = (["compress", "merge", "split", "organize", "word", "excel", "ocr"] as ToolId[]).filter((id) => !(isMac && id === "word"));

export const TOOL_ICON: Record<ToolId, string> = {
  compress: "ph ph-arrows-in",
  merge: "ph ph-stack",
  split: "ph ph-scissors",
  organize: "ph ph-squares-four",
  word: "ph ph-file-doc",
  excel: "ph ph-file-xls",
  ocr: "ph ph-scan",
};

export const SOON = [
  { id: "edit", icon: "ph ph-signature", nameKey: "editName", descKey: "editDesc" },
] as const;
