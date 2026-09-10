/**
 * Encode the captured PNGs into what the site actually serves. **Not shipped.**
 *
 *     npm run shots:encode
 *
 * Reads `tools/shots/raw/*.png`, writes `public/shots/*.{avif,webp}` and
 * `src/data/shots.json`.
 *
 * **The encoder is the product.** `openconvert convert shot.png -t avif` is the
 * same binary, the same sandboxed worker and the same route table the download
 * page is selling, and it leaves a receipt for every image on the website —
 * kept in `tools/shots/receipts/`. A site whose argument is "every conversion
 * is receipted" should not have its own pictures produced by something else.
 *
 * If the binary is missing the script says so and stops, rather than quietly
 * falling back to a different encoder and leaving `shots.json` claiming one
 * that never ran.
 */

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { scenes } from "./scenes.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const SITE = join(HERE, "..", "..");
const REPO = join(SITE, "..");
const RAW = join(HERE, "raw");
const RECEIPTS = join(HERE, "receipts");
const OUT = join(SITE, "public", "shots");
const DATA = join(SITE, "src", "data", "shots.json");

const FORMATS = ["avif", "webp"];

const cli =
  process.env.OPENCONVERT ??
  [join(REPO, "target", "release", "openconvert.exe"), join(REPO, "target", "release", "openconvert")].find((p) =>
    existsSync(p),
  );

if (!cli) {
  console.error(
    "no openconvert binary found. Build one (`cargo build --release`) or point OPENCONVERT at it.\n" +
      "The site's own images are converted by the product; there is no second encoder to fall back to.",
  );
  process.exit(1);
}

if (!existsSync(RAW) || readdirSync(RAW).filter((f) => f.endsWith(".png")).length === 0) {
  console.error("tools/shots/raw is empty — run `npm run shots` first.");
  process.exit(1);
}

const version = execFileSync(cli, ["--version"], { encoding: "utf8" }).trim();
const sha = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: REPO, encoding: "utf8" }).trim();

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
mkdirSync(RECEIPTS, { recursive: true });

/** `<id>.png` and `<id>-dark.png` are the same scene in two themes. */
const themeOf = (file) => (file.endsWith("-dark.png") ? "dark" : "light");
const idOf = (file) => basename(file, ".png").replace(/-dark$/, "");

const pngs = readdirSync(RAW)
  .filter((f) => f.endsWith(".png") && !f.startsWith("FAILED-"))
  .sort();

/** Width and height straight out of the IHDR chunk. No decoder needed. */
function pngSize(buf) {
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

const kb = (n) => `${(n / 1024).toFixed(0)} KB`;

/**
 * Move a file the worker has just finished writing.
 *
 * `renameSync` fails here with EPERM often enough to matter: on Windows the
 * handle the sandboxed worker used can still be closing, and a virus scanner
 * opens anything that appears in a watched directory. Copy-then-delete with a
 * short backoff survives both, and the delete is allowed to fail — a stray
 * scratch file in `raw/` is not worth ending a twenty-two image run over.
 */
function move(from, to) {
  for (let attempt = 1; ; attempt++) {
    try {
      copyFileSync(from, to);
      break;
    } catch (err) {
      if (attempt === 10) throw err;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 200);
    }
  }
  try {
    unlinkSync(from);
  } catch {
    // Held by something. It is scratch; the next full run wipes the directory.
  }
}

/**
 * One conversion, with one retry.
 *
 * AVIF encoding at 1800×1360 takes 10–20 s in the sandboxed worker, and under
 * load the host occasionally gives up on it with "oc-images stopped
 * responding". That is a worker liveness timeout, not a bad image — the same
 * file converts on the next attempt. Retrying once beats failing a twenty-two
 * image run on a stall, and a second failure is reported with the worker's own
 * words rather than a node stack.
 */
function convert(input, format) {
  for (let attempt = 1; ; attempt++) {
    try {
      execFileSync(cli, ["convert", input, "-t", format], { stdio: "pipe" });
      return;
    } catch (err) {
      const said = String(err.stderr ?? "").trim() || err.message;
      if (attempt === 2) {
        console.error(`\n${basename(input)} → ${format}: ${said}`);
        process.exit(1);
      }
      console.warn(`  retrying ${basename(input)} → ${format} (${said})`);
    }
  }
}

/** Encoded byte counts, keyed id → theme → format. */
const sizes = new Map();
let dimensions = null;

for (const png of pngs) {
  const id = idOf(png);
  const theme = themeOf(png);
  const stem = basename(png, ".png");

  for (const format of FORMATS) {
    // The CLI writes beside its input, which is what we want: raw/ is scratch.
    convert(join(RAW, png), format);

    const produced = join(RAW, `${stem}.${format}`);
    const receipt = `${produced}.receipt.json`;
    const target = join(OUT, `${stem}.${format}`);

    move(produced, target);
    if (existsSync(receipt)) move(receipt, join(RECEIPTS, `${stem}.${format}.receipt.json`));

    if (!sizes.has(id)) sizes.set(id, {});
    const byTheme = sizes.get(id);
    byTheme[theme] ??= {};
    byTheme[theme][format] = statSync(target).size;
  }

  dimensions ??= pngSize(readFileSync(join(RAW, png)));
  console.log(`  ${stem}  ${FORMATS.map((f) => `${f} ${kb(sizes.get(id)[theme][f])}`).join("  ")}`);
}

const missing = scenes.filter((s) => !sizes.has(s.id)).map((s) => s.id);
const orphans = [...sizes.keys()].filter((id) => !scenes.some((s) => s.id === id));

const shots = scenes
  .filter((s) => sizes.has(s.id))
  .map((s) => ({
    id: s.id,
    alt: s.alt,
    width: dimensions.width,
    height: dimensions.height,
    themes: sizes.get(s.id),
  }));

writeFileSync(
  DATA,
  `${JSON.stringify(
    {
      _comment:
        "WRITTEN BY site/tools/shots/encode.mjs, never by hand. Alt text is authored in tools/shots/scenes.mjs, " +
        "beside the click path that produces each picture. app_sha is the commit the app was captured at; " +
        "scripts/gates.mjs refuses a build whose screenshots are more than one release behind it.",
      generated_by: "site/tools/shots/encode.mjs",
      encoder: version,
      app_sha: sha,
      captured: new Date().toISOString().slice(0, 10),
      shots,
    },
    null,
    2,
  )}\n`,
);

const total = readdirSync(OUT).reduce((n, f) => n + statSync(join(OUT, f)).size, 0);
console.log(`\n${shots.length} scenes · ${readdirSync(OUT).length} files · ${kb(total)} in public/shots`);
if (missing.length) console.warn(`not captured: ${missing.join(", ")} — run \`npm run shots\``);
if (orphans.length) console.warn(`no scene declares: ${orphans.join(", ")}`);
