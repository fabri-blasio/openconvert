/**
 * Capture the product screenshots. **Not shipped.**
 *
 *     npm run shots            capture every scene, light and dark
 *     npm run shots -- drop-idle batch-plan     capture a subset
 *
 * Starts `apps/desktop`'s browser preview (the real UI over mocked IPC),
 * drives it with Playwright, and writes 2× PNGs to `tools/shots/raw/`.
 * `encode.mjs` turns those into the AVIF and WebP the site serves.
 *
 * Determinism is the point (13-SITE-REBUILD §4.2). Motion is frozen at both
 * ends — the context is created with `reducedMotion: 'reduce'` and each
 * screenshot passes `animations: 'disabled'` — and the fixtures already use
 * fixed durations, so a changed PNG in `git status` means the UI changed,
 * which is the only reason this is worth automating.
 */

import { spawn } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { scenes } from "./scenes.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..", "..");
const RAW = join(HERE, "raw");

const PORT = Number(process.env.SHOTS_PORT ?? 5174);
const ORIGIN = `http://localhost:${PORT}`;

/* The preview's cold start is dependency optimisation, not our code, and it is
   slow the first time — measured at ~29 s on a warm disk. A 30 s timeout here
   would fail on exactly the run that matters, the first one on a fresh clone. */
const READY_TIMEOUT_MS = 180_000;

/* The desktop window is 900x680. The preview frames it at exactly that size on
   a 24 px desk, so the viewport only has to be big enough to hold the frame;
   the shot itself is clipped to `#app`, which IS the window. */
const VIEWPORT = { width: 1200, height: 860 };
const SCALE = 2;

const THEMES = [
  { name: "light", scheme: "light", suffix: "" },
  { name: "dark", scheme: "dark", suffix: "-dark" },
];

const only = process.argv.slice(2).filter((a) => !a.startsWith("-"));
const wanted = only.length ? scenes.filter((s) => only.includes(s.id)) : scenes;

if (only.length && wanted.length !== only.length) {
  const known = new Set(scenes.map((s) => s.id));
  const bad = only.filter((id) => !known.has(id));
  console.error(`unknown scene(s): ${bad.join(", ")}`);
  process.exit(1);
}

async function waitForServer(deadline) {
  for (;;) {
    try {
      const res = await fetch(ORIGIN, { signal: AbortSignal.timeout(2_000) });
      if (res.ok) return;
    } catch {
      // Not up yet. Vite is still optimising dependencies.
    }
    if (Date.now() > deadline) throw new Error(`${ORIGIN} never came up`);
    await new Promise((r) => setTimeout(r, 500));
  }
}

/* Vite's JS entry point directly, rather than `npm run preview:ui`. On Windows
   the npm shim is a `.cmd`, and Node 18.20+/20.12+/24 refuse to spawn one
   without a shell (EINVAL) — spawning through a shell to get around that would
   trade a clear failure for a quoting problem. This is also two processes
   fewer, and it kills cleanly. */
const DESKTOP = join(REPO, "apps", "desktop");

function startPreview() {
  const child = spawn(
    process.execPath,
    [
      join(DESKTOP, "node_modules", "vite", "bin", "vite.js"),
      "--config",
      join(DESKTOP, "vite.preview.config.ts"),
      "--port",
      String(PORT),
      "--strictPort",
    ],
    { cwd: DESKTOP, stdio: ["ignore", "pipe", "pipe"] },
  );
  child.stdout.on("data", () => {});
  child.stderr.on("data", (b) => process.stderr.write(`  preview: ${b}`));
  return child;
}

const main = async () => {
  // A full run owns the directory and starts clean, so a scene deleted from
  // scenes.mjs cannot leave a stale PNG behind that a page still references.
  // A subset run is someone iterating on one click path; wiping the other
  // twenty shots to re-take two would be a rude way to say "only".
  if (!only.length) rmSync(RAW, { recursive: true, force: true });
  mkdirSync(RAW, { recursive: true });

  // Reuse a preview that is already running — `npm run preview:ui` in another
  // terminal is the normal way to work on the app, and starting a second one on
  // a strict port would only fail.
  const already = await fetch(ORIGIN, { signal: AbortSignal.timeout(1_500) })
    .then((r) => r.ok)
    .catch(() => false);

  console.log(
    already
      ? `reusing the preview already serving ${ORIGIN}`
      : `starting the desktop UI preview on ${ORIGIN} …`,
  );
  const server = already ? null : startPreview();
  const stop = () => {
    if (server && !server.killed) server.kill();
  };
  process.on("exit", stop);
  process.on("SIGINT", () => {
    stop();
    process.exit(130);
  });

  let failures = 0;
  try {
    await waitForServer(Date.now() + READY_TIMEOUT_MS);
    console.log("preview is up\n");

    const browser = await chromium.launch();
    try {
      for (const theme of THEMES) {
        const context = await browser.newContext({
          viewport: VIEWPORT,
          deviceScaleFactor: SCALE,
          colorScheme: theme.scheme,
          reducedMotion: "reduce",
        });

        for (const scene of wanted) {
          const file = join(RAW, `${scene.id}${theme.suffix}.png`);
          const page = await context.newPage();
          try {
            await page.goto(ORIGIN, { waitUntil: "domcontentloaded" });
            await page.locator("#app").waitFor({ state: "visible" });
            await scene.run(page);
            await page.locator("#app").screenshot({ path: file, animations: "disabled" });
            console.log(`  ✓ ${scene.id} · ${theme.name}`);
          } catch (err) {
            failures += 1;
            console.error(`  ✗ ${scene.id} · ${theme.name} — ${err.message.split("\n")[0]}`);
            // A failed scene still gets a picture, so the click path can be
            // debugged from what the app actually showed.
            await page
              .locator("#app")
              .screenshot({ path: join(RAW, `FAILED-${scene.id}${theme.suffix}.png`) })
              .catch(() => {});
          } finally {
            await page.close();
          }
        }

        await context.close();
      }
    } finally {
      await browser.close();
    }
  } finally {
    stop();
  }

  const total = wanted.length * THEMES.length;
  console.log(`\n${total - failures}/${total} shots written to tools/shots/raw`);
  if (failures) process.exitCode = 1;
};

await main();
