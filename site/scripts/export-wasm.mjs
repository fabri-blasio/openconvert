#!/usr/bin/env node
/* ============================================================================
   export-wasm — build the browser executor and prove it answers before it
   ships.

   `openconvert-core` compiles to wasm32 unchanged, which is the property
   03 §4 claims for it and the reason the homepage can route a visitor's file
   with the same code the CLI routes with. This script builds that artifact,
   copies it into public/, and then RUNS it against fixtures — because a wasm
   file that loads and returns nonsense fails silently, and the whole point of
   the island is that the page and the binary agree.

     node scripts/export-wasm.mjs [--skip-build]

   Writes public/openconvert.wasm and src/data/wasm.json (the size, for the gate).
   ========================================================================== */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const REPO = join(ROOT, '..');

const ARTIFACT = join(
  REPO,
  'target',
  'wasm32-unknown-unknown',
  'wasm',
  'openconvert_wasm.wasm',
);
const DEST = join(ROOT, 'public', 'openconvert.wasm');

// 10-WEBSITE §9: the homepage island, lazy-loaded, ≤ 20 KB.
const BUDGET_KB = 20;

if (!process.argv.includes('--skip-build')) {
  console.log('  building openconvert-wasm (profile: wasm)…');
  execFileSync(
    'cargo',
    [
      'build',
      '-p',
      'openconvert-wasm',
      '--target',
      'wasm32-unknown-unknown',
      '--profile',
      'wasm',
    ],
    { cwd: REPO, stdio: ['ignore', 'inherit', 'inherit'] },
  );
}

const bytes = readFileSync(ARTIFACT);
const gz = gzipSync(bytes, { level: 9 }).length;

/* ---- run it, against real signatures ----------------------------------- */
const { instance } = await WebAssembly.instantiate(bytes, {});
const x = instance.exports;
const mem = () => new Uint8Array(x.memory.buffer);

function ask(head, target = 0xffffffff) {
  const ptr = x.alloc(head.length);
  mem().set(head, ptr);
  const out = x.plan(ptr, head.length, target);
  const len = x.last_len();
  const json = new TextDecoder().decode(mem().subarray(out, out + len));
  x.dealloc(ptr, head.length);
  return JSON.parse(json);
}

const b = (...parts) =>
  Uint8Array.from(
    parts.flatMap((p) => (typeof p === 'string' ? [...p].map((c) => c.charCodeAt(0)) : p)),
  );

const cases = [
  { name: 'png', head: b([0x89], 'PNG\r\n', [0x1a], '\n'), expect: 'png' },
  { name: 'jpeg', head: b([0xff, 0xd8, 0xff, 0xe0]), expect: 'jpeg' },
  { name: 'gif', head: b('GIF89a'), expect: 'gif' },
  { name: 'zip', head: b('PK', [0x03, 0x04], 'hello.txt'), expect: 'zip' },
  { name: 'pdf', head: b('%PDF-1.7'), expect: 'pdf' },
  { name: 'json', head: b('{"a": 1}'), expect: 'json' },
  { name: 'csv', head: b('name,amount\nx,1\n'), expect: 'csv' },
  { name: 'postscript (an EPS)', head: b('%!PS-Adobe-3.0'), expect: 'postscript' },
  { name: 'empty', head: b(), expect: 'unknown' },
];

let bad = 0;
for (const c of cases) {
  const got = ask(c.head);
  if (got.detected !== c.expect) {
    console.error(`  MISMATCH ${c.name}: expected ${c.expect}, got ${got.detected}`);
    bad++;
  }
}

// The page must offer the same routes the CLI prints. This is the check that
// would catch the site and the binary drifting apart.
const png = ask(b([0x89], 'PNG\r\n', [0x1a], '\n'));
const targets = png.targets.map((t) => t.id).sort();
const expected = ['jpeg', 'webp'].sort();
const missing = expected.filter((t) => !targets.includes(t));
if (missing.length) {
  console.error(`  png targets missing ${missing.join(', ')} (got ${targets.join(', ')})`);
  bad++;
}

// A page cannot finish a HEIC decode — libde265 is a C library in a worker
// process. If the browser executor ever offers it, `available` has stopped
// meaning "a plan routed here finishes".
if (targets.includes('heic')) {
  console.error('  png → heic offered in the browser, which cannot finish it');
  bad++;
}

if (bad) {
  console.error(`\n  ${bad} check(s) failed — not writing the artifact.\n`);
  process.exit(1);
}

/* ---- ship it ------------------------------------------------------------ */
mkdirSync(join(ROOT, 'public'), { recursive: true });
copyFileSync(ARTIFACT, DEST);

writeFileSync(
  join(ROOT, 'src', 'data', 'wasm.json'),
  JSON.stringify(
    {
      generated_by: 'scripts/export-wasm.mjs',
      file: '/openconvert.wasm',
      bytes: bytes.length,
      gzip_bytes: gz,
      budget_kb: BUDGET_KB,
      head_bytes: x.head_bytes(),
      exports: Object.keys(x).filter((k) => typeof x[k] === 'function').sort(),
    },
    null,
    2,
  ) + '\n',
  'utf8',
);

const kb = (gz / 1024).toFixed(1);
console.log(`  ${cases.length} signatures verified through the wasm module`);
console.log(`  head_bytes = ${x.head_bytes()} (derived from the format table)`);
console.log(
  `  openconvert.wasm ${(bytes.length / 1024).toFixed(1)} KB raw, ${kb} KB gzipped / ${BUDGET_KB} KB`,
);
if (gz / 1024 > BUDGET_KB) {
  console.error('  OVER BUDGET');
  process.exit(1);
}
