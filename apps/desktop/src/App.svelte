<!--
  The shell.

  Four screens — drop, preview, running, done — and the transitions between
  them are the *only* state this file owns about converting. Everything shown
  on any of them arrived from a Tauri command. There is no branch here that
  decides what a file is, whether a conversion is allowed, or what it cost;
  asking the webview those questions is what `09` §1 puts it outside the trust
  boundary to avoid.

  On top of the conversion machine sit two overlay views (Settings, History)
  and the Tools entry point, none of which decide anything either: they render
  what the config/history/tool commands report and write patches back through
  them.

  Focus moves with the screens (`07` §11): each transition lands the user on
  the new screen's primary element, so keyboard-only use never loses the thread.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import DropZone from "./components/DropZone.svelte";
  import FileRow from "./components/FileRow.svelte";
  import FormatPicker from "./components/FormatPicker.svelte";
  import Icon from "./components/Icon.svelte";
  import ProgressList from "./components/ProgressList.svelte";
  import SettingsView from "./components/SettingsView.svelte";
  import AiSetup from "./components/AiSetup.svelte";
  import HistoryPanel from "./components/HistoryPanel.svelte";
  import ToolsMenu from "./components/ToolsMenu.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import ToolWorkspace from "./components/ToolWorkspace.svelte";
  import { prediction } from "./lib/stores/prediction.svelte";
  import { job } from "./lib/stores/job.svelte";
  import { undo } from "./lib/stores/undo.svelte";
  import BatchSummary from "./components/BatchSummary.svelte";
  import ReceiptView from "./components/ReceiptView.svelte";
  import ConversionResults from "./components/ConversionResults.svelte";
  import { settings } from "./lib/stores/settings.svelte";
  import { alternateIndex, resolve } from "./lib/keys";
  import { next, effective } from "./lib/theme";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { drop } from "./lib/drop.svelte";
  import DropOverlay from "./components/DropOverlay.svelte";
  import {
    pickFiles,
    panicStop,
    expandPaths,
    errorText,
    type ConversionResult,
    type ToolDescriptor,
  } from "./lib/ipc";

  /** The eight grab areas an undecorated window needs. */
  const RESIZE_EDGES = [
    "north",
    "south",
    "east",
    "west",
    "north-west",
    "north-east",
    "south-west",
    "south-east",
  ] as const;

  /** Hand a resize gesture to the OS rather than tracking the pointer here. */
  function startResize(e: MouseEvent, edge: (typeof RESIZE_EDGES)[number]) {
    if (e.button !== 0) return;
    e.preventDefault();
    void getCurrentWindow().startResizeDragging(edge as never);
  }

  type Screen = "drop" | "preview" | "running" | "done";
  type View = "main" | "settings" | "history" | "ai-setup";

  let screen = $state<Screen>("drop");
  let view = $state<View>("main");
  $effect(() => { void view; undo.dismissNotice(); });

  /**
   * The AI chooser, once, on first run.
   *
   * Driven off the config rather than a local flag so it survives a restart:
   * the question is answered once per install, including when the answer was
   * "not now". `config` is null until the first load lands, which is why this
   * waits for it rather than firing on mount — a prompt that appears before the
   * app knows whether it was already answered is a prompt that reappears.
   */
  $effect(() => {
    const cfg = settings.config;
    if (cfg && !cfg.aiSetupDone && view === "main" && !activeTool) {
      view = "ai-setup";
    }
  });
  let focused = $state(0);
  let expanded = $state(false);

  /** Why Convert declined to run, or null. Cleared by the next attempt. */
  let convertRefusal = $state<string | null>(null);
  let bulkTarget = $state("");

  $effect(() => {
    const targets = prediction.bulkTargets;
    if (!targets.includes(bulkTarget)) bulkTarget = targets[0] ?? "";
  });

  // While governed, every row follows the shared choice — including rows added
  // after it was made, which is the case a one-shot "Apply" could never cover.
  $effect(() => {
    if (prediction.governed && bulkTarget) void prediction.applyBulkTarget(bulkTarget);
  });

  async function applyBulkFormat(target: string) {
    if (!target) return;
    bulkTarget = target;
    await prediction.applyBulkTarget(target);
  }

  // The banner is where anything the user has to read ends up. `convertRefusal`
  // joins it rather than getting a place of its own: one region for "something
  // needs your attention" is easier to learn than four.
  const banner = $derived(
    prediction.error ?? job.error ?? undo.error ?? settings.error ?? convertRefusal,
  );

  /**
   * The banner clears itself after a few seconds.
   *
   * **Every SOURCE is cleared, not the derived value.** `banner` is read-only —
   * assigning to it would be a compile error, and clearing only one contributor
   * would let the next one in the chain surface and look like a new failure.
   *
   * Three seconds is enough to read one line and short enough that a stale
   * refusal is not still on screen when the next action starts. Nothing is
   * lost: a conversion that failed says so on its own row and in its receipt,
   * both of which persist. This banner is the announcement, not the record.
   */
  let bannerTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const showing = banner;
    if (bannerTimer !== null) {
      clearTimeout(bannerTimer);
      bannerTimer = null;
    }
    if (showing === null) return;
    bannerTimer = setTimeout(() => {
      prediction.error = null;
      job.error = null;
      undo.error = null;
      settings.error = null;
      convertRefusal = null;
      bannerTimer = null;
    }, 3000);
    return () => {
      if (bannerTimer !== null) {
        clearTimeout(bannerTimer);
        bannerTimer = null;
      }
    };
  });

  // Component handles for focus and the Alt+T path.
  let dropZone: { focusPrimary(): void } | null = $state(null);
  let toolsMenu: { toggle(): void } | null = $state(null);

  // The one tool dialog that can be open at a time.
  let activeTool = $state<ToolDescriptor | null>(null);

  /**
   * Whether a tool of this category can be pointed at a file of this kind.
   *
   * The two vocabularies are not the same and neither is wrong: a tool's
   * category is what the tool is FOR ("pdf"), and a probe's kind is what the
   * file IS ("document"). PDF is the one place they diverge, and video counts
   * as audio because the audio workspace is what opens for it.
   *
   * An unknown kind is let through rather than filtered out. The queue holds
   * files the backend could not classify, and dropping them here would mean a
   * tool silently ignoring a file the user can see in the list -- worse than
   * the tool refusing it out loud.
   */
  function toolTakes(category: string, kind: string | undefined): boolean {
    if (!kind || kind === "unknown") return true;
    if (category === "pdf") return kind === "document";
    if (category === "audio" || category === "video") {
      return kind === "audio" || kind === "video";
    }
    return kind === category;
  }
  let toolSeedPaths = $state<string[]>([]);

  /**
   * The shell's claim on dropped files.
   *
   * Registered first, so it sits at the bottom of the stack: anything that
   * opens over the top — a tool workspace — registers later, takes drops while
   * it is open, and hands them back when it closes. The shell no longer has to
   * know what might be covering it, which is what the `activeTool !== null`
   * check here used to be doing badly.
   */
  const converterTarget = {
    enabled: () =>
      activeTool === null && view === "main" && screen !== "running" && !prediction.loading,
    onFiles: (paths: string[]) => void accept(paths),
  };

  /** True while a drag is over the window and THIS screen would take it. */
  const dragging = $derived(drop.dragging && drop.active === converterTarget);

  onMount(() => {
    void settings.load();
    return drop.register(converterTarget);
  });

  /** Focus lands on the primary element of whatever just appeared. */
  async function focusScreen() {
    await tick();
    switch (screen) {
      case "drop":
        dropZone?.focusPrimary();
        break;
      case "preview":
        document.querySelector<HTMLElement>(".suggestion")?.focus();
        break;
      case "running":
        document.querySelector<HTMLElement>(".running")?.focus();
        break;
      case "done":
        document.getElementById("results-heading")?.focus();
        break;
    }
  }

  $effect(() => {
    void screen;
    if (view === "main") void focusScreen();
  });

  /** What the last drop expanded to, when it contained a folder. */
  let dropNote = $state<string | null>(null);

  /**
   * Take files from any source — the drop zone, Ctrl+O, Browse.
   *
   * **ADDING FILES ADDS THEM.** This replaced the list unconditionally, so
   * dropping one more file onto a batch of forty left one file and no trace of
   * the other thirty-nine. The rule now: a list that is already on screen is
   * extended, and a list is only replaced when there is nothing to replace —
   * which is the drop screen, the only place where "start over" is what the
   * gesture means.
   *
   * `back()` and the Backspace binding are what clear the list, and they say
   * so; a second drop does not.
   */
  async function accept(paths: string[]) {
    // Appending to a finished batch means starting a new one: the results on
    // screen belong to files that have already been converted, and the undo
    // window is about those outputs, not these inputs.
    const adding = screen === "preview" && prediction.files.length > 0;

    undo.reset();
    job.reset();
    if (!adding) focused = 0;
    expanded = false;
    dropNote = null;

    // A dropped FOLDER is one path, and everything downstream expects files.
    // Expanding host-side keeps the walk where the filesystem access already
    // lives: the webview never gets one.
    let files = paths;
    try {
      const drop = await expandPaths(paths);
      files = drop.files;
      if (drop.folders > 0) {
        const n = drop.files.length;
        dropNote =
          `${n} file${n === 1 ? "" : "s"} found in ` +
          `${drop.folders} folder${drop.folders === 1 ? "" : "s"}` +
          (drop.skipped > 0 ? ` · ${drop.skipped} not convertible` : "") +
          (drop.truncated ? " · stopped at the limit" : "");
      }
    } catch (e) {
      // Expansion failing must not lose the drop: fall back to the paths as
      // they arrived and let detection say what it thinks of them.
      prediction.error = errorText(e);
    }

    if (files.length === 0) {
      if (!adding) screen = "drop";
      return;
    }

    const had = prediction.files.length;
    if (adding) {
      await prediction.add(files);
      const gained = prediction.files.length - had;
      // A drop that added nothing has to say so. Silence here reads exactly
      // like the bug this replaced — the list did not grow, and nothing on
      // screen explains whether the files were rejected or thrown away.
      //
      // NO "N FILES ADDED" MESSAGE. The list grew by N rows in front of the
      // user; a banner restating it is a second answer to a question the
      // screen has already answered, and it appeared on every single drop.
      //
      // A drop that added NOTHING still has to speak, because there the screen
      // shows no change at all and silence is indistinguishable from the bug
      // this replaced.
      if (gained === 0) {
        dropNote =
          files.length === 1
            ? "That file is already in the list."
            : "Those files are already in the list.";
      }
    } else {
      await prediction.accept(files);
    }
    screen = prediction.files.length > 0 ? "preview" : "drop";
  }

  /** Ctrl+O / Browse: the host-side dialog. Drag-drop stays. */
  async function browse() {
    try {
      const picked = await pickFiles();
      if (picked && picked.length > 0) await accept(picked);
    } catch (e) {
      prediction.error = errorText(e);
    }
  }

  /**
   * Enter runs only what the backend was confident about.
   *
   * A row whose chosen format is below the confidence threshold does not get
   * guessed at — its format menu opens instead, and nothing converts until the
   * user picks (02 §12.2). The threshold lives in the suggest response
   * (`armed`); nothing here hard-codes a number.
   */
  async function confirm() {
    if (screen !== "preview" || !prediction.ready) return;

    // ASK ONCE, THEN BELIEVE THE ANSWER.
    //
    // A prediction below the arming threshold should open the chooser rather
    // than convert on a guess. This used to test `armed` alone -- a property of
    // the PREDICTION that nothing the user did could change -- so answering the
    // chooser left the condition true and the next click re-opened it. Any
    // conversion whose top prediction scored under 0.85 was unreachable, and
    // the button looked simply broken.
    //
    // `confirmed` is set when the user picks a target, so the second click
    // converts.
    const unconfirmed = prediction.files.findIndex(
      (f) =>
        f.plan?.executable === true &&
        f.options[f.selected]?.armed !== true &&
        f.confirmed !== true,
    );
    if (unconfirmed >= 0) {
      convertRefusal = null;
      focused = unconfirmed;
      await tick();
      document.querySelectorAll<HTMLElement>(".row .picker .trigger")[unconfirmed]?.click();
      return;
    }

    const batches = prediction.batches();
    if (batches.length === 0) {
      // SILENTLY RETURNING IS WHAT MADE THE ORIGINAL BUG INVISIBLE. Convert
      // did nothing and said nothing, so there was no thread to pull. Any
      // refusal to run has to name itself on screen — even one that should be
      // unreachable, because "should be unreachable" is exactly the state
      // nobody investigates.
      convertRefusal =
        prediction.files.length === 0
          ? "There are no files to convert."
          : "None of these files has a conversion that can run. Choose a format for them.";
      return;
    }
    convertRefusal = null;

    screen = "running";
    const results = await job.run(batches);
    screen = "done";
    undo.offer(results.filter((r) => r.success).length);
  }

  /** Drop one file from the list; an empty list is not a screen worth being on. */
  function removeFile(index: number) {
    prediction.remove(index);
    if (prediction.files.length === 0) back();
  }

  function back() {
    job.reset();
    undo.reset();
    prediction.reset();
    screen = "drop";
  }

  async function undoConversion() {
    await undo.invoke();
    if (undo.done && activeTool === null && screen === "done") {
      job.reset();
      screen = prediction.files.length ? "preview" : "drop";
      await focusScreen();
    }
  }

  /** Stop everything, and say honestly how far that reached. */
  async function panic() {
    try {
      await panicStop();
      if (job.running) job.cancelling = true;
      undo.reset();
    } catch (e) {
      settings.error = errorText(e);
    }
  }

  /** A tool run reuses the running/done screens unchanged. */
  /**
   * Save from the tool workspace.
   *
   * **Does not navigate.** The workspace is where the user is working — a tool
   * run that throws them back to the conversion screens loses the preview they
   * were judging the result against, and makes a second operation on the same
   * file a fresh start. It confirms in place; the receipt is in History like
   * every other run.
   */
  /** Run one tool and hand the workspace back what it produced.
   *
   * The results, not a boolean: a tool whose output is TEXT has to be read
   * back and shown, and that needs the path. The transcript panel sat
   * permanently empty because this returned `true` and threw the rest away. */
  async function runToolFromWorkspace(
    id: string,
    paths: string[],
    params: Record<string, number | string | boolean>,
  ): Promise<ConversionResult[]> {
    const results = await job.runOneTool(id, paths, params);
    undo.offer(results.filter((r) => r.success).length);
    return results;
  }

  /**
   * Go to an overlay view, closing whatever is over the top of it.
   *
   * **THE OVERLAY WAS OPENING UNDERNEATH THE TOOL.** The render chain is
   * `{#if activeTool} … {:else if view === "settings"}`, so setting `view`
   * while a workspace was open changed a value nothing could show: Settings
   * WAS open, beneath a tool that still had the screen. Pressing Close then
   * revealed it, which is exactly the "you have to close first" report.
   *
   * One function rather than three call sites each remembering to clear
   * `activeTool` — that is how the third one comes to forget.
   */
  function goTo(next: View) {
    activeTool = null;
    view = view === next ? "main" : next;
  }

  function closePanel() {
    view = "main";
    void focusScreen();
  }

  async function openHistory() {
    goTo("history");
  }

  async function onkeydown(e: KeyboardEvent) {
    if (e.defaultPrevented) return;
    const action = resolve(e);
    if (action === null) return;

    // Panels take Escape before the machine does.
    if (view !== "main") {
      if (action === "cancel") {
        e.preventDefault();
        closePanel();
      }
      if (action === "openSettings") {
        e.preventDefault();
        view = view === "settings" ? "main" : "settings";
      }
      return;
    }
    if (activeTool) {
      // The workspace owns the screen while it is open — with two exceptions.
      // Escape closes it, and the overlay shortcuts open OVER it, because a
      // binding that silently does nothing is indistinguishable from one that
      // is broken. `Ctrl+,` used to be swallowed here entirely.
      if (action === "cancel") {
        e.preventDefault();
        activeTool = null;
      }
      if (action === "openSettings") {
        e.preventDefault();
        goTo("settings");
      }
      return;
    }

    const alternate = alternateIndex(action);
    if (alternate !== null) {
      if (screen !== "preview") return;
      e.preventDefault();
      await prediction.selectFile(focused, alternate);
      return;
    }

    switch (action) {
      case "confirm":
        e.preventDefault();
        if (screen === "preview") await confirm();
        else if (screen === "done") back();
        break;
      case "cancel":
        e.preventDefault();
        if (screen === "running") await job.cancel();
        else if (screen === "preview") back();
        break;
      case "undo":
        if (undo.offered) {
          e.preventDefault();
          await undoConversion();
        }
        break;
      case "reset":
        if (screen === "done" || screen === "preview") {
          e.preventDefault();
          back();
        }
        break;
      case "toggleAlternates":
        if (screen === "preview") {
          e.preventDefault();
          expanded = !expanded;
        }
        break;
      case "toggleTheme":
        e.preventDefault();
        await settings.cycleTheme(next(settings.theme));
        break;
      case "openFiles":
        e.preventDefault();
        if (screen === "drop" || screen === "done" || screen === "preview") await browse();
        break;
      case "openSettings":
        e.preventDefault();
        goTo("settings");
        break;
      case "openTools":
        e.preventDefault();
        toolsMenu?.toggle();
        break;
      case "panic":
        e.preventDefault();
        await panic();
        break;
      default:
        break;
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="app">
  <!-- One bar: the app's chrome and the window's controls are the same strip.
       A white system title bar above a dark application reads as two programs
       stacked, and the seam is the first thing anyone sees. -->
  <TitleBar>
    <ToolsMenu
      bind:this={toolsMenu}
      onpick={(tool) => {
        // ONLY THE FILES THIS TOOL CAN ACTUALLY WORK ON.
        //
        // This handed the workspace the WHOLE conversion queue. Queue three
        // photographs and a PDF, open Invert colours, and the image tool was
        // given the PDF too -- so it previewed a document it cannot invert
        // and refused to run at all. The reported symptom was that a mixed
        // selection makes the image tools unusable, and it was exactly that:
        // one unrelated file in the list poisons the whole run.
        //
        // Filtering rather than clearing, because the queue is usually right:
        // ten photographs queued for conversion are ten photographs to
        // invert. It is the fourth file of another kind that is not.
        toolSeedPaths =
          screen === "preview"
            ? prediction.files
                .filter((f) => toolTakes(tool.category, f.probe.properties?.kind))
                .map((f) => f.probe.path)
            : [];
        activeTool = tool;
      }}
    />

    <button
      type="button"
      class="theme titlebar-control"
      onclick={() => settings.cycleTheme(next(settings.theme))}
      aria-label={`Theme: ${settings.theme}. Switch to ${next(settings.theme)}.`}
    >
      <Icon name={effective(settings.theme) === "dark" ? "moon" : "sun"} />
    </button>

    <button
      type="button"
      class="gear titlebar-control"
      onclick={() => goTo("settings")}
      aria-label="Settings"
      aria-expanded={view === "settings"}
      title="Settings — Ctrl+,"
    >
      <Icon name="settings" />
    </button>
  </TitleBar>

  <!-- The native frame is gone, so the resize borders are too. Eight thin
       strips give them back; each hands the gesture straight to the OS. -->
  {#each RESIZE_EDGES as edge (edge)}
    <button
      type="button"
      class="resize {edge}"
      aria-label={`Resize window from the ${edge.replace("-", " ")}`}
      tabindex="-1"
      onmousedown={(e) => startResize(e, edge)}
    ></button>
  {/each}

  {#if banner}
    <p class="banner" role="alert">{banner}</p>
  {/if}

  {#if activeTool}
    <main class="overlay">
      <!-- Keyed on the tool's id so switching section — Audio to Image, say —
           REMOUNTS the workspace. Without the key the same instance is reused:
           `onMount` never re-runs, so the command rail, the applied operations
           and the rendered preview all stay on the previous tool while the
           title changes. That is the "page does not update" bug. -->
      {#key activeTool.id}
        <ToolWorkspace
          tool={activeTool}
          paths={toolSeedPaths}
          destination={settings.config?.outputDestination ?? "downloads"}
          onclose={() => (activeTool = null)}
          onrun={runToolFromWorkspace}
        />
      {/key}
    </main>
  {:else if view === "ai-setup"}
    <main class="overlay">
      <AiSetup onclose={() => (view = "main")} />
    </main>
  {:else if view === "settings"}
    <main class="overlay">
      <SettingsView onclose={closePanel} onopenhistory={openHistory} onpanic={panic} />
    </main>
  {:else if view === "history"}
    <main class="overlay">
      <HistoryPanel onclose={closePanel} onrepeat={async (request) => { view = "main"; const results = await job.run([request]); undo.offer(results.filter(r=>r.success).length); }} />
    </main>
  {:else}
    <main class:drop-screen={screen === "drop"}>
      {#if job.running}
        <!-- The single hairline bar, top of the content area (07 §10). -->
        <div
          class="content-progress"
          role="progressbar"
          aria-label="Files converted"
          aria-valuemin={0}
          aria-valuemax={job.total}
          aria-valuenow={job.completed}
        >
          <div
            class="progress-track"
          >
            <div
              class="progress-fill"
              style:--fraction={job.total > 0 ? Math.min(job.completed / job.total, 1) : 0}
            ></div>
          </div>
        </div>
      {/if}

      {#if screen === "drop"}
        <!-- The drop screen is one full-bleed target; every other screen is a
             stack of cards that scrolls. Hence the row template below. -->
        <DropZone
          bind:this={dropZone}
          onBrowse={browse}
          disabled={prediction.loading}
          {dragging}
        />
        {#if prediction.loading}
          <p class="working" role="status">Reading the files…</p>
        {:else if dropNote}
          <p class="working" role="status">{dropNote}</p>
        {/if}
      {:else if screen === "preview"}
        <!-- One row per file, and a bar that acts on the lot. The list takes
             the height; the bar stays pinned at the bottom. -->
        <!-- A drop onto a loaded batch ADDS, and the overlay says so over the
             list rather than as a line above it: the files staying visible
             underneath is the part that makes "added" believable, and a
             one-line status above a full-height list is easy to miss entirely.
             The overlay itself lives at the bottom of this block, so it can
             cover the whole screen rather than sit in the flow. -->
        {#if dropNote && !dragging}
          <p class="drop-hint" role="status">{dropNote}</p>
        {/if}
        {#if prediction.files.length > 1}
        <!-- ONE FILE HAS NOTHING TO SELECT ALL OF. The bar is a bulk control;
             over a single row it offers to do to one file what that file's own
             format menu already does, and "1 selected" is not information. -->
        <!--
          SELECT ALL GOVERNS; IT DOES NOT MERELY SELECT.

          Ticking rows and then pressing a separate "Apply format" was two
          gestures for one intention, and it left the rows and the shared
          dropdown able to disagree in between. On, the dropdown IS the target
          for every file and the per-row menus lock; off, every row decides for
          itself. There is nothing left for an Apply button to do.

          Off by default: a batch of mixed formats usually wants its own
          answers, and taking them over silently is the more surprising default.
        -->
        <div class="batch-tools" aria-label="Batch format selection">
          <label class="select-all">
            <input
              type="checkbox"
              checked={prediction.governed}
              onchange={(event) => prediction.setGoverned(event.currentTarget.checked)}
            />
            Convert all to
          </label>
          <FormatPicker
            options={prediction.bulkOptions}
            selected={prediction.bulkOptions.findIndex((o) => o.target === bulkTarget)}
            onselect={(i) => applyBulkFormat(prediction.bulkOptions[i]?.target ?? "")}
            disabled={!prediction.governed || prediction.bulkOptions.length === 0}
          />

          <!--
            CLEAR ALL, at the other end of the bar the batch controls are on.

            Emptying the list took removing every row one at a time, or
            pressing Escape -- which is not a thing anyone finds, and which
            reads as "go back" rather than "throw this away". Dropping a folder
            you did not mean to drop is a common enough mistake to have a
            button.

            On the batch bar rather than beside Convert: it acts on the whole
            list, which is what this bar is for, and putting a destructive
            control next to the one people press without reading is how it gets
            pressed without reading. The bar only appears above two files or
            more, which is also when clearing is worth doing.
          -->
          <button type="button" class="clear-all" onclick={back}>Clear all</button>
        </div>
        {/if}
        <div class="file-list">
          {#each prediction.files as file, i (file.probe.path)}
            <FileRow
              probe={file.probe}
              options={file.options}
              selected={file.selected}
              plan={file.plan}
              planning={file.planning}
              checked={file.checked}
              onselect={(o) => prediction.selectFile(i, o)}
              lockedTo={prediction.governed ? bulkTarget : null}
              onchecked={(checked) => prediction.setChecked(i, checked)}
              onremove={() => removeFile(i)}
            />
          {/each}
        </div>

        <div class="action-bar">
          <span class="ready" role="status">
            {prediction.runnable} of {prediction.totalFiles} ready
            {#if prediction.totalFiles !== prediction.runnable}
              · {prediction.totalFiles - prediction.runnable} cannot run
            {/if}
          </span>
          <button type="button" class="secondary" onclick={browse}>Add more files</button>
          <button type="button" class="primary" onclick={confirm} disabled={!prediction.ready}>
            Convert
            <span class="key" aria-hidden="true">⏎</span>
          </button>
        </div>

        {#if dragging}
          <DropOverlay
            label="Release to add to this list"
            sublabel="Nothing already here is replaced"
          />
        {/if}
      {:else if screen === "running"}
        <ProgressList
          progress={job.progress}
          total={job.total}
          completed={job.completed}
          cancelling={job.cancelling}
          oncancel={() => job.cancel()}
        />
      {:else}
        <div class="results">
          {#if job.results.length === 1}
            <!-- One file: the receipt IS the result. Nothing is summarised,
                 because there is nothing to summarise over. -->
            <h2 id="results-heading" tabindex="-1" class="summary" role="status">
              {job.succeeded} converted{job.failed > 0 ? `, ${job.failed} did not` : ""}
            </h2>
            {#each job.results as result}<ReceiptView {result}/>{/each}
          {:else}
            <!-- Many files: totals first, then one row each. The full receipt
                 is still one click away on any row that needs explaining. -->
            <!-- A label, not a count: BatchSummary below carries the counts
                 and is already a live region, so repeating them here would
                 announce the same fact twice to a screen reader. -->
            <h2 id="results-heading" tabindex="-1" class="sr-only">Results</h2>
            <BatchSummary results={job.results} durationMs={job.durationMs} />
            <ConversionResults results={job.results} />
          {/if}
        </div>
      {/if}
    </main>
  {/if}

  <!--
    ONE TOAST, AND IT IS WHERE THE ACTIONS LIVE.

    There used to be a `Done` button stranded at the bottom of the results
    card and a separate toast floating over it, so the two things a person
    wants after a conversion — undo it, or do another — were in different
    places, and one of them moved depending on how many files there were.

    The expired state also carried a sentence, "Outputs stay put; the record
    lives in History", explaining a thing nobody asked about at the moment they
    were deciding what to do next. The buttons say what is available; the
    sentence said what had already happened.
  -->
  <!--
    NOT OVER A TOOL WORKSPACE.

    The workspace has its own after-save row -- in the footer, in place of the
    destination picker and Save, where the action that produced the file was.
    This toast floating over it said the same thing a second time, in a
    different place, with a countdown the workspace row does not need: two
    Undos on screen at once, and the toast's one covering the picture the
    tool had just produced.
  -->
  {#if activeTool}
    <!-- nothing: the workspace footer owns this -->
  {:else if undo.offered}
    <div class="toast" role="status">
      <span>
        {undo.count} output{undo.count === 1 ? "" : "s"} written
      </span>
      <button type="button" class="undo" onclick={() => void undoConversion()}>
        Undo <span class="countdown">{undo.remaining}s</span>
      </button>
      {#if screen === "done"}
        <button type="button" class="again" onclick={back}>Convert again</button>
      {/if}
    </div>
  {:else if undo.expired}
    <div class="toast" role="status">
      <button type="button" class="undo" onclick={() => { undo.reset(); void openHistory(); }}>
        Show history
      </button>
      {#if screen === "done"}
        <button type="button" class="again" onclick={back}>Convert again</button>
      {/if}
    </div>
  {:else if undo.done}
    <div class="toast" role="status">
      <span>Outputs removed.</span>
      {#if screen === "done"}
        <button type="button" class="again" onclick={back}>Convert again</button>
      {/if}
    </div>
  {:else if screen === "done"}
    <!-- The toast is the only way off this screen, so it has to exist even
         when there is no undo left to offer. -->
    <div class="toast" role="status">
      <button type="button" class="again" onclick={back}>Convert again</button>
    </div>
  {/if}

  <!--
    NO KEYBOARD-HINT FOOTER.

    Eleven `<kbd>` chips sat across the bottom of every screen, on every screen,
    for the life of the window. They were not help: help is available when you
    look for it, and this was a permanent band of syntax under a program whose
    main gesture is dropping a file on it — Enter, Space and 1-9 in particular
    describe the preview screen and were shown just as prominently over the
    drop screen, where they do nothing.

    THE BINDINGS ARE UNCHANGED. `keys.ts` still resolves every one of them, and
    Settings still lists the whole table with what each does, which is where
    someone goes to find out. What is gone is the band, not the shortcuts.
  -->
</div>

<style>
  /* Flex column, not a four-row grid.
     
     The grid template was `auto auto 1fr auto` for header / banner / main /
     footer — but the banner is conditional. With no error showing there were
     three children for four rows, so `main` took an `auto` row and the FOOTER
     took the `1fr`: measured at 302 px of keyboard hints against 254 px of
     content, on every screen. A column that sizes `main` by flex has no
     positional assumption to get wrong. */
  /* Full width, not a 720 px column.
     
     `--content-max` is a READING measure and was being applied to the whole
     shell: on a 1280 px window it left 560 px of empty margin either side and
     capped the drop target at 672 px, which is the "small box, lots of empty
     space" everyone hits first. Views that genuinely want a measure now set it
     themselves. */
  /* Resize borders, since the native frame no longer supplies them.
     6 px is the Windows grab width; the corners take priority by sitting
     later in the source with a larger box. */
  .resize {
    position: fixed;
    z-index: 40;
    background: transparent;
    border: 0;
    padding: 0;
  }

  .resize.north,
  .resize.south {
    left: 6px;
    right: 6px;
    height: 6px;
    cursor: ns-resize;
  }

  .resize.north {
    top: 0;
  }
  .resize.south {
    bottom: 0;
  }

  .resize.east,
  .resize.west {
    top: 6px;
    bottom: 6px;
    width: 6px;
    cursor: ew-resize;
  }

  .resize.east {
    right: 0;
  }
  .resize.west {
    left: 0;
  }

  .resize.north-west,
  .resize.north-east,
  .resize.south-west,
  .resize.south-east {
    width: 10px;
    height: 10px;
  }

  .resize.north-west {
    top: 0;
    left: 0;
    cursor: nwse-resize;
  }
  .resize.north-east {
    top: 0;
    right: 0;
    cursor: nesw-resize;
  }
  .resize.south-west {
    bottom: 0;
    left: 0;
    cursor: nesw-resize;
  }
  .resize.south-east {
    bottom: 0;
    right: 0;
    cursor: nwse-resize;
  }

  /* ONE CONTENT INSET, AT THE SHELL.

     Horizontal padding was zero, so every card met the window frame with its
     shadow and border cut off flat against the edge — a rounded corner on the
     inside and a hard crop on the outside, on every screen. The overlay views
     were given a vertical inset when History was fixed; the conversion screens
     never got the horizontal one.

     Here rather than on each card, because per-card margins are what drift
     apart. The drop screen opts out below: it is deliberately full-bleed. */
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    /* No top padding: the title bar is the top edge now. */
    padding: 0 var(--space-5) var(--space-5);
    gap: var(--space-4);
  }

  .app > :global(main) {
    margin-inline: calc(var(--space-6) - var(--shadow-gutter));
    /* The block gutter is given back the same way, so nothing moves. */
    margin-block: calc(var(--shadow-gutter) * -1);
  }

  .app > .banner {
    margin-inline: var(--space-6);
  }

  .app > .banner {
    flex: none;
  }

  .app > main {
    flex: 1 1 auto;
  }





  .theme,
  .gear {
    width: 28px;
    height: 28px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: var(--radius-full);
    color: var(--text-secondary);
  }


  .theme:hover,
  .gear:hover {
    color: var(--text-primary);
    background: var(--surface-sunken);
  }

  main {
    display: grid;
    align-content: start;
    gap: var(--space-6);
    overflow-y: auto;
    min-height: 0;
    /* ROOM FOR THE CARDS' OWN EDGES -- see `--shadow-gutter`.

       `overflow-y: auto` makes this a scroll container, and a scroll container
       clips horizontally too. Every card inside it lost the 0.5px ring that
       draws its border, flat against both sides. The margin below is reduced
       by the same amount, so the cards do not move: the gutter is space this
       element gives back to its children, not new space in the layout. */
    padding-inline: var(--shadow-gutter);
    /* AND ON THE BLOCK AXIS, for the same reason and the same amount.

       Only the inline axis was given a gutter, so the fix held on the left and
       right and the bottom edge of the last card -- the bar carrying "3 of 3
       ready", "Add more files" and Convert -- was still cut off flat against
       the end of the scroller. Reported as "the bottom is cut off like the
       sides were", which is exactly what it was.

       `.app` already has bottom padding; that is the gap between the window
       frame and the scroller. This is the gap between the scroller and the
       card inside it, which is a different gap and was missing. */
    padding-block: var(--shadow-gutter);
  }

  /* The drop target takes the whole content area. `1fr auto` gives the zone
     every spare pixel and leaves the "Reading the files…" status its own row;
     with only one child the zone simply gets the lot. */
  /* One explicit row, so the zone gets the whole area. The optional
     "Reading the files…" status lands in an implicit auto row below it —
     declaring that row up front left a 12 px gap under the zone whenever the
     status was absent, which is nearly always. */
  /* Preview: the list takes the slack, everything else keeps its height.

     FLEX, NOT A ROW TEMPLATE — AND THIS IS THE THIRD TIME.

     This was `grid-template-rows: auto minmax(0, 1fr) auto`: three named rows
     for a screen with FOUR possible children, two of which are conditional.
     With nothing above the bar the rows lined up. Set `dropNote` — which
     adding a second file does, every time, to say "1 file added." — and a
     fourth child appears at the top, every element shifts down one row, and
     `.batch-tools` inherits the `1fr`. The format bar then grew to fill the
     window, the file list collapsed to its content height, and the action bar
     fell into an implicit row off the bottom.

     The same mistake is recorded twice already in this file: once for the app
     shell, where a conditional banner handed the FOOTER the `1fr`, and once
     for `main.drop-screen`. A row template is a positional promise, and a
     conditional sibling breaks it silently — the layout does not error, it
     just assigns the wrong thing.

     Flex removes the positional assumption entirely: whatever is present
     stacks, and `.file-list` is the only thing that grows. */
  main:has(.file-list) {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    overflow: hidden;
    /* The drop overlay is `inset: 0` against this. */
    position: relative;
  }

  main.drop-screen {
    align-content: stretch;
    grid-template-rows: 1fr;
    gap: var(--space-4);
    overflow: hidden;
  }

  /* An overlay view owns the whole area and scrolls inside itself. `grid` with
     `align-content: start` sizes the row to CONTENT, so a child asking for
     `height: 100%` got its own content height and never scrolled. A single
     definite row fixes that. */
  /* AN INSET ON ALL FOUR SIDES, SO THE ROUNDING READS ON ALL FOUR.

     `main` sets `padding: 0 0 var(--space-5)` — no top padding, because on the
     conversion screens the title bar is the top edge. An overlay panel stretched
     into that row therefore met the window at its top and bottom edges, where
     its `--radius-xl` corners were simply clipped away. The panel then looked
     like a rounded card down the left and right and a full-bleed sheet top and
     bottom, which is exactly how it reads in the History screenshot: a
     different container on two sides and the background on the other two. */
  main.overlay {
    display: grid;
    grid-template-rows: minmax(0, 1fr);
    align-content: stretch;
    overflow: hidden;
    padding-top: var(--space-4);
    padding-bottom: var(--space-2);
  }

  /* Settings and History are prose-shaped, so they keep a measure — centred
     inside the now full-width shell rather than dragging the shell in. */
  main.overlay > :global(*) {
    width: 100%;
    max-width: var(--content-max);
    margin: 0 auto;
    min-height: 0;
  }

  /* Pinned to the very top of the content area: the design system's
     "single hairline bar" for progress (07 §10). */
  .content-progress {
    position: sticky;
    top: calc(-1 * var(--space-6));
    z-index: 5;
    padding-top: 2px;
  }

  /* `.progress-track` and `.progress-fill` are styled in `base.css`, globally,
     and always were.

     A scoped copy stood here for a while and it was a mistake twice over: it
     duplicated rules that already worked, and it introduced `var(--accent)`,
     which is not a token in this system -- the accents are
     `--accent-attention` and `--accent-critical`. An undefined custom property
     with no fallback makes the whole declaration invalid at computed-value
     time, so the fill painted transparent. Measured as `rgba(0, 0, 0, 0)`.

     Two rules for one element is how the second one comes to disagree with the
     first. There is one. */

  /* NO LEFT RAIL. A raised surface with a coloured edge reads as a card that
     happens to be about an error; the ground itself carries it now, and the
     ink is picked per scheme rather than being a red chosen to sit on grey. */
  .banner {
    padding: var(--space-4) var(--space-5);
    border-radius: var(--radius-md);
    background: var(--banner-bg);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--banner-ink);
    overflow-wrap: anywhere;
  }

  .working,
  .summary {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
    text-align: center;
  }

  .results {
    display: grid;
    gap: var(--space-4);
  }

  /* The drop, as a list. It takes the height and scrolls; the bar below is
     pinned so Convert never walks off the bottom of a long drop. */
  .file-list {
    display: flex;
    flex-direction: column;
    /* The one element on this screen that takes the slack. Everything else
       sizes to its content, so no conditional sibling can steal the space. */
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
  }

  /* A CONTROL STRIP OVER THE LIST, NOT A CARD BESIDE IT.
     
     This carried the same raised surface, border and radius as the file list
     below it and the action bar beneath that — three cards of equal weight for
     three things of very different importance, which is why the screen read as
     one repeated shape. What acts on the list belongs visually TO the list:
     no fill, no border, tighter padding, sitting directly above it. */
  .batch-tools {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-2) var(--space-1);
    flex-wrap: wrap;
  }

  /* Pushed to the trailing edge, on the checkbox's baseline. `margin-inline-
     start: auto` rather than a spacer element: the bar wraps, and a spacer
     would wrap with it and leave the button under the dropdown. */
  .clear-all {
    margin-inline-start: auto;
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    background: none;
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .clear-all:hover {
    background: var(--surface-sunken);
    color: var(--text-primary);
  }

  .select-all {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-small-size);
  }

  .select-all input {
    width: 16px;
    height: 16px;
    accent-color: var(--emphasis);
  }

  .selected-count {
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
  }

  .file-list :global(.row + .row),
  .file-list :global(.details + .row),
  .file-list :global(.flag + .row) {
    border-top: 0.5px solid var(--hairline);
  }

  .action-bar {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--surface-raised);
    box-shadow: var(--shadow-card);
  }

  .ready {
    margin-right: auto;
    font-size: var(--text-small-size);
    color: var(--text-secondary);
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
  }

  .key {
    opacity: 0.6;
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: var(--space-8);
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: var(--space-5);
    /* Lopsided ON PURPOSE, for the usual case: a line of text, then a pill
       button whose own padding supplies the right-hand margin. */
    padding: var(--space-3) var(--space-3) var(--space-3) var(--space-6);
    border-radius: var(--radius-full);
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    box-shadow: var(--shadow-float);
    font-size: var(--text-small-size);
    white-space: nowrap;
    z-index: 20;
  }

  /* A TOAST WITH NOTHING TO CLICK IS NOT LOPSIDED.

     "Outputs removed." arrives on its own -- no Undo, no Convert again -- and
     inherited padding built for a trailing button: 24 px of space on the left
     and 8 px on the right. The text was centred in the window and off-centre
     in the thing holding it, which is the version you actually see. */
  .toast:not(:has(button)) {
    padding-inline: var(--space-6);
    justify-content: center;
  }

  /* Same rule PlanTable uses: the heading still exists for assistive
     technology and for the focus target, while BatchSummary shows the counts
     visually. Two visible copies of the same sentence is the alternative. */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .undo {
    min-height: 28px;
    padding: 0 var(--space-5);
    border-radius: var(--radius-full);
    background: var(--surface-sunken);
    font-weight: var(--weight-medium);
  }

  .countdown {
    color: var(--text-secondary);
    font-weight: var(--weight-regular);
  }

  /* The way on to the next conversion. Sits beside Undo rather than in the
     results card, so the two things worth doing after a batch are together. */
  .again {
    min-height: 28px;
    padding: 0 var(--space-5);
    border-radius: var(--radius-full);
    background: var(--text-primary);
    color: var(--surface-base);
    font-weight: var(--weight-medium);
  }

  .again:hover {
    opacity: 0.88;
  }

  /* What a drop is about to do, on a screen that already has files. */
  .drop-hint {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-sunken);
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    text-align: center;
  }

  @media (prefers-reduced-motion: reduce) {
    .toast {
      translate: 0 0;
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .toast {
      background: var(--surface-raised);
      backdrop-filter: none;
    }
  }
</style>
