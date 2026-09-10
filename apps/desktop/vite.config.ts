import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

/**
 * Plain Vite, not SvelteKit.
 *
 * SvelteKit emits an inline `<script>` carrying hydration state into every
 * page. Under this app's CSP — `script-src 'self'`, no nonce and no
 * 'unsafe-inline' — that script is dropped and the app never boots. Spike W9
 * recorded the same class of failure with an inline handler. A single-window
 * desktop shell needs no router, so the fix is to not have the problem.
 */
export default defineConfig({
  plugins: [svelte()],
  // Tauri owns the terminal; clearing it hides the Rust build's output.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // The Rust side has its own rebuild loop.
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Every asset a file: no inlined data: URIs for scripts or styles, so
    // nothing depends on a CSP relaxation that is not there.
    assetsInlineLimit: 0,
    target: "esnext",
    sourcemap: false,
  },
});
