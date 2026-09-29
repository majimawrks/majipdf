import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Local-only folders (_assets, _samples…) and the Rust side aren't frontend sources. OneDrive can
    // lock a file there mid-sync, and a watch error (EBUSY) kills the whole dev server.
    watch: { ignored: ["**/_*/**", "**/src-tauri/**"] },
  },
});
