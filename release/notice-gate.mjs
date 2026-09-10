#!/usr/bin/env node
/* ============================================================================
   notice-gate â€” NOTICE must agree with the tables it claims to be generated
   from, and every LGPL engine must be dynamically linked.

     node release/notice-gate.mjs

   Three checks, and the third is the one that matters:

     1. every engines.toml row appears in NOTICE at its exact version
     2. every models.toml row with enabled = true appears in NOTICE
     3. no LGPL engine is recorded with link_mode = "static"

   Check 3 is the licence obligation itself. The LGPL requires that a user be
   able to substitute their own build of the library, and OpenConvert discharges
   that by shipping each LGPL engine as a replaceable shared library. A row that
   flipped to "static" would be a distribution the project has no right to make,
   and it would be a one-word diff nobody would notice in review.

   Check 1 exists because an attribution file that drifts from the table it was
   generated from is worse than one written by hand: it carries the authority of
   having been generated and none of the accuracy.
   ========================================================================== */

import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (f) => readFileSync(join(ROOT, f), 'utf8');

/** The `[[table]]` subset both licence files use. */
function parseTables(src, table) {
  const rows = [];
  let cur = null;
  for (const raw of src.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;
    if (line === '[[' + table + ']]') {
      cur = {};
      rows.push(cur);
      continue;
    }
    if (/^\[/.test(line)) {
      cur = null;
      continue;
    }
    if (!cur) continue;
    const m = line.match(/^([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*(.+?)\s*$/);
    if (!m) continue;
    const v = m[2];
    cur[m[1]] = /^".*"$/.test(v) ? v.slice(1, -1) : v === 'true' ? true : v === 'false' ? false : v;
  }
  return rows;
}

const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const notice = read('NOTICE');
const engines = parseTables(read('engines.toml'), 'engine');
const models = parseTables(read('models.toml'), 'model');

let failures = 0;
const pass = (m) => console.log('  \x1b[32mPASS\x1b[0m  ' + m);
const fail = (m) => {
  failures++;
  console.log('  \x1b[31mFAIL\x1b[0m  ' + m);
};

console.log('\n\x1b[1m  NOTICE gate\x1b[0m\n');

/* ---- 1. engines.toml âŠ† NOTICE ------------------------------------------ */
const missingEngines = engines.filter(
  (e) => !new RegExp(escape(e.name) + '\\s+' + escape(e.version)).test(notice),
);
missingEngines.length === 0
  ? pass(`every engine attributed at its version  ${engines.length} rows`)
  : fail(
      'engines missing from NOTICE: ' +
        missingEngines.map((e) => `${e.name} ${e.version}`).join(', '),
    );

/* ---- 2. shipped models âŠ† NOTICE ---------------------------------------- */
const shipped = models.filter((m) => m.enabled === true);
const missingModels = shipped.filter((m) => !notice.includes(m.name));
missingModels.length === 0
  ? pass(`every shipped model attributed  ${shipped.length} rows`)
  : fail('models missing from NOTICE: ' + missingModels.map((m) => m.name).join(', '));

/* ---- 3. THE LICENCE OBLIGATION ----------------------------------------- */
const copyleft = engines.filter((e) => /LGPL/.test(e.licence));
const statics = copyleft.filter((e) => e.link_mode !== 'dynamic');
statics.length === 0
  ? pass(`every LGPL engine dynamically linked  ${copyleft.length} of ${engines.length}`)
  : fail(
      'LGPL engines not dynamically linked: ' +
        statics.map((e) => `${e.name} (${e.link_mode})`).join(', '),
    );

// GPL and AGPL are refused whatever the linkage; no link mode discharges them
// for a work we distribute.
const gpl = engines.filter((e) => /(^|[^L])GPL/.test(e.licence) && !/LGPL/.test(e.licence));
gpl.length === 0
  ? pass('no GPL or AGPL engine declared')
  : fail('GPL/AGPL engines declared: ' + gpl.map((e) => `${e.name} (${e.licence})`).join(', '));

console.log('');
if (failures) {
  console.log(`\x1b[31m  ${failures} check(s) failed.\x1b[0m\n`);
  process.exit(1);
}
console.log(`\x1b[32m  NOTICE agrees with engines.toml and models.toml.\x1b[0m\n`);
