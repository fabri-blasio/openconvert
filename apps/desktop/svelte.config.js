import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/vite-plugin-svelte').SvelteConfig} */
export default {
  // TypeScript in `<script lang="ts">`, and nothing else. No SvelteKit adapter:
  // this is a single-window desktop shell, not a site.
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Runes only. `$state`/`$derived`/`$props` everywhere, so there is one way
    // to hold state rather than two that behave differently under the same
    // syntax.
    runes: true,
  },
};
