#!/usr/bin/env node
/* ============================================================================
   export-receipt — put a REAL receipt on the site.

   10-WEBSITE §7, A6: "always generated from a real run, never hand-written. A
   receipt on the marketing site that does not match what the binary emits is
   the one inconsistency this product cannot survive."

   So this builds a fixture, converts it, and writes exactly what the binary
   printed. The isolation string in the output is the one the WORKER read back
   about itself after applying its own confinement — not a claim the host made
   on its behalf — which is why it names what did not engage as well as what did.

     node scripts/export-receipt.mjs [path-to-openconvert]

   Writes src/data/receipt.json.
   ========================================================================== */

import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateRawSync, crc32 } from 'node:zlib';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');

const BIN =
  process.argv[2] ||
  process.env.OPENCONVERT_BIN ||
  join(ROOT, '..', 'target', 'debug', process.platform === 'win32' ? 'openconvert.exe' : 'openconvert');

/* ---- a minimal stored-entry ZIP, built here so the run is reproducible --- */
function zip(entries) {
  const locals = [];
  const central = [];
  let offset = 0;
  for (const [name, text] of entries) {
    const nameBuf = Buffer.from(name, 'utf8');
    const data = Buffer.from(text, 'utf8');
    const sum = crc32(data);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0, 6); // flags
    local.writeUInt16LE(0, 8); // stored
    local.writeUInt32LE(sum, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    locals.push(local, nameBuf, data);

    const cen = Buffer.alloc(46);
    cen.writeUInt32LE(0x02014b50, 0);
    cen.writeUInt16LE(20, 4);
    cen.writeUInt16LE(20, 6);
    cen.writeUInt32LE(sum, 16);
    cen.writeUInt32LE(data.length, 20);
    cen.writeUInt32LE(data.length, 24);
    cen.writeUInt16LE(nameBuf.length, 28);
    cen.writeUInt32LE(offset, 42);
    central.push(cen, nameBuf);

    offset += 30 + nameBuf.length + data.length;
  }
  const centralBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, centralBuf, end]);
}

const dir = mkdtempSync(join(tmpdir(), 'openconvert-receipt-'));
try {
  // Named RELATIVE and converted from `dir`. Handing the binary an absolute
  // path puts an absolute path in `output`, and a receipt often ends up in the
  // same shared folder as the file it describes.
  writeFileSync(
    join(dir, 'bundle.zip'),
    zip([
      ['notes.txt', 'one\ntwo\n'],
      ['data/rows.csv', 'name,amount\nashcombe,4820\n'],
    ]),
  );

  const stdout = execFileSync(BIN, ['convert', 'bundle.zip', '-t', 'tar', '--json'], {
    encoding: 'utf8',
    cwd: dir,
  });
  const receipt = JSON.parse(stdout);

  writeFileSync(
    join(ROOT, 'src', 'data', 'receipt.json'),
    JSON.stringify(
      {
        generated_by: 'scripts/export-receipt.mjs',
        command: 'openconvert convert bundle.zip -t tar --json',
        platform: `${process.platform} ${process.arch}`,
        receipt,
      },
      null,
      2,
    ) + '\n',
    'utf8',
  );

  console.log(`  receipt captured: ${receipt.steps.length} step(s), class ${receipt.class}`);
  console.log(`  isolation: ${receipt.steps[0].isolation}`);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
