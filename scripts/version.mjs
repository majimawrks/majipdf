// App version, computed at build time — single source for the exe (via scripts/tauri.mjs) and the
// UI (via vite.config.ts `__APP_VERSION__`).
//
// Pre-1.0 scheme (decided 2026-09-30): 0.<features shipped>.<git commit count>
//   FEATURES = user-facing tools shipped: Compress, Merge, Split, Organize, Word, Excel = 6.
//   Bump it when a feature ships (OCR, Edit & sign…). The commit count only ever grows.
// From 1.0.0 on: plain semver MAJOR.MINOR.PATCH, set by hand.
import { execSync } from "node:child_process";

export const FEATURES = 6;

export function appVersion() {
  let commits = 0;
  try {
    commits = Number(execSync("git rev-list --count HEAD", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim()) || 0;
  } catch {
    // no git (e.g. building from a source zip): patch 0
  }
  return `0.${FEATURES}.${commits}`;
}
