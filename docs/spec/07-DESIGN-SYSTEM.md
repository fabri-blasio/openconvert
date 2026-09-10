# Design System & Brand

Visual language, tokens, and naming.

Status: design record, v0.2 · 2026-08-13
Part 7 of 9 — see [README](README.md).

---

## Contents

1. [The feel](#1-the-feel)
2. [The tension, resolved](#2-the-tension-resolved)
3. [Colour](#3-colour)
4. [Encoding state without hue](#4-encoding-state-without-hue)
5. [Materials & elevation](#5-materials--elevation)
6. [Typography](#6-typography)
7. [Space, radius, layout](#7-space-radius-layout)
8. [Motion](#8-motion)
9. [Iconography](#9-iconography)
10. [Component patterns](#10-component-patterns)
11. [Accessibility floor](#11-accessibility-floor)
12. [Token architecture](#12-token-architecture)
13. [CLI & web parity](#13-cli--web-parity)
14. [Naming](#14-naming)
15. [Open decisions](#15-open-decisions)

---

## 1. The feel

**A precision instrument, not a consumer app.** This software is trusted with contracts, medical scans, and legal discovery. It should feel like a well-made tool: quiet, certain, and unhurried.

| Principle | In practice |
|---|---|
| **Monochrome by default** | The interface is neutral. Colour appears only where meaning is at stake. |
| **Hierarchy through weight, size and opacity** — never hue | Three text opacities and two weights do the work colour usually does |
| **Depth through translucency**, not borders and shadows | Layered materials; content shows through |
| **Space is the primary tool** | Generous padding; few dividers; grouping by proximity |
| **Motion is functional only** | It shows where something came from. Nothing decorative. |
| **Nothing decorative anywhere** | No gradients-as-ornament, no illustrations, no mascots, no confetti |
| **The content is the interface** | File names, sizes, formats and times — set them beautifully and get out of the way |

Reference points: macOS system utilities, Things, Linear's density, Apple's Settings. **Not** Notion, not a dashboard, not anything with a hero gradient.

---

## 2. The tension, resolved

The brief asks for near-monochrome. The product needs four operation classes — **lossless / lossy / inferred / generative** — to be unmistakable, because [the determinism boundary](01-VISION.md#42-verifiable-conversion) is the core promise.

These would conflict, except that **colour was never allowed to carry that meaning anyway.** Section 508 and EN 301 549 are procurement gates ([04-GROWTH §10.5](04-GROWTH.md#105-compliance-gates)), and both forbid status encoded by colour alone.

So the constraint and the aesthetic point the same way:

> **State is carried by glyph, label and prominence. Colour is reinforcement, never the signal.**

This makes the monochrome brief *easier* to satisfy than a colourful one, and it makes the accessibility requirement free instead of a retrofit.

---

## 3. Colour

### 3.1 The neutral ramp

Pure neutral — no cast by default, with an optional 2% cool tint exposed as a token for anyone who wants it.

| Token | Light | Dark |
|---|---|---|
| `neutral-0` | `#FFFFFF` | `#000000` |
| `neutral-50` | `#FAFAFA` | `#0A0A0A` |
| `neutral-100` | `#F4F4F5` | `#141414` |
| `neutral-200` | `#E8E8EA` | `#1F1F1F` |
| `neutral-300` | `#D4D4D6` | `#2A2A2A` |
| `neutral-400` | `#A1A1A6` | `#3D3D3D` |
| `neutral-500` | `#79797E` | `#5A5A5A` |
| `neutral-600` | `#5A5A5F` | `#7C7C7C` |
| `neutral-700` | `#3D3D42` | `#A3A3A3` |
| `neutral-800` | `#2A2A2E` | `#C9C9C9` |
| `neutral-900` | `#1D1D1F` | `#EDEDED` |
| `neutral-1000` | `#000000` | `#FFFFFF` |

Note the dark column **inverts** rather than mirrors — `neutral-900` is the primary text colour in both themes.

**Never pure black on pure white.** Text is `neutral-900` (`#1D1D1F`), background `neutral-0`. Pure `#000` on `#FFF` is harsh and reads as unfinished.

### 3.2 Text opacity, not text colour

Three levels do almost all hierarchy work:

| Role | Value |
|---|---|
| `text-primary` | `neutral-900` |
| `text-secondary` | `neutral-900` at **62%** |
| `text-tertiary` | `neutral-900` at **38%** |
| `text-disabled` | `neutral-900` at **24%** |

Using opacity rather than separate greys means text sits correctly on *any* surface, including translucent ones, with no per-surface variants.

### 3.3 Accents — exactly two

| Token | Light | Dark | Used for | Never used for |
|---|---|---|---|---|
| `accent-attention` | `#B4690E` | `#E0A458` | Class D (generative), warnings, "this changes your file" | Decoration, branding, links, hover |
| `accent-critical` | `#C0392B` | `#E5766A` | Blocked, unsafe, quarantined, destructive confirmation | Anything non-critical |

Both are **desaturated** and appear as small areas only — a chip, a glyph, a 2 px rule. Neither is ever a background fill larger than a chip.

**There is no brand colour in the UI.** If a brand hue exists, it lives on the website and the icon, not in the product chrome. That's the most Apple-minimal decision available and it removes a whole class of taste arguments.

---

## 4. Encoding state without hue

The four classes, ordered by how much the user should care:

| Class | Chip | Glyph | Label | Prominence |
|---|---|---|---|---|
| **A — Lossless** | none, or hairline outline | `=` | `lossless` | **Silent.** It's the good outcome; don't decorate it. |
| **B — Lossy** | hairline outline, `text-secondary` | `≈` | `lossy` | Quiet |
| **C — Inferred** | **dashed** outline, `text-secondary` | `⌇` | `AI-read` | Visibly provisional |
| **D — Generative** | **filled**, `accent-attention` | `✦` | `AI-generated` | The only loud thing on screen |

Four mechanisms distinguish them — **fill, border style, glyph, and label** — and all four survive greyscale, colour-blindness, and a monochrome printout.

> ⚠️ **The glyphs are a dependency, and they get a CI gate.** `⌇` (U+2307) and `⛨` (U+26E8) are almost certainly *not* in Inter, and `✦` (U+2726) may not be either — which would mean the mechanism the accessibility argument leans on silently falls back to whatever the platform happens to have, differing on all three, and to whatever a terminal has in the CLI. Two fixes, both cheap and both required before the GUI ships: **subset the shipped font to include every state glyph**, drawing them into the family if upstream lacks them, and **gate it in CI** — a check that every glyph in the state vocabulary resolves in the shipped font file, no fallback permitted. In the CLI, where we control no font at all, the renderer detects capability and falls back to ASCII (`=`, `~`, `?`, `*`, `[#]`) rather than emitting a box. **Label and border style carry the meaning even if a glyph fails**, which is why this is a defect to fix rather than a hole in the argument — but a mechanism nobody verified is not a mechanism.

```
  ⇄ stream copy   = lossless          ← hairline, quiet
  ≈ re-encode     ≈ lossy  −71%       ← hairline, secondary
  ⌇ OCR           ⌇ AI-read           ← dashed, provisional
  ✦ upscale 4×    ✦ AI-generated      ← filled, attention
```

**Prominence scales with consequence.** A user scanning a 40-step plan sees exactly the steps that alter their data.

---

## 5. Materials & elevation

Translucency instead of shadows.

| Token | Blur | Light fill | Dark fill | Use |
|---|---|---|---|---|
| `material-thin` | 20 px | `neutral-0` @ 62% | `neutral-100` @ 62% | Chips, inline popovers |
| `material-regular` | 32 px | `neutral-0` @ 76% | `neutral-100` @ 76% | Sidebars, the Edge Strip, the tray popover |
| `material-thick` | 48 px | `neutral-0` @ 88% | `neutral-100` @ 88% | Sheets, settings panels |

**Dark-mode elevation gets *lighter*, not shadowed.** Each layer forward moves one step up the neutral ramp. Shadows in dark mode read as dirt.

| Surface | Light | Dark |
|---|---|---|
| `surface-base` | `neutral-0` | `neutral-50` |
| `surface-raised` | `neutral-0` + hairline | `neutral-100` |
| `surface-overlay` | `material-regular` | `material-regular` |

Borders are hairlines: **1 physical pixel**, `neutral-900` at 8% (light) / `neutral-1000` at 10% (dark). Not grey lines — tinted transparency, so they sit correctly on any surface.

⚠️ **Two costs to budget for.** Backdrop blur is expensive in a webview — cap the number of simultaneously blurred surfaces and profile on integrated graphics. And **`prefers-reduced-transparency` must be honoured**: both macOS and Windows expose this setting, and translucency must degrade to opaque fills without any layout change.

---

## 6. Typography

### 6.1 Faces

| Role | Face | Licence |
|---|---|---|
| **UI** | **Inter** (variable) | SIL OFL 1.1 |
| **Mono** | **JetBrains Mono** or **Geist Mono** | SIL OFL 1.1 |

**Inter** is the recommendation: neutral grotesque, exceptional at small sizes, a real `opsz` optical-size axis, tabular figures, and very broad language coverage. Alternatives worth a look before committing: **Geist** (warmer, more distinctive), **Public Sans** (institutional-neutral), **IBM Plex Sans** (more character, slightly technical).

**OFL is not a preference — it's a rule.** A converter that warns users about font EULAs cannot itself ship a restrictively licensed typeface.

**Ship the font; don't use `system-ui`.** Consistency across the three platforms matters more than native mimicry, and Inter looks correct on all of them. Cost is ~300 KB against a 60 MB budget. *(The tradeoff: on Windows the UI will read as designed rather than as native. Accepted.)*

### 6.2 Scale

Desktop-utility density — closer to macOS's 13 px body than the web's 16 px.

| Token | Size / line-height | Weight | Use |
|---|---|---|---|
| `text-caption` | 11 / 14 | 400 | Metadata, receipts, timestamps |
| `text-small` | 12 / 16 | 400 | Secondary rows, chip labels |
| `text-body` | 13 / 18 | 400 | **Default** |
| `text-emphasis` | 13 / 18 | 550 | The armed suggestion, key numbers |
| `text-title` | 15 / 20 | 550 | Section headers, card titles |
| `text-heading` | 21 / 26 | 600 | Screen titles |
| `text-display` | 28 / 34 | 600 | Empty states, onboarding |

Two weights only — **400 and 550/600**. A third weight is almost always a hierarchy problem in disguise.

### 6.3 Numerals — non-negotiable

```css
font-variant-numeric: tabular-nums;
```

Applied to **every** size, duration, count, percentage, and dimension. This is a converter; numbers sit in columns and must align. Proportional figures in a file list look broken even to people who can't say why.

### 6.4 Monospace

Reserved for **paths, hashes, format identifiers, CLI output, and receipt JSON**. Never for prose, never for numbers in a UI list (that's what tabular figures are for).

---

## 7. Space, radius, layout

**4 px base unit.** Spacing scale: `2, 4, 8, 12, 16, 24, 32, 48, 64`.

| Radius | Value | Use |
|---|---|---|
| `radius-sm` | 6 px | Chips, inputs, small controls |
| `radius-md` | 10 px | Cards, list rows |
| `radius-lg` | 14 px | Sheets, panels |
| `radius-full` | 999 px | Pills, avatars |

⚠️ Apple's look comes partly from **continuous corners (squircles)**, which CSS `border-radius` only approximates. If it matters, use an SVG or Houdini path for the largest surfaces; for chips and rows the difference is invisible.

**Density:** default `comfortable` (36 px rows), with `compact` (28 px) available. Power users converting 200 files want compact; first-time users don't.

**Layout:** single-column-first. Content max-width ~720 px in the main flow — a converter is a list, not a dashboard, and full-width rows are harder to scan.

---

## 8. Motion

| Token | Duration | Curve | Use |
|---|---|---|---|
| `motion-micro` | 120 ms | `ease-out` | Hover, focus, chip state |
| `motion-standard` | 220 ms | `cubic-bezier(0.32, 0.72, 0, 1)` | Panels, expansion, list changes |
| `motion-large` | 340 ms | same | Edge Strip, sheets, view transitions |

That curve is a spring approximation — fast start, long settle. It's most of what makes motion feel Apple-like.

**Rules:** motion only ever shows *where something came from* · progress and undo toasts are the only ambient motion · nothing loops · nothing bounces on arrival · **`prefers-reduced-motion` replaces every transition with an opacity fade**, including the Edge Strip.

---

## 9. Iconography

A single line set — 1.5 px stroke, rounded caps, 20 px grid, drawn to sit on the same optical weight as `text-body` at 550.

- **No filled icons** except the Class-D glyph and destructive confirmations
- **File-type icons are generated from the *detected* type**, never the extension — a `.jpg` that is actually PostScript shows the PostScript glyph, and that discrepancy is a feature
- **No icon-only buttons** without an accessible name
- **Never an icon where a word is clearer.** `Convert` beats an arrow.

---

## 10. Component patterns

### The armed suggestion

The single most important component. It should read as *the answer*, not as an option among options.

- `surface-raised` + hairline, `radius-md`
- Target format at `text-title`
- Parameters at `text-small` / `text-secondary`, in plain language
- Cost line at `text-small`, tabular figures
- **The "why" line at `text-small` / `text-secondary`** — *not* `text-caption` / `text-tertiary`, which is what v0.2 specified and which its own [accessibility floor](#11-accessibility-floor) forbids. 11 px at 38% opacity does not clear 4.5:1 on any surface we ship, and §11 reserves `text-tertiary` for "genuinely non-essential text." The "why" line is the product's honesty mechanism — [the vision](01-VISION.md#41-the-guess-is-the-product) makes "always able to explain itself" a pillar — so it is not non-essential, and it cannot be set in the one style the system marks as failing. Quiet is achieved with size and weight, not with contrast the user cannot read.
- `⏎` glyph right-aligned — no button chrome; the whole card is the target

### Alternates

Numbered rows, 36 px, no dividers — separated by space alone. Number at `text-tertiary`, format at `text-body`, cost right-aligned and tabular.

### The plan preview

A dense table where the class chips do the work. Steps that don't alter data recede; Class C and D advance. This is the screen that appears in launch screenshots — it should be the most beautiful thing in the product.

### Progress

A single hairline bar at the top of the content area, plus per-file states. **No spinners** — always show either a determinate bar or a live count.

### The confinement badge

Every plan preview and every receipt names the isolation that will actually be in force, because [the sandbox profile is a value, not a promise](03-ARCHITECTURE.md#9-isolation-the-profile-and-the-floor) and a claim the user can't see is a claim they can't rely on.

Three states, distinguished by glyph and label before colour:

```
  ⛨ sandboxed                              ← hairline, quiet. The good outcome; don't decorate it.
  ⛨ sandboxed · reduced   AppContainer unavailable   ← hairline + attention glyph, one line of why
  ⚠ blocked               requires full confinement  ← accent-critical; the plan has no runnable steps
```

The reduced state is the one that matters and the one most likely to be designed badly. It must read as *"this still works, and here is precisely what is different"* — never as a scary interstitial the user learns to dismiss, and never as silence. Hovering names every mechanism that engaged and every one that did not. The same three states render in the CLI as text plus glyph, and in the receipt as structured JSON.

### Explanations vs advice

The deterministic "why" line and the [advisor's](06-ML-RUNBOOK.md#1-which-layer-needs-what) output must be **visually distinct**: the "why" line is `text-tertiary` inline; advisor output sits in a `material-thin` container with the `⌇` inferred glyph. Measured fact and model output must never look alike. *(The advisor is post-v1; the visual rule is recorded now so it isn't retrofitted later.)*

---

## 11. Accessibility floor

Not a checklist — a procurement gate.

| Requirement | Standard |
|---|---|
| Text contrast | **≥ 4.5:1** body, ≥ 3:1 for ≥18 px. `text-tertiary` at 38% **fails on every surface we ship** — it is available for decorative rules and disabled-state affordances only, and **no text a user must read may use it**. (v0.2 said "reserve it for genuinely non-essential text" and then specified it for the "why" line, which is the most essential explanatory text in the product.) |
| Non-text contrast | ≥ 3:1 for borders, focus rings, glyphs |
| Focus ring | 2 px, `neutral-900`/`neutral-1000`, 2 px offset — **visible on every surface including translucent ones** |
| Colour independence | Every state readable in greyscale (§4) |
| Motion | `prefers-reduced-motion` honoured everywhere |
| Transparency | `prefers-reduced-transparency` → opaque fills, no layout shift |
| Text scaling | to 200% without breakage — **no fixed-height text containers** |
| Keyboard | Every action reachable; visible focus order |
| Screen readers | Every control labelled; progress and completion announced via live regions |

**Contrast is verified in CI over rendered component pairs, not over the token files.** That distinction is the whole value of the gate: v0.2 checked tokens, and the tokens were fine — the defect was a *component* pairing an essential string with a token the system itself marks as failing, which a token-file check passes every time. The gate enumerates every (foreground token, background surface, size) triple that any component actually uses and asserts the computed ratio, so a component can only fail contrast by introducing a pairing, never by inheriting one.

---

## 12. Token architecture

Two layers, because users can theme the app and themes must not need to know about components.

```
tokens/
├─ primitive.json     neutral-0…1000, accent-attention, accent-critical,
│                     space, radius, duration, type scale
├─ semantic.light.json   surface-base, text-primary, border-hairline, …
├─ semantic.dark.json
└─ semantic.contrast.json
```

**Components reference only semantic tokens. Themes override only primitives.** A user theme that changes `neutral-*` gets a coherent result everywhere without touching a component.

Emitted as CSS custom properties for the Tauri UI, as a Rust const table for the CLI, and as CSS for the website — **one source of truth across three surfaces.**

---

## 13. CLI & web parity

The class system must be identical everywhere it appears.

| Class | GUI | CLI | Receipt JSON |
|---|---|---|---|
| A lossless | hairline chip | plain text | `"class": "A"` |
| B lossy | hairline chip | dim | `"class": "B"` |
| C inferred | dashed chip | dim + `⌇` | `"class": "C"` |
| D generative | filled, attention | attention colour + `✦` | `"class": "D"`, `"generative": true` |

CLI rules: honour **`NO_COLOR`**, detect non-TTY and drop colour automatically, and never rely on colour alone — the glyph and word are always present.

The website uses the same tokens, so a screenshot of the app sits on a landing page without a seam.

---

## 14. Naming

> **Decided: OpenConvert.** Latin *openconvert* — a crossing over, a passage. Domain `openconvert.dev`; `openconvert.dev` and `openfile.dev` redirect. Decision and rationale in [01-VISION §10](01-VISION.md#10-the-name--decided).
>
> What follows is the candidate pool, kept as a record — useful if a sub-brand, a module family, or a company name is ever needed.

Candidates from Greek and Roman sources, on themes of transformation, threshold and making.

⚠️ **None of these were availability-checked.** Any future use needs a registry check, a USPTO/EUIPO search, npm/crates.io/PyPI, and a GitHub-org check.

### Transformation & bringing-forth

| Name | Origin | Meaning | Fit | Collision risk |
|---|---|---|---|---|
| **Poiesis** | Gk. ποίησις | *the act of bringing something into being* — root of "poetry"; also inside *chrysopoeia*, alchemical transmutation | Exactly the product: something new is brought forth from something that existed | **Low.** Pronunciation (poy-EE-sis) is the main friction |
| **Proteus** | Gk. Πρωτεύς | the shape-shifting sea god; "protean" = versatile, changeable | The best pure metaphor on the list | **High** — Proteus Design Suite (EDA) is established |
| **Mutare** | Lat. | *to change* | Direct, easy to say | Medium |
| **Morphe** | Gk. μορφή | *form, shape* | Clear, but "morph" is generic and slightly dated | Medium |

### Threshold & passage

| Name | Origin | Meaning | Fit | Collision risk |
|---|---|---|---|---|
| **Limen** | Lat. | *threshold* — root of "liminal" | The point of passage between one form and another. Short, calm, modern. | **Low**, though it sits near "Lumen" |
| **Janus** | Rom. | god of doorways, transitions, beginnings and endings; two faces looking both ways | Almost too apt — a converter looks at input and output at once | **Medium-high** — Janus WebRTC gateway is well known to developers |
| **Portunus** | Rom. | god of keys, doors, harbours — and **ports** | The pun is genuinely good for a technical tool | Low, but 8 letters and obscure |

### Craft, forge & making

| Name | Origin | Meaning | Fit | Collision risk |
|---|---|---|---|---|
| **Fornax** | Rom. | goddess of the furnace; also a constellation | Transformation by heat. Sounds like infrastructure — solid, technical, unfussy. | **Low** |
| **Athanor** | Gk./Ar. | the alchemist's slow furnace | Evocative and almost certainly free | Low; 7 letters, needs explaining |
| **Techne** | Gk. τέχνη | *craft, skill, art* — root of "technology" | Clean and meaningful without being cute | Medium |
| **Faber** | Lat. | *maker, craftsman* | Short, strong, confident | Medium — Faber & Faber, Faber-Castell (different classes) |
| **Incus** | Lat. | *anvil* — where matter is reshaped | Short and concrete | Medium — also an ear bone, so reads slightly medical |

### Form & essence

| Name | Origin | Meaning | Fit | Collision risk |
|---|---|---|---|---|
| **Eidos** | Gk. εἶδος | *form, essence* — Plato's Forms | Philosophically perfect: the essence persists while the form changes | **High** — Eidos Interactive / Eidos-Montréal |
| **Hyle** | Gk. ὕλη | *matter, substance* — Aristotle's counterpart to form | Conceptually elegant; four letters | Low, but opaque to users |

### Containers & vessels

"Container" is the actual technical term for MP4, MKV and ZIP — the strongest seam in the set.

| Name | Origin | Meaning | Collision risk |
|---|---|---|---|
| **Pyxis** | Gk. πυξίς | a small round box — *and* the constellation of the mariner's compass box | Medium — BD Pyxis (medical dispensing), different industry |
| **Alveus** | Lat. | channel, riverbed, trough | Low |
| **Crater** | Gk. κρατήρ | the mixing bowl — also a constellation | High — the English word |

### Instruments & shaping tools

The southern constellations are almost all workshop instruments.

| Name | Origin | Meaning | Collision risk |
|---|---|---|---|
| **Caelum** | Lat. | **the chisel** (constellation); also *sky* | Low |
| **Norma** | Lat. | the carpenter's square — root of "norm" | Medium — common given name |
| **Circinus** | Lat. | drawing compasses | Low |
| **Antlia** | Gk. ἀντλία | the pump | Low |

### Creation & revealing

| Name | Origin | Meaning | Collision risk |
|---|---|---|---|
| **Phanes** | Gk. Φάνης | Orphic god of creation, "**the revealer**" — from *phainein*, to bring to light | Low |
| **Artifex** | Lat. | craftsman, maker, artificer | Medium |
| **Officina** | Lat. | workshop, laboratory | Low |
| **Demiurge** | Gk. δημιουργός | the craftsman who shapes the world | ⚠️ Gnostic baggage — often the *malevolent* creator |

### Alchemical operations

Transformation is alchemy's entire subject, and its vocabulary is almost untouched in software.

| Name | Origin | Meaning | Collision risk |
|---|---|---|---|
| **Alkahest** | alch. Lat. | **the universal solvent** — dissolves anything into its essence | Very low |
| **Spagyria** | Gk. *span* + *ageirein* | "**separate and recombine**" — literally the IR-hub architecture | Very low; hard to spell |
| **Rubedo** | Lat. | the final alchemical stage, completion | Low |

### Passage, change & documents

| Name | Origin | Meaning | Collision risk |
|---|---|---|---|
| **Cardo** | Lat. | **the hinge**; also a Roman town's main street | Low |
| **Poros** | Gk. πόρος | passage, ford, way through | Medium — "porous"; a Greek island |
| **Metabole** | Gk. μεταβολή | change, transition — root of "metabolism" | Low; may read as "metabolic" |
| **Plasis** | Gk. πλάσις | a moulding, a forming | Very low |
| **Volumen** | Lat. | a scroll — root of "volume" | ⚠️ collides with audio "volume" |
| **Tabula** · **Charta** · **Pinax** | Lat./Gk. | writing tablet · papyrus sheet · register | Low |
| **Liber** | Lat. | *book* — and *free* | Medium — LibreOffice adjacency |

### Shortlist

Weighted for the CLI test — `brew install X`, `docker pull X`, `X convert photo.heic` — where short names win decisively.

| | Name | Why |
|---|---|---|
| **1** | **Pyxis** | A container — the literal technical term for what it manipulates. 5 letters, contemporary, constellation heritage gives the icon somewhere to go. |
| **2** | **Fornax** | The most product-like. Sounds like infrastructure. Transformation by fire plus an astronomical second meaning. |
| **3** | **Limen** | Calmest and most modern. *Threshold* is precisely what a converter is; "liminal" makes it graspable. |
| **4** | **Phanes** | "The revealer" — bringing to light what you couldn't open. Beautiful once explained, obscure enough to be free. |
| **5** | **Poiesis** | Meaning is exactly right; reads considered rather than clever. Pronunciation is the friction. |
| **6** | **Cardo** | The hinge on which turning happens. Short and concrete, slightly under-explained. |

**Dark horse: Alkahest** — nobody will confuse it with anything. Long, but the universal-solvent meaning maps onto "convert anything into anything" more precisely than any other candidate.

**Runner-up: Janus**, if the WebRTC collision proves tolerable — the two-faced-god metaphor is the single best fit on the list.

**Test each finalist:** say it on a call · type it as a domain · say `brew install <name>` and `docker pull <name>` out loud · imagine it as a wordmark in Inter 600 · check what it means in three other languages.

---

## 15. Open decisions

1. **Typeface.** Inter is the safe recommendation. Set Geist and Public Sans alongside it at 11/13/15 px before committing — it's cheap now and expensive later.
2. **One accent or two?** §3.3 proposes two (attention, critical). One is more minimal and defensible; the argument for two is that "this file was blocked as unsafe" and "this step invents pixels" are different kinds of alarm.
3. **Squircles.** Worth the SVG/Houdini complexity for large surfaces, or is `border-radius` close enough? Suggest starting with `border-radius` and revisiting only if it looks wrong at 14 px.
4. **Brand hue on the website.** The product chrome is neutral; the site and icon may want one colour. Deciding it later is fine and costs nothing.
5. ~~**Name.**~~ **Decided: OpenConvert.** Remaining work is trademark and registry checks, not selection.
6. **Icon/logomark.** Not designed here. Whatever it is should read at 16 px in a tray.
