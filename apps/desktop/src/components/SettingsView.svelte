<!--
  Settings.

  Five things people actually change — sandbox, where files go, what they are
  called, what is kept, which models are on — and an Advanced fold for the rest.

  **Copy rule: one line, or none.** A control whose label needs a paragraph is
  a control that is named wrong. The long explanations that used to sit under
  every row are gone; what survives is the sentence a user could not guess.
  Nothing is stated twice — where two rows shared a caveat, the caveat sits with
  the one that owns it.

  Every control reads and writes through the config commands via the settings
  store. A value the backend refused shows the refusal verbatim beneath itself;
  nothing silently snaps back.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import {
    receiptsSize,
    receiptsCount,
    gpuDevice,
    deleteAllReceipts,
    listModels,
    downloadModel,
    setModelEnabled,
    deleteModel,
    listAiFeatures,
    setModelTier,
    downloadAiFeature,
    errorText,
    type ModelInfo,
    type ModelProgress,
    type AiFeature,
  } from "../lib/ipc";
  import { settings } from "../lib/stores/settings.svelte";
  import { downloads } from "../lib/stores/downloads.svelte";
  import { HELP, PANIC_DOES, PANIC_LABEL } from "../lib/keys";
  import InfoTip from "./InfoTip.svelte";

  // The offered sizes, in MB. Doubling from the default: a limit like this is
  // not tuned by tens of megabytes, and a free-text box invites a number
  // someone will regret typing. 8 GB clears the largest model we ship
  // (measured at 6,465 MB peak); the entries above it are for machines that
  // have the memory and files that need it.
  const MEMORY_STEPS = [1024, 2048, 4096, 8192, 16384];

  function memoryLabel(mb: number): string {
    return mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`;
  }

  let {
    onclose,
    onopenhistory,
    onpanic,
  }: {
    onclose: () => void;
    onopenhistory: () => void;
    onpanic: () => void;
  } = $props();

  const cfg = $derived(settings.config);

  /** Which sections have their explanation open.
   *
   * Every key is present from the start: `bind:` refuses an `undefined`
   * initial value against a prop that declares a default, so an absent key is
   * a runtime error rather than a closed tip. */
  let info = $state({
    sandbox: false,
    compute: false,
    output: false,
    history: false,
    models: false,
    appearance: false,
  });

  /**
   * The graphics card's name, once the host has been asked.
   *
   * `undefined` while the question is out and `null` once it comes back empty,
   * because the two render differently: nothing at all, then the switch on its
   * own. Collapsing them would flash "no card" on every visit to this screen.
   */
  let gpuName = $state<string | null | undefined>(undefined);
  onMount(() => {
    // A failure here is not worth a banner. The row degrades to the switch,
    // which is a complete control on its own.
    void gpuDevice()
      .then((n) => (gpuName = n))
      .catch(() => (gpuName = null));
  });

  function mb(bytes: number): string {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  // --- Output destination ------------------------------------------------
  // Replacing originals is the one choice that destroys something, so it takes
  // a second, explicit acknowledgement before it sticks.
  let confirmingReplace = $state(false);
  let replaceAcknowledged = $state(false);

  function chooseDestination(value: string) {
    if (value === "replace_source") {
      confirmingReplace = true;
      replaceAcknowledged = false;
      return;
    }
    confirmingReplace = false;
    void settings.patch({
      outputDestination: value as "same_folder" | "downloads" | "desktop",
    });
  }

  async function confirmReplace() {
    if (!replaceAcknowledged) return;
    await settings.patch({ outputDestination: "replace_source" });
    confirmingReplace = false;
  }

  // --- Naming template ---------------------------------------------------
  // Committed on blur/Enter so typing is never interrupted, and validated by
  // the backend — the same gate the conversion path applies.
  let namingDraft = $state<string | null>(null);
  let namingError = $state<string | null>(null);

  async function commitNaming() {
    if (namingDraft === null || !cfg) return;
    if (namingDraft === cfg.namingTemplate) {
      namingDraft = null;
      return;
    }
    const ok = await settings.patch({ namingTemplate: namingDraft });
    namingError = ok ? null : (settings.error ?? "that template was refused");
    if (ok) namingDraft = null;
  }

  // --- Receipt store -----------------------------------------------------
  let receiptCount = $state(0);
  let receiptBytes = $state(0);
  let storeError = $state<string | null>(null);

  async function refreshReceipts() {
    try {
      const [count, size] = await Promise.all([receiptsCount(), receiptsSize()]);
      receiptCount = count;
      receiptBytes = size;
      storeError = null;
    } catch (e) {
      storeError = errorText(e);
    }
  }

  async function clearReceipts() {
    try {
      await deleteAllReceipts();
      await refreshReceipts();
    } catch (e) {
      storeError = errorText(e);
    }
  }

  // --- Models ------------------------------------------------------------
  let models = $state<ModelInfo[]>([]);
  let modelError = $state<string | null>(null);
  // NO LOCAL DOWNLOAD STATE. It was `$state` here, and this panel closing
  // therefore erased every trace of a download that was still running --
  // reopening offered to start one that was already halfway done.
  // `downloads.svelte.ts` holds it above the panels now.

  /**
   * The capabilities, which is the unit a person actually wants.
   *
   * **A FEATURE, NOT A ROW.** This screen listed artifacts: `whisper-encoder`,
   * `whisper-decoder`, `whisper-tokenizer`, `silero-vad` — four downloads, four
   * switches, and nothing on screen saying that transcription needs all four
   * and does nothing without them. Someone who downloaded three of them got a
   * tool that stayed unavailable for a reason no row explained.
   *
   * `models.toml` has always grouped them; `features()` has always summed the
   * sizes and reported whether the whole set is present. Only this screen was
   * still asking the question one artifact at a time.
   */
  let features = $state<AiFeature[]>([]);
  /** Feature ids currently being fetched, so the row can say so. */
  // Also not local, and for the same reason: a feature is several artifacts,
  // so between the button press and the first byte there are no progress
  // events at all, and this is what says "asked for" in that gap.
  /** Which features have their artifact list expanded. */
  let openFeatures = $state<Set<string>>(new Set());

  /** The artifacts one feature needs, in registry order. */
  function artifactsOf(feature: AiFeature): ModelInfo[] {
    return feature.models
      .map((id) => models.find((m) => m.id === id))
      .filter((m): m is ModelInfo => m !== undefined);
  }

  function toggleFeatureDetail(id: string) {
    const next = new Set(openFeatures);
    if (!next.delete(id)) next.add(id);
    openFeatures = next;
  }

  /** Fetch every artifact one capability needs, as one action. */
  async function installFeature(feature: AiFeature) {
    downloads.begin(feature.id);
    try {
      await downloadAiFeature(feature.id);
      await refreshModels();
    } catch (e) {
      modelError = errorText(e);
    } finally {
      downloads.end(feature.id);
    }
  }

  /**
   * Re-read the rows and the capabilities.
   *
   * **Settled, not `Promise.all`.** These are two independent reads and they
   * were awaited as one: either rejecting threw before EITHER assignment, so a
   * failure in the capability list silently discarded a perfectly good model
   * list and left the screen showing state the backend no longer had. The
   * two views disagreeing is the thing to avoid; throwing both away is not how
   * to avoid it. Each lands if it can, and an error is reported either way.
   */
  async function refreshModels() {
    const [rows, caps] = await Promise.allSettled([listModels(), listAiFeatures()]);
    if (rows.status === "fulfilled") models = rows.value;
    if (caps.status === "fulfilled") features = caps.value;
    const failed = [rows, caps].find((r) => r.status === "rejected");
    modelError = failed ? errorText((failed as PromiseRejectedResult).reason) : null;
  }

  async function startDownload(m: ModelInfo) {
    // Seeded before the first event so the bar appears on the press rather
    // than on the first packet -- the same gap `begin` covers for a feature.
    downloads.progress = {
      ...downloads.progress,
      [m.id]: {
        id: m.id,
        receivedBytes: 0,
        totalBytes: m.sizeBytes,
        phase: "downloading",
        message: null,
      },
    };
    downloads.begin(m.id);
    try {
      await downloadModel(m.id);
      await refreshModels();
    } catch (e) {
      modelError = errorText(e);
    } finally {
      downloads.end(m.id);
    }
  }

  /**
   * Capabilities, each with its tiers.
   *
   * **A CAPABILITY IS A JOB; A TIER IS A BUDGET.** The list was flat, one row
   * per bundle, so "remove image backgrounds" appearing twice would read as
   * two features rather than two ways to do one thing. Features naming the
   * same tool ARE that one thing, and grouping is what lets the screen ask the
   * only question worth asking: small or good.
   *
   * A feature with no tool keeps a group of its own -- it is declared, not
   * reachable, and squashing it in with something else would hide that.
   */
  type Capability = { key: string; title: string; does: string; tiers: AiFeature[] };

  const capabilities = $derived.by(() => {
    const groups: Capability[] = [];
    for (const f of features) {
      const key = f.tool ?? `feature:${f.id}`;
      const existing = groups.find((g) => g.key === key);
      if (existing) {
        existing.tiers.push(f);
      } else {
        groups.push({ key, title: f.title, does: f.does, tiers: [f] });
      }
    }
    // Small first, then better, then best: the order the sizes go up in, which
    // is the order someone reads a choice like this.
    const rank = (t: string) => (t === "small" ? 0 : t === "better" ? 1 : 2);
    for (const g of groups) g.tiers.sort((a, b) => rank(a.tier) - rank(b.tier));
    return groups;
  });

  /** Which tier the user has pinned for a tool, or `auto`. */
  let tierChoice = $state<Record<string, string>>({});

  async function chooseTier(tool: string, tier: string) {
    tierChoice = { ...tierChoice, [tool]: tier };
    try {
      await setModelTier(tool, tier);
      await refreshModels();
    } catch (e) {
      modelError = errorText(e);
    }
  }

  /** Every capability that is missing and could actually be used. */
  let missingFeatures = $derived(features.filter((f) => !f.downloaded && f.usable));
  /** True while a "Download all" pass is walking the list. */
  let downloadingAll = $state(false);

  /** Fetch every missing model, one at a time.
   *
   * Sequentially, not in parallel: these are tens of megabytes each over one
   * connection, and starting six at once makes all six slower and the progress
   * bars meaningless. One failure does not abandon the rest — the error is
   * reported by `startDownload` and the walk continues, because a "download
   * all" that stops silently at the third row is worse than one that finishes
   * and says which row failed. */
  async function downloadAll() {
    downloadingAll = true;
    // Snapshot first: `downloadable` is derived from `models`, which each
    // completed download refreshes, so iterating it live would walk a list
    // shrinking underneath the loop and skip every other row.
    const queue = [...missingFeatures];
    try {
      for (const f of queue) {
        await installFeature(f);
      }
    } finally {
      downloadingAll = false;
    }
  }

  /**
   * Switch one model on or off.
   *
   * The row is updated from the call's own success BEFORE the refresh lands.
   * The user has just told this screen what the value is; a row that keeps
   * showing the old one until a round trip completes is a row that lags behind
   * the person using it -- and "the trash icon only appears after I collapse
   * and reopen the list" is exactly what that lag looks like, because
   * reopening rebuilds the rows from whatever `models` holds by then.
   *
   * The refresh still runs and still wins: it is the backend's answer, and if
   * the two disagree the backend is right. This only removes the window in
   * which the screen shows something the user has already changed.
   */
  async function toggleModel(m: ModelInfo, enabled: boolean) {
    try {
      await setModelEnabled(m.id, enabled);
      models = models.map((row) => (row.id === m.id ? { ...row, enabled } : row));
      await refreshModels();
    } catch (e) {
      modelError = errorText(e);
    }
  }

  async function removeModel(m: ModelInfo) {
    try {
      await deleteModel(m.id);
      await refreshModels();
    } catch (e) {
      modelError = errorText(e);
    }
  }

  /** 0–1, or null when the source declared no length. */
  function fraction(p: ModelProgress): number | null {
    return p.totalBytes > 0 ? Math.min(p.receivedBytes / p.totalBytes, 1) : null;
  }

  onMount(() => {
    void refreshReceipts();
    void refreshModels();
    // THE SUBSCRIPTION IS NOT OURS ANY MORE. It was a local one, and the
    // progress map was local `$state`, so closing this panel took every trace
    // of an in-flight download with it -- and reopening offered to start a
    // download that was already halfway done. See `downloads.svelte.ts`.
    return downloads.watch();
  });
</script>

<section class="sheet" aria-labelledby="settings-heading">
  <header>
    <h2 id="settings-heading">Settings</h2>
    <button type="button" class="close" onclick={onclose} aria-label="Close settings">
      Close <span class="key" aria-hidden="true">Esc</span>
    </button>
  </header>

  {#if !cfg}
    <p class="loading" role="status">Loading…</p>
  {:else}
    <!-- 1. SANDBOX ------------------------------------------------------- -->
    <div class="head">
      <h3 id="sec-sandbox">Sandbox</h3>
      <InfoTip label="Sandbox" bind:open={info.sandbox} />
    </div>
    {#if info.sandbox}
      <p class="info-text">
        A file from the network, Downloads or a removable drive always gets its
        own sandbox, whichever option is chosen.
        <strong>Share when trusted</strong> reuses one sandbox across the files
        it converts. <strong>One per file</strong> builds a new sandbox for
        every conversion. That is stronger, and slower.
      </p>
      <p class="info-text">
        <strong>Sandbox everything possible</strong> is a separate control. It
        confines the conversions that normally run inside this app, which are
        the formats decoded by memory-safe Rust. Those are not currently a
        known hole; this closes them anyway. Routes with no worker to run in
        cannot be covered, and they are named below when it is on.
      </p>
    {/if}
    <div class="panel">
      <div class="line">
        <span class="label"><span>Worker reuse</span></span>
        <div class="seg" role="group" aria-labelledby="sec-sandbox">
          <button
            type="button"
            aria-pressed={cfg.workerReuse === "balanced"}
            disabled={cfg.workerReuseLocked}
            onclick={() => settings.patch({ workerReuse: "balanced" })}
          >
            Share when trusted
          </button>
          <button
            type="button"
            aria-pressed={cfg.workerReuse === "isolated"}
            disabled={cfg.workerReuseLocked}
            onclick={() => settings.patch({ workerReuse: "isolated" })}
          >
            One per file
          </button>
        </div>
      </div>
      <p class="note">
        {#if cfg.workerReuseLocked}
          Pinned to one per file by policy.
        {:else if cfg.workerReuse === "isolated"}
          Strongest. Costs about 38 ms per file on Windows.
        {:else}
          Faster. Trusted files share one sandbox.
        {/if}
      </p>


      <div class="line">
        <div class="label">
          <span>Sandbox everything possible</span>
          <span class="hint">Confines conversions that would run in this app.</span>
        </div>
        <input
          class="switch"
          type="checkbox"
          role="switch"
          aria-label="Sandbox everything possible"
          checked={cfg.forceSandbox}
          onchange={(e) => settings.patch({ forceSandbox: e.currentTarget.checked })}
        />
      </div>
      <!-- The note that stood here restated the switch in two forms, one per
           state. The hint on the row above already says what it does, and a
           sentence that changes as you toggle invites re-reading it each time
           to check whether anything else changed. -->
    </div>


    <!-- 2. COMPUTE ------------------------------------------------------- -->
    <!--
      WHAT THE WORK RUNS ON, which is not a confinement question.

      Both of these were under Sandbox, and neither belongs there. The graphics
      card is about speed. The memory limit is a ceiling on a worker, which is
      adjacent to confinement but is the thing people come here to raise when a
      model will not run -- and looking for it under "Sandbox" is looking for a
      performance setting under a security heading. Sandbox now holds only the
      two controls that decide what is confined and how.
    -->
    <div class="head">
      <h3 id="sec-compute">Compute</h3>
      <InfoTip label="Compute" bind:open={info.compute} />
    </div>
    {#if info.compute}
      <p class="info-text">
        The graphics card is used only where it is faster: the larger models,
        for upscaling, transcription and reading text. Small models stay on the
        processor because moving them costs more than they save.
      </p>
      <p class="info-text">
        The memory limit is what a single conversion may claim before it is
        stopped. It is a ceiling on a hostile or malformed file as much as a
        budget for a large model, which is why it is not simply set high.
      </p>
    {/if}
    <div class="panel">
      <div class="line">
        <div class="label">
          <span>Use the graphics card</span>
          <!--
            THE CARD'S OWN NAME, not a sentence about graphics cards.

            The hint here used to be "Several times faster for upscaling,
            transcription and reading text" -- true of any machine, and so
            about none. The question a user has in front of this switch is
            whether the card they know they have is the one being used, and
            only its name answers that.

            The dot is lit only when the switch is ON. A green dot beside a
            card whose acceleration is turned off would say the opposite of
            what is true. It means "installed and asked for" -- not "in use
            right now", which nothing on this screen can honestly claim: the
            provider loads per run, and a model too large for the card falls
            back to the processor without telling this panel.
          -->
          {#if gpuName}
            <span class="hint device">
              <span
                class="dot"
                class:on={cfg.useGpu}
                aria-hidden="true"
              ></span>
              {gpuName}
            </span>
          {:else if gpuName === null}
            <span class="hint">
              Several times faster for upscaling, transcription and reading text.
            </span>
          {/if}
        </div>
        <input
          class="switch"
          type="checkbox"
          role="switch"
          aria-label={gpuName ? `Use ${gpuName}` : "Use the graphics card"}
          checked={cfg.useGpu}
          onchange={(e) => settings.patch({ useGpu: e.currentTarget.checked })}
        />
      </div>
      <p class="note">
        {#if cfg.useGpu}
          Used only where it is faster. Small models stay on the processor, and
          anything the card cannot hold falls back to it automatically.
        {:else}
          Everything runs on the processor. Upscaling a photograph takes about
          seven times longer.
        {/if}
      </p>

      <div class="line">
        <div class="label">
          <span>Memory limit per conversion</span>
          <span class="hint">
            What one file is allowed to use before it is stopped.
          </span>
        </div>
        <div class="seg" role="group" aria-label="Memory limit per conversion">
          {#each MEMORY_STEPS.filter((mb) => mb <= cfg.workerMemoryMaxMb) as mb}
            <button
              type="button"
              aria-pressed={cfg.workerMemoryMb === mb}
              onclick={() => settings.patch({ workerMemoryMb: mb })}
            >
              {memoryLabel(mb)}
            </button>
          {/each}
        </div>
      </div>
      <p class="note">
        {#if cfg.workerMemoryMb <= 1024}
          The default. Enough for every conversion except the largest AI
          models, and low enough that a malformed file cannot exhaust this
          machine.
        {:else}
          Raised from 1 GB. This is what a hostile or malformed file would be
          allowed to claim before it is stopped, so raise it for the models
          that need it and not as a general setting.
        {/if}
      </p>
    </div>

    <!-- 3. OUTPUT -------------------------------------------------------- -->
    <div class="head">
      <h3 id="sec-output">Output</h3>
      <InfoTip label="Output" bind:open={info.output} />
    </div>
    {#if info.output}
      <p class="info-text">
        Where converted files are written, and what they are called. Originals
        are never touched unless <strong>Replace originals</strong> is chosen,
        which asks first. Names are built from the template below.
      </p>
    {/if}
    <div class="panel">
      <div class="line">
        <span class="label"><span>Where files go</span></span>
        <div class="seg wrap" role="group" aria-labelledby="sec-output">
          <button
            type="button"
            aria-pressed={cfg.outputDestination === "same_folder"}
            onclick={() => chooseDestination("same_folder")}>Same folder</button
          >
          <button
            type="button"
            aria-pressed={cfg.outputDestination === "downloads"}
            onclick={() => chooseDestination("downloads")}>Downloads</button
          >
          <button
            type="button"
            aria-pressed={cfg.outputDestination === "desktop"}
            onclick={() => chooseDestination("desktop")}>Desktop</button
          >
          <button
            type="button"
            aria-pressed={cfg.outputDestination === "replace_source"}
            onclick={() => chooseDestination("replace_source")}>Replace originals</button
          >
        </div>
      </div>

      {#if confirmingReplace}
        <div class="confirm" role="alertdialog" aria-label="Confirm replacing originals">
          <label class="ack">
            <input type="checkbox" bind:checked={replaceAcknowledged} />
            <span>I understand originals are overwritten.</span>
          </label>
          <div class="confirm-actions">
            <button type="button" class="quiet" onclick={() => (confirmingReplace = false)}>
              Cancel
            </button>
            <button
              type="button"
              class="danger"
              disabled={!replaceAcknowledged}
              onclick={confirmReplace}>Replace originals</button
            >
          </div>
        </div>
      {/if}

      <div class="line">
        <span class="label"><span>File name</span></span>
        <input
          class="field mono"
          type="text"
          aria-label="File name template"
          value={namingDraft ?? cfg.namingTemplate}
          oninput={(e) => (namingDraft = e.currentTarget.value)}
          onblur={commitNaming}
          onkeydown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
        />
      </div>
      <p class="note">{"{name}"}  = original name without extension · {"{ext}"} = output extension · {"{date}"} = conversion date · {"{index}"} = file number. Text outside braces is kept exactly as entered.</p>
      <p class="note">Example input: Holiday.heic · Output: PNG · File number: 1</p>
      <p class="note mono">Template: {namingDraft ?? cfg.namingTemplate}</p>
      <p class="note" role="status">Result: {(namingDraft ?? cfg.namingTemplate).replaceAll("{name}","Holiday").replaceAll("{ext}","png").replaceAll("{index}","1").replaceAll("{date}",new Date().toISOString().slice(0,10))}</p>
      {#if namingError}<p class="error" role="alert">{namingError}</p>{/if}

    </div>

    <!-- 3. HISTORY ------------------------------------------------------- -->
    <div class="head">
      <h3>History</h3>
      <InfoTip label="History" bind:open={info.history} />
    </div>
    {#if info.history}
      <p class="info-text">
        The app keeps conversion records in one local SQLite database in its
        own data folder. Turn this off when you do not want new records stored.
      </p>
    {/if}
    <div class="panel">
      <div class="line">
        <div class="label">
          <span>Keep a record in the app</span>
          <span class="hint">Lets you review and re-run past conversions here.</span>
        </div>
        <input
          class="switch"
          type="checkbox"
          role="switch"
          aria-label="Keep a record in the app"
          checked={cfg.writeReceipts}
          onchange={(e) => settings.patch({ writeReceipts: e.currentTarget.checked })}
        />
      </div>

      <div class="line">
        <span class="label"><span>Past conversions</span></span>
        <div class="stored">
          <span class="stored-size">{receiptCount} stored · {mb(receiptBytes)}</span>
          <div class="stored-actions">
            <button
              type="button"
              class="quiet"
              onclick={clearReceipts}
              disabled={receiptCount === 0}>Delete all</button
            >
            <button type="button" class="quiet" onclick={onopenhistory}>Open</button>
          </div>
        </div>
      </div>
      {#if storeError}<p class="error" role="alert">{storeError}</p>{/if}
    </div>

    <!-- 4. MODELS -------------------------------------------------------- -->
    <div class="head">
      <h3>Local AI models</h3>
      <InfoTip label="Local AI models" bind:open={info.models} />
      {#if missingFeatures.length > 0}
        <button type="button" class="quiet head-action" disabled={downloadingAll} onclick={downloadAll}>
          {downloadingAll
            ? "Downloading…"
            : `Download all · ${mb(missingFeatures.reduce((n, f) => n + f.sizeBytes, 0))}`}
        </button>
      {/if}
    </div>
    {#if info.models}
      <p class="info-text">
        Nothing downloads until you ask for it. Every model runs on this
        machine, and no file and no prompt leaves it. A model you switch off
        stays on disk until you remove it.
      </p>
    {/if}
    <!-- CAPABILITIES FIRST. What someone wants is "transcribe speech", not
         four ONNX artifacts; the artifacts are still here, one disclosure
         down, because a privacy tool that hides what it is fetching would be
         the wrong kind of simple. -->
    <div class="panel">
      {#each capabilities as cap (cap.key)}
        {@const tiered = cap.tiers.length > 1}
        {#if tiered}
          {@const chosen = tierChoice[cap.key] ?? "auto"}
          <!-- ONE JOB, TWO WAYS TO DO IT. The heading names the job; the
               control below picks which model does it. Automatic is the
               default and reproduces what the app did before the choice
               existed: run the best one that is installed. -->
          <div class="capability">
            <div class="label">
              <span>{cap.title}</span>
              <span class="hint">{cap.does}</span>
            </div>
            <div class="seg tiers" role="group" aria-label={`Which model for ${cap.title}`}>
              <button
                type="button"
                aria-pressed={chosen === "auto"}
                onclick={() => chooseTier(cap.key, "auto")}
              >
                Automatic
              </button>
              {#each cap.tiers as t (t.id)}
                <button
                  type="button"
                  aria-pressed={chosen === t.tier}
                  disabled={!t.ready}
                  title={t.ready
                    ? `${mb(t.sizeBytes)}`
                    : "Install this tier before it can be chosen"}
                  onclick={() => chooseTier(cap.key, t.tier)}
                >
                  {t.tier === "small" ? "Small" : t.tier === "better" ? "Better" : "Best"}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      {#each cap.tiers as f (f.id)}
        {@const parts = artifactsOf(f)}
        {@const busy = downloads.active.has(f.id) || parts.some((m) => downloads.progress[m.id])}
        {@const open = openFeatures.has(f.id)}
        <div class="feature">
          <div class="model-head">
            <div class="label">
              <span>{f.title}</span>
              <span class="hint">
                {f.does} · {mb(f.sizeBytes)} · {parts.length} file{parts.length === 1 ? "" : "s"}
              </span>
            </div>

            {#if busy}
              <span class="phase">Downloading…</span>
            {:else if f.downloaded}
              <span class="phase installed">Installed</span>
            {:else if !f.usable}
              <!-- Listed, never offered: downloading something with no button
                   behind it costs disk space and does nothing. -->
              <button type="button" class="quiet" disabled>Download</button>
            {:else}
              <button
                type="button"
                class="quiet"
                disabled={downloadingAll}
                onclick={() => installFeature(f)}
              >
                Download · {mb(f.sizeBytes)}
              </button>
            {/if}
          </div>

          {#if !f.usable}
            <p class="note blocked">No tool in this build uses this yet.</p>
          {/if}

          <button
            type="button"
            class="parts-toggle"
            aria-expanded={open}
            onclick={() => toggleFeatureDetail(f.id)}
          >
            {open ? "Hide" : "Show"} the {parts.length} file{parts.length === 1 ? "" : "s"} this needs
          </button>

          {#if open}
            <div class="parts">
              {#each parts as m (m.id)}
                {@const dl = downloads.progress[m.id]}
                {@render modelRow(m, dl)}
              {/each}
            </div>
          {/if}
        </div>
      {/each}
      {/each}
      {#if modelError}<p class="error" role="alert">{modelError}</p>{/if}
    </div>

    <!-- The per-artifact row, shared by the disclosure above. -->
    {#snippet modelRow(m: ModelInfo, dl: ModelProgress | undefined)}
        <div class="model">
          <div class="model-head">
            <div class="label">
              <span>{m.title}</span>
              <span class="hint">{m.purpose} · {mb(m.sizeBytes)} · {m.licence}</span>
            </div>

            {#if dl}
              <span class="phase">{dl.phase === "verifying" ? "Verifying…" : "Downloading…"}</span>
            {:else if m.downloaded}
              <div class="actions">
                <!-- Reclaiming space is only offered for a model that is off:
                     deleting one you are using is a different decision, and
                     putting it beside the switch would invite it by accident. -->
                {#if !m.enabled}
                  <button
                    type="button"
                    class="trash"
                    onclick={() => removeModel(m)}
                    aria-label={`Remove ${m.title}, frees ${mb(m.sizeBytes)}`}
                    title={`Remove ${m.title}`}
                  >
                    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
                      <path
                        d="M6.5 2.5h3M2.5 4.5h11M4.5 4.5l.6 8a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8M6.8 7v4M9.2 7v4"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                    <span class="frees">{mb(m.sizeBytes)}</span>
                  </button>
                {/if}
                <input
                  class="switch"
                  type="checkbox"
                  role="switch"
                  aria-label={`Enable ${m.title}`}
                  checked={m.enabled}
                  onchange={(e) => toggleModel(m, e.currentTarget.checked)}
                />
              </div>
            {:else if m.blocked}
              <!-- A row that will refuse every press. The button stays, greyed,
                   so the row does not look like it is missing an action; the
                   reason sits underneath rather than waiting to be discovered
                   by pressing it. -->
              <button type="button" class="quiet" disabled>Download</button>
            {:else}
              <button
                type="button"
                class="quiet"
                disabled={downloadingAll}
                onclick={() => startDownload(m)}
              >
                Download
              </button>
            {/if}
          </div>

          {#if m.blocked && !m.downloaded}
            <p class="note blocked">{m.blocked}</p>
          {/if}

          {#if dl}
            {@const f = fraction(dl)}
            <div
              class="bar"
              role="progressbar"
              aria-label={`Downloading ${m.title}`}
              aria-valuemin={0}
              aria-valuemax={f === null ? undefined : 100}
              aria-valuenow={f === null ? undefined : Math.round(f * 100)}
            >
              <div class="bar-fill" class:indeterminate={f === null} style:--fraction={f ?? 0}></div>
            </div>
            <p class="note">
              {#if f === null}
                {mb(dl.receivedBytes)} so far
              {:else}
                {mb(dl.receivedBytes)} of {mb(dl.totalBytes)}
              {/if}
            </p>
          {/if}
        </div>
    {/snippet}

    <!-- 5. APPEARANCE ---------------------------------------------------- -->
    <div class="head">
      <h3>Appearance</h3>
      <InfoTip label="Appearance" bind:open={info.appearance} />
    </div>
    {#if info.appearance}
      <p class="info-text">
        System follows the operating system and changes with it.
      </p>
    {/if}
    <div class="panel">
      <div class="line">
        <span class="label"><span>Theme</span></span>
        <div class="seg" role="group" aria-label="Theme">
          {#each ["system", "light", "dark"] as t (t)}
            <button
              type="button"
              aria-pressed={cfg.theme === t}
              onclick={() => settings.cycleTheme(t as "system" | "light" | "dark")}
            >
              {t[0].toUpperCase() + t.slice(1)}
            </button>
          {/each}
        </div>
      </div>
    </div>

    <!-- 6. ADVANCED ------------------------------------------------------ -->
    <div class="head">
      <h3 id="sec-advanced">Advanced</h3>
    </div>
    <div class="panel">
      <!--
        ONE NETWORK CONTROL, AND IT IS READ.

        Four have stood here over this file's life and three of them did
        nothing: "Network: Offline / Sealed" was stored, validated and read by
        nothing; "Weekly update check" described a fetch that existed nowhere
        in the build; "Diagnostics" controlled nothing at all. Each told the
        reader the app phones home and then invited them to feel safer for
        having turned it off.

        This one gates `models::update_outdated`. Detecting that an artifact
        has been superseded is a comparison between two local files and costs
        no network; the switch decides whether the app may then fetch the
        replacement on its own. Off, and nothing reaches the network unless a
        person presses Download.
      -->
      <!-- THE NAME NOW DESCRIBES THE CODE.

           "Model auto update" promises a program that asks somewhere whether a
           newer model exists. It does not, and it never has: "outdated" here
           means this BUILD pins a different artifact than the one recorded on
           disk, which only becomes true when the app itself is updated. The old
           name invited the reader to picture a background check that does not
           happen — the same overstatement the three switches removed from this
           section were removed for. -->
      <div class="line">
        <div class="label">
          <span>Re-download models when an app update changes them</span>
          <span class="hint">
            No version check and no request until then. New models arrive with a
            new version of OpenConvert, not on their own.
          </span>
        </div>
        <input
          class="switch"
          type="checkbox"
          role="switch"
          aria-label="Re-download models when an app update changes them"
          checked={cfg.modelAutoUpdate}
          onchange={(e) => settings.patch({ modelAutoUpdate: e.currentTarget.checked })}
        />
      </div>

      <div class="line">
        <div class="label">
          <span>Stop everything</span>
          <span class="hint">Halts queued batches at the next file boundary.</span>
        </div>
        <button type="button" class="danger" onclick={onpanic}>
          Stop <span class="key" aria-hidden="true">{PANIC_LABEL}</span>
        </button>
      </div>
    </div>

    <!-- 7. SHORTCUTS ----------------------------------------------------- -->
    <div class="head">
      <h3>Shortcuts</h3>
    </div>
    <div class="panel">
      <dl class="keys">
        {#each HELP as binding (binding.label)}
          <dt><kbd>{binding.label}</kbd></dt>
          <dd>{binding.does}</dd>
        {/each}
        <dt><kbd>{PANIC_LABEL}</kbd></dt>
        <dd>{PANIC_DOES}</dd>
      </dl>
    </div>

    {#if settings.error}
      <p class="error" role="alert">{settings.error}</p>
    {/if}
  {/if}
</section>

<style>
  /* Block flow, deliberately not grid.
     
     `.sheet` is a fixed-height scroller. As a grid its children were grid
     items, and a grid item whose `overflow` is not `visible` has its automatic
     minimum size resolve to **0** — so every section was compressed to ~34 px
     and its content clipped, because the global `.group` in base.css sets
     `overflow: hidden`. Block layout has no such rule: children take their
     content height and the sheet scrolls. */
  .sheet {
    height: 100%;
    overflow-y: auto;
    /* ROOM FOR THE CARDS' OWN EDGES.

       A scroll container clips on BOTH axes -- `overflow-x: visible` resolves
       to `auto` the moment the other axis is not visible -- so with zero
       horizontal padding every `.panel` met this element's edge with its
       shadow, and the first layer of `--shadow-card` is the 0.5px ring that
       DRAWS THE BORDER. Rounded corners on the inside, a hard crop on the
       outside, on every section of this screen.

       The shell inset fixed the same thing one level out last round. This is
       the level in. `--shadow-gutter` is a token rather than a number so the
       next scroller inherits the answer instead of rediscovering the bug. */
    padding-inline: var(--shadow-gutter);
    padding-bottom: var(--space-8);
    /* A thin scrollbar, not none.

       Hiding it entirely removed the only signal that there IS more below —
       the sheet is ~2000 px of content in a ~600 px window, and with no track
       and no thumb it reads as a screen with four settings on it. Thin and
       quiet keeps the right edge tidy without lying about the length. */
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
    scrollbar-gutter: stable;
  }

  .sheet::-webkit-scrollbar {
    width: 8px;
  }

  .sheet::-webkit-scrollbar-track {
    background: transparent;
  }

  .sheet::-webkit-scrollbar-thumb {
    background: var(--hairline-strong);
    border-radius: var(--radius-full);
  }

  .sheet::-webkit-scrollbar-thumb:hover {
    background: var(--text-tertiary);
  }

  /* Section title and its info toggle share a line. */
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-6) 0 var(--space-3);
  }

  /* THE HEADING TAKES THE SLACK. Nothing else does.

     Two `margin-left: auto` in one flex row SPLIT the free space between them
     rather than pushing everything to the end -- that is what an auto margin
     means. In the one section with both an action and an info button, "Download
     all · 63.9 MB" ended up stranded in the middle of the row with the info
     icon at the edge, which is the reported bug and is also why the previous
     attempt at this (ordering the info button last) did not fix it: the second
     auto margin was still there.

     One growing element and no auto margins at all. The heading is the thing
     with a natural claim to the space; the controls travel together at the
     trailing edge, in a fixed order, in every section that has them. */
  .head h3 {
    margin: 0;
    flex: 1 1 auto;
    min-width: 0;
  }

  .head :global(.info) {
    order: 3;
  }

  .info-text {
    margin: 0 0 var(--space-3);
    padding: var(--space-4) var(--space-6);
    border-radius: var(--radius-md);
    background: var(--surface-sunken);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  .info-text :global(strong) {
    color: var(--text-primary);
    font-weight: var(--weight-medium);
  }

  header {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    position: sticky;
    top: 0;
    z-index: 1;
    padding-bottom: var(--space-3);
    background: var(--surface-base);
    box-shadow: -60px 0 var(--surface-base), 60px 0 var(--surface-base);
    isolation: isolate;
  }

  h2 {
    font-size: var(--text-heading-size);
    line-height: var(--text-heading-lh);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.01em;
  }

  h3 {
    margin: 0;
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    font-weight: var(--weight-medium);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }

  .close {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .key {
    color: var(--text-tertiary);
    font-size: var(--text-caption-size);
  }

  /* Named `.panel`, not `.group`: base.css already owns `.group` and applies
     `overflow: hidden` plus a divider between every child. */
  .panel {
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    padding: var(--space-3) 0;
  }

  /* A hairline between controls, but not between a control and its own note. */
  .line + .line,
  .note + .line {
    border-top: 0.5px solid var(--hairline);
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-5);
    min-height: var(--row-h);
    padding: var(--space-3) var(--space-6);
  }

  .seg.wrap {
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  /* Rows carry wide controls now, so a narrow window wraps them onto their own
     line — still right-aligned, so the column of controls never breaks. */
  .line {
    flex-wrap: wrap;
    row-gap: var(--space-3);
  }

  .line > .label {
    flex: 1 1 200px;
  }

  .line > .seg,
  .line > .field,
  .line > .stored,
  .line > .switch,
  .line > .danger {
    margin-left: auto;
  }

  /* Size on the left, actions on the right, under the label. */
  .stored {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .stored-size {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  .stored-actions {
    display: flex;
    gap: var(--space-3);
  }

  /* Shortcut list: key in the left column, what it does in the right. */
  .keys {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-3) var(--space-5);
    margin: 0;
    padding: var(--space-2) var(--space-6);
    align-items: baseline;
  }

  .keys dt {
    margin: 0;
  }

  .keys dd {
    margin: 0;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  kbd {
    display: inline-block;
    padding: 2px var(--space-3);
    border-radius: var(--radius-sm);
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-sunken);
    font-family: var(--font-ui);
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--text-primary);
    white-space: nowrap;
  }

  .label {
    display: grid;
    gap: 1px;
    font-size: var(--text-body-size);
    line-height: var(--text-body-lh);
    min-width: 0;
  }

  .hint,
  .note {
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  /* The ready dot beside the card's name. Grey when acceleration is off, so
     the row reads "this card, not in use" rather than claiming otherwise. */
  .device {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--text-disabled);
    flex: none;
  }

  .dot.on {
    background: var(--accent-positive);
  }

  .note {
    margin: 0;
    padding: 0 var(--space-6) var(--space-3);
  }

  .head-action {
    order: 2;
  }

  /* The reason a download is refused: same weight as any other note, so it
     reads as a fact about the row rather than an error the user caused. */
  .blocked {
    color: var(--muted);
  }

  .error {
    margin: 0;
    padding: var(--space-2) var(--space-6);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--accent-critical);
    overflow-wrap: anywhere;
  }

  /* A small text box, not a panel.

     `flex: 1 1 220px` was written for a ROW; inside `.line.column` the main
     axis is vertical, so 220 px became a height basis and the input grew to
     fill — a 440 px tall field with the template floating in the middle of it.
     A fixed height and a capped width is what the control actually is. */
  .field {
    flex: none;
    align-self: flex-start;
    width: 100%;
    max-width: 320px;
    height: 30px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-sm);
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-base);
    color: var(--text-primary);
    font-size: var(--text-small-size);
  }

  .field:focus-visible {
    border-color: var(--text-tertiary);
  }

  .quiet {
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .quiet:hover:not(:disabled) {
    color: var(--text-primary);
  }

  .quiet:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .danger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    background: var(--accent-critical);
    color: var(--on-accent);
    font-size: var(--text-small-size);
    font-weight: var(--weight-medium);
  }

  .danger:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .danger .key {
    color: inherit;
    opacity: 0.7;
  }

  .confirm {
    margin: 0 var(--space-6) var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-md);
    border: 0.5px solid var(--accent-critical);
    display: grid;
    gap: var(--space-3);
  }

  .ack {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-small-size);
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }

  /* --- models --- */
  .model {
    padding: var(--space-3) 0;
  }

  /* One capability. The card the user reads; the artifacts are inside it. */
  /* The job, and the choice of how to do it. Not a `.model` row: those are
     artifacts, and this is the question above them. */
  .capability {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
    padding: var(--space-3) var(--space-6) var(--space-2);
  }

  .capability + .feature {
    border-top: 0;
  }

  .seg.tiers {
    flex: none;
  }

  .feature {
    padding: var(--space-4) 0;
  }

  .feature + .feature {
    border-top: 0.5px solid var(--hairline);
  }

  .installed {
    color: var(--text-tertiary);
  }

  /* Deliberately quiet: what a capability is made of is available, not
     advertised. Someone auditing what gets fetched will look for it; nobody
     choosing whether to transcribe speech needs to read four ONNX filenames
     first. */
  /* ALIGNED WITH THE DESCRIPTION ABOVE IT. `.model-head` carries
     `padding: 0 var(--space-6)` and this did not, so the link sat a step to
     the left of the title and the sentence it belongs to. */
  .parts-toggle {
    margin-top: var(--space-2);
    margin-inline-start: var(--space-6);
    padding: 0;
    border: 0;
    background: none;
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .parts-toggle:hover {
    color: var(--text-secondary);
  }

  .parts {
    margin-top: var(--space-2);
    margin-inline-start: var(--space-6);
    padding-inline-start: var(--space-4);
    border-inline-start: 2px solid var(--hairline);
  }

  .model + .model {
    border-top: 0.5px solid var(--hairline);
  }

  .model-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-5);
    min-height: var(--row-h);
    padding: 0 var(--space-6);
  }

  /* Reclaim-space control and the on/off switch share one line. */
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex: none;
  }

  .trash {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 28px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-full);
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
    white-space: nowrap;
  }

  .trash svg {
    width: 14px;
    height: 14px;
    flex: none;
  }

  .trash:hover {
    color: var(--accent-critical);
    background: var(--surface-sunken);
  }

  .frees {
    font-variant-numeric: tabular-nums;
  }

  .phase {
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .bar {
    height: 3px;
    margin: var(--space-3) var(--space-6) var(--space-2);
    border-radius: var(--radius-full);
    background: var(--surface-sunken);
    overflow: hidden;
  }

  .bar-fill {
    /* Scaled rather than width-calc'd — see `@property --fraction` in
       tokens.css for why the calc form silently stops updating. */
    width: 100%;
    height: 100%;
    transform: scaleX(var(--fraction, 0));
    transform-origin: left center;
    border-radius: var(--radius-full);
    background: var(--emphasis);
    transition: transform var(--motion-standard);
  }

  /* No declared length: show motion, not a fabricated percentage. */
  .bar-fill.indeterminate {
    width: 35%;
    transform: none;
    animation: slide 1.1s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      translate: -100% 0;
    }
    100% {
      translate: 300% 0;
    }
  }

  .loading {
    padding: var(--space-6) var(--space-6);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  @media (prefers-reduced-motion: reduce) {
    .bar-fill {
      transition: none;
    }
    .bar-fill.indeterminate {
      animation: none;
      width: 100%;
      transform: none;
      opacity: 0.5;
    }
  }
</style>
