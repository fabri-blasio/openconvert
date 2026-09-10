<!--
  Where a step runs, and under what confinement.

  The colour is the only thing this component decides, and it decides it from
  the string the backend rendered from `SandboxProfile::strength()`. There is no
  second opinion about isolation on this side of the boundary.
-->
<script lang="ts">
  let { display }: { display: string } = $props();

  // "elevated" is what `Strength::Full` prints. It is not called "full" in the
  // UI on purpose: users read "full" as "complete", and it is not — CIG does
  // not engage even in that tier.
  const elevated = $derived(display.includes("elevated"));
  const reduced = $derived(display.includes("reduced") || display.includes("minimal"));
  const inProcess = $derived(display === "in-process");

  // Hovering names every mechanism that engaged and every one that did not
  // (07 §10, the confinement badge). The facts are the measured ones from the
  // spikes: ACG engages inside an AppContainer whether or not it is asked;
  // CIG does not engage even when asked.
  const mechanisms = $derived(
    inProcess
      ? "Pure-Rust parser running inside the host process. No engine runs here, so no sandbox boundary applies."
      : elevated
        ? "AppContainer with a restricted token and Job Object limits engaged. ACG engaged; CIG did not (a known WebView2/Windows limitation, recorded honestly)."
        : reduced
          ? `Confinement below policy (${display}): AppContainer unavailable or partial. The conversion still runs; fewer guarantees engaged than this machine could offer.`
          : "Confinement as reported by the worker that ran the step.",
  );
</script>

<span class="badge" class:elevated class:reduced class:inProcess title={mechanisms}>
  <span class="glyph" aria-hidden="true">{inProcess ? "·" : "⛨"}</span>
  {display}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: 20px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .glyph {
    font-size: 10px;
  }

  /* In-process is the quiet case: pure-Rust parsers, nothing to warn about. */
  .inProcess {
    color: var(--text-tertiary);
    border-style: dashed;
  }

  /* Everything the platform offers. Still quiet — this is the good outcome. */
  .elevated {
    color: var(--text-primary);
  }

  /*
    Reduced confinement is the one case worth an accent: the conversion will
    run, and it will run with less separation than the machine could have
    given it. `--accent-attention`, never `--accent-critical`; nothing is
    broken.
  */
  .reduced {
    color: var(--accent-attention);
    border-color: var(--accent-attention);
  }
</style>
