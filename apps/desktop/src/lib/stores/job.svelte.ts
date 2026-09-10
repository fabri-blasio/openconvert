/**
 * A running batch.
 *
 * Progress is **reported by the backend**, one `conversion-progress` event per
 * file, never simulated here. A progress bar driven by `setInterval` is a lie
 * told by the one screen whose job is to say what is happening — it can show
 * "3 of 40" while the third file has already failed.
 */

import {
  cancelAll,
  convertBatch,
  type Choice,
  runTool,
  onProgress,
  errorText,
  type ConversionResult,
  type ProgressEvent,
} from "../ipc";
import type { UnlistenFn } from "@tauri-apps/api/event";

export type Phase = "idle" | "running" | "done";

/** One file's place in the batch, as last reported. */
export interface FileProgress {
  index: number;
  fileName: string;
  phase: ProgressEvent["phase"];
}

export interface JobRequest { paths: string[]; target: string; params?: Record<string, number | string | boolean>; choices?: Choice[]; }

class JobStore {
  phase = $state<Phase>("idle");
  /** In the order the backend reported them. */
  progress = $state<FileProgress[]>([]);
  results = $state<ConversionResult[]>([]);
  error = $state<string | null>(null);
  cancelling = $state(false);

  total = $state(0);
  /**
   * Wall time for the whole batch, in milliseconds.
   *
   * Measured here rather than summed from the per-file `durationMs`: the
   * difference between the two is the time spent between files, and a summary
   * that hides it would claim the batch was faster than the user watched it
   * be.
   */
  durationMs = $state(0);
  #startedAt = 0;
  #batchOffset = 0;
  lastRequests: JobRequest[] = [];
  #outcomes: ConversionResult[][] = [];

  readonly completed = $derived(
    this.progress.filter((p) => p.phase !== "started").length,
  );
  readonly succeeded = $derived(this.results.filter((r) => r.success).length);
  readonly failed = $derived(this.results.filter((r) => !r.success).length);
  readonly running = $derived(this.phase === "running");

  #unlisten: UnlistenFn | null = null;

  reset() {
    this.phase = "idle";
    this.progress = [];
    this.results = [];
    this.error = null;
    this.cancelling = false;
    this.total = 0;
    this.durationMs = 0;
    this.#startedAt = 0;
    this.#batchOffset = 0;
  }

  /**
   * Convert every group, in order.
   *
   * Sequential rather than concurrent: each `convert_batch` call owns a worker
   * pool, and two pools racing for the same limits would make the receipts
   * describe a machine neither of them had to itself.
   *
   * A batch with `params` is a tool run instead: same pipeline, `run_tool`
   * underneath.
   */
  async run(
    batches: {
      paths: string[];
      target: string;
      params?: Record<string, number | string | boolean>;
      choices?: Choice[];
    }[],
  ): Promise<ConversionResult[]> {
    this.reset();
    this.lastRequests = JSON.parse(JSON.stringify(batches));
    this.#outcomes = [];
    this.phase = "running";
    this.total = batches.reduce((n, b) => n + b.paths.length, 0);
    this.#startedAt = performance.now();

    this.#unlisten = await onProgress((e) => this.#record(e));

    const all: ConversionResult[] = [];
    try {
      for (const batch of batches) {
        if (this.cancelling) break;
        const out =
          batch.params === undefined
            ? await convertBatch(batch.paths, batch.target, batch.choices)
            : await runTool(batch.target, batch.paths, batch.params);
        this.#outcomes.push(out);
        all.push(...out);
        this.#batchOffset += batch.paths.length;
      }
      this.results = all;
    } catch (e) {
      // The backend returns one `Err` per file wherever it can; a top-level
      // error means the batch itself did not start. Shown verbatim.
      this.error = errorText(e);
      this.results = all;
    } finally {
      await this.#stopListening();
      this.durationMs = Math.round(performance.now() - this.#startedAt);
      this.phase = "done";
      this.cancelling = false;
    }
    return this.results;
  }

  async retryFailed(): Promise<ConversionResult[]> {
    if (this.running) return [];
    const previous = this.results;
    const requests = this.lastRequests.flatMap((batch, i) => {
      const outcomes = this.#outcomes[i] ?? [];
      if (outcomes.length !== batch.paths.length) return outcomes.every(r => r.success) && outcomes.length ? [] : [batch];
      const failed = batch.paths.map((path, index) => ({path,index})).filter(({index}) => !outcomes[index].success);
      return failed.length ? [{...batch, paths: failed.map(f => f.path), choices: batch.choices?.filter((_, index) => failed.some(f => f.index === index))}] : [];
    });
    if (!requests.length) return [];
    const fresh = await this.run(requests);
    this.results = [...previous.filter(r => r.success), ...fresh];
    return fresh;
  }

  /** Ask the backend to stop after the file currently running. */
  async cancel() {
    if (this.phase !== "running" || this.cancelling) return;
    this.cancelling = true;
    try {
      await cancelAll();
    } catch (e) {
      this.error = errorText(e);
      this.cancelling = false;
    }
  }

  /**
   * Run one tool through the same pipeline as any conversion.
   *
   * Progress, cancel, results, receipts and undo are shared with
   * `run` on purpose — tools are not a second-class path.
   */
  async runOneTool(
    id: string,
    paths: string[],
    params: Record<string, number | string | boolean>,
  ): Promise<ConversionResult[]> {
    return this.run([{ paths, target: id, params }]);
  }

  #record(e: ProgressEvent) {
    const index = this.#batchOffset + e.index;
    const existing = this.progress.findIndex((p) => p.index === index);
    if (existing >= 0) {
      this.progress[existing].phase = e.phase;
    } else {
      this.progress.push({ index, fileName: e.fileName, phase: e.phase });
    }
  }

  async #stopListening() {
    if (this.#unlisten) {
      this.#unlisten();
      this.#unlisten = null;
    }
  }
}

export const job = new JobStore();
