#!/usr/bin/env node
/* ============================================================================
   write-release-manifest — put the real hashes into site/src/data/release.json.

   Called by .github/workflows/release.yml with the SHA256SUMS the bundle jobs
   produced. Never run by hand on a release: the whole point is that the number
   on the download page came from the bytes, not from someone reading a terminal
   and typing it back in.

   `site/src/data/release.json` carries a placeholder hash — 64 zeroes — for
   every artifact, and `site/scripts/gates.mjs` fails the site build if one of
   those reaches a page while `state` is "published". So the failure mode of
   forgetting this step is a red build, not a download page telling people to
   verify against a hash that verifies nothing.

     node release/write-release-manifest.mjs SHA256SUMS [--date YYYY-MM-DD]
     node release/write-release-manifest.mjs SHA256SUMS --check [--date YYYY-MM-DD]

   MATCHING IS BY FILENAME AND NOTHING ELSE. An artifact in release.json with no
   file, or a file with no entry, is an error rather than a warning: the first
   means the download page offers something the release does not contain, and
   the second means the release contains something nobody reviewed.
   ========================================================================== */

import { readFileSync, writeFileSync, statSync, existsSync } from 'node:fs';
import { join, dirname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = join(ROOT, 'site', 'src', 'data', 'release.json');

const sumsPath = process.argv[2];
if (!sumsPath) {
  console.error(
    '  usage: node release/write-release-manifest.mjs <SHA256SUMS> [--check] [--date YYYY-MM-DD]',
  );
  process.exit(2);
}
if (!existsSync(sumsPath)) {
  console.error(`  ${sumsPath} does not exist`);
  process.exit(2);
}

/* `sha256sum` prints "<hash>  <path>", two spaces, path possibly nested. Only
   the basename is used: the path is wherever the CI runner happened to put the
   downloaded artifact, and it is not stable enough to key on. */
const built = new Map();
for (const line of readFileSync(sumsPath, 'utf8').split('\n')) {
  const m = line.match(/^([0-9a-f]{64})\s+\*?(.+)$/i);
  if (!m) continue;
  const name = basename(m[2].trim());
  if (built.has(name)) {
    console.error(`  two files named ${name} in the same release — refusing to guess`);
    process.exit(1);
  }
  built.set(name, { sha256: m[1].toLowerCase(), path: m[2].trim() });
}

const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8'));
const dateFlag = process.argv.indexOf('--date');
const releaseDate = dateFlag === -1 ? manifest.date : process.argv[dateFlag + 1];
if (!/^\d{4}-\d{2}-\d{2}$/.test(releaseDate ?? '')) {
  console.error('  --date must be followed by YYYY-MM-DD');
  process.exit(2);
}

const missing = [];
const next = {
  ...manifest,
  state: 'published',
  date: releaseDate,
  artifacts: manifest.artifacts.map((a) => {
    const hit = built.get(a.file);
    if (!hit) {
      missing.push(a.file);
      return a;
    }
    return { ...a, bytes: statSync(hit.path).size, sha256: hit.sha256 };
  }),
};

/* Both directions. An unmatched FILE is as much a problem as an unmatched
   entry: it means the build produced an artifact the download page will never
   offer, and nobody would notice until someone asked where it went. */
const declared = new Set(manifest.artifacts.map((a) => a.file));
const undeclared = [...built.keys()].filter((f) => !declared.has(f));

if (missing.length || undeclared.length) {
  if (missing.length) {
    console.error('  declared in release.json, not built:');
    for (const f of missing) console.error(`    ${f}`);
  }
  if (undeclared.length) {
    console.error('  built, not declared in release.json:');
    for (const f of undeclared) console.error(`    ${f}`);
  }
  process.exit(1);
}

const text = `${JSON.stringify(next, null, 2)}\n`;

if (process.argv.includes('--check')) {
  const current = readFileSync(MANIFEST, 'utf8');
  if (current === text) {
    console.log(`  release.json matches the built artifacts (${built.size})`);
    process.exit(0);
  }
  console.error('  release.json does not match the built artifacts');
  process.exit(1);
}

writeFileSync(MANIFEST, text, 'utf8');
console.log(`  release.json written: ${built.size} artifacts, state=published`);
