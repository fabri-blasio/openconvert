<!--
  An audio waveform, drawn as bars from REAL peak data.

  It used to be drawn from a seeded random number generator and labelled
  "Waveform of <the user's file>". It was deterministic, so it looked stable
  across renders, and it had nothing whatever to do with the audio — a picture
  of a file the program had not read.

  `peaks` is one value per bar, 0–255, being the loudest sample in that slice,
  measured by `oc-audio` in the confined worker. Peak rather than RMS on
  purpose: RMS is the better measure of loudness and the worse measure here,
  because it averages away the transients, and the difference a denoise makes
  shows up between the peaks.

  With no peaks yet the component draws a flat resting line rather than a
  guess — the same reason the page thumbnails show a placeholder instead of the
  wrong page.
-->
<script lang="ts">
  let {
    peaks = [],
    progress = 0,
    label,
  }: {
    /** One value per bar, 0–255. Empty until the file has been read. */
    peaks?: number[];
    /** 0–1 playhead position. */
    progress?: number;
    label: string;
  } = $props();

  const heights = $derived(
    peaks.map((p) => Math.max(0.02, Math.min(1, p / 255))),
  );
</script>

<div class="wave" role="img" aria-label={label} class:empty={heights.length === 0}>
  {#if heights.length === 0}
    <span class="resting"></span>
  {:else}
    {#each heights as h, i (i)}
      <span class="bar" class:played={i / heights.length <= progress} style:--h={h}></span>
    {/each}
  {/if}
</div>

<style>
  .wave {
    display: flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    height: 100%;
    min-height: var(--wave-height, 120px);
    height: var(--wave-height, 120px);
  }

  .bar {
    flex: 1 1 0;
    /* `--h` is registered in tokens.css alongside --fraction. */
    height: calc(var(--h, 0.2) * 100%);
    min-height: 2px;
    border-radius: var(--radius-full);
    background: var(--text-tertiary);
    transition: height var(--motion-standard);
  }

  .played {
    background: var(--emphasis);
  }

  /* Nothing read yet: a resting line, which claims nothing about the file. */
  .resting {
    width: 100%;
    height: 2px;
    border-radius: var(--radius-full);
    background: var(--fill-tertiary);
  }

  @media (prefers-reduced-motion: reduce) {
    .bar {
      transition: none;
    }
  }
</style>
