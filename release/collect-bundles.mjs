#!/usr/bin/env node
/* ============================================================================
   collect-bundles — rename what Tauri produced to what /download promises.

   Tauri names bundles for the platform's own conventions: `OpenConvert_0.1.0_x64-
   setup.exe`, `OpenConvert_0.1.0_x64_en-US.msi`, `OpenConvert_0.1.0_aarch64.dmg`,
   `openconvert_0.1.0_amd64.deb`. Four naming schemes, none of which says which
   *target triple* the binary was built for.

   `site/src/data/release.json` names them one way — target-explicit, because a
   download page for a security tool should let someone say out loud which
   binary they are verifying. This script is the join between the two, and it is
   the reason the manifest job downstream can match on filename alone.

     node release/collect-bundles.mjs <target-triple> <out-dir>

   It is strict in both directions, for the same reason the manifest writer is:
   a declared artifact with no file means the download page offers something the
   release does not contain, and a produced file with no declaration means the
   release contains something nobody reviewed. Either is an error here rather
   than a surprise three jobs later.
   ========================================================================== */

import { readFileSync, readdirSync, statSync, copyFileSync, mkdirSync, existsSync } from 'node:fs';
import { join, dirname, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = join(ROOT, 'site', 'src', 'data', 'release.json');

const [target, outDir] = process.argv.slice(2);
if (!target || !outDir) {
  console.error('  usage: node release/collect-bundles.mjs <target-triple> <out-dir>');
  process.exit(2);
}

/* Three ways cargo can place the output, and all three are ordinary.

   `CARGO_TARGET_DIR` wins when set, which is how a shared build cache and every
   containerised build works -- and skipping it is not a harmless omission: the
   Linux bundles built correctly, this script reported "nothing under ..." with
   the path it had guessed, and the artifacts sat somewhere else entirely.
   `--target <triple>` adds a triple directory; a build without one does not.
   The first candidate that exists wins, and the last one is what gets named in
   the error so the message points at a real place. */
const CARGO_TARGET_DIR = process.env.CARGO_TARGET_DIR;
const DEFAULT_TARGET_DIR = join(ROOT, 'apps', 'desktop', 'src-tauri', 'target');
const TARGET_DIR = CARGO_TARGET_DIR ?? DEFAULT_TARGET_DIR;
const CANDIDATES = [
  join(TARGET_DIR, target, 'release', 'bundle'),
  join(TARGET_DIR, 'release', 'bundle'),
];
const BUNDLE_ROOT = CANDIDATES.find((d) => existsSync(d)) ?? CANDIDATES[0];

/** Every file under the bundle tree, at any depth — the layout differs per
 *  bundler (`nsis/`, `msi/`, `dmg/`, `deb/`, `appimage/`) and is not worth
 *  encoding here. */
function walk(dir, out = []) {
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const e of entries) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

const produced = walk(BUNDLE_ROOT);
if (produced.length === 0) {
  console.error(`  nothing under ${BUNDLE_ROOT} — did the build run?`);
  console.error(`  (looked in: ${CANDIDATES.join(', ')})`);
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8'));
const wanted = manifest.artifacts.filter((a) => a.target === target);
if (wanted.length === 0) {
  console.error(`  release.json declares no artifact for ${target}`);
  process.exit(1);
}

/* Matched by EXTENSION within one target, which is unambiguous because no
   target declares two artifacts with the same extension — and if one ever
   does, the duplicate check below turns that into a build failure rather than
   a coin flip. */
const byExt = new Map();
for (const a of wanted) {
  const ext = extname(a.file).toLowerCase();
  if (byExt.has(ext)) {
    console.error(`  ${target} declares two ${ext} artifacts — cannot match by extension`);
    process.exit(1);
  }
  byExt.set(ext, a);
}

mkdirSync(outDir, { recursive: true });

const seen = new Set();
const extras = [];
for (const file of produced) {
  const ext = extname(file).toLowerCase();
  const want = byExt.get(ext);
  if (!want) {
    // `.sig`, update artifacts and other by-products are not extras; only a
    // file whose extension we ship but did not declare is.
    if (['.exe', '.msi', '.dmg', '.deb', '.appimage', '.rpm'].includes(ext)) extras.push(file);
    continue;
  }
  if (seen.has(ext)) {
    console.error(`  two ${ext} files produced for ${target}: ${file}`);
    process.exit(1);
  }
  seen.add(ext);
  copyFileSync(file, join(outDir, want.file));
  console.log(`  ${file.slice(BUNDLE_ROOT.length + 1)}  ->  ${want.file}`);
}

const missing = wanted.filter((a) => !seen.has(extname(a.file).toLowerCase()));
if (missing.length || extras.length) {
  for (const a of missing) console.error(`  declared but not produced: ${a.file}`);
  for (const f of extras) console.error(`  produced but not declared: ${f}`);
  process.exit(1);
}

console.log(`  ${seen.size} artifact(s) collected for ${target}`);
