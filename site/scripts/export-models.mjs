#!/usr/bin/env node
/* ============================================================================
   export-models — pull the AI capability table OUT OF models.toml.

   The features page carried this table by hand, and by 2026-08-27 it had
   drifted into four separate untruths on one public page:

     * two cells contained the literal strings `[size]` and `[verify]`,
       shipped as production copy;
     * it listed **Kokoro-82M** for narration, a row removed from the registry
       because its hash was never pinned, its `source` was a repository page
       rather than an artifact, and no tool could reach it;
     * it listed **BiRefNet-lite**, which has no ONNX export that loads on the
       CPU provider — MODNet fills that slot and models.toml says so;
     * it listed **Granite-Docling-258M**, which is not in the registry at all,
       with a confident "248 MB" beside it.

   Every one of those is the same failure as a hand-transcribed format table:
   a page describing a product that does not exist. So the table is generated,
   the sizes are summed from the rows that will actually be fetched, and a
   model that is not in models.toml cannot appear on the site.

     node scripts/export-models.mjs

   Writes src/data/models.json.
   ========================================================================== */

import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const TOML = join(ROOT, '..', 'models.toml');

/* ---- a very small TOML reader, for the shape this one file has ----------
   Not a general parser and not pretending to be. models.toml is arrays of
   tables with flat string/int/bool/array-of-string values, and pulling in a
   dependency to read one file the repo owns would be the larger risk. If the
   file grows a shape this cannot read, the assertions below fail loudly rather
   than emitting a half-table. */
function parseTables(text) {
  const out = { model: [], feature: [] };
  let current = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;

    const header = /^\[\[(\w+)\]\]$/.exec(line);
    if (header) {
      current = {};
      if (!out[header[1]]) out[header[1]] = [];
      out[header[1]].push(current);
      continue;
    }
    if (current === null) continue;

    const kv = /^(\w+)\s*=\s*(.+)$/.exec(line);
    if (!kv) continue;
    const [, key, rawValue] = kv;
    const value = rawValue.trim();

    if (value.startsWith('[')) {
      current[key] = [...value.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
    } else if (value.startsWith('"')) {
      current[key] = value.slice(1, value.lastIndexOf('"'));
    } else if (value === 'true' || value === 'false') {
      current[key] = value === 'true';
    } else {
      current[key] = Number(value);
    }
  }
  return out;
}

const { model: models, feature: features } = parseTables(readFileSync(TOML, 'utf8'));

if (models.length === 0 || features.length === 0) {
  throw new Error('models.toml parsed to nothing — the reader above needs updating');
}

const byId = new Map(models.map((m) => [m.name, m]));

/**
 * The page a person can read, derived from the artifact URL we pin.
 *
 * models.toml pins a file — a `.onnx`, a `.bin`, a `.json` — because that is
 * what the hash covers. A reader wants the model card that file belongs to, so
 * the repository root is what gets linked. Anything whose shape is not
 * recognised returns null and renders as plain text rather than as a guess.
 */
function home(source) {
  if (!source) return null;
  let u;
  try {
    u = new URL(source);
  } catch {
    return null;
  }
  const seg = u.pathname.split('/').filter(Boolean);
  if (u.hostname === 'huggingface.co' && seg.length >= 2) {
    return `https://huggingface.co/${seg[0]}/${seg[1]}`;
  }
  if ((u.hostname === 'github.com' || u.hostname === 'raw.githubusercontent.com') && seg.length >= 2) {
    return `https://github.com/${seg[0]}/${seg[1]}`;
  }
  return null;
}

function hostOf(source) {
  const h = home(source);
  if (!h) return null;
  return h.startsWith('https://huggingface.co') ? 'Hugging Face' : 'GitHub';
}

/** Bytes, at the precision a person reads. */
function mb(bytes) {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}

const rows = features.map((f) => {
  const parts = f.models.map((id) => {
    const row = byId.get(id);
    if (!row) throw new Error(`feature ${f.id} names ${id}, which is not in models.toml`);
    return row;
  });

  const licences = [...new Set(parts.map((p) => p.licence))].sort();
  return {
    id: f.id,
    title: f.title,
    does: f.does,
    // The artifacts, named. A capability is several files and the page said so
    // for OCR and nothing else; naming them all is what makes the size honest.
    models: parts.map((p) => `${p.name} ${p.version}`),
    // The same artifacts with a link to where each one actually comes from.
    // NOT "the Hugging Face link" for all of them: two of these are published
    // on GitHub, and pointing a GitHub-hosted model at a Hugging Face page
    // would be a fabricated citation on the one table whose whole job is to
    // say where the weights came from. `home()` derives the human page from
    // the pinned artifact URL and returns null when it cannot.
    artifacts: parts.map((p) => ({
      name: p.name,
      version: p.version,
      licence: p.licence,
      href: home(p.source),
      host: hostOf(p.source),
    })),
    sizeBytes: parts.reduce((n, p) => n + (p.size_bytes || 0), 0),
    size: mb(parts.reduce((n, p) => n + (p.size_bytes || 0), 0)),
    licence: licences.join(' + '),
    // Whether anything in the product can reach it. A capability with no tool
    // is listed and never offered, and the page has to say which is which
    // rather than implying all of them work.
    usable: typeof f.tool === 'string' && f.tool.length > 0,
  };
});

const payload = {
  generated_by: 'scripts/export-models.mjs from models.toml',
  features: rows,
};

writeFileSync(
  join(ROOT, 'src', 'data', 'models.json'),
  JSON.stringify(payload, null, 2) + '\n',
  'utf8',
);

const usable = rows.filter((r) => r.usable).length;
console.log(
  `  ${rows.length} capabilities over ${models.length} artifacts` +
    ` (${usable} reachable), ${mb(rows.reduce((n, r) => n + r.sizeBytes, 0))} total`,
);
