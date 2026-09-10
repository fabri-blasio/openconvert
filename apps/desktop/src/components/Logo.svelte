<!--
  The OpenConvert mole, as a theme-aware inline SVG.

  # Why inline rather than an <img>

  The master in `brand/logo/` paints its own ground (`#FCFCFB`) and draws the
  mole in near-black (`#1D1D1F`). That is exactly right for an app icon, which
  sits on a desktop the app does not control, and exactly wrong inside a window
  that has a dark mode: the artwork becomes a white card with black lines on a
  near-black surface.

  Inlining it lets the ink and the paper become CSS custom properties, so the
  same drawing follows the theme instead of fighting it. An <img> cannot: an
  external SVG's styles are not reachable from the page.

  # What changes with the theme, and what does not

  Only two of the five colours move:

  * `--logo-ink` — the outline, eye, nostrils and paw detail. Near-black on
    light, near-white on dark.
  * `--logo-paper` — the paws and the eye's highlight, which are holes punched
    in the ink and must match the surface behind them.

  The folder sage and the snout green stay put in both themes. They carry the
  brand, they have enough contrast either way, and a logo whose colours all
  change is a different logo.
-->
<script lang="ts">
  interface Props {
    /** Rendered height in pixels; the width follows the artwork's ratio. */
    size?: number;
    /**
     * Whether the mark is decorative.
     *
     * It usually is — it sits beside the word "OpenConvert", and a screen reader
     * announcing "OpenConvert logo, OpenConvert" is noise. Pass a title only where
     * the mark stands alone.
     */
    title?: string | null;
  }
  let { size = 24, title = null }: Props = $props();

  // The master's viewBox. Kept as one value so the aspect ratio cannot drift
  // from the artwork.
  const W = 1450;
  const H = 1244;
</script>

<svg
  class="logo"
  viewBox="0 0 {W} {H}"
  width={(size * W) / H}
  height={size}
  role={title ? "img" : "presentation"}
  aria-label={title ?? undefined}
  aria-hidden={title ? undefined : "true"}
  focusable="false"
>
  <!-- No background rect. The window's own surface is the paper. -->
  <path
    d="M200 61h390c27 0 48 9 66 31l80 101c13 16 29 22 51 22h504c58 0 105 47 105 105v630c0 58-47 105-105 105H200c-58 0-105-47-105-105V166c0-58 47-105 105-105Z"
    fill="none"
    stroke="var(--logo-folder)"
    stroke-width="47"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <path
    d="M320 1009C317 788 363 604 459 494C532 410 628 366 733 366C867 366 971 434 1018 545C1033 580 1036 617 1048 650C1060 683 1082 704 1119 721"
    fill="none"
    stroke="var(--logo-ink)"
    stroke-width="47"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <circle cx="710" cy="630" r="63" fill="var(--logo-ink)" />
  <circle cx="711" cy="619" r="20" fill="var(--logo-paper)" />
  <g fill="none" stroke="var(--logo-ink)" stroke-width="32" stroke-linecap="round">
    <path d="M800 806C757 793 716 790 681 792" />
    <path d="M806 846C764 854 728 869 696 890" />
    <path d="M824 879C787 905 757 933 734 958" />
    <path d="M1133 747C1170 721 1207 703 1241 692" />
    <path d="M1148 791C1192 789 1231 794 1265 805" />
    <path d="M1135 836C1173 850 1207 870 1235 893" />
  </g>
  <path
    d="M788 829c29-70 107-126 196-145 78-17 149 2 170 51 22 51-17 116-95 159-81 45-180 55-239 20-37-22-49-52-32-85Z"
    fill="var(--logo-snout)"
    stroke="var(--logo-ink)"
    stroke-width="43"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <circle cx="997" cy="789" r="19" fill="var(--logo-ink)" />
  <circle cx="1057" cy="789" r="19" fill="var(--logo-ink)" />
  <g fill="var(--logo-paper)" stroke="var(--logo-ink)" stroke-linecap="round" stroke-linejoin="round">
    <path
      d="M285 1107C286 1033 298 985 333 952C360 927 393 918 430 922C487 928 527 959 545 1004C560 1042 558 1084 545 1118C536 1143 519 1158 499 1156C477 1154 463 1137 455 1111C446 1138 430 1154 410 1156C386 1158 368 1142 359 1113C349 1138 334 1152 315 1150C294 1147 284 1130 285 1107Z"
      stroke-width="43"
    />
    <path
      d="M380 1028C376 1064 368 1094 359 1113M461 1029C459 1067 457 1095 455 1111"
      fill="none"
      stroke-width="37"
    />
  </g>
  <g
    transform="translate(1430 0) scale(-1 1)"
    fill="var(--logo-paper)"
    stroke="var(--logo-ink)"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path
      d="M285 1107C286 1033 298 985 333 952C360 927 393 918 430 922C487 928 527 959 545 1004C560 1042 558 1084 545 1118C536 1143 519 1158 499 1156C477 1154 463 1137 455 1111C446 1138 430 1154 410 1156C386 1158 368 1142 359 1113C349 1138 334 1152 315 1150C294 1147 284 1130 285 1107Z"
      stroke-width="43"
    />
    <path
      d="M380 1028C376 1064 368 1094 359 1113M461 1029C459 1067 457 1095 455 1111"
      fill="none"
      stroke-width="37"
    />
  </g>
</svg>

<style>
  .logo {
    display: block;
    flex: none;
  }
</style>
