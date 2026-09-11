# openconvert.dev

The product site. Fourteen pages are prerendered, with one server route for the
contact form and a lazy, local-only conversion island on the homepage. The
published experience makes no cross-origin browser requests. Vercel Web
Analytics is disclosed on the privacy page and uses Vercel's same-origin
analytics route without cookies.

Built to [10-WEBSITE](../docs/spec/10-WEBSITE.md). Visual language is inherited
from [07-DESIGN-SYSTEM](../docs/spec/07-DESIGN-SYSTEM.md) — the tokens in `src/styles/tokens.css`
are transcribed from it, not invented — with exactly three stated divergences:
a brand hue the app is forbidden, a 16 px body scale against the app's 13 px, and
a web-only `motion-narrative` tier.

## Run it

```bash
npm install
npm run dev
```

## Verify it

```bash
npm run verify
```

`verify` builds and then runs `scripts/gates.mjs`, the executable form of
10-WEBSITE §9's budget table. **A budget that is not checked is a preference.**

| Gate | Current |
|---|---|
| Transferred weight per page, gzipped | checked against a 150 KB ceiling |
| Scripts | four reviewed local scripts plus the lazy homepage island |
| Lazy conversion payload | checked against a 240 KB gzip ceiling |
| External requests at runtime | 0 |
| CSP on every page | 14 pages; route-specific exceptions are explicit |
| Banned voice tokens | 26 phrases, 0 matches |
| Words on `/privacy` | 300 / 300 |
| `/privacy` matches `privacy.json` and the README | 5 sections, 2 surfaces |
| Placeholder checksums on a published release | refused |
| Reduced-motion branch on every animated page | 14 pages |
| State glyphs render literally | `=` `≈` `⌇` `✦` |
| Title and description on every page | 14 pages |
| Animated properties restricted to the compositor | pass |
| Dead internal links | 0 / 14 routes |
| Screenshot weight — directory / worst page / hero | 6708 / 8000 KB · 599 / 950 KB · 94 / 120 KB |
| Screenshot integrity — intrinsic size, alt, AVIF fallback, file present | 18 scenes, both themes, both formats |
| Screenshot freshness — captured commit is an ancestor of HEAD | pass |

The word count is taken from the prose inside `<main>`. Counting words in the raw
HTML file returns about 977, most of which are CSS tokens and attribute names.

## Regenerate the data layer

Four of the site's routes are generated from the binary and from the design
record, not written by hand:

```bash
npm run export
```

| Script | Reads | Writes |
|---|---|---|
| `export-data.mjs` | `openconvert formats`, `openconvert routes` over 1,056 pairs | `src/data/formats.json` |
| `export-tools.mjs` | the app's tool descriptors | `src/data/tools.json` |
| `export-cli.mjs` | 15 real CLI invocations against fixtures it builds | `src/data/cli.json` |
| `export-receipt.mjs` | one real `convert --json` run | `src/data/receipt.json` |
| `export-threats.mjs` | `09-THREAT-MODEL.md` §8 | `src/data/not-covered.json` |

It needs a built binary; it looks in `../target/debug/` or takes a path:

```bash
npm run export --prefix site -- ../target/release/openconvert
```

The prose that no table holds — what a format *is*, and what you lose leaving
it — stays authored in `src/data/prose.json`, keyed by the format id the binary
prints. A format with facts and no prose gets a page with facts and no prose,
which is the correct failure.

## The screenshots

Every product screenshot is captured from the desktop preview and encoded by
the product itself. Source captures and conversion receipts are private working
material: they are generated locally and intentionally excluded from Git.

```bash
npm run shots          # 18 scenes x light/dark -> tools/shots/raw/*.png
npm run shots:encode   # openconvert convert *.png -t avif|webp -> public/shots/
```

`encode.mjs` runs `openconvert convert` for every image and keeps the receipt of
each conversion in `tools/shots/receipts/`. A site whose argument is *every
conversion is receipted* should not have its own pictures made by something
else.

Scenes, and the alt text for each, are authored in `tools/shots/scenes.mjs`
beside the click path that produces them. `src/data/shots.json` is generated and
records the commit the app was captured at; the freshness gate fails a build
whose screenshots are older than the code.

## Layout

```
src/
├── data/         formats.json · cli.json · receipt.json · not-covered.json  ← generated
│                 prose.json · release.json                                  ← authored
├── lib/          inline.ts (markdown subset) · toml-tables.ts
├── styles/       tokens.css (inherited) · base.css
├── layouts/      Base.astro                        ← nav, footer, CSP
├── components/   the five sequences + ClassChips
└── pages/        /  /docs  /security  /privacy  /models  /enterprise
                  /download  /formats  /formats/<id>
```

## The five sequences

| ID | Component | What it teaches |
|---|---|---|
| **A2** | `Containment.astro` | Four escape attempts, each denied by the OS by name, and the conversion completes anyway |
| **A3** | `RouteChoice.astro` | The route table is ordered, requirements are named, and the eliminated route records why |
| **A4** | `OnDevice.astro` | A 21 MB model reads a page with the byte counter pinned at zero |
| **A5** | `NothingOverwritten.astro` | A member named `.bashrc` is not traversal — every check passes it, and most extractors then destroy your file |
| **A6** | `Receipt.astro` | The literal stdout of a real conversion, captured by a script |

Each plays once on entry and never loops. Under `prefers-reduced-motion` both
duration *and delay* are zeroed, so every sequence resolves to its end state
instantly — nobody loses information by turning motion off.

### Why they animate `transform` and nothing else

The first version faded rows in from `opacity: 0` on a delay with
`animation-fill-mode: both`. In a tab whose document timeline has not started —
opened in the background, restored from a session, never composited — the
animation sits pending at t=0, the backwards fill holds, and **every row renders
at opacity 0 with the markup fully present.** Measured here:
`document.timeline.currentTime` was 0 while all four containment rows computed
to `opacity: 0`.

10-WEBSITE §6 names this hazard for `requestAnimationFrame`; it exists in pure
CSS too. A frozen clock now costs a few pixels of offset instead of the content.

## The one place a script is budgeted

The homepage carries a mount point for the WebAssembly conversion island —
`#convert-island`, 20 KB, lazy, homepage only. It is not built in this tree, so
the honest measurement today is zero script on all 41 pages and the gate reports
that number rather than the budget.

When the island lands it mounts under its own policy rather than loosening the
page's: production CSP is `script-src 'none'`, so a content page that grew a
script would stop executing it instead of quietly shipping it.
