#!/usr/bin/env node
/* ============================================================================
   export-data — pull the site's factual layer OUT OF THE BINARY.

   10-WEBSITE §4 marks four routes as generated, and the reason is not tidiness:
   a reference page that is hand-transcribed will eventually describe a
   conversion the binary does not have, and on a product whose second pillar is
   telling you what it is about to do, that is the one inconsistency it cannot
   survive.

   So the facts come from `openconvert formats` and `openconvert routes <a> <b>`,
   and the prose stays authored in prose.json keyed by format id. A format whose
   facts change gets new facts on the next export; a format whose prose is wrong
   is a writing problem and stays a writing problem.

     node scripts/export-data.mjs [path-to-openconvert]

   Writes  src/data/formats.json.
   The receipt in src/data/receipt.json is written by scripts/export-receipt.mjs,
   which needs real input files rather than just the binary.
   ========================================================================== */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');

const BIN =
  process.argv[2] ||
  process.env.OPENCONVERT_BIN ||
  join(ROOT, '..', 'target', 'debug', process.platform === 'win32' ? 'openconvert.exe' : 'openconvert');

const run = (...args) =>
  execFileSync(BIN, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

/* ---- 1. every format the binary knows ---------------------------------- */
// FORMAT       KIND       EXT      FROM   TO  PARSER
// png          image      png         8   19  pure Rust (in-process)
//
// The two counts are how many routes START and END at this format. They were
// added to the CLI so a format with 0 and 0 — detected, converted nowhere —
// stops reading as capability on the line where someone meets it, and they are
// captured here for the same reason.
const formats = [];
for (const line of run('formats').split(/\r?\n/).slice(1)) {
  const m = line.trim().match(/^(\S+)\s+(\S+)\s+(\S+)\s+(\d+)\s+(\d+)\s+(.+?)\s*$/);
  if (!m) continue;
  formats.push({
    id: m[1],
    kind: m[2],
    ext: m[3],
    routesFrom: Number(m[4]),
    routesTo: Number(m[5]),
    parser: m[6],
    isolation: /sandboxed/i.test(m[6]) ? 'Sandboxed' : 'InProcess',
  });
}
if (formats.length === 0) throw new Error(`no formats parsed from ${BIN}`);

/* ---- 2. every route between them --------------------------------------- */
// 1. class A
//    steps    [StreamCopy]
//    requires codecs are compatible with the destination container
const routesFor = (from, to) => {
  let out;
  try {
    out = run('routes', from, to);
  } catch {
    return [];
  }
  const routes = [];
  let cur = null;
  for (const raw of out.split(/\r?\n/)) {
    const line = raw.trimEnd();
    let m;
    if ((m = line.match(/^\s*\d+\.\s*class\s+(\S+)/))) {
      cur = { class: m[1], steps: '', requires: '' };
      routes.push(cur);
    } else if (cur && (m = line.match(/^\s*steps\s+(.+)$/))) {
      cur.steps = m[1].replace(/^\[|\]$/g, '');
    } else if (cur && (m = line.match(/^\s*requires\s+(.+)$/))) {
      cur.requires = m[1];
    }
  }
  return routes;
};

let pairs = 0;
for (const f of formats) {
  f.routes = [];
  for (const t of formats) {
    if (t.id === f.id) continue;
    pairs++;
    const rs = routesFor(f.id, t.id);
    if (rs.length === 0) continue;
    f.routes.push({ to: t.id, kind: t.kind, options: rs });
  }
}

/* ---- 3. merge the authored prose --------------------------------------- */
const prose = JSON.parse(readFileSync(join(ROOT, 'src', 'data', 'prose.json'), 'utf8'));
const missing = [];
for (const f of formats) {
  const p = prose[f.id];
  if (!p) {
    missing.push(f.id);
    continue;
  }
  Object.assign(f, p);
}

const version = run('--version').trim();
const payload = {
  generated_by: `scripts/export-data.mjs from ${version}`,
  formats,
};

writeFileSync(
  join(ROOT, 'src', 'data', 'formats.json'),
  JSON.stringify(payload, null, 2) + '\n',
  'utf8',
);

console.log(`  ${formats.length} formats, ${pairs} pairs probed, from ${version}`);
console.log(
  `  ${formats.reduce((n, f) => n + f.routes.length, 0)} routes recorded` +
    (missing.length ? `\n  no prose for: ${missing.join(', ')}` : ''),
);
