#!/usr/bin/env node
/* ============================================================================
   render-installer-text — the page the installer shows before it copies a byte.

   THE FOURTH SURFACE. `release/privacy.json` already feeds openconvert.dev/privacy,
   the README's privacy section, and the app's settings screen, because three
   hand-maintained copies of a privacy statement are three statements that will
   disagree. The installer is the surface most people read FIRST and the one
   least likely to be updated by hand, so it is generated from the same file.

   It renders three things, in the order someone installing software actually
   wants them:

     1. What this program does, in four lines.
     2. What it does with your data — verbatim from privacy.json.
     3. What the installer itself will do to this machine, including the
        things it will NOT do. A converter that quietly seizes every .pdf
        association is the kind of install people uninstall in anger, so the
        absence of that is stated rather than left to be discovered.

   Then the Apache-2.0 text, because the NSIS licence page is where a licence
   belongs and a page that shows only marketing is not a licence page.

     node release/render-installer-text.mjs          # write it
     node release/render-installer-text.mjs --check  # fail if stale (CI)
   ========================================================================== */

import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'apps', 'desktop', 'src-tauri', 'installer', 'LICENSE-AND-PRIVACY.txt');

const privacy = JSON.parse(readFileSync(join(ROOT, 'release', 'privacy.json'), 'utf8'));
const modelsToml = readFileSync(join(ROOT, 'models.toml'), 'utf8');
const licence = readFileSync(join(ROOT, 'LICENSE'), 'utf8').trimEnd();
const release = JSON.parse(readFileSync(join(ROOT, 'site', 'src', 'data', 'release.json'), 'utf8'));

/* ASCII only.

   The first version of this comment claimed the transliteration was load-bearing
   because NSIS reads a licence file as ANSI without a BOM. Then the installer
   was built and the bundled copy measured 15,100 bytes against this file's
   15,097: **Tauri prepends the BOM itself**, so UTF-8 would have rendered fine.
   The claim was wrong and is corrected rather than quietly deleted.

   ASCII stays for a smaller and true reason: this text is the lowest common
   denominator across the NSIS page, anything that later renders it as a plain
   file, and a terminal. Nothing in a privacy statement needs a typographic dash.
   No WORD changes here -- it is the statement's own text with dashes and quotes
   spelled in ASCII. */
function ascii(text) {
  return text
    .replace(/—/g, ' -- ')
    .replace(/–/g, '-')
    .replace(/[‘’]/g, "'")
    .replace(/[“”]/g, '"')
    .replace(/…/g, '...')
    .replace(/ /g, ' ')
    .replace(/\s{2,}/g, ' ');
}

/* Hard-wrapped, because the NSIS licence control does not reflow: a paragraph
   written as one long line is shown as one long line with a horizontal
   scrollbar, which is how licence pages become things nobody reads. */
const WIDTH = 76;
function wrap(text, indent = '') {
  const words = ascii(text).split(/\s+/);
  const lines = [];
  let line = indent;
  for (const w of words) {
    if (line.length + w.length + 1 > WIDTH && line.trim()) {
      lines.push(line);
      line = indent + w;
    } else {
      line = line.trim() ? `${line} ${w}` : indent + w;
    }
  }
  if (line.trim()) lines.push(line);
  return lines;
}

const rule = (ch = '=') => ch.repeat(WIDTH);

/* ---------------------------------------------------------------------------
   The AI capabilities, read from models.toml — the same table the app's chooser
   is built from, and the reason the sizes on this page are the sizes that get
   fetched. A number typed in by hand here would be a number that drifts the
   first time a model is repinned.

   A deliberately small TOML reader rather than a dependency: this script has
   none, the site build has none, and adding one to print six lines would be the
   wrong trade. It understands exactly what models.toml uses — `[[section]]`
   headers, `key = "string"`, `key = 123`, and `key = ["a", "b"]`.
   ------------------------------------------------------------------------- */
function readTomlTables(text, wanted) {
  const tables = [];
  let current = null;
  for (const raw of text.split(String.fromCharCode(10))) {
    const line = raw.replace(/(^|\s)#.*$/, '').trim();
    if (!line) continue;
    const header = line.match(/^\[\[([A-Za-z_][\w-]*)\]\]$/);
    if (header) {
      current = header[1] === wanted ? {} : null;
      if (current) tables.push(current);
      continue;
    }
    if (!current) continue;
    const kv = line.match(/^([A-Za-z_][\w-]*)\s*=\s*(.+)$/);
    if (!kv) continue;
    const [, key, value] = kv;
    if (value.startsWith('[')) {
      current[key] = [...value.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
    } else if (value.startsWith('"')) {
      current[key] = value.slice(1, value.lastIndexOf('"'));
    } else if (/^-?\d+$/.test(value)) {
      current[key] = Number(value);
    } else if (value === 'true' || value === 'false') {
      // BOOLEANS, WHICH THIS READER SILENTLY DROPPED.
      //
      // Every other type here fails loudly enough -- a missing string renders
      // as `undefined` on the page. A missing boolean does not: the caller
      // writes `!== false`, the key is absent, and the row it was meant to
      // remove stays on the page with nothing to show that the registry asked
      // for otherwise. `offered_at_install = false` was exactly that.
      current[key] = value === 'true';
    }
  }
  return tables;
}

const modelRows = readTomlTables(modelsToml, 'model');
const featureRows = readTomlTables(modelsToml, 'feature');

const mb = (bytes) => {
  const m = bytes / 1048576;
  return m >= 100 ? `${Math.round(m)} MB` : `${m.toFixed(1)} MB`;
};

/** Summed from the artifact rows, never written down. */
function featureSize(feature) {
  return (feature.models ?? []).reduce((total, id) => {
    const row = modelRows.find((r) => r.name === id);
    return total + (row?.size_bytes ?? 0);
  }, 0);
}

// Only what the app can actually reach, and only what it will actually offer.
//
// A capability with no `tool` would cost disk space and do nothing, and the
// chooser refuses to offer it -- so this page must not advertise it either.
//
// `offered_at_install` is the second half of the same rule, added when this
// page listed "Remove image backgrounds" three times and totalled 515 MB. A
// tier fetched on demand from inside the tool is not part of what the
// installer is about to put on the machine, so it is not on the page that
// says what the installer is about to put on the machine. THIS PAGE AND THE
// FIRST-RUN SCREEN MUST AGREE, and they agree by reading the same field.
const offerable = featureRows.filter((f) => f.tool && f.offered_at_install !== false);
const offerableTotal = offerable.reduce((n, f) => n + featureSize(f), 0);

const out = [
  rule(),
  `  OPENCONVERT ${release.version}`,
  '  Convert and modify files locally. Nothing is uploaded.',
  rule(),
  '',
  'WHAT YOU ARE INSTALLING',
  '',
  ...wrap(
    'A desktop file converter that runs every conversion on this machine. ' +
      'Each format is handled by an engine confined by the operating system — ' +
      'its own filesystem slice, no network access, resource limits — and every ' +
      'output is written beside a receipt recording which engine ran, what it ' +
      'did, and what it was allowed to do.',
    '  ',
  ),
  '',
  ...wrap(
    'Free software under the Apache License 2.0, reproduced in full below. ' +
      'There is no account, no subscription, and no paid tier of the program.',
    '  ',
  ),
  '',
  '',
  'PRIVACY',
  '',
  ...wrap(
    `The same words as openconvert.dev/privacy, revised ${privacy.revised}. The ` +
      'section about the website itself is left out here and is on that page; ' +
      'everything the PROGRAM does is below, in full.',
    '  ',
  ),
  '',
  ...privacy.sections
    .filter((s) => s.id !== 'website')
    .flatMap((s) => [...wrap(`${s.heading}. ${s.body}`, '  '), '']),
  '',
  'WHAT THIS INSTALLER DOES',
  '',
  '  It will:',
  '',
  '    - Ask where to install, and whether to install for you alone or for',
  '      every user of this computer. Both are your choice on the next screens.',
  '    - Copy the program and its conversion engines into that folder.',
  '    - Create a Start menu shortcut.',
  '    - Register an uninstaller with Windows.',
  '',
  '  It will NOT:',
  '',
  '    - Take over file associations. OpenConvert does not claim .pdf, .jpg or',
  '      any other extension. Opening a file with it stays your decision.',
  '    - Install a service, a background process, a startup entry, a browser',
  '      extension, or a shell extension.',
  '    - Bundle other software, toolbars, or offers.',
  '    - Send anything anywhere during installation.',
  '',
  ...wrap(
    'Settings, history and receipts are written under your user profile the ' +
      'first time you run the program, never by this installer. Uninstalling ' +
      'offers to remove them.',
    '  ',
  ),
  '',
  '',
  'AI FEATURES ARE OPTIONAL, AND NONE ARE INCLUDED',
  '',
  ...wrap(
    'This installer contains no AI models. The first time you run OpenConvert it ' +
      'asks which of these you want, if any, and fetches only those. Each one ' +
      'runs entirely on this machine once it is here. You can add or remove ' +
      'them at any time in Settings.',
    '  ',
  ),
  '',
  ...offerable.flatMap((f) => [
    `    ${f.title}  --  ${mb(featureSize(f))}`,
    ...wrap(f.does, '      '),
    '',
  ]),
  ...wrap(`All of them together: ${mb(offerableTotal)}.`, '  '),
  '',
  '',
  'WHEN THIS PROGRAM USES THE NETWORK',
  '',
  ...wrap(
    'Two things, both about AI features and both started by you: fetching a ' +
      'feature you chose, and checking whether a feature you already have has a ' +
      'newer version. Nothing else in the program opens a connection -- no ' +
      'telemetry, no crash reports, no licence check, no periodic request of any ' +
      'kind, and no setting to switch any of it off, because none of it is in ' +
      'the build. Conversion engines reach nothing at all: each runs in a ' +
      'sandbox with networking denied by the operating system.',
    '  ',
  ),
  '',
  '',
  rule('-'),
  '  APACHE LICENSE, VERSION 2.0',
  rule('-'),
  '',
  licence,
  '',
];

// Keep the checked-in artifact byte-stable on every runner. Git stores text as
// LF; Tauri handles the Windows installer encoding when it bundles this file.
// Emitting CRLF here made `--check` pass only in an autocrlf working tree and
// fail in every clean GitHub Actions checkout, including Windows.
const text = out.join('\n');

if (process.argv.includes('--check')) {
  const current = existsSync(OUT) ? readFileSync(OUT, 'utf8') : null;
  if (current === text) {
    console.log('  installer text matches privacy.json + LICENSE');
    process.exit(0);
  }
  console.error(
    current === null
      ? '  installer text is missing — run `node release/render-installer-text.mjs`'
      : '  installer text is stale — run `node release/render-installer-text.mjs`',
  );
  process.exit(1);
}

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, text, 'utf8');
console.log(
  `  installer text written (${privacy.sections.length} privacy sections, ${text.length} bytes)`,
);
