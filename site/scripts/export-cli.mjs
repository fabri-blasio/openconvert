#!/usr/bin/env node
/* ============================================================================
   export-cli — capture REAL CLI transcripts for /docs.

   10-WEBSITE §5 asks /docs for "a real invocation and real output" per command,
   and for the flag list to come from the parser so it cannot drift. So every
   block on that page is stdout captured here, against fixtures this script
   builds, and checked in beside the page.

   A transcript that was true once and is edited afterwards is worse than no
   transcript, so nothing in the output file is touched by hand.

     node scripts/export-cli.mjs [path-to-openconvert]

   Writes src/data/cli.json.
   ========================================================================== */

import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync, copyFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync, crc32 } from 'node:zlib';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');

const BIN =
  process.argv[2] ||
  process.env.OPENCONVERT_BIN ||
  join(ROOT, '..', 'target', 'debug', process.platform === 'win32' ? 'openconvert.exe' : 'openconvert');

const dir = mkdtempSync(join(tmpdir(), 'openconvert-docs-'));

/* The binary keeps recipes and batch journals in a per-user state directory,
   chosen from the environment (state/paths.rs). Left alone, this script writes
   into the developer's real one, and the second run captures
   "recipe 'web-thumb' already exists" instead of a save — an error transcript
   published as if it were the happy path, which is exactly the drift the
   generated-transcript rule exists to prevent.

   So state goes in the temp tree with everything else: every run starts from
   an empty one, and `batch --resume` finds the journal this run wrote rather
   than one left over from a previous afternoon. */
const stateDir = join(dir, 'state');
const env = {
  ...process.env,
  LOCALAPPDATA: stateDir, // windows
  XDG_STATE_HOME: stateDir, // linux
  HOME: stateDir, // macos, and the linux fallback
};

/** Run and capture both streams, because usage goes to stderr. */
function capture(args, cwd = dir) {
  try {
    return execFileSync(BIN, args, { encoding: 'utf8', cwd, env, stdio: ['ignore', 'pipe', 'pipe'] })
      .replace(/\r\n/g, '\n')
      .trimEnd();
  } catch (e) {
    return ((e.stdout || '') + (e.stderr || '')).replace(/\r\n/g, '\n').trimEnd();
  }
}

/* ---- fixtures ----------------------------------------------------------- */
function png(w, h) {
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const sum = Buffer.alloc(4);
    sum.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([len, body, sum]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // truecolour
  const rows = [];
  for (let y = 0; y < h; y++) {
    const row = Buffer.alloc(1 + w * 3);
    for (let x = 0; x < w; x++) {
      row[1 + x * 3] = (x * 7) & 0xff;
      row[2 + x * 3] = (y * 11) & 0xff;
      row[3 + x * 3] = 128;
    }
    rows.push(row);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(Buffer.concat(rows))),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

try {
  mkdirSync(stateDir, { recursive: true });
  writeFileSync(join(dir, 'scan.png'), png(96, 64));
  mkdirSync(join(dir, 'shots'));
  copyFileSync(join(dir, 'scan.png'), join(dir, 'shots', 'a.png'));
  copyFileSync(join(dir, 'scan.png'), join(dir, 'shots', 'b.png'));

  /* A second folder, deliberately impure: two images and one file that routes
     nowhere. `shots` stays clean so the real-run transcript on /features can
     show a batch that finishes; `inbox` is what a real folder looks like, and
     its dry run is the only place the site can show a refusal being planned
     rather than described. 12-SITE-PAGES asks #conversions for exactly that. */
  mkdirSync(join(dir, 'inbox'));
  copyFileSync(join(dir, 'scan.png'), join(dir, 'inbox', 'a.png'));
  copyFileSync(join(dir, 'scan.png'), join(dir, 'inbox', 'b.png'));
  writeFileSync(join(dir, 'inbox', 'notes.txt'), `a folder is never all one kind\n`);

  const blocks = {
    version: capture(['--version']),
    usage: capture([]),
    inspect: capture(['inspect', 'scan.png']),
    plan: capture(['plan', 'scan.png', '-t', 'jpeg']),
    convert: capture(['convert', 'scan.png', '-t', 'webp']),
    convert_json: capture(['convert', 'scan.png', '-t', 'jpeg', '--json']),
    routes_streamcopy: capture(['routes', 'mkv', 'mp4']),
    routes_lossless: capture(['routes', 'wav', 'flac']),
    routes_lossy: capture(['routes', 'heic', 'jpeg']),
    formats: capture(['formats']),
    batch_dry_run: capture(['batch', 'inbox', '-t', 'jpeg', '--dry-run']),
    batch: capture(['batch', 'shots', '-t', 'jpeg', '--manifest', 'manifest.json']),
    batch_resume: capture(['batch', 'shots', '-t', 'jpeg', '--resume']),
    recipe_usage: capture(['recipe']),
    recipe_save: capture(['recipe', 'save', 'web-thumb', 'scan.png', '-t', 'webp']),
  };

  // Machine-local paths appear in journal and recipe lines. The site is not the
  // place to publish someone's profile directory, so they are folded to a
  // placeholder — the ONLY edit this script makes, and it is a redaction.
  const home = process.env.USERPROFILE || process.env.HOME || '';
  const redact = (s) =>
    [stateDir, dir, home]
      .filter(Boolean)
      .reduce((acc, root) => acc.split(root).join('~'), s)
      .replace(
        /~[\\/][^\s]*[\\/](journal|recipes)[\\/]/g,
        (_m, kind) => `<app data dir>/${kind}/`,
      );

  for (const k of Object.keys(blocks)) blocks[k] = redact(blocks[k]);

  writeFileSync(
    join(ROOT, 'src', 'data', 'cli.json'),
    JSON.stringify(
      {
        generated_by: 'scripts/export-cli.mjs',
        version: blocks.version,
        platform: `${process.platform} ${process.arch}`,
        blocks,
      },
      null,
      2,
    ) + '\n',
    'utf8',
  );

  console.log(`  ${Object.keys(blocks).length} transcripts captured from ${blocks.version}`);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
