/**
 * Browser preview entry point. **Not shipped.**
 *
 * Mounts the real `src/App.svelte` with the real stylesheet. The only
 * difference from the desktop build is the three Vite aliases in
 * `vite.preview.config.ts` that point `@tauri-apps/api/*` at the fixtures.
 */

import { mount } from "svelte";
import App from "../src/App.svelte";
import "../src/lib/styles/base.css";
import "./preview.css";

const target = document.getElementById("app");
if (target === null) throw new Error("preview/index.html has no #app element");

// The desktop window is 900×680. Framing the preview to match keeps the
// layout honest — a maximised browser tab would show a wider app than exists.
document.documentElement.classList.add("preview-frame");

export default mount(App, { target });
