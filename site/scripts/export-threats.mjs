#!/usr/bin/env node
/* ============================================================================
   export-threats — lift 09-THREAT-MODEL §8 onto /security VERBATIM.

   10-WEBSITE §5 makes this the one section published without softening, and
   the reliable way to keep that true is to never retype it. The extractor takes
   the table rows between the §8 heading and the §9 heading and writes them
   unaltered; the page renders the inline markdown and adds nothing.

   Re-running this after an edit to the threat model is the whole maintenance
   story. If a row here reads badly, the row in the threat model reads badly.

     node scripts/export-threats.mjs

   Writes src/data/not-covered.json.
   ========================================================================== */

import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const SRC = join(ROOT, '..', '09-THREAT-MODEL.md');

const lines = readFileSync(SRC, 'utf8').split(/\r?\n/);
const start = lines.findIndex((l) => /^##\s*8\.\s*What we don't protect against/.test(l));
if (start < 0) throw new Error('09-THREAT-MODEL.md §8 heading not found');
const end = lines.findIndex((l, i) => i > start && /^##\s*9\./.test(l));

const rows = [];
for (const raw of lines.slice(start, end < 0 ? undefined : end)) {
  const line = raw.trim();
  if (!line.startsWith('|')) continue;
  const cells = line.replace(/^\||\|$/g, '').split('|').map((c) => c.trim());
  if (cells.length !== 2) continue;
  if (/^[-: ]+$/.test(cells[0])) continue; // separator
  if (cells[0] === 'Not covered') continue; // header
  rows.push({ item: cells[0], why: cells[1] });
}
if (rows.length === 0) throw new Error('no §8 rows extracted');

writeFileSync(
  join(ROOT, 'src', 'data', 'not-covered.json'),
  JSON.stringify(
    {
      _comment:
        'VERBATIM from 09-THREAT-MODEL.md §8, extracted by scripts/export-threats.mjs. 10-WEBSITE §5 says this section is published unsoftened, and the way to keep that true is to never retype it.',
      source: '09-THREAT-MODEL.md §8',
      rows,
    },
    null,
    2,
  ) + '\n',
  'utf8',
);

console.log(`  ${rows.length} rows lifted from 09-THREAT-MODEL §8`);
