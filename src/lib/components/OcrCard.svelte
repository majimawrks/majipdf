<script lang="ts">
  // Prominent OCR switch for scanned files in the Word / Excel tools.
  import { t } from "../i18n";
  import Switch from "./Switch.svelte";
  import { ocrEngine } from "../state.svelte";

  let { on, kind, onchange }: { on: boolean; kind: "Word" | "Excel"; onchange: (v: boolean) => void } = $props();
</script>

<div
  role="group"
  aria-label={t("ocrCardTitle")}
  style="display:flex;align-items:center;gap:16px;padding:16px 18px;border-radius:14px;border:2px solid {on ? 'var(--accent)' : 'var(--warn)'};background:{on ? 'var(--accent-soft)' : 'var(--warn-soft)'};color:{on ? 'var(--text)' : 'var(--warn)'}"
>
  <span
    style="width:48px;height:48px;flex-shrink:0;border-radius:50%;display:grid;place-items:center;background:var(--surface);color:{on ? 'var(--accent-text)' : 'var(--warn)'}"
    ><i class="ph {on ? 'ph-scan' : 'ph-warning'}" style="font-size:28px"></i></span
  >
  <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:3px">
    <div style="font-size:17px;font-weight:700;color:{on ? 'var(--accent-text)' : 'var(--warn)'}">{t("ocrCardTitle")}</div>
    <div style="font-size:14px;line-height:1.4">{on ? t(kind === "Word" ? "ocrCardOnWord" : "ocrCardOnExcel", { s: ocrEngine() === "accurate" ? 10 : 3 }) : t(kind === "Word" ? "ocrCardOffWord" : "ocrCardOffExcel")}</div>
  </div>
  <div style="flex-shrink:0;min-width:84px;display:flex;justify-content:flex-end">
    <Switch {on} label={on ? t("ocrSwitchOn") : t("ocrSwitchOff")} {onchange} />
  </div>
</div>
