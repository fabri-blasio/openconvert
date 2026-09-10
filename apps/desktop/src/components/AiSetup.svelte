<!--
  The AI chooser — shown once, on first run.

  **Why this is here and not in the installer.** The obvious home is a
  components page in the Windows installer, and that was tried: NSIS aborts when
  a page is inserted from Tauri's hook file, and the alternative is forking a
  900-line bundler template that then has to be maintained against every Tauri
  release. But there is a better reason than "NSIS refused".

  Models are per-user data under `%LOCALAPPDATA%`, and an installer running
  per-machine runs elevated — it would fetch half a gigabyte into the wrong
  profile. The download itself has to go through the registry, which pins every
  artifact by sha256 and refuses the unpinned sentinel row; that machinery is in
  the app, not in an install script. So the installer's licence page states that
  AI features are optional and none are included, and the choosing happens at
  the first moment it can actually be honoured.

  **Nothing is preselected.** Every row starts off. The sizes are summed from
  the registry rather than written here, so the number on screen is the number
  fetched, and a capability the app cannot yet reach is shown greyed with the
  reason rather than quietly hidden — hiding it would make this screen and the
  model registry disagree.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { listAiFeatures, downloadAiFeature, type AiFeature } from "../lib/ipc";
  import { settings } from "../lib/stores/settings.svelte";
  import { downloads } from "../lib/stores/downloads.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let features = $state<AiFeature[]>([]);
  let chosen = $state<Set<string>>(new Set());
  let loading = $state(true);
  let busy = $state<string | null>(null);
  let failed = $state<string[]>([]);
  let error = $state<string | null>(null);

  /**
   * How far the row being fetched has got, 0 to 1, or `null` for "cannot say".
   *
   * The backend has emitted this the whole time and this screen never
   * listened: a row said "downloading…" and then, some minutes later, stopped
   * saying it. For a 132 MB feature on a slow line that is indistinguishable
   * from a hung window.
   *
   * It comes from the store rather than from a subscription here, so it
   * survives this panel closing -- see `downloads.svelte.ts` for why that
   * matters. A feature is SEVERAL artifacts reported one after another under
   * the feature's id, so the fraction restarts per file: the ring is a sign of
   * life rather than an estimate, which is what it can honestly be while the
   * backend fetches a set one file at a time.
   */
  const downloadedFraction = $derived(busy === null ? null : downloads.fractionFor(busy));

  onMount(() => downloads.watch());

  /**
   * WHAT THIS SCREEN OFFERS, which is not every tier the registry has.
   *
   * It listed features, and background removal has three tiers, so it printed
   * "Remove image backgrounds" three times -- twice at 4 and 11 MB, once at
   * 224 MB -- and "Add all" came to 515 MB. Three rows with identical titles
   * is not a choice anyone can make, and a 224 MB model is not a tick-box
   * decision at the moment someone is trying to start using the program.
   *
   * `offeredAtInstall` comes from `models.toml`, and the installer's own
   * licence page filters on the same field. The two lists say the same thing
   * because they read the same source, not because someone keeps them level.
   *
   * Nothing is hidden by this: every tier is in Settings, and the ones that
   * are not here are chosen inside the tool that uses them, where the size
   * sits beside the reason for wanting it.
   */
  const offerable = $derived(
    features.filter((f) => f.usable && f.offeredAtInstall && !f.downloaded),
  );
  const unreachable = $derived(features.filter((f) => !f.usable));
  const selectedBytes = $derived(
    offerable.filter((f) => chosen.has(f.id)).reduce((n, f) => n + f.sizeBytes, 0),
  );
  const allBytes = $derived(offerable.reduce((n, f) => n + f.sizeBytes, 0));

  function mb(bytes: number): string {
    const m = bytes / 1_048_576;
    return m >= 100 ? `${Math.round(m)} MB` : `${m.toFixed(1)} MB`;
  }

  function toggle(id: string) {
    // A new Set each time: Svelte 5 tracks the binding, not mutation inside it.
    const next = new Set(chosen);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    chosen = next;
  }

  onMount(async () => {
    try {
      features = await listAiFeatures();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  /** Record that the question was asked, whatever the answer was. */
  async function remember() {
    await settings.patch({ aiSetupDone: true });
  }

  async function skip() {
    await remember();
    onclose();
  }

  /**
   * Add all: TICK EVERY BOX, then fetch.
   *
   * It called `install` straight off the derived list, so the screen showed a
   * set of empty boxes while five features downloaded. The boxes are the
   * record of what was asked for; a download nothing on screen accounts for
   * reads as the button having done something else.
   */
  async function addEverything() {
    chosen = new Set(offerable.map((f) => f.id));
    await install([...chosen]);
  }

  async function install(ids: string[]) {
    error = null;
    failed = [];
    for (const id of ids) {
      busy = id;
      downloads.begin(id);
      try {
        await downloadAiFeature(id);
      } catch (e) {
        // Retry exhaustion must not hold setup open. Remaining features
        // can be installed later from Settings.
        failed = [...failed, id];
        error = String(e);
        break;
      } finally {
        downloads.end(id);
      }
    }
    busy = null;
    await remember();
    // The downloader owns the initial attempt plus three retries.
    // Keep installed models, skip exhausted downloads, and enter the app.
    onclose();
  }
</script>

<div class="sheet" role="dialog" aria-labelledby="ai-setup-title" aria-modal="true">
  <div class="head">
    <h2 id="ai-setup-title">Add AI features?</h2>
    <p class="lede">
      Optional, and none are installed. They run entirely on this machine. The
      download is the only time anything is fetched, and you can add or remove
      them later in Settings.
    </p>
  </div>

  {#if loading}
    <p class="status" role="status">Reading the registry…</p>
  {:else if features.length === 0}
    <p class="status" role="status">This build declares no AI features.</p>
  {:else}
    <ul class="list">
      {#each offerable as f (f.id)}
        <li class="row">
          <input
            type="checkbox"
            id={`ai-${f.id}`}
            checked={chosen.has(f.id)}
            disabled={busy !== null}
            onchange={() => toggle(f.id)}
          />
          <label for={`ai-${f.id}`} class="text">
            <span class="title">{f.title}</span>
            <span class="does">{f.does}</span>
            <!-- THE SIZE IS NOT PART OF THIS LINE ANY MORE. It sat at the head
                 of a run-on that continued into the licence, so the one number
                 a person is weighing was in the middle of a sentence and at a
                 different horizontal position on every row. It is at the right
                 edge now, where the rows line up and the column can be read
                 down. -->
            <span class="meta">
              {f.licences.join(", ")}
              {#if failed.includes(f.id)}<span class="failed"> · did not download</span>{/if}
            </span>
          </label>
          <!-- THE RING REPLACES THE SIZE while that row is being fetched, in
               the same slot: the number stops being the useful thing the
               moment the download starts, and swapping them in place means the
               row does not change width or reflow its neighbours. -->
          {#if busy === f.id}
            <span class="ring" role="status" aria-label={`Downloading ${f.title}`}>
              <svg viewBox="0 0 36 36" aria-hidden="true" class:sweep={downloadedFraction === null}>
                <circle class="track" cx="18" cy="18" r="15.5" />
                <circle
                  class="fill"
                  cx="18"
                  cy="18"
                  r="15.5"
                  style:--dash={downloadedFraction === null ? 0.25 : downloadedFraction}
                />
              </svg>
            </span>
          {:else}
            <span class="size">{mb(f.sizeBytes)}</span>
          {/if}
        </li>
      {/each}

      <!-- Already-installed rows follow the same rule, or the screen would
           still print a title twice: once as an offer, once as installed. -->
      <!-- "DOWNLOADED" AND "WORKS" ARE TWO CLAIMS, and this made only the
           first. A row said "already installed" whether the app could reach
           the model or not, so a feature whose artifacts are all on disk and
           whose adapter this build does not have looked exactly like one that
           was ready to use.

           `usable` is the second claim -- whether anything in the app can
           actually reach it -- and the dot is it. Green and "Ready" together:
           the colour for people who have learnt it, the word for everyone
           else and for a screen reader. -->
      {#each features.filter((f) => f.offeredAtInstall && f.downloaded) as f (f.id)}
        <li class="row done">
          <span class="tick" aria-hidden="true">=</span>
          <span class="text">
            <span class="title">{f.title}</span>
            <span class="meta">
              {#if f.usable}
                <span class="dot ready" aria-hidden="true"></span>Ready
              {:else}
                <span class="dot" aria-hidden="true"></span>Downloaded, but nothing
                in this build uses it yet
              {/if}
            </span>
          </span>
          <span class="size">{mb(f.sizeBytes)}</span>
        </li>
      {/each}
    </ul>

    {#if unreachable.length > 0}
      <p class="note">
        Not offered yet:
        {unreachable.map((f) => f.title.toLowerCase()).join(", ")}. The models
        exist and are pinned, but nothing in this build uses them. Downloading
        them would cost you the disk space and do nothing.
      </p>
    {/if}

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <div class="actions">
      <button type="button" class="secondary" onclick={skip} disabled={busy !== null}>
        Not now
      </button>
      <span class="spacer"></span>
      <button
        type="button"
        class="secondary"
        onclick={addEverything}
        disabled={busy !== null || offerable.length === 0}
      >
        Add all ({mb(allBytes)})
      </button>
      <button
        type="button"
        class="primary"
        onclick={() => install([...chosen])}
        disabled={busy !== null || chosen.size === 0}
      >
        Add selected{chosen.size > 0 ? ` (${mb(selectedBytes)})` : ""}
      </button>
    </div>
  {/if}
</div>

<style>
  .sheet {
    display: grid;
    gap: var(--space-5);
    align-content: start;
    padding: var(--space-6);
    overflow-y: auto;
    min-height: 0;
  }

  .head {
    display: grid;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-display-size);
    line-height: var(--text-display-lh);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.02em;
  }
  .lede {
    margin: 0;
    max-width: 62ch;
    font-size: var(--text-body-size);
    line-height: var(--text-body-lh);
    color: var(--text-secondary);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1px;
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: var(--surface-raised);
  }
  .row + .row {
    border-top: 0.5px solid var(--hairline);
  }
  .row input {
    margin-top: 2px;
    flex: none;
  }
  .text {
    display: grid;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .title {
    font-size: var(--text-body-size);
    font-weight: var(--weight-medium);
  }
  .does {
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }
  .meta {
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
  }
  .failed {
    color: var(--accent-critical);
  }

  /* The dot carries the state and the word beside it carries the meaning.
     Colour alone would say nothing to anyone who cannot separate these two
     hues, and this is a status, not decoration. */
  .dot {
    display: inline-block;
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    margin-inline-end: var(--space-2);
    background: var(--text-tertiary);
    vertical-align: baseline;
  }

  .dot.ready {
    background: var(--accent-positive, #34c759);
  }
  .done .tick {
    font-family: var(--mono);
    color: var(--text-tertiary);
    flex: none;
  }
  /* THE RIGHT-HAND COLUMN. Size or ring, never both, in one slot of one
     width -- so the rows line up down the screen and swapping one for the
     other when a download starts does not move anything. */
  .size,
  .ring {
    flex: none;
    align-self: center;
    inline-size: 68px;
    text-align: right;
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .ring {
    display: grid;
    justify-items: end;
  }

  .ring svg {
    inline-size: 18px;
    block-size: 18px;
    /* Start the stroke at twelve o'clock and run clockwise, which is what
       "filling" means to anyone who has seen a progress ring before. SVG
       starts at three o'clock and runs the other way. */
    transform: rotate(-90deg);
  }

  .ring circle {
    fill: none;
    stroke-width: 3;
    /* r = 15.5, so the circumference is 2*pi*15.5 = 97.39. The dash array is
       written in those units and `--dash` is the fraction of it to draw. */
    stroke-dasharray: 97.39;
  }

  .ring .track {
    stroke: var(--hairline-strong);
  }

  .ring .fill {
    stroke: var(--emphasis);
    stroke-linecap: round;
    stroke-dashoffset: calc(97.39 * (1 - var(--dash, 0)));
    transition: stroke-dashoffset var(--motion-micro, 120ms) linear;
  }

  /* NO LENGTH DECLARED, so there is no fraction to draw. A quarter-circle
     turning says work is happening, which is true; a ring frozen at zero for
     a minute says the opposite of what is true. */
  .ring svg.sweep {
    animation: ring-sweep 900ms linear infinite;
  }

  .ring svg.sweep .fill {
    transition: none;
  }

  @keyframes ring-sweep {
    from {
      transform: rotate(-90deg);
    }
    to {
      transform: rotate(270deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ring svg.sweep {
      animation-duration: 2.4s;
    }
  }

  .note,
  .error {
    margin: 0;
    max-width: 62ch;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }
  .error {
    color: var(--accent-critical);
  }
  .status {
    margin: 0;
    font-size: var(--text-body-size);
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .spacer {
    margin-left: auto;
  }
  .primary,
  .secondary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    height: 32px;
    padding: 0 var(--space-6);
    border-radius: var(--radius-full);
    font-size: var(--text-body-size);
    font-weight: var(--weight-medium);
    border: 0;
  }
  .primary {
    background: var(--text-primary);
    color: var(--surface-base);
  }
  .primary:disabled {
    background: var(--surface-sunken);
    color: var(--text-disabled);
    cursor: default;
  }
  .secondary {
    border: 0.5px solid var(--hairline-strong);
    color: var(--text-secondary);
    background: transparent;
  }
  .secondary:disabled {
    color: var(--text-disabled);
    cursor: default;
  }
</style>
