#!/usr/bin/env node
/* ============================================================================
   site gates — run against dist/ after `npm run build`.

   10-WEBSITE §9: "A budget that is not checked is a preference." So the gate
   table is executable, and the build fails on a violation rather than filing it.

   Usage:  npm run gates
   ============================================================================ */

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, extname, relative, dirname } from 'node:path';
import { gzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
/**
 * Where the built site actually landed.
 *
 * `dist/` until the Vercel adapter arrived for `/api/contact`. With an adapter
 * present the prerendered pages are copied to `.vercel/output/static` and
 * `dist/` keeps only the client assets, so a gate run reading `dist/` would
 * find no pages and pass every check by inspecting nothing. Preferring the
 * adapter's directory when it exists is what keeps these gates pointed at the
 * bytes a visitor is actually served.
 */
const VERCEL_STATIC = join(HERE, '..', '.vercel', 'output', 'static');
const DIST = existsSync(join(VERCEL_STATIC, 'index.html'))
  ? VERCEL_STATIC
  : join(HERE, '..', 'dist');
const REPO = join(HERE, '..', '..');

const BUDGET = {
  gzipKb: 150,        // transferred weight per page
  jsBundles: 0,       // no JS bundles on content pages
  lazyJsKb: 240,      // pdf-lib, fetched only when a PDF tool in the sample is opened.
                      // Outside the island budget because it is not on the path to
                      // reading the page — and measured anyway, because it is a real
                      // download for the person who does open one.
  homepageJsKb: 40,   // island shell + router wasm + ambient pause logic, lazy, homepage only.
                      // Grew from 20 when the sample gained a real conversion path: the honest
                      // receipt needs the pipeline that produces it. Still one script, one wasm,
                      // fetched on first interaction — and still counted gzipped, wasm included,
                      // because visitors download bytes rather than categories.
  externalHosts: 0,   // zero third-party requests, ever
  privacyWords: 300,  // hard cap, and the reason anyone finishes the page

  // Screenshots (13-SITE-REBUILD §3.2). The document budget above is NOT
  // relaxed to make room for pictures — the image budgets sit beside it, so a
  // page that grew 200 KB of markup still fails even while its hero fits.
  heroImageKb: 120,       // the one above-the-fold shot on a page
  imagesKbPerPage: 950,   // every image a page references, summed. Raised from 450 when
                          // the tool section began cycling three captures per group: the
                          // homepage shows eleven screens now, all but the hero lazy.
  imagesKbTotal: 8000,    // public/shots as a directory, both themes, both formats.
                          // Eighteen scenes x2 themes x2 formats; a visitor fetches one
                          // theme in one format, so this is the repository's bill, not theirs.
};

// The four operation-class glyphs plus the confinement glyph. The app gates
// these against the shipped font; the site uses the same five and inherits it.
const STATE_GLYPHS = ['=', '≈', '⌇', '✦', '⛨'];

/* ---------------------------------------------------------------------------
   THE BANNED-TOKEN LIST — 10-WEBSITE §2.

   "The banned-token gate is the one that keeps this plan honest." v0.1 of this
   site shipped a download page reading "Not yet. And we would rather say so."
   and a demo slot labelled "not yet live". Every one of those tells a visitor
   to come back later, which is the opposite of what a product page is for.

   Each phrase is replaced by a number or by the name of the mechanism that does
   the work — never by a softer adjective.
   ------------------------------------------------------------------------- */
const BANNED = [
  // status
  'coming soon', 'not yet', 'in progress', 'will be', 'once we',
  'our roadmap', 'stay tuned', 'work in progress', 'under construction',
  // hedging
  'we believe', 'we think', 'our goal is', 'we aim to', 'we are working on',
  'we hope', 'we plan to',
  // adjectives that carry no information
  'blazing', 'cutting-edge', 'seamless', 'robust', 'powerful',
  'lightning fast', 'blazingly',
  // bare "fast" and "planned" as whole words
  '\\bfast\\b', '\\bplanned\\b', '\\bsimply\\b',
];

// Exempt: the threat-model text lifted verbatim onto /security uses "fail open"
// and similar, and a quotation from a CVE description is evidence rather than
// voice. Nothing currently needs an exemption; the list exists so that adding
// one is a visible decision rather than a quiet edit to the pattern above.
const EXEMPT = [];

let failures = 0;
const pass = (name, detail = '') =>
  console.log(`  \x1b[32mPASS\x1b[0m  ${name}${detail ? '  ' + detail : ''}`);
const fail = (name, detail) => {
  failures++;
  console.log(`  \x1b[31mFAIL\x1b[0m  ${name}  ${detail}`);
};

function walk(dir, out = []) {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

let files;
try {
  files = walk(DIST);
} catch {
  console.error('\n  dist/ not found — run `npm run build` first.\n');
  process.exit(1);
}

const html = files.filter((f) => extname(f) === '.html');
const js = files.filter((f) => extname(f) === '.js');
const rel = (f) => relative(DIST, f).replace(/\\/g, '/');
/**
 * The URL a built file is served at.
 *
 * The adapter emits directory-style pages (`about/index.html`), so a gate that
 * keys off the filename now sees `index.html` for every page. The route is the
 * stable name, and it is what the policies below are actually written about.
 */
const route = (f) => '/' + rel(f).replace(/\.html$/, '').replace(/(^|\/)index$/, '');

/**
 * Text inside <main>, tags stripped, entities folded. The page's own prose.
 *
 * Cells marked `data-verbatim` are dropped: they carry text lifted word for word
 * out of the design record, and the one thing the voice gate must never do is
 * pressure someone into rewording a quotation to get a build green.
 */
function mainText(src) {
  const m = src.match(/<main[^>]*>([\s\S]*?)<\/main>/i);
  const body = m ? m[1] : src;
  return body
    .replace(/<td\b[^>]*\bdata-verbatim\b[\s\S]*?<\/td>/gi, ' ')
    .replace(/<(script|style)[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/g, ' ')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#\d+;/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

console.log('\n\x1b[1m  openconvert.dev — site gates\x1b[0m\n');

/* ---- 1. transferred weight -------------------------------------------- */
let worst = { name: '', kb: 0 };
for (const f of html) {
  const kb = gzipSync(readFileSync(f)).length / 1024;
  if (kb > worst.kb) worst = { name: rel(f), kb };
}
worst.kb <= BUDGET.gzipKb
  ? pass('page weight, gzipped', `worst ${worst.name} ${worst.kb.toFixed(1)} KB / ${BUDGET.gzipKb} KB`)
  : fail('page weight, gzipped', `${worst.name} is ${worst.kb.toFixed(1)} KB, budget ${BUDGET.gzipKb} KB`);

/* ---- 2. JavaScript: three files, and this gate is the whole list --------
   Every script this site serves is named here, with the page allowed to carry
   it. Anything else fails the build.

     theme.js    every page — reads a preference, stamps an attribute
     island.js   index.html — the lazy conversion sample, plus its wasm
     copy.js     download.html — the clipboard button on the install commands
     contact.js  contact.html — composes a mailto from the form. Nothing posts.

   `theme.js` is on every page, so this site no longer has a page that ships
   zero JavaScript, and the gate says so plainly rather than keeping a name
   that stopped being true. Six hundred bytes buys a theme control that works
   before first paint; the honest accounting is the budget below, which counts
   what a visitor actually downloads.

   Checked three ways, because each catches something the others do not:

     a. every page's scripts are all on the list, and only on pages that may
     b. no <script> has a src we did not write
     c. the island plus its wasm, gzipped, fits its budget

   (c) counts the .wasm. It is not JavaScript and 10-WEBSITE §9 says
   "JavaScript", but a visitor downloads bytes rather than categories, and a
   budget that a 900 KB module walks straight through is not a budget.        */
const ALLOWED_SCRIPTS = {
  '/theme.js': () => true,
  '/island.js': (name) => name === 'index.html',
  '/copy.js': (name) => name === 'download.html',
  '/contact.js': (name) => name === 'contact.html',
};

const scriptProblems = [];
const inline = [];
for (const f of html) {
  const src = readFileSync(f, 'utf8');
  const name = rel(f);
  for (const tag of src.matchAll(/<script[^>]*>/gi)) {
    const url = tag[0].match(/src="([^"]+)"/)?.[1];
    if (!url) {
      inline.push(name);
      continue;
    }
    const allowed = ALLOWED_SCRIPTS[url];
    if (!allowed) scriptProblems.push(`${name} → ${url} is not one of ours`);
    else if (!allowed(name)) scriptProblems.push(`${name} → ${url} is not allowed on this page`);
  }
}

scriptProblems.length === 0 && inline.length === 0
  ? pass(
      'every script is one of the four we wrote',
      `theme.js on ${html.length}, island on index, copy on download, contact on contact`,
    )
  : fail(
      'every script is one of the four we wrote',
      [...scriptProblems, ...inline.map((n) => `${n} → inline <script>`)].join(', '),
    );

/* No BUNDLES: Astro emitting its own chunks would mean a component hydrated,
   which is a different failure from the island growing.

   VENDORED THIRD-PARTY CODE IS NAMED HERE, one entry per library, with the
   version it was copied from. `pdf-lib` arrived so the seven PDF page tools in
   the sample actually run rather than being drawn; it is the only file on this
   site nobody here wrote. Naming it in the gate is the point — an unlisted
   third-party script still fails, so the next one has to be a decision rather
   than a drop-in. It is loaded ONLY when a PDF tool is opened, which is what
   the lazy budget below measures. */
const VENDORED = {
  'vendor/pdf-lib.min.js': 'pdf-lib 1.17.1, MIT, from npm',
};
const ours = /(island|copy|theme|contact)\.js$/;
const strays = js.filter((f) => !ours.test(rel(f)) && !VENDORED[rel(f)]);
strays.length <= BUDGET.jsBundles
  ? pass(
      'no bundler chunks emitted',
      `${js.length} JS file(s): ${js.length - Object.keys(VENDORED).length} hand-written, ` +
        `${Object.keys(VENDORED).map((k) => VENDORED[k]).join('; ')}`,
    )
  : fail('no bundler chunks emitted', strays.map(rel).join(', '));

/* ---- 2b. the lazy payload, bounded -------------------------------------
   `pdf-lib` is not in the island budget above because nobody downloads it to
   read the page — it is fetched when a PDF tool is opened and not before. That
   is a real cost to the person who opens one, though, so it is measured and
   capped rather than being invisible for sitting outside the other number. */
{
  const lazy = js.filter((f) => VENDORED[rel(f)]);
  const kbOf = (f) => gzipSync(readFileSync(f), { level: 9 }).length / 1024;
  const total = lazy.reduce((n, f) => n + kbOf(f), 0);
  total <= BUDGET.lazyJsKb
    ? pass(
        'lazy payload budget',
        `${total.toFixed(1)} KB / ${BUDGET.lazyJsKb} KB gzipped, fetched only when a PDF tool is opened`,
      )
    : fail('lazy payload budget', `${total.toFixed(1)} KB, budget ${BUDGET.lazyJsKb} KB`);
}

// The island's transferred weight.
const islandFiles = files.filter((f) => /island\.js$|\.wasm$/.test(rel(f)));
if (islandFiles.length === 0) {
  pass('homepage island budget', 'no island in this build');
} else {
  const parts = islandFiles.map((f) => ({
    name: rel(f),
    kb: gzipSync(readFileSync(f), { level: 9 }).length / 1024,
  }));
  const total = parts.reduce((n, p) => n + p.kb, 0);
  total <= BUDGET.homepageJsKb
    ? pass(
        'homepage island budget',
        `${total.toFixed(1)} KB / ${BUDGET.homepageJsKb} KB gzipped (${parts
          .map((p) => `${p.name} ${p.kb.toFixed(1)}`)
          .join(', ')})`,
      )
    : fail('homepage island budget', `${total.toFixed(1)} KB, budget ${BUDGET.homepageJsKb} KB`);
}

/* ---- 3. zero cross-origin requests ------------------------------------- */
// Anything that would make the browser contact another host. Canonical and
// og:url point at our own origin and are not requests; w3.org is an XML
// namespace; nvd.nist.gov links on /security are anchors a reader clicks, not
// resources the page fetches — so only resource-loading attributes are scanned.
const ALLOW = /(?:openconvert\.dev|www\.w3\.org)/;
const leaks = [];
for (const f of [...html, ...files.filter((x) => extname(x) === '.css')]) {
  const text = readFileSync(f, 'utf8');
  const patterns = [
    /\bsrc\s*=\s*["']?(https?:\/\/[^"'\s>]+)/gi,
    /<link\b[^>]*\bhref\s*=\s*["']?(https?:\/\/[^"'\s>]+)/gi,
    /url\(\s*["']?(https?:\/\/[^"')\s]+)/gi,
    /@import\s+["'(]?\s*(https?:\/\/[^"')\s]+)/gi,
  ];
  for (const p of patterns) {
    for (const m of text.matchAll(p)) if (!ALLOW.test(m[1])) leaks.push(`${rel(f)} → ${m[1]}`);
  }
}
leaks.length === BUDGET.externalHosts
  ? pass('zero cross-origin requests')
  : fail('zero cross-origin requests', leaks.slice(0, 5).join('; '));

/* ---- 4. CSP present and locked down ------------------------------------ */
// Every page carries one, and each page's policy is exactly as wide as what it
// actually runs — the JavaScript budget expressed where a browser enforces it.
//
//   every page     script 'self' + 'unsafe-inline', connect 'self' for Analytics
//   index.html     also adds 'wasm-unsafe-eval' for the router demo
//
// No page may name a host in any case, so none of the three can reach another
// origin however the markup changes.
const noCsp = html.filter((f) => !/Content-Security-Policy/.test(readFileSync(f, 'utf8')));
const looseCsp = html.filter((f) => {
  const t = readFileSync(f, 'utf8');
  if (!/Content-Security-Policy/.test(t)) return false;
  const csp = t.match(/Content-Security-Policy" content="([^"]*)"/)?.[1] ?? '';
  const name = route(f);

  const wantScript =
    name === '/'
      ? /script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'/
      : /script-src 'self' 'unsafe-inline'/;

  // Analytics posts a page view through Vercel's same-origin route on every
  // page. The homepage and contact page use that same permission for their own
  // first-party requests; no external host is admitted.
  const wantConnect = /connect-src 'self'/;

  // Only /contact has a form, so only /contact may submit one — and only back
  // here. Every other page must refuse outright.
  const wantForm = name === '/contact' ? /form-action 'self'/ : /form-action 'none'/;

  // No page may name a host. 'self', 'none' and 'wasm-unsafe-eval' only.
  const noHosts = !/(?:https?:|\/\/)[^;'"]*/.test(csp);
  return !(wantScript.test(csp) && wantConnect.test(csp) && wantForm.test(csp) && noHosts);
});
noCsp.length === 0 && looseCsp.length === 0
  ? pass(
      'CSP on every page',
      `${html.length} pages; Analytics stays same-origin, island adds wasm-unsafe-eval`,
    )
  : fail('CSP on every page', `${noCsp.length} missing, ${looseCsp.map(rel).join(', ')} too loose`);

/* ---- 5. BANNED VOICE TOKENS -------------------------------------------- */
const hits = [];
for (const f of html) {
  const text = mainText(readFileSync(f, 'utf8'));
  if (EXEMPT.includes(rel(f))) continue;
  for (const phrase of BANNED) {
    const re = phrase.startsWith('\\b')
      ? new RegExp(phrase, 'gi')
      : new RegExp(phrase.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi');
    for (const m of text.matchAll(re)) {
      const at = Math.max(0, m.index - 40);
      hits.push(`${rel(f)}: "${m[0]}" in …${text.slice(at, m.index + m[0].length + 40)}…`);
    }
  }
}
hits.length === 0
  ? pass('banned voice tokens', `${BANNED.length} phrases, ${html.length} pages, 0 matches`)
  : fail('banned voice tokens', '\n        ' + hits.slice(0, 6).join('\n        '));

/* ---- 6. /privacy word count -------------------------------------------- */
const privacy = html.find((f) => route(f) === '/privacy');
if (!privacy) fail('/privacy word count', '/privacy not built');
else {
  const words = mainText(readFileSync(privacy, 'utf8')).split(/\s+/).filter(Boolean).length;
  words <= BUDGET.privacyWords
    ? pass('/privacy word count', `${words} / ${BUDGET.privacyWords} words`)
    : fail('/privacy word count', `${words} words, cap ${BUDGET.privacyWords}`);
}

/* ---- 7. /privacy agrees with its source -------------------------------- */
// The page, the README and the app settings screen render one file. The gate is
// what makes "generated from a shared source" a fact rather than an intention.
try {
  const src = JSON.parse(readFileSync(join(REPO, 'release', 'privacy.json'), 'utf8'));
  const rendered = mainText(readFileSync(privacy, 'utf8'));
  const readme = readFileSync(join(REPO, 'README.md'), 'utf8');

  // THE WHOLE BODY, NOT A PREFIX.
  //
  // This compared `s.body.slice(0, 60)` — the first sentence and a bit. A
  // rewrite that kept the opening and changed everything after it passed the
  // gate with the README and the page saying different things, which is
  // exactly the drift the gate exists to catch. Sixty characters made
  // "generated from a shared source" true of the first line only.
  //
  // Whitespace is normalised on both sides: the README wraps at a different
  // width from the rendered page, and a line break is not a disagreement.
  const flat = (t) => t.replace(/\s+/g, ' ').trim();
  const flatRendered = flat(rendered);
  const flatReadme = flat(readme);
  const missingPage = src.sections.filter((s) => !flatRendered.includes(flat(s.body)));
  const missingReadme = src.sections.filter((s) => !flatReadme.includes(flat(s.body)));

  missingPage.length === 0 && missingReadme.length === 0
    ? pass('/privacy matches privacy.json and README', `${src.sections.length} sections, 2 surfaces`)
    : fail(
        '/privacy matches privacy.json and README',
        [
          missingPage.length ? `page missing: ${missingPage.map((s) => s.id).join(', ')}` : '',
          missingReadme.length
            ? `README stale, run node release/render-privacy.mjs — missing: ${missingReadme.map((s) => s.id).join(', ')}`
            : '',
        ]
          .filter(Boolean)
          .join('; '),
      );
} catch (e) {
  fail('/privacy matches privacy.json and README', String(e.message));
}

/* ---- 7b. every model named on the site exists in models.toml ------------ */
// The features page carried a hand-written AI table until 2026-08-27, and it
// had drifted into four separate untruths at once: two cells contained the
// literal strings `[size]` and `[verify]`; it listed Kokoro-82M, a row removed
// from the registry for never having been pinned; it listed BiRefNet-lite,
// which has no ONNX export that loads; and it listed Granite-Docling-258M,
// which is not in the registry at all, with a confident size beside it.
//
// The table is generated now. This is the gate that keeps it generated: it
// re-derives what the exporter would produce and refuses a `models.json` that
// disagrees with `models.toml`, so a stale export fails the build instead of
// shipping.
try {
  const toml = readFileSync(join(REPO, 'models.toml'), 'utf8');
  const declared = new Set([...toml.matchAll(/^name\s*=\s*"([^"]+)"/gm)].map((m) => m[1]));
  const data = JSON.parse(readFileSync(join(HERE, '..', 'src', 'data', 'models.json'), 'utf8'));

  const named = data.features.flatMap((f) => f.models.map((m) => m.split(' ')[0]));
  const unknown = named.filter((n) => !declared.has(n));

  // And no page may carry a bracketed placeholder as production copy.
  const pages = html.map((f) => rel(f));
  const placeholders = html
    .filter((f) => /\[(size|verify|todo|tbd)\]/i.test(readFileSync(f, 'utf8')))
    .map((f) => rel(f));

  unknown.length === 0 && placeholders.length === 0
    ? pass('site names only models that exist', `${named.length} artifacts, ${data.features.length} capabilities`)
    : fail(
        'site names only models that exist',
        [
          unknown.length ? `not in models.toml: ${[...new Set(unknown)].join(', ')}` : '',
          placeholders.length ? `placeholder text on: ${placeholders.join(', ')}` : '',
        ]
          .filter(Boolean)
          .join('; '),
      );
} catch (e) {
  fail('site names only models that exist', String(e.message));
}

/* ---- 7c. the generated data layer is current ---------------------------- */
// The failure this exists to catch already happened. The site shipped "126
// conversions" and "eleven tools" for a release in which the binary had 212
// routes and the menu had twenty, because `formats.json` and the hand-typed
// tool lists were snapshots nobody re-took. A generated file that is never
// checked against its source is just a slower kind of hand-written one.
//
// Both halves are re-derived here from files in the repository rather than
// from the binary, so this runs on a checkout with nothing built:
//
//   tools.json   against `all_tools()` in crates/openconvert-run/src/tools.rs
//   formats.json against docs/ROUTES.md, which `cargo xtask routes --check`
//                already pins to `RouteTable::v1()`
try {
  const problems = [];

  // -- tools ---------------------------------------------------------------
  const rs = readFileSync(join(REPO, 'crates', 'openconvert-run', 'src', 'tools.rs'), 'utf8');
  const body = rs.slice(rs.indexOf('pub fn all_tools()'));
  const code = body.replace(/\/\/\/.*$/gm, '').replace(/\/\/.*$/gm, '');
  const fromSource = code
    .split(/ToolDescriptor\s*\{/)
    .slice(1)
    .map((b) => ({
      id: (b.match(/\bid:\s*"([^"]+)"/) || [])[1],
      advertised: /\badvertised:\s*true/.test(b),
    }));

  const toolData = JSON.parse(readFileSync(join(HERE, '..', 'src', 'data', 'tools.json'), 'utf8'));
  const shipped = new Map(toolData.tools.map((t) => [t.id, t.advertised]));

  for (const t of fromSource) {
    if (!shipped.has(t.id)) problems.push(`tools.json is missing ${t.id}`);
    else if (shipped.get(t.id) !== t.advertised) problems.push(`${t.id} advertised flag disagrees`);
  }
  for (const id of shipped.keys()) {
    if (!fromSource.some((t) => t.id === id)) problems.push(`tools.json still lists ${id}`);
  }

  // -- formats -------------------------------------------------------------
  // `### <id>` under a kind heading is one source format in the generated
  // reference; anything below "Read but never written" is not a source.
  const routesMd = readFileSync(join(REPO, 'docs', 'ROUTES.md'), 'utf8');
  const cut = routesMd.indexOf('## Read but never written');
  const sourcesInDocs = new Set(
    [...(cut < 0 ? routesMd : routesMd.slice(0, cut)).matchAll(/^###\s+(\S+)\s*$/gm)].map((m) => m[1]),
  );

  const formatData = JSON.parse(readFileSync(join(HERE, '..', 'src', 'data', 'formats.json'), 'utf8'));
  const sourcesOnSite = new Set(formatData.formats.filter((f) => f.routes.length > 0).map((f) => f.id));

  for (const id of sourcesInDocs) {
    if (!sourcesOnSite.has(id)) problems.push(`formats.json converts nothing from ${id}`);
  }
  for (const id of sourcesOnSite) {
    if (!sourcesInDocs.has(id)) problems.push(`formats.json has a source ROUTES.md does not: ${id}`);
  }

  // Every format the picker renders needs its authored prose, or the page
  // shows facts with a hole where the sentence explaining them belongs.
  const noProse = formatData.formats.filter((f) => !f.name).map((f) => f.id);
  if (noProse.length) problems.push(`no prose for: ${noProse.join(', ')}`);

  problems.length === 0
    ? pass(
        'generated data matches the engine',
        `${shipped.size} tools, ${sourcesOnSite.size} source formats, ` +
          `${formatData.formats.reduce((n, f) => n + f.routes.length, 0)} routes`,
      )
    : fail('generated data matches the engine', problems.slice(0, 4).join('; ') +
        ' — re-run `node scripts/export-tools.mjs` and `node scripts/export-data.mjs`');
} catch (e) {
  fail('generated data matches the engine', String(e.message));
}

/* ---- 8. no placeholder checksum on a published release ------------------ */
// A stale checksum on a security product is worse than none, and a placeholder
// one is worse again. While release.state is "unpublished" the zeroes are the
// shape of a hash; the day it flips, this gate refuses them.
try {
  const release = JSON.parse(readFileSync(join(HERE, '..', 'src', 'data', 'release.json'), 'utf8'));
  const placeholders = release.artifacts.filter((a) => /^0+$/.test(a.sha256));
  release.state !== 'published' || placeholders.length === 0
    ? pass('release checksums', `state=${release.state}, ${release.artifacts.length} artifacts`)
    : fail('release checksums', `${placeholders.length} placeholder hash(es) on a published release`);
} catch (e) {
  fail('release checksums', String(e.message));
}

/* ---- 9. reduced-motion contract ---------------------------------------- */
// Every page carrying a narrative sequence must also carry the rule that
// resolves it. A sequence with no reduced-motion branch is a defect: the
// visitor who turned motion off would lose the information the sequence exists
// to deliver.
const withMotion = html.filter((f) => /animation-delay|animation:\s*seq-/.test(readFileSync(f, 'utf8')));
const missingRm = withMotion.filter((f) => !/prefers-reduced-motion/.test(readFileSync(f, 'utf8')));
missingRm.length === 0
  ? pass('reduced-motion branch on every animated page', `${withMotion.length} pages`)
  : fail('reduced-motion branch', `missing on ${missingRm.map(rel).join(', ')}`);

/* ---- 10. state glyphs render literally ---------------------------------
   NOT "every page shows all four classes" — a format with only lossless routes
   correctly shows only `=`. What must hold is that every glyph a page DOES
   render is a literal character from the known set: not HTML-escaped, not a
   replacement character, not silently dropped by an encoding step.           */
const glyphIssues = [];
const seen = new Set();
for (const f of html) {
  const t = readFileSync(f, 'utf8');
  if (t.includes('�')) glyphIssues.push(`${rel(f)} contains U+FFFD (encoding damage)`);

  for (const m of t.matchAll(/<span class="glyph"[^>]*>([^<]*)<\/span>/g)) {
    const g = m[1].trim();
    if (!STATE_GLYPHS.includes(g)) glyphIssues.push(`${rel(f)} rendered "${g}" — not a state glyph`);
    else seen.add(g);
  }
  for (const ent of ['&#x2307;', '&#8967;', '&#x2726;', '&#10022;', '&#x26E8;', '&#9960;']) {
    if (t.includes(ent)) glyphIssues.push(`${rel(f)} contains escaped glyph ${ent}`);
  }
}
glyphIssues.length === 0
  ? pass('state glyphs render literally', `${[...seen].sort().join(' ')} across ${html.length} pages`)
  : fail('state glyphs', glyphIssues.slice(0, 5).join('; '));

/* ---- 11. title and description on every page ---------------------------- */
const metaMiss = html.filter((f) => {
  const t = readFileSync(f, 'utf8');
  return !/<title>[^<]{8,}<\/title>/.test(t) || !/name="description" content="[^"]{20,}"/.test(t);
});
metaMiss.length === 0
  ? pass('title + description on every page', `${html.length} pages`)
  : fail('title + description', metaMiss.map(rel).join(', '));

/* ---- 12. no layout-animating properties --------------------------------
   Transform and opacity only. Anything animating a layout property is rejected:
   the CLS budget is zero, not "low".                                         */
// Scanned against CSS ONLY — <style> blocks and style="" attributes. Run over
// whole HTML the pattern reads through markup and prose: a page whose copy says
// "the only animation format" and mentions "right" fifty words later matched,
// and a gate with false positives is a gate people switch off.
const LAYOUT = /\b(width|height|top|left|right|bottom|margin|padding|inset)\b/;
const layoutAnim = [];
for (const f of html) {
  const src = readFileSync(f, 'utf8');
  const css = [
    ...[...src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/gi)].map((m) => m[1]),
    ...[...src.matchAll(/\bstyle="([^"]*)"/gi)].map((m) => m[1]),
  ].join('\n');

  for (const decl of css.split(/[;{}\n]/)) {
    const m = decl.match(/^\s*(transition|transition-property|animation|animation-name)\s*:(.*)$/i);
    if (m && LAYOUT.test(m[2])) layoutAnim.push(`${rel(f)} → ${decl.trim().slice(0, 60)}`);
  }
}
layoutAnim.length === 0
  ? pass('animations restricted to compositor properties')
  : fail('animations restricted to compositor properties', layoutAnim.slice(0, 3).join('; '));

/* ---- 13. no timeline inside the `animation` shorthand -------------------
   A gate that exists because the bug it catches is invisible in the source.

   Written as two declarations in one block —

     animation: flow-step linear both;
     animation-timeline: --flow;

   — the CSS minifier folds them into `animation: linear both flow-step --flow`,
   using the shorthand's timeline component. That component was in an early
   draft of scroll-driven animations and no shipping browser accepts it, so the
   declaration is invalid as a whole: `animation-name` computes to `none` and
   NOTHING animates. Anything resting under `opacity: 0` waiting to be revealed
   then stays hidden, and `@supports (animation-timeline: view())` is satisfied,
   so the fallback layout does not step in either. The section renders blank.

   The fix is to keep `animation-timeline` in a rule the minifier cannot merge.
   The fix holding is what this checks.                                        */
const shorthandTimeline = [];
for (const f of [...html, ...files.filter((x) => extname(x) === '.css')]) {
  const css = readFileSync(f, 'utf8');
  for (const m of css.matchAll(/animation\s*:([^;}"]*)/gi)) {
    // `var(--curve)` is a custom PROPERTY, not a timeline name — strip the
    // var() calls before looking for a bare `--name`.
    const value = m[1].replace(/var\([^)]*\)/g, ' ');
    if (/(^|\s)--[\w-]+/.test(value)) shorthandTimeline.push(`${rel(f)} → animation:${m[1].trim().slice(0, 50)}`);
  }
}
shorthandTimeline.length === 0
  ? pass('no timeline folded into the animation shorthand')
  : fail('no timeline folded into the animation shorthand', [...new Set(shorthandTimeline)].slice(0, 3).join('; '));

/* ---- 14. no dead internal links ----------------------------------------
   Root-relative hrefs must resolve to a built route. Paths with a file
   extension (/fonts/inter-latin-var.woff2) are public/ assets served as-is,
   not pages, and are skipped — a missing font would 404 in the network panel,
   which is where that class of mistake belongs.                              */
const routes = new Set(html.map((f) => '/' + rel(f).replace(/\.html$/, '').replace(/\/index$/, '')));
routes.add('/');
const dead = [];
for (const f of html) {
  for (const m of readFileSync(f, 'utf8').matchAll(/href="(\/[^"#?]*)(?:[#?][^"]*)?"/g)) {
    const target = m[1].replace(/\/$/, '') || '/';
    if (/\.\w+$/.test(target)) continue; // static asset, not a route
    if (!routes.has(target)) dead.push(`${rel(f)} → ${m[1]}`);
  }
}
dead.length === 0
  ? pass('no dead internal links', `${routes.size} routes`)
  : fail('no dead internal links', [...new Set(dead)].slice(0, 6).join('; '));


/* ---- 15. screenshots: weight, integrity, and freshness ------------------
   The site is now carried by pictures of the product, and a picture has three
   ways to rot that markup does not. Each gets a gate.

     a. WEIGHT. A hero nobody waits for is a hero nobody sees.
     b. INTEGRITY. Intrinsic width and height on every <img>, because the
        layout-shift budget is zero; a non-empty alt, because a screenshot
        carrying the argument has to carry it to a screen reader too; and an
        AVIF always paired with a fallback, because `<source type>` failing
        silently is how a page ships with holes in it.
     c. FRESHNESS. `shots.json` records the commit the app was captured at.
        A screenshot from two releases ago is a picture of software the
        visitor will not receive — the same failure as a stale checksum, and
        it fails the build the same way.                                      */
const SHOTS_DIR = join(DIST, 'shots');
const shotsData = existsSync(join(REPO, 'site', 'src', 'data', 'shots.json'))
  ? JSON.parse(readFileSync(join(REPO, 'site', 'src', 'data', 'shots.json'), 'utf8'))
  : { shots: [] };
const declared = new Map(shotsData.shots.map((s) => [s.id, s]));

if (!existsSync(SHOTS_DIR)) {
  pass('screenshot weight', 'no screenshots in this build');
} else {
  const shotFiles = walk(SHOTS_DIR);
  const totalKb = shotFiles.reduce((n, f) => n + statSync(f).size, 0) / 1024;

  // Per page: every /shots/… the HTML names, counted once, in the format a
  // modern browser actually takes (AVIF where offered).
  let worstPage = { name: '', kb: 0 };
  let worstHero = { name: '', kb: 0 };
  for (const f of html) {
    const src = readFileSync(f, 'utf8');
    // A visitor fetches ONE theme, so the page's image weight is one theme's
    // worth — counted as the heavier of the two, which is the weight somebody
    // actually waits for. Summing light and dark would budget against a
    // download nobody performs.
    const named = new Set(
      [...src.matchAll(/\/shots\/([\w-]+)\.avif/g)].map((m) => m[1].replace(/-dark$/, '')),
    );
    let kb = 0;
    for (const stem of named) {
      const both = ['', '-dark']
        .map((t) => join(SHOTS_DIR, `${stem}${t}.avif`))
        .filter((f) => existsSync(f))
        .map((f) => statSync(f).size / 1024);
      kb += both.length ? Math.max(...both) : 0;
    }
    if (kb > worstPage.kb) worstPage = { name: rel(f), kb };

    // The hero is the one that is not lazy.
    const eager = src.match(/<img[^>]*loading="eager"[^>]*>/);
    const heroSrc = eager && src.slice(0, src.indexOf(eager[0])).match(/\/shots\/([\w-]+)\.avif(?![\s\S]*\/shots\/)/);
    if (heroSrc) {
      const file = join(SHOTS_DIR, `${heroSrc[1]}.avif`);
      const hkb = existsSync(file) ? statSync(file).size / 1024 : 0;
      if (hkb > worstHero.kb) worstHero = { name: `${rel(f)} ${heroSrc[1]}`, kb: hkb };
    }
  }

  const weightProblems = [];
  if (totalKb > BUDGET.imagesKbTotal) weightProblems.push(`public/shots ${totalKb.toFixed(0)} KB / ${BUDGET.imagesKbTotal} KB`);
  if (worstPage.kb > BUDGET.imagesKbPerPage) weightProblems.push(`${worstPage.name} ${worstPage.kb.toFixed(0)} KB / ${BUDGET.imagesKbPerPage} KB`);
  if (worstHero.kb > BUDGET.heroImageKb) weightProblems.push(`hero ${worstHero.name} ${worstHero.kb.toFixed(0)} KB / ${BUDGET.heroImageKb} KB`);

  weightProblems.length === 0
    ? pass(
        'screenshot weight',
        `dir ${totalKb.toFixed(0)}/${BUDGET.imagesKbTotal} KB · worst page ${worstPage.kb.toFixed(0)}/${BUDGET.imagesKbPerPage} KB · hero ${worstHero.kb.toFixed(0)}/${BUDGET.heroImageKb} KB`,
      )
    : fail('screenshot weight', weightProblems.join('; '));

  // Integrity.
  const imgProblems = [];
  for (const f of html) {
    const src = readFileSync(f, 'utf8');
    for (const m of src.matchAll(/<img[^>]*>/g)) {
      const tag = m[0];
      if (!/src="\/shots\//.test(tag)) continue;
      const stem = tag.match(/\/shots\/([\w-]+)\./)?.[1]?.replace(/-dark$/, '');
      if (!/width="\d+"/.test(tag) || !/height="\d+"/.test(tag)) imgProblems.push(`${rel(f)} ${stem}: no intrinsic size`);
      if (!/alt="[^"]+"/.test(tag)) imgProblems.push(`${rel(f)} ${stem}: empty alt`);
      if (stem && !declared.has(stem)) imgProblems.push(`${rel(f)} ${stem}: not in shots.json`);
    }
    // Every AVIF source needs a fallback in the same <picture>.
    for (const pic of src.matchAll(/<picture[\s\S]*?<\/picture>/g)) {
      if (/type="image\/avif"/.test(pic[0]) && !/<img[^>]*\.(?:webp|png|jpg)"/.test(pic[0])) {
        imgProblems.push(`${rel(f)}: avif with no fallback`);
      }
    }
  }
  for (const shot of shotsData.shots) {
    for (const theme of ['', '-dark']) {
      for (const fmt of ['avif', 'webp']) {
        if (!existsSync(join(SHOTS_DIR, `${shot.id}${theme}.${fmt}`))) {
          imgProblems.push(`missing file ${shot.id}${theme}.${fmt}`);
        }
      }
    }
  }
  imgProblems.length === 0
    ? pass('screenshot integrity', `${declared.size} scenes, both themes, both formats`)
    : fail('screenshot integrity', [...new Set(imgProblems)].slice(0, 4).join('; '));

  // Freshness: the captured commit has to be one this branch contains.
  let known = false;
  try {
    execFileSync('git', ['merge-base', '--is-ancestor', shotsData.app_sha, 'HEAD'], { cwd: REPO, stdio: 'pipe' });
    known = true;
  } catch {
    known = false;
  }
  known
    ? pass('screenshot freshness', `captured at ${shotsData.app_sha} on ${shotsData.captured}`)
    : fail(
        'screenshot freshness',
        `shots.json says ${shotsData.app_sha}, which is not an ancestor of HEAD — re-run \`npm run shots\``,
      );
}

console.log('');
if (failures) {
  console.log(`\x1b[31m  ${failures} gate(s) failed.\x1b[0m\n`);
  process.exit(1);
}
console.log(`\x1b[32m  All gates green.\x1b[0m  ${html.length} pages checked.\n`);
