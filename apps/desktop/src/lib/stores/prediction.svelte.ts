/**
 * What was dropped, what it is, and what would happen to it.
 *
 * **This store holds no decisions.** Every value in it arrived from a Tauri
 * command; the only thing tracked locally is which suggestion the user has
 * pointed at, because that is a UI selection and not a fact about a file.
 */

import {
  getPlan,
  probeFile,
  suggestTargets,
  errorText,
  type PlanPreview,
  type Prediction,
  type ProbeResult,
  type Suggestion,
  type Choice,
} from "../ipc";

/**
 * One dropped file, with the targets offered for it and the one chosen.
 *
 * The **file** is the unit the interface works in — a drop is a list of files,
 * and each row aims itself. Grouping stays the backend's business: it is what
 * decides which files share a set of candidate targets, and it is still what
 * batches the conversion, but it is no longer what the screen is made of.
 */
export interface FileEntry {
  probe: ProbeResult;
  /** Ranked most-likely-first, from the backend. */
  options: Suggestion[];
  /** Index into `options`. */
  selected: number;
  plan: PlanPreview | null;
  planning: boolean;
  /** Monotonic token that prevents an older async plan replacing a newer one. */
  planGeneration: number;
  /**
   * Whether the user has explicitly chosen this file's target.
   *
   * **Distinct from `options[selected].armed`, and that distinction was a
   * deadlock.** `armed` is a property of the PREDICTION -- score >= 0.85 -- and
   * nothing the user does changes it. `confirm()` refused to convert an unarmed
   * selection and opened the format picker instead, so choosing a format left
   * `armed` exactly as it was and the next click opened the picker again. Any
   * conversion whose top prediction scored below the threshold could never run:
   * Convert, picker, choose, Convert, picker, forever, with nothing on screen
   * saying why.
   *
   * A low-confidence prediction should make the app ASK. Answering is what
   * clears it, and this is where the answer is recorded.
   */
  confirmed: boolean;
  /** Selected for a bulk format change. Conversion still includes every row. */
  checked: boolean;
}

/** One group of same-format files, its chosen target, and its plan. */
export interface Group {
  prediction: Prediction;
  probes: ProbeResult[];
  /** Index into `prediction.suggestions`. */
  selected: number;
  plan: PlanPreview | null;
  /** Set while `get_plan` is in flight for this group. */
  planning: boolean;
}

class PredictionStore {
  groups = $state<Group[]>([]);
  /** The drop as a flat list, in the order the files arrived. */
  files = $state<FileEntry[]>([]);
  loading = $state(false);
  error = $state<string | null>(null);

  /**
   * Whether one shared format governs every row.
   *
   * OFF BY DEFAULT. "Select all" used to tick rows for a separate Apply
   * button, which meant the ticks and the dropdown could disagree until
   * someone pressed it. Governing is the whole intention, so it is the whole
   * control: on, every row takes the shared target and its own menu is
   * disabled; off, each row decides for itself.
   */
  governed = $state(false);

  /** At least one file would actually run. */
  readonly ready = $derived(this.files.some((f) => f.plan?.executable === true));

  readonly totalFiles = $derived(this.files.length);

  readonly selectedCount = $derived(this.files.filter((f) => f.checked).length);

  readonly allSelected = $derived(
    this.files.length > 0 && this.files.every((f) => f.checked),
  );

  /** Formats every selected row can reach, in the first row's ranked order. */
  readonly bulkTargets = $derived.by(() => {
    // Governed means "all of them", so the intersection is over every row
    // rather than over a tick state the user no longer manages by hand.
    const selected = this.governed ? this.files : this.files.filter((f) => f.checked);
    if (selected.length === 0) return [];
    return selected[0].options
      .map((option) => option.target)
      .filter((target) =>
        selected.every((file) => file.options.some((option) => option.target === target)),
      );
  });

  /** How many would run, for the action bar's count. */
  readonly runnable = $derived(this.files.filter((f) => f.plan?.executable === true).length);

  /** True when any file's plan carries a blocking warning. */
  readonly blocked = $derived(
    this.files.some((f) => f.plan?.warnings.some((w) => w.blocking) === true),
  );

  reset() {
    this.groups = [];
    this.files = [];
    this.error = null;
    this.loading = false;
  }

  /**
   * Take a drop: probe every file, group it, and plan the top suggestion.
   *
   * Grouping is the backend's — a mixed drop is several decisions, not one,
   * and which files belong together is a fact about their content.
   *
   * **Replaces whatever was loaded.** Adding to a list is {@link add}; this is
   * the fresh start, and the two are separate because getting it wrong in
   * either direction destroys work the user did.
   */
  async accept(paths: string[]) {
    await this.take(paths, "replace");
  }

  /**
   * Add files to what is already loaded.
   *
   * **Choosing a second file used to throw the first one away.** Every path
   * into the app — the drop zone, Ctrl+O, the Browse button — called
   * `accept()`, which assigns `this.files`, so dropping one more file onto a
   * batch of forty silently replaced all forty. There was no warning and no
   * undo: the list simply became one row, and the only tell was that it used
   * to be longer.
   *
   * A file already in the list is not added twice. Dropping the same folder
   * again is a thing people do to check it arrived, and it should not double
   * every row.
   */
  async add(paths: string[]) {
    const known = new Set(this.files.map((f) => f.probe.path));
    const fresh = paths.filter((p) => !known.has(p));
    if (fresh.length === 0) return;
    await this.take(fresh, "append");
  }

  /** Probe, group and plan `paths`, either replacing the list or extending it. */
  private async take(paths: string[], mode: "replace" | "append") {
    this.loading = true;
    this.error = null;
    // What to fall back to if this throws. Replacing empties the list, as it
    // always did; appending must leave what was already there ALONE — a failed
    // second drop that wiped the first one would be the same bug wearing a
    // different hat.
    const before = mode === "append" ? [...this.files] : [];
    const beforeGroups = mode === "append" ? [...this.groups] : [];
    try {
      const [predictions, probes] = await Promise.all([
        suggestTargets(paths),
        Promise.all(paths.map((p) => probeFile(p))),
      ]);

      const byPath = new Map(probes.map((p) => [p.path, p]));
      const groups = predictions.map((prediction) => ({
        prediction,
        probes: prediction.paths
          .map((p) => byPath.get(p))
          .filter((p): p is ProbeResult => p !== undefined),
        selected: 0,
        plan: null,
        planning: false,
        confirmed: false,
      }));

      // Flatten to rows, in the order the files were given rather than the
      // order the backend grouped them: the list should match the drop.
      const optionsFor = new Map<string, Suggestion[]>();
      for (const p of predictions) {
        for (const path of p.paths) optionsFor.set(path, p.suggestions);
      }
      const rows: FileEntry[] = paths
        .map((path) => byPath.get(path))
        .filter((probe): probe is ProbeResult => probe !== undefined)
        .map((probe) => ({
          probe,
          options: optionsFor.get(probe.path) ?? [],
          selected: 0,
          plan: null,
          planning: false,
          planGeneration: 0,
          confirmed: false,
          checked: true,
        }));

      const from = mode === "append" ? this.files.length : 0;
      this.groups = mode === "append" ? [...this.groups, ...groups] : groups;
      this.files = mode === "append" ? [...this.files, ...rows] : rows;

      // Plan only what just arrived. Re-planning the whole list on every added
      // file makes the twentieth drop twenty times the work of the first, and
      // would also discard formats the user had already chosen by hand.
      await Promise.all(rows.map((_, i) => this.planFile(from + i)));
      // Anything that did not come back with a plan gets one. See `settle`.
      await this.settle();
    } catch (e) {
      this.error = errorText(e);
      this.groups = beforeGroups;
      this.files = before;
    } finally {
      this.loading = false;
    }
  }

  /** Aim one row at a different format, and re-plan just that file. */
  async selectFile(fileIndex: number, optionIndex: number) {
    const file = this.files[fileIndex];
    if (!file || optionIndex < 0 || optionIndex >= file.options.length) return;
    file.selected = optionIndex;
    // Choosing IS the confirmation. Without this the picker can be answered
    // and the answer changes nothing.
    file.confirmed = true;
    file.plan = null;
    await this.planFile(fileIndex);
    await this.settle();
  }

  /** Drop one file from the list. Nothing on disk is touched. */
  remove(fileIndex: number) {
    if (fileIndex < 0 || fileIndex >= this.files.length) return;
    this.files.splice(fileIndex, 1);
  }

  /** Select one row for the shared format control. */
  setChecked(fileIndex: number, checked: boolean) {
    const file = this.files[fileIndex];
    if (file) file.checked = checked;
  }

  /** Select or clear the whole upload list. */
  selectAll(checked: boolean) {
    for (const file of this.files) file.checked = checked;
  }

  /** Turn shared-format mode on or off. */
  setGoverned(on: boolean) {
    this.governed = on;
    // Governing acts on everything, so the row ticks follow rather than being
    // a second, separate selection the user has to keep in step.
    for (const file of this.files) file.checked = on;
  }

  /**
   * The shared menu's options, in the shape `FormatPicker` renders.
   *
   * Only targets EVERY row can reach: offering one that half the batch cannot
   * do would produce a bulk change that silently skips files.
   */
  readonly bulkOptions = $derived.by(() => {
    if (this.files.length === 0) return [];
    const shared = this.bulkTargets;
    const first = this.files[0]?.options ?? [];
    return shared
      .map((t) => first.find((o) => o.target === t))
      .filter((o): o is Suggestion => o !== undefined);
  });

  /** Apply one shared target to every selected row that offers it. */
  async applyBulkTarget(target: string) {
    const replans: Promise<void>[] = [];
    this.files.forEach((file, index) => {
      if (!this.governed && !file.checked) return;
      // Already there: re-planning an unchanged row on every keystroke of the
      // governing effect would re-run the backend for nothing.
      if (file.options[file.selected]?.target === target) return;
      const option = file.options.findIndex((candidate) => candidate.target === target);
      if (option < 0) return;
      file.selected = option;
      file.confirmed = true;
      file.plan = null;
      replans.push(this.planFile(index));
    });
    await Promise.all(replans);
    await this.settle();
  }

  /** The format one row is aimed at, if any. */
  fileTarget(fileIndex: number): string | null {
    const file = this.files[fileIndex];
    return file?.options[file.selected]?.target ?? null;
  }

  /**
   * The runnable files, gathered by target so one command converts each set.
   *
   * Rows choose individually, so two files of the same type may be going to
   * different formats; batching by the chosen target is what keeps a single
   * `convert_batch` call correct for each.
   *
   * Each batch also carries what was SHOWN for its files -- the top-ranked
   * target, its score, and where the chosen one sat in the ranking. That is
   * the only record of whether a suggestion was right, and this store is the
   * only place that knows it: by the time the backend journals the result, the
   * ranking it would recompute could already differ from the one on screen.
   */
  batches(): { paths: string[]; target: string; choices: Choice[] }[] {
    const byTarget = new Map<string, { paths: string[]; choices: Choice[] }>();
    this.files.forEach((f, i) => {
      if (f.plan?.executable !== true) return;
      const target = this.fileTarget(i);
      if (target === null) return;
      const group = byTarget.get(target) ?? { paths: [], choices: [] };
      group.paths.push(f.probe.path);
      // `options` is ranked, so the index IS the rank. A target absent from
      // the ranking scores -1, which is the strongest negative signal there
      // is and the one a plain right/wrong count would miss.
      const top = f.options[0];
      if (top !== undefined) {
        group.choices.push({
          path: f.probe.path,
          suggested: top.target,
          suggestedScore: top.score,
          chosenRank: f.options.findIndex((o) => o.target === target),
        });
      }
      byTarget.set(target, group);
    });
    return [...byTarget].map(([target, g]) => ({
      target,
      paths: g.paths,
      choices: g.choices,
    }));
  }

  /**
   * Fetch the plan for one row.
   *
   * **The early return used to leave `planning` set.** A row with no target
   * was marked as being planned forever, and since `ready` is
   * `some(plan?.executable)`, Convert stayed disabled with nothing on screen
   * explaining why. The reported workaround -- click a format and it comes
   * back to life -- is this function being reached a second time with a target
   * in hand.
   */
  private async planFile(fileIndex: number) {
    const file = this.files[fileIndex];
    if (!file) return;
    const target = this.fileTarget(fileIndex);
    if (target === null) {
      file.planning = false;
      return;
    }
    const generation = ++file.planGeneration;
    file.planning = true;
    try {
      const plan = await getPlan([file.probe.path], target);
      if (this.files[fileIndex] === file && file.planGeneration === generation) {
        file.plan = plan;
      }
    } catch (e) {
      if (this.files[fileIndex] === file && file.planGeneration === generation) {
        this.error = errorText(e);
        file.plan = null;
      }
    } finally {
      if (this.files[fileIndex] === file && file.planGeneration === generation) {
        file.planning = false;
      }
    }
  }

  /**
   * Plan every row that has a target and no plan.
   *
   * **A convergence guarantee, not a retry loop.** "Convert is sometimes
   * stuck until you click a format again" is a report about a row that ended
   * up with a target and no plan, and there is more than one way to get there:
   * a `get_plan` that threw, a drop that appended while an earlier pass was in
   * flight, a row whose options arrived after its first planning attempt. Each
   * of those is worth fixing on its own and none of them is worth trusting to
   * stay fixed, because the failure is silent -- a disabled button.
   *
   * So the invariant is stated directly: a row with a target and no plan is a
   * row that gets planned. It runs once per settled state and does nothing at
   * all when every row already has one.
   */
  async settle() {
    const pending = this.files
      .map((file, index) => ({ file, index }))
      .filter(({ file, index }) => file.plan === null && this.fileTarget(index) !== null);
    if (pending.length === 0) return;
    await Promise.all(pending.map(({ index }) => this.planFile(index)));
  }

  /** Point a group at a different suggestion, and re-plan it. */
  async select(groupIndex: number, suggestionIndex: number) {
    const group = this.groups[groupIndex];
    if (!group || suggestionIndex < 0) return;
    if (suggestionIndex >= group.prediction.suggestions.length) return;
    group.selected = suggestionIndex;
    group.plan = null;
    await this.plan(groupIndex);
  }

  /** The format name a group is currently aimed at, if any. */
  target(groupIndex: number): string | null {
    const group = this.groups[groupIndex];
    return group?.prediction.suggestions[group.selected]?.target ?? null;
  }

  private async plan(groupIndex: number) {
    const group = this.groups[groupIndex];
    const target = this.target(groupIndex);
    if (!group || target === null) return;
    group.planning = true;
    try {
      group.plan = await getPlan(group.prediction.paths, target);
    } catch (e) {
      this.error = errorText(e);
      group.plan = null;
    } finally {
      group.planning = false;
    }
  }
}

export const prediction = new PredictionStore();
