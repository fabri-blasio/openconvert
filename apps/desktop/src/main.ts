/**
 * Mount point.
 *
 * The stylesheet is imported here rather than linked from `index.html` so Vite
 * emits it as a hashed asset under `dist/assets/`. That matters: `style-src
 * 'self'` with no `unsafe-inline` means every rule the app uses has to arrive
 * as a file, and a `<style>` block in the HTML would be silently dropped —
 * spike W9 hit exactly that with an inline `<style>` and an inline handler.
 */

import { mount } from "svelte";
import App from "./App.svelte";
import "./lib/styles/base.css";

const target = document.getElementById("app");
if (target === null) {
  throw new Error("index.html has no #app element to mount into");
}

export default mount(App, { target });
