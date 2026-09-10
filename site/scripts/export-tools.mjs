#!/usr/bin/env node
/* ============================================================================
   export-tools — pull the tool menu OUT OF THE SOURCE OF TRUTH.

   `openconvert formats` and `openconvert routes` gave export-data.mjs the
   conversion layer straight from the binary. The tool menu has no CLI command
   to ask, because it is a GUI surface: `all_tools()` in
   crates/openconvert-run/src/tools.rs is the one list the desktop menu and the
   host dispatcher both read, and `every_named_tool_exists` keeps them paired.

   So this reads that function. Parsing Rust is a bad idea in general and an
   acceptable one here for exactly one reason: it FAILS LOUDLY. Every
   descriptor must yield an id, a category, a title and an `advertised` flag,
   and a block that does not throws. A refactor that changes the shape breaks
   this script rather than quietly shipping a website that lists eleven tools
   when the product ships twenty.

     node scripts/export-tools.mjs

   Writes  src/data/tools.json.
   ========================================================================== */

import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const SRC = join(ROOT, '..', 'crates', 'openconvert-run', 'src', 'tools.rs');

const text = readFileSync(SRC, 'utf8');

/* Only the body of `all_tools()`; the file also holds helpers and tests that
   mention ToolDescriptor and must not be mistaken for menu entries. */
const start = text.indexOf('pub fn all_tools()');
if (start < 0) throw new Error('all_tools() not found in tools.rs');
const body = text.slice(start);

/* Comments carry prose about descriptors that were REMOVED, including their
   literal ids, so they are stripped before anything is matched. */
const code = body
  .replace(/\/\/\/.*$/gm, '')
  .replace(/\/\/.*$/gm, '');

const blocks = code.split(/ToolDescriptor\s*\{/).slice(1);
if (blocks.length === 0) throw new Error('no ToolDescriptor blocks in all_tools()');

const field = (block, name, re) => {
  const m = block.match(re);
  if (!m) throw new Error(`descriptor is missing \`${name}\`:\n${block.slice(0, 240)}`);
  return m[1];
};

const tools = blocks.map((b) => ({
  id: field(b, 'id', /\bid:\s*"([^"]+)"/),
  category: field(b, 'category', /\bcategory:\s*ToolCategory::(\w+)/).toLowerCase(),
  title: field(b, 'title', /\btitle:\s*"([^"]+)"/),
  advertised: field(b, 'advertised', /\badvertised:\s*(true|false)/) === 'true',
  previewOnly: field(b, 'preview_only', /\bpreview_only:\s*(true|false)/) === 'true',
  multiInput: field(b, 'multi_input', /\bmulti_input:\s*(true|false)/) === 'true',
  /* `available: true` is unconditional; anything else is measured at runtime,
     which on this product always means a model has to be downloaded first. */
  needsModel: !/\bavailable:\s*true\b/.test(b),
}));

const ids = new Set();
for (const t of tools) {
  if (ids.has(t.id)) throw new Error(`two descriptors share the id ${t.id}`);
  ids.add(t.id);
}

const payload = {
  generated_by: 'scripts/export-tools.mjs from crates/openconvert-run/src/tools.rs',
  tools,
};

writeFileSync(join(ROOT, 'src', 'data', 'tools.json'), JSON.stringify(payload, null, 2) + '\n', 'utf8');

const shown = tools.filter((t) => t.advertised);
const byCat = {};
for (const t of shown) (byCat[t.category] ||= []).push(t.title);
console.log(`  ${tools.length} descriptors, ${shown.length} in the menu`);
for (const [k, v] of Object.entries(byCat)) console.log(`    ${k.padEnd(9)} ${v.join(', ')}`);
