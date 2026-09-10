import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

/**
 * Browser preview of the desktop UI. **Never used by the shipping build.**
 *
 * `vite.config.ts` builds the real thing. This config exists so the UI can be
 * looked at in a browser without a Tauri window: it mounts the same
 * `src/App.svelte` with the same components and the same `tokens.css`, and
 * aliases the three `@tauri-apps/api` entry points to fixture modules under
 * `preview/`.
 *
 * Nothing in `src/` is modified or aware of this — which is the point. A
 * preview that needed an `IS_TAURI` branch in shipping code would put a second
 * behaviour in the app to keep the first one viewable.
 *
 *     npm run preview:ui
 */
const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  root: here("./preview"),
  // The real app's public dir, so Inter loads here exactly as it does there.
  publicDir: here("./public"),
  plugins: [svelte({ configFile: here("./svelte.config.js") })],
  clearScreen: false,
  resolve: {
    alias: {
      "@tauri-apps/api/core": here("./preview/tauri-core.ts"),
      "@tauri-apps/api/event": here("./preview/tauri-event.ts"),
      "@tauri-apps/api/webview": here("./preview/tauri-webview.ts"),
      "@tauri-apps/api/window": here("./preview/tauri-window.ts"),
    },
  },
  server: {
    port: 5174,
    strictPort: true,
    open: false,
    // The entry lives in preview/ but imports src/ and node_modules/ above it.
    fs: { allow: [here(".")] },
  },
  build: {
    outDir: here("./preview-dist"),
    emptyOutDir: true,
  },
});
