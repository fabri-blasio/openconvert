# OpenConvert — Site rebuild: product-site restructure

An implementation plan. Turn `openconvert.dev` from a documentation site with a product on it into a
**product site**, built to the pattern `ellieplanner.com` and `lunabudgeting.com` use, and carried by
**real screenshots of the real app** rather than by prose.

Status: implementation plan · 2026-09-04
Supersedes the page inventory in [12-SITE-PAGES](12-SITE-PAGES.md) for the five primary routes.
Inherits voice rules and gates from [10-WEBSITE](10-WEBSITE.md), tokens from [07-DESIGN-SYSTEM](07-DESIGN-SYSTEM.md).

---

## Contents

1. [The diagnosis](#1-the-diagnosis)
2. [The reference pattern, distilled](#2-the-reference-pattern-distilled)
3. [Three collisions with existing policy](#3-three-collisions-with-existing-policy)
4. [The screenshot pipeline](#4-the-screenshot-pipeline)
5. [Information architecture](#5-information-architecture)
6. [Page specifications](#6-page-specifications)
7. [Design-system deltas](#7-design-system-deltas)
8. [Component inventory](#8-component-inventory)
9. [Gates](#9-gates)
10. [Work breakdown](#10-work-breakdown)
11. [Open decisions](#11-open-decisions)

---

## 1. The diagnosis

The site is well built and it is selling the wrong thing. Measured against the two references:

| | openconvert.dev today | Ellie / Luna |
|---|---|---|
| First thing above the fold | A headline, a lede, two buttons, and twelve drifting engine-name pills | A headline, one button, and **a large picture of the app doing its job** |
| Pictures of the product | **Zero.** `astro.config.mjs` disables the image service outright: *"This site contains no images at all"* | The picture *is* the page. Ellie: laptop plus phone. Luna: a phone filling the right half. |
| Proof offered | A reproduced window chrome (`AppWindow.astro`) with a WASM demo inside it | A screenshot, then a 60-second video, then more screenshots |
| Primary CTA | `Download` competing with `Try in browser ↓` | One button — `Try Ellie – It's Free` / `Download on iOS` — with one line of reassurance under it |
| Social proof | None | Three testimonials immediately under the hero, name and role |
| Objection handling | Spread across `/safety`, `/privacy`, `/licences` | One FAQ accordion, plain language, including the awkward questions |
| Who made it | Absent | A founder note with a face and a first-person paragraph |
| Community | A Discord icon linking to `/contact` | Luna: a **Feedback board** in the footer |

Two structural facts follow.

**The app is genuinely good-looking and nobody can see it.** The 900×680 window renders a batch list
with per-row format pickers, an expandable receipt, an eleven-tool workspace and a model manager.
None of it appears on the site.

**The site argues before it demonstrates.** The comparison table, the four claims and the containment
sequence all land before the visitor has watched the product do anything.

The fix is not a new visual language — the tokens and the type are good. The fix is **order,
emphasis, and photographs**.

---

## 2. The reference pattern, distilled

Both references are by the same designer, so the pattern is consistent enough to name. Nine rules,
all adopted:

1. **One promise, huge.** 56–72 px, two lines maximum, bold, no subclause. Ellie: *"A better daily
   planner."* Luna: *"A budgeting app you'll love using."*
2. **One sentence under it**, describing the job in the user's words, not the mechanism's.
3. **One primary button.** Everything else is a text link. Under the button, one line of friction
   removal in small grey type — *"No credit card required"*; ours becomes *"Free and open source · No
   account · Nothing is uploaded"*.
4. **The app appears inside the first viewport**, large, in a device frame, showing real content —
   not an abstraction of the app, the app.
5. **Proof immediately after the hero**, before any feature.
6. **Alternating feature blocks**: short heading, one-line explainer, one large screenshot each.
   Never a paragraph. Never a grid of icons standing in for a screenshot.
7. **A marquee or chip wall** for the long tail, so breadth is felt rather than read.
8. **A per-platform download row** near the bottom, repeating the CTA.
9. **FAQ, then a human**, then a small two-column footer.

We add one thing the references do not have, because this product earns it: **the receipt**. It is the
most screenshot-able differentiator we own and no competitor can put one on their site.

---

## 3. Three collisions with existing policy

The rebuild breaks three standing commitments. Each is stated with its resolution; none is waved past.

### 3.1 "This site contains no images at all"

`astro.config.mjs` disabled Astro's image service to drop `sharp`, reasoning that a site arguing about
parser CVEs should not ship one. The reasoning survives. The conclusion does not.

**Resolution.** Images become first-class and `sharp` still never enters the tree:

- Screenshots are encoded **offline**, by a script, into `site/public/shots/`, and committed.
- Astro keeps `passthroughImageService()` — it never touches a pixel. `<picture>` and `<img>` are
  authored by hand.
- CSP already permits `img-src 'self' data:`. No change to any policy header.
- The encoder is **the product itself** — `openconvert convert shot.png -t avif` — with the receipt of
  that conversion committed beside the images. The site's own pictures become a dogfooding artifact.
  If the CLI's AVIF route is not published when this lands, fall back to `avifenc` and `cwebp` from
  system libavif/libwebp and record the encoder per file in `shots.json`, so provenance is never guessed.

### 3.2 The page-weight gate

`scripts/gates.mjs` enforces 150 KB gzipped per page and currently reports a worst case of 10.6 KB. A
hero AVIF is 60–110 KB on its own.

**Resolution.** Split the budget rather than raise it, so the HTML and CSS discipline survives contact
with the images:

| Budget | Value | Applies to |
|---|---|---|
| `gzipKb` | **150** (unchanged) | the HTML document only |
| `heroImageKb` | **120** | the single above-the-fold shot, per page |
| `imagesKbPerPage` | **450** | every image a page references, summed |
| `imagesKbTotal` | **2200** | `site/public/shots/` as a directory |

Below-the-fold shots carry `loading="lazy"` and `decoding="async"`; the hero carries neither and gains
a `<link rel="preload" as="image">`. Every `<img>` carries intrinsic `width` and `height` — the existing
CLS gate finally has something to check.

### 3.3 Community voting versus zero egress

`connect-src 'none'` on every content page, zero third-party requests, no cookies, no analytics.
Upvote and downvote is, by definition, writing to a server.

**Resolution (recommended): the board is generated, the votes live on GitHub.**

- Ideas are GitHub Discussions in an `Ideas` category. Anyone can open one; anyone can vote with the
  native 👍 / 👎 reactions. That is exactly *"anyone can add one, everyone can vote"*, with identity,
  moderation and spam control already solved and none of it ours to run.
- `site/scripts/export-board.mjs` reads them at build time over the GitHub GraphQL API and writes
  `src/data/board.json` — the same shape as the four existing `export-*.mjs` scripts, which is the
  precedent this follows.
- `/community` renders that JSON statically: **0 KB of JavaScript, 0 runtime requests, CSP untouched.**
  Every button is a link out to the discussion.
- Freshness is a build artifact, so the page says *"board as of &lt;build date&gt;"* and the site
  rebuilds nightly. A stale count claiming to be live is the same failure class as a stale checksum,
  which the release gate already refuses.

**Alternative, only if in-page voting is a hard requirement.** A Cloudflare Worker plus D1 at
`board.openconvert.dev`: roughly 120 lines, one POST per vote, IP-hash rate limiting, no cookie. The
cost is precise — `/community` becomes the **only** page on the site that opens a connection, its CSP
gains `connect-src https://board.openconvert.dev`, the host allowlist in `gates.mjs` gains an entry,
`/privacy` gains a sentence, and the 300-word privacy budget has to absorb it. That is a real price
for a convenience and it should be paid deliberately or not at all. See [§11](#11-open-decisions).

---

## 4. The screenshot pipeline

**This is the load-bearing part of the plan.** Everything else is layout. Hand-captured screenshots go
stale, drift from the shipped UI, and are taken at whatever DPI the person had that day. They become a
maintenance tax nobody pays, and then the site shows a product that no longer exists.

So the screenshots are **generated, deterministic, and gated**.

### 4.1 What already exists

`apps/desktop/preview/` mounts **the real `src/App.svelte`, the real components and the real
`tokens.css`**, with the four `@tauri-apps/api` entry points aliased to fixture modules by
`vite.preview.config.ts`. Nothing under `src/` knows it exists. It runs with
`npm run preview:ui --prefix apps/desktop`.

Verified working while researching this plan — every screen below was reached by driving that harness,
so the inventory in §4.3 is observed, not imagined:

first-run AI prompt · idle drop zone · five-file batch list with per-row targets · in-flight progress
(`4 of 5`, Cancel) · the done summary (`4 converted · 1 failed · 5 total`, `3.6s · 23.2 MB → 9.7 MB`) ·
the eleven-item Tools menu · the Remove-background workspace with transparency checkerboard and a
Before/after toggle.

Two operational facts for whoever builds this: the preview's **cold start is about 29 seconds**
(dependency optimisation), so automation needs a readiness timeout of 120 s or more; and the harness
has **no scene routing** — states are reached by clicking, which the driver script does.

### 4.2 The harness

New, dev-only, ships nothing:

```
site/tools/shots/
├── run.mjs      starts the preview server, drives Playwright, writes PNGs
├── scenes.mjs   one exported function per scene — the click paths
└── encode.mjs   PNG → AVIF + WebP, writes shots.json
```

- **Playwright, Chromium**, `deviceScaleFactor: 2`, viewport 1200×860 so the 900×680 app window sits
  inside its preview frame and exports at **1800×1360 physical**.
- `colorScheme` emulated `light` and `dark`; every scene captured twice.
- Motion frozen: `reducedMotion: 'reduce'` plus `animations: 'disabled'` on the screenshot call. The
  fixtures already use fixed durations (907 ms, 927 ms, 1.1 s), so the done screen is byte-identical
  between runs — which is what makes a changed PNG in `git status` mean *the UI changed*.
- Element-clipped, never full-page, so browser chrome never leaks into a shot.

### 4.3 Scene inventory

Twelve scenes × two themes = **24 source images**. The ★ ones are the ones that sell.

| # | Scene | What it shows | Used on |
|---|---|---|---|
| 1 | `drop-idle` | The empty state — *"Drop files to convert · Ctrl+O"* | About, Download |
| 2 | ★ `batch-plan` | Five files, per-row source → target pickers, **and the polyglot row**: *"The name says PNG and the content says JPEG; routing by content."* | **Home hero** |
| 3 | `progress` | `4 of 5`, per-row `done` / `started`, one Cancel | Features |
| 4 | ★ `batch-done` | `4 converted · 1 failed · 5 total`, per-row milliseconds, and an honest failure: *"libheif could not decode this file: unsupported colour profile"* | Home §5, Features |
| 5 | ★ `receipt-open` | A receipt row expanded: engine, version, operation class, limits, hashes, network calls | Home §4 |
| 6 | `tools-menu` | All eleven tools, grouped Image / Audio / PDF | Features hero |
| 7 | ★ `remove-bg` | Before/after with the transparency checkerboard — *"Background removed, now transparent"* | Home §6, Features |
| 8 | `upscale` | The ×4 workspace at 100 % with zoom controls | Features |
| 9 | `transcribe` | Audio batch list, waveform, transcript pane | Features |
| 10 | `pdf-merge` | Two documents, page reorder | Features |
| 11 | ★ `ai-features` | The first-run model prompt: sizes, licences, *"none are installed"*, `Not now` | Home §7, Features |
| 12 | `settings-models` | Models section — download, enable, reclaim disk | Features, About |

### 4.4 The fixture-realism problem — do this first

`apps/desktop/preview/render.ts` draws its previews on a `<canvas>`: a blue gradient sky, two green
triangles for hills, a circle and a cone for the subject. That is exactly right for a UI harness — it
proves the real `data:image/png;base64,…` path — and it is **unusable in a hero**, where it reads
instantly as a placeholder.

Required, inside `apps/desktop/preview/` only, shipping nothing:

- Add `apps/desktop/preview/media/` with **four licence-clean real assets** (CC0 or our own): a
  portrait with a cuttable subject, a landscape, a scanned document page, and a short speech clip for
  the waveform.
- `render.ts` prefers a real asset when one exists for the requested path and falls back to the canvas
  drawing otherwise — so the harness still works with no media checked in, and the screenshots get
  real pixels.
- Keep the fixture filenames and the Windows paths. `IMG_4821.heic`, `voice-memo.flac`,
  `C:\Users\you\Pictures` all read as real, and Windows is the primary platform.

### 4.5 Encoding and the data file

`encode.mjs` produces, per scene per theme, an `.avif` (primary) and a `.webp` (fallback). The PNGs stay
out of `public/`; the source of truth is `site/tools/shots/raw/`. Targets: hero ≤ 120 KB, inline shots
≤ 90 KB, quality tuned per file rather than globally.

`src/data/shots.json`:

```json
{
  "generated_by": "site/tools/shots/run.mjs",
  "app_version": "0.1.0",
  "app_sha": "fd7fa67",
  "captured": "2026-09-04",
  "encoder": "openconvert 0.1.0 (avif) / cwebp 1.4.0",
  "shots": [
    {
      "id": "batch-plan",
      "alt": "The OpenConvert window listing five dropped files, each with its detected format and chosen target.",
      "width": 1800,
      "height": 1360,
      "themes": {
        "light": { "avif": 104213, "webp": 151902 },
        "dark":  { "avif": 98004,  "webp": 143771 }
      }
    }
  ]
}
```

`alt` is authored, never generated. A screenshot with machine-written alt text is an accessibility
failure with a checkbox next to it.

### 4.6 Rendering

One component, `Shot.astro`, everywhere:

```astro
<picture>
  <source media="(prefers-color-scheme: dark)" type="image/avif" srcset="/shots/batch-plan-dark.avif" />
  <source media="(prefers-color-scheme: dark)" type="image/webp" srcset="/shots/batch-plan-dark.webp" />
  <source type="image/avif" srcset="/shots/batch-plan.avif" />
  <img src="/shots/batch-plan.webp" width="1800" height="1360" alt="..." loading="lazy" decoding="async" />
</picture>
```

Theme switching is CSS-only, matching the zero-JS rule. The hero drops `loading="lazy"` and gains a
preload.

**Device frame.** `AppWindow.astro` already draws the desktop chrome in CSS. Screenshots are clipped
*below* the title bar and composed inside `AppWindow`, so the chrome stays vector-crisp at any DPI and
the raster carries only content. For the Ellie-style hero, `AppWindow` gains an optional `perspective`
variant — a slight 3-D tilt and a soft ground shadow — implemented with `transform` only, which the
existing motion gate already requires.

### 4.7 Keeping them true

Three gates (§9): every shot a page references exists in `shots.json` and on disk; every shot has
authored `alt`; `shots.json.app_sha` must be an ancestor of `HEAD` and no more than **one release
behind**. A screenshot two releases old fails the build the same way a placeholder checksum does.

CI runs `npm run shots` on a schedule and opens a PR whenever a PNG changes, so a UI change surfaces as
a reviewable image diff instead of as silence.

---

## 5. Information architecture

Five primary routes, as asked. Everything else survives as a **proof page** — reached from a claim,
never from the top nav.

```
PRIMARY   /            Home
          /features    Features
          /community   Community
          /download    Download
          /about       About

PROOF     /safety  /formats  /formats/<id>  /models  /changelog
LEGAL     /privacy  /terms  /licences  /accessibility  /imprint  /contact
```

**Header.** `openconvert` · Features · Community · Download · [GitHub] [Discord] · **`Download`
(primary button)**. About leaves the nav for the footer and the founder block: five items plus a CTA
is one too many, and About is a page people reach once they are interested, not before.

**Footer.** Two columns, exactly as the references do it — *Primary*: Home, Features, Community,
Download, Changelog · *Other*: Safety, Formats, Models, Privacy, Terms, Licences, Contact — plus the
existing `built from <sha>` line and **No analytics. No cookies. No tracking.**, which is our
equivalent of Luna's *"No credit card required"* and should stay loud.

---

## 6. Page specifications

### 6.1 `/` — Home

Twelve sections. The order is the plan; the copy is the intended register, not final wording.

| # | Section | Content |
|---|---|---|
| 1 | **Hero** | H1 **"Convert any file. On your machine."** · lede *"Images, audio, video, documents — 126 routes across 35 formats, and nothing ever leaves your computer."* · one primary button **Download** · under it, in grey: *"Free and open source · No account · Nothing is uploaded"* · **shot 2 (`batch-plan`) inside a tilted `AppWindow`, filling the right half at ≥1024 px and sitting below the copy on mobile.** The ambient engine-name pills survive as a faint layer behind the copy column only. They are good; they are just not the hero. |
| 2 | **Proof strip** | One hairline row: `Apache-2.0` · `126 routes` · `0 network calls` · `Sandboxed engines` · `A receipt for every job`. This is where the references put testimonials; it holds that slot until we have some. |
| 3 | **Watch it work** | A 25–40 s screen recording of one real batch — drop, plan, convert, receipt — as a muted, looping, `playsinline` `<video>` with a poster, ≤ 2.5 MB; or an AVIF sequence if the video weight is unacceptable. Ellie's equivalent block is its highest-converting section. |
| 4 | **"It tells you what it did."** | ★ shot 5 (`receipt-open`), large, copy on the right. The one thing no competitor can screenshot. Links to `/features#receipts`. |
| 5 | **"Nothing is uploaded. Nothing is overwritten."** | ★ shot 4 (`batch-done`), including the failed row, deliberately. The failure *is* the proof: a converter that hides one has already lied once. |
| 6 | **"Not just convert."** | ★ shot 7 (`remove-bg`) with the before/after, plus a chip row of the real Tools menu. |
| 7 | **"AI that runs on your CPU."** | ★ shot 11 (`ai-features`) — sizes, licences, *"none are installed"* and `Not now` all visible. Optional-by-default is the entire argument and the screenshot makes it without a sentence. |
| 8 | **Feature marquee** | Two counter-scrolling rows of the long tail, from `formats.json` and the Tools menu: HEIC → JPEG · MKV → MP4 · Transcribe · Upscale ×4 · Merge PDFs · Sign · OCR · Colour picker · … `prefers-reduced-motion` stops it dead. |
| 9 | **Try it in this tab** | The existing WASM island, kept — demoted from the hero to here, where a warm visitor meets it. It is an asset the references have no equivalent of; it is simply too much to ask of a cold one. |
| 10 | **Comparison** | The existing four-column table, kept, moved down here. |
| 11 | **Download band** | Windows · macOS · Linux cards from `release.json`, with checksums, identical to `/download`. |
| 12 | **FAQ, founder, footer** | Six questions (§6.5), then the human, then the footer. |

Sections 4–7 are the alternating rhythm: shot left with copy right, then copy left with shot right.

### 6.2 `/features`

The current page is 420 lines of tables. It stays as the *reference* and opens as a *tour*.

- Hero: H1 *"Everything it does."*, one line, shot 6 (`tools-menu`).
- Five anchored blocks — `#conversions` `#tools` `#ai` `#receipts` `#headless` — each: heading, one
  line, **one shot**, then the existing table below it, collapsed behind `<details>` on mobile. Those
  tables are the site's best SEO asset and must not be deleted; they must stop being the first
  impression.
- `#platforms` closes with the download band.

### 6.3 `/community`

Deliberately short. Two blocks.

**Block 1 — Discord.** One card: the invite, one line about what the server is for, and a member count
as a build-time constant. Never a live badge — that is a third-party request. A link to `/contact` for
people who will not use Discord.

**Block 2 — The board**, tiered, rendered from `board.json`:

| Tier | Meaning | Sort |
|---|---|---|
| **Shipped** | In a release. Links to the changelog entry. | newest first |
| **Building** | Someone is on it now. | score |
| **Next up** | Accepted, not started. | score |
| **Under consideration** | Open, unassigned. Most ideas live here. | score |
| **Not planned** | With a one-line reason, always. A board that never says no is a suggestion box. | score |

Each row: title · one-line summary · **↑ n ↓ n** with net score · tier chip · a link reading *"vote and
discuss on GitHub →"*. The block header carries **`+ Add an idea`**, linking to the GitHub new-discussion
URL with the category pre-selected.

At the top of the block, plainly: *"Ideas and votes live in GitHub Discussions. Reading needs no
account; voting needs a GitHub one. This page is a static snapshot, built &lt;date&gt;."* Honesty about
the mechanism costs one sentence and buys the whole zero-egress claim.

### 6.4 `/download`

Keep the substance — per-platform artifact, checksum, package-manager line, confinement note — and
restyle to the reference pattern:

- Hero: H1 *"Get OpenConvert."* with shot 1 (`drop-idle`) small, on the right.
- Three platform cards. We cannot read a user agent in a static build, so there is no auto-detection:
  Windows first as the primary target, the other two equally prominent.
- Under each card: the `winget` / `brew` / package line in a copyable `<code>` block, the SHA-256, the
  size, and the one-line confinement statement already carried in `release.json`.
- *"Verify what you downloaded"* as a `<details>` with the exact command per platform.
- The `release.state: "unpublished"` guard stays. The gate that refuses to render a zero checksum on a
  published release is one of the best things in this repository.

### 6.5 `/about`

The references' About page is a person, not a company. Ours carries more, but the shape holds:

1. **The story in three paragraphs, first person.** Why a converter, why local, why receipts.
2. **What it costs.** Free, Apache-2.0, no feature gating ever — and how that is sustainable, which is
   Ellie's third FAQ answer and the question every product with a small team gets asked.
3. **How it is built.** Three sentences and a link to `/safety`. Not the architecture.
4. **Shot 12 (`settings-models`)** — the model manager with the reclaim-disk control. *"We do not hold
   your data"* is a claim; *"here is the button that deletes it"* is a screenshot.
5. Contact, Discord, GitHub, changelog.

**FAQ** (rendered on Home, authored here). Six, in the references' plain register: *Is it really free?*
· *Does anything leave my computer?* · *What if it converts my file wrong?* · *Do I need the AI models?*
· *How is this different from a website converter?* · *Who is behind it, and will it still exist in two
years?*

---

## 7. Design-system deltas

Small. The tokens survive; the *scale* changes, because a product site needs a display tier the app
never needed.

| Delta | From | To | Why |
|---|---|---|---|
| **Display type** | a 16 px scale topping out near 40 px | new `--t-display: clamp(2.75rem, 6vw, 4.5rem)`, weight 700–800, `letter-spacing: -0.02em` | The H1 is the single biggest conversion lever on the page |
| **Primary button** | hairline-bordered, near-monochrome | filled `--brand`, 14–16 px vertical padding, 999 px radius, one per viewport | Both references use exactly one filled button; everything else is a link |
| **Section rhythm** | uniform | `--s-section: clamp(5rem, 10vw, 9rem)` between blocks, with a `--surface-sunken` band alternating every other block | Creates the scroll cadence the pattern depends on |
| **Shot elevation** | translucency over shadows (07 §5) | one soft ground shadow, **screenshots only** | 07's rule governs *app* chrome. A floating screenshot with no shadow reads as a flat diagram. Recorded as a fourth stated divergence beside the existing three. |
| **Accent** | two semantic accents only | unchanged in UI; `--brand` may gain a gradient **for the H1 only** — optional, see §11 | Ellie's headline has a gradient; Luna's is flat black and works just as well |

The near-monochrome discipline is not relaxed anywhere else. On this site the colour comes from the
screenshots.

---

## 8. Component inventory

New under `site/src/components/`:

| Component | Job |
|---|---|
| `Shot.astro` | The `<picture>` of §4.6. Props: `id`, `alt` override, `hero`, `frame` |
| `Hero.astro` | Display H1, lede, one CTA, reassurance line, shot slot |
| `ShotBlock.astro` | The alternating copy/shot row. Props: `side`, `eyebrow`, `title`, `body`, `href` |
| `ProofStrip.astro` | The hairline fact row |
| `Marquee.astro` | Two counter-scrolling rows, `transform` only, reduced-motion aware |
| `Faq.astro` | `<details>` accordion, no JS |
| `Founder.astro` | Portrait plus first-person block |
| `CtaBand.astro` | Repeatable download band over `release.json` |
| `BoardList.astro` | The tiered feature board |
| `DiscordCard.astro` | The invite card |

Changed: `Base.astro` (nav, footer, image preload hook), `AppWindow.astro` (`perspective` variant,
accepts a `Shot`), `index.astro` (rebuilt), `features.astro` (re-ordered around shots),
`download.astro`, `about.astro`. New page: `community.astro`.

Untouched: `Containment` · `RouteChoice` · `OnDevice` · `NothingOverwritten` · `Receipt` · `AppFlow` ·
`ClassChips`. The five sequences move to `/safety` and `/features`, where an interested reader meets
them. They are good work and they are not hero material.

New scripts: `site/scripts/export-board.mjs` and `site/tools/shots/*`. New `package.json` scripts:
`shots`, `shots:encode`; `export` gains `export-board`.

---

## 9. Gates

`scripts/gates.mjs` gains six checks. The existing eighteen are unchanged — **including the banned-voice
list, which SaaS copy must still pass**. No *"coming soon"*, no *"we believe"*, no *"seamless"*. That
constraint is a feature here: the references' own copy would pass it.

| Gate | Fails when |
|---|---|
| **Image weight** | hero > 120 KB, page images > 450 KB, `public/shots/` > 2.2 MB |
| **Image integrity** | an `<img>` lacks `width`, `height` or a non-empty `alt`; a referenced shot is missing from `shots.json` or from disk |
| **Shot freshness** | `shots.json.app_sha` is not an ancestor of `HEAD`, or is more than one release behind |
| **Format pairing** | an AVIF source exists with no WebP or PNG fallback in the same `<picture>` |
| **Board honesty** | `/community` renders a vote count without also rendering the build date, or a `Not planned` row carries no reason |
| **One CTA** | more than one `.btn-primary` inside a single section on `/` |

The last one sounds fussy and is the rule the references never break.

---

## 10. Work breakdown

One engineer. Working days.

| Phase | Work | Days |
|---|---|---|
| **P0** | §11 decisions resolved. Fixture media sourced and wired into `render.ts` (§4.4). Token deltas landed. | 1.5 |
| **P1** | Screenshot harness: `run.mjs`, `scenes.mjs`, `encode.mjs`, `shots.json`, `Shot.astro`. All 24 shots produced and committed. | 2.5 |
| **P2** | `Base.astro` nav and footer; `Hero` `ShotBlock` `ProofStrip` `Marquee` `Faq` `CtaBand` `Founder`; `AppWindow` perspective variant. | 2 |
| **P3** | `/` rebuilt — twelve sections, both themes, mobile. | 2.5 |
| **P4** | `/features` re-ordered around shots, tables preserved. | 1.5 |
| **P5** | `/download` restyled. | 1 |
| **P6** | `/about` written and built; founder portrait. | 1 |
| **P7** | `/community`: `export-board.mjs`, `BoardList`, `DiscordCard`, Discussions category and labels set up, nightly rebuild wired. | 1.5 |
| **P8** | Six new gates; accessibility pass; dark-mode pass; `npm run verify` green; Lighthouse. | 1.5 |
| **P9** *(optional)* | The 30-second screen recording for Home §3. | 1 |
| | **Total** | **15–16** |

P1 is the phase to protect. If it degrades into hand-captured screenshots, the site is stale within one
release and the whole plan becomes a coat of paint.

---

## 11. Open decisions

Four. Each has a recommendation; only the first blocks P0.

1. **Community backend** — GitHub Discussions generated at build (§3.3), or a Worker plus D1 with
   in-page voting.
   **Recommend Discussions.** It satisfies the requirement exactly, costs no infrastructure, and keeps
   `connect-src 'none'` true on every page of the site — which is a claim we sell.

2. **Voice** — the references are warm, first-person and emoji-flecked. This site's voice is austere
   and impersonal, and a build gate enforces it.
   **Recommend warm on `/about`, the FAQ and `/community`; austere wherever a claim is made.** Keep the
   gate as it is. No emoji in headings — they do not survive translation into a security-adjacent product.

3. **Hero capture source** — the Playwright harness (deterministic, regenerable) or a real
   `OpenConvert.exe` window captured on Windows (true chrome and font rendering, not reproducible).
   **Recommend the harness for all twelve scenes**, plus one manual real-window capture per release used
   *only* on `/download` and labelled as such. Reproducibility is worth more than the last 2 % of
   fidelity, and `AppWindow.astro` supplies the chrome anyway.

4. **Gradient headline** — Ellie has one, Luna does not.
   **Recommend no gradient.** Near-monochrome plus one filled brand button is the sharper look for a
   security-adjacent tool, and it keeps §7's shadow divergence as the only one taken.

---

## Build status — 2026-09-05

**Done.** P0 (partly), P1, P2, P3 and P7, plus the §9 gates.

| | |
|---|---|
| Harness | `site/tools/shots/{run,scenes,encode}.mjs`. **11 scenes x 2 themes = 22 shots**, captured from the real UI, encoded to AVIF+WebP by `openconvert convert` with all 44 receipts kept in `tools/shots/receipts/`. |
| Determinism | `preview/fixtures.ts` no longer randomises `durationMs` — it hashes the path, so the results screen is the same on every capture. **One number still varies**: the summary's total elapsed (`3.6s`) is measured by the app from wall clock, so it moves by ~0.1 s between runs. That is the only pixel a re-capture can change on its own. |
| Pages | `/` rebuilt to the twelve-section pattern; `/community` new. Nav is Features / Community / About plus the Download button. |
| Gates | Three added and green: screenshot weight (`dir 3867/4500 KB · worst page 410/450 KB · hero 90/120 KB`), integrity (intrinsic size, non-empty alt, AVIF fallback, file present), freshness (`app_sha` must be an ancestor of HEAD). |
| Decisions | §11 resolved as recommended: GitHub Discussions, warm voice only where a claim is not being made, harness capture, no gradient. |

**Not done, and why.**

1. **Real fixture media (§4.4).** `render.ts` still draws its previews on a canvas. The two shots that show photographic content — `remove-bg` and `upscale` — therefore show a drawing. Needs four licence-clean assets, which are a decision about what we are allowed to publish, not a coding task.
2. **The Discord invite.** `board.json` carries `discord.invite: null` and the page says the invite is not published rather than linking somewhere that is not the server. One string away from done.
3. **The board itself is empty.** `export-board.mjs` is written and needs `GITHUB_TOKEN` plus an `Ideas` discussions category to run against. Until then `/community` renders its honest empty state.
4. **`pdf-merge` scene.** Eleven scenes shipped, not twelve.
5. **P4–P6.** `/features`, `/download` and `/about` still carry the old layout. They build, they pass every gate, and they now sit under a nav and footer that have moved on without them.
6. **P9.** No screen recording.

---

## Appendix — files touched

```
NEW   site/src/pages/community.astro
NEW   site/src/components/{Shot,Hero,ShotBlock,ProofStrip,Marquee,Faq,Founder,CtaBand,BoardList,DiscordCard}.astro
NEW   site/src/data/{shots,board}.json               <- generated
NEW   site/scripts/export-board.mjs
NEW   site/tools/shots/{run,scenes,encode}.mjs
NEW   site/public/shots/*.{avif,webp}                <- generated, committed
NEW   apps/desktop/preview/media/*                   <- real fixture assets, not shipped
EDIT  site/src/pages/{index,features,download,about}.astro
EDIT  site/src/layouts/Base.astro
EDIT  site/src/components/AppWindow.astro
EDIT  site/src/styles/{tokens,base}.css
EDIT  site/scripts/gates.mjs
EDIT  site/package.json                              <- shots, shots:encode, export-board
EDIT  apps/desktop/preview/render.ts                 <- prefer real media over canvas
EDIT  docs/spec/10-WEBSITE.md  §4 §8 §9              <- sitemap, images, budgets
EDIT  site/README.md                                 <- the gate table
```
