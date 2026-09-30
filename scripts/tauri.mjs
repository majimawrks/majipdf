// Sets CARGO_TARGET_DIR outside the project (D:\majipdf-target, or MAJIPDF_TARGET_DIR) then runs
// the local Tauri CLI. A synced folder + a Rust target dir (hundreds of thousands of files) is slow/flaky.
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { writeFileSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { appVersion } from "./version.mjs";

const targetDir = process.env.MAJIPDF_TARGET_DIR || "D:\\majipdf-target";
const projectRoot = path.dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const binDir = path.join(projectRoot, "node_modules", ".bin");

// dev/build get the computed version as a config overlay (exe file version, getVersion()).
const args = process.argv.slice(2);
if (args[0] === "dev" || args[0] === "build") {
  const overlay = path.join(tmpdir(), "majipdf-version.conf.json");
  writeFileSync(overlay, JSON.stringify({ version: appVersion() }));
  args.splice(1, 0, "--config", `"${overlay}"`);
}

// Release builds: strip local paths (user profile, project folder) that rustc embeds in panic
// messages and debug info, so a published exe doesn't reveal the builder's username or folders.
const extraEnv = {};
if (args[0] === "build") {
  const home = homedir();
  const remap = [`--remap-path-prefix=${home}=~`, `--remap-path-prefix=${projectRoot}=majipdf`];
  // CARGO_ENCODED_RUSTFLAGS (0x1f-separated): plain RUSTFLAGS splits on spaces, and paths have them.
  extraEnv.CARGO_ENCODED_RUSTFLAGS = remap.join("\x1f");
}

const cli = spawn("tauri", args, {
  stdio: "inherit",
  shell: true,
  env: {
    ...process.env,
    CARGO_TARGET_DIR: targetDir,
    ...extraEnv,
    PATH: `${binDir}${path.delimiter}${process.env.PATH ?? ""}`,
  },
});

cli.on("exit", (code) => process.exit(code ?? 0));
cli.on("error", (err) => {
  console.error(err);
  process.exit(1);
});
