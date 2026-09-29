// Sets CARGO_TARGET_DIR outside OneDrive (per-hostname) then runs the local Tauri CLI.
// OneDrive sync + a Rust target dir (hundreds of thousands of small files) is slow/flaky.
import { hostname } from "node:os";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const targetDir = "D:\\majipdf-target";
const projectRoot = path.dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const binDir = path.join(projectRoot, "node_modules", ".bin");

const cli = spawn("tauri", process.argv.slice(2), {
  stdio: "inherit",
  shell: true,
  env: {
    ...process.env,
    CARGO_TARGET_DIR: targetDir,
    PATH: `${binDir}${path.delimiter}${process.env.PATH ?? ""}`,
  },
});

cli.on("exit", (code) => process.exit(code ?? 0));
cli.on("error", (err) => {
  console.error(err);
  process.exit(1);
});
