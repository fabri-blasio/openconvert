# OpenConvert Desktop — UI Style Guide

The Apple-minimal system from [07-DESIGN-SYSTEM.md](../../07-DESIGN-SYSTEM.md), made concrete.
Tokens live in `src/lib/styles/tokens.css`; shared primitives in `src/lib/styles/base.css`.
If a value isn't here or in those files, it doesn't exist — don't invent one.

---

## 1. Principles

1. **Calm neutrality.** No brand hue in the product chrome. Colour appears only as the two accents, and only in small areas (a chip, a glyph, a 2 px rule).
2. **Prominence scales with consequence.** The only loud thing on screen is a Class D step or a blocked plan. Everything good is silent.
3. **Hierarchy by opacity, not grey soup.** Three text levels: `62%` secondary, `38%` tertiary, `24%` disabled — all derived from `--text-primary`.
4. **Depth by translucency, not shadow.** Materials are blurred fills; shadows exist only as a hairline + whisper to lift cards off the base.
5. **Numbers align.** `font-variant-numeric: tabular-nums` is global. Never break it.

## 2. Rounded corners

Radius tokens (the Apple feel comes mostly from these being *generous but consistent*):

| Token | Value | Use |
|---|---|---|
| `--radius-sm` | 6 px | Chips, inputs, segmented controls |
| `--radius-md` | 10 px | List rows, small cards, icon tiles |
| `--radius-lg` | 14 px | Cards, groups, receipts |
| `--radius-xl` | 20 px | Drop zone, sheets |
| `--radius-full` | 999 px | Pills, badges, switches, toasts, primary buttons |

Rules:

- Every interactive surface has a radius. Square corners appear nowhere.
- Nested radii stay concentric: inner radius = outer − padding (`lg` card + 16 px padding → `md` children).
- Buttons that act as *the* action (Undo toast) are pills (`radius-full`); everything else is `sm`.

## 3. Colour

Neutral ramp (light / dark columns invert — `neutral-900`-equivalent text in both themes):

```
--neutral-0 #FFFFFF   --neutral-50 #FAFAFA   ...   --neutral-1000 #000000
```

Surfaces: `--surface-base` (window), `--surface-raised` (cards/groups), `--surface-sunken` (wells, icon tiles).

Text roles — never raw greys, always opacity on `--text-primary`:

| Role | Value | Use |
|---|---|---|
| primary | 100% | Titles, values |
| secondary | 62% | Metadata, why-lines |
| tertiary | 38% | Column headers, hints |
| disabled | 24% | Dead state only |

Accents — exactly two, never decoration:

| Token | Light | Dark | Used for |
|---|---|---|---|
| `--accent-attention` | `#B4690E` | `#E0A458` | Class D, reduced sandbox, non-blocking warnings |
| `--accent-critical` | `#C0392B` | `#E5766A` | Blocked plans, mismatches, destructive confirm |

Never pure black text on pure white; never an accent larger than a chip.

## 4. Materials

```
thin     blur(20px)  fill @ 62%   inline popovers, drag-over tint
regular  blur(32px)  fill @ 76%   sidebars
thick    blur(48px)  fill @ 88%   sheets, toasts
```

Hairlines are `0.5px`–`1px`, `--hairline` (8 % / 10 % alpha), never solid grey.
`prefers-reduced-transparency` collapses every material to an opaque surface — layout must not change.
Dark mode elevation gets *lighter* (`base → raised → sunken` up the ramp), never shadowed.

## 5. Typography

Faces: `--font-ui` = InterVariable → SF Pro Text → Segoe UI fallbacks. Mono = JetBrains/Geist Mono, for paths, hashes, receipt JSON only.

| Token | Size/LH | Weight | Use |
|---|---|---|---|
| caption | 11/14 | 400 | Receipt labels, fineprint |
| small | 12/16 | 400 | Secondary rows, chip labels |
| body | 13/18 | 400 | Default |
| title | 15/20 | 550 | Card titles, nav bar |
| heading | 21/26 | 600 | Screen titles (unused yet) |
| display | 28/34 | 600 | Empty states (drop zone headline) |

Two weights only: 400 and 550/600. Letter-spacing tightens as size grows (`-0.02em` display → `-0.01em` title).

## 6. Space & density

4 px base: `2 · 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64`.
Content column: `min(720px, 100% - 64px)`, centred — a converter is a list.
Rows: `--row-h` 36 px comfortable / 28 px compact (`data-density="compact"`).
Section headers inside content: caption size, uppercase, `+0.06em` tracking, tertiary.

## 7. Motion

```
micro     120 ms ease-out                          hover, focus, chip state
standard  220 ms cubic-bezier(.32,.72,0,1)         panels, expansion, switch
large     340 ms cubic-bezier(.32,.72,0,1)         toast entrance
```

Motion shows *where something came from*: toast slides up 8 px while fading; drop zone scales 1.01 on drag-over. Nothing loops, nothing bounces. `prefers-reduced-motion` flattens all transitions to ~1 ms fades.

Hover language: background swap (`--surface-sunken`) for icon buttons, `scale: 1.005→1.02` for large targets, pressed state scales down (`0.995`). Subtle or nothing.

## 8. Components

### Card / Group
`.card` / `.group` — raised surface, `radius-lg`, hairline ring via `box-shadow: 0 0 0 0.5px`. Groups separate rows with `0.5px` top hairlines (inset-grouped-list look).

### Class chips (greyscale-first)
| Class | Chip class | Look |
|---|---|---|
| A lossless | `.chip.quiet` | glyph `=`, hairline outline |
| B lossy | `.chip.lossy` | glyph `≈`, hairline, secondary text |
| C AI-read | `.chip.inferred` | glyph `⌇`, **dashed** outline |
| D AI-generated | `.chip.generous` | glyph `✦`, filled accent — the only loud element |

Glyph + label + border style carry meaning before colour; all four survive greyscale.

### Confinement badge
`.badge` pill, hairline: `⛨ sandboxed` quiet · reduced adds amber tint + reason line · blocked is critical red with `⚠`. Never colour-only.

### Switch
`.switch` — iOS-style 38×22 track, white knob, checked = near-black track (no green; hue budget is spent). Animates on `--motion-standard`.

### Segmented control
`.seg` — sunken well, 6 px inner buttons, active segment pops with surface fill + hairline shadow.

### Progress
Single 2 px hairline bar at top of content + per-file dots/checkmarks. No spinners, ever.

### Toast (undo)
Fixed bottom-centre pill: thick material, blur 48, float shadow, action button as inverted pill with live countdown.

## 9. Accessibility floor

- Focus: `:focus-visible` → 2 px neutral ring, 2 px offset. Never removed.
- Contrast: body text clears 4.5:1; the why-line uses `text-secondary` (62 %), *never* tertiary/caption — quietness comes from size, not unreadability.
- Hit targets ≥ 28 px. No icon-only control without `aria-label`.
- Live regions: progress list is `aria-live="polite"`; warnings are `role="alert"`.
- State glyphs (`⌇ ✦ ⛨ ⚠`) are a font dependency — CI must verify they resolve in the shipped font, ASCII fallback (`= ~ ? * [#]`) elsewhere.

## 10. Do / Don't

| Do | Don't |
|---|---|
| One accent moment per screen | Gradient heroes, coloured backgrounds |
| Hairline borders at 8–10 % alpha | Solid grey borders |
| Blur materials for depth | Drop-shadow stacks |
| Pills & generous radii everywhere | Mixed radii per screen |
| Tabular figures in every number | Proportional digits in lists |
| Words over icons ("Convert" beats →) | Icon-only buttons |
