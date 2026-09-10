/**
 * Stand-in for `@tauri-apps/api/core` in the browser preview. **Not shipped.**
 *
 * Swapped in by Vite alias in `vite.preview.config.ts`, so `src/lib/ipc.ts`
 * imports this instead of the real bridge and every component runs unmodified.
 */

import { emit } from "./tauri-event";
import * as fx from "./fixtures";
import { photo, page, applyOps } from "./render";

const latency = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Which target each group is currently aimed at, so results match the plan. */
let lastTarget = "jpeg";
let cancelled = false;
let config = { ...fx.CONFIG };
let history = [...fx.HISTORY];
let signatures = [...fx.SIGNATURES];
const editedText = new Map<string,string>();

/**
 * A transparent PNG with a name written across it, as a stand-in for an
 * uploaded signature.
 *
 * Drawn rather than shipped: it has real alpha, so the placement ghost and the
 * checkerboard behind it are exercised the way a real transparent PNG would
 * exercise them, and it can never be mistaken for anyone's actual signature.
 */
function drawnSignature(name: string): string {
  const c = document.createElement("canvas");
  c.width = 480;
  c.height = 160;
  const ctx = c.getContext("2d");
  if (!ctx) return "";
  ctx.strokeStyle = "#1b3a6b";
  ctx.lineWidth = 5;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(30, 110);
  ctx.bezierCurveTo(120, 20, 170, 150, 250, 80);
  ctx.bezierCurveTo(320, 20, 360, 130, 450, 60);
  ctx.stroke();
  ctx.fillStyle = "#1b3a6b";
  ctx.font = "italic 26px Georgia, serif";
  ctx.fillText(name.slice(0, 24), 34, 148);
  return c.toDataURL("image/png");
}
let receipts = [...fx.RECEIPTS];
let models = fx.MODELS.map((m) => ({ ...m }));
let aiFeatures = fx.AI_FEATURES.map((f) => ({ ...f }));

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const a = args ?? {};

  switch (cmd) {
    case "pick_files": {
      await latency(150);
      // The real dialog filters by kind; the harness answers with paths of
      // that kind so each workspace has something plausible to work on.
      const want = a.kind as string | null;
      if (want === "pdf") return fx.PDF_PATHS as unknown as T;
      if (want === "audio") return fx.AUDIO_PATHS as unknown as T;
      if (want === "image") return fx.PATHS.slice(0, 1) as unknown as T;
      return fx.PATHS as unknown as T;
    }

    case "probe_file":
      await latency(60);
      return fx.PROBES[a.path as string] as unknown as T;

    case "suggest_targets":
      await latency(220);
      return fx.PREDICTIONS as unknown as T;

    // The real command walks dropped folders host-side. In the browser there
    // are no folders to walk, so a drop passes through unchanged — which is
    // exactly what the real one does for a drop of plain files.
    case "expand_paths":
      await latency(40);
      return {
        files: a.paths as string[],
        folders: 0,
        skipped: 0,
        truncated: false,
      } as unknown as T;

    // No save dialog in a browser. `null` is the real command's answer when
    // the dialog is dismissed, so the UI path exercised here is a real one.
    // No save dialog in a browser, so nothing is written and the UI takes the
    // same "dismissed" path a real cancel produces.
    case "save_text_file":
      await latency(200);
      return null as unknown as T;

    case "export_batch_log":
      await latency(120);
      return null as unknown as T;

    case "get_plan": {
      await latency(180);
      lastTarget = a.targetFormat as string;
      return fx.planFor(a.paths as string[], lastTarget) as unknown as T;
    }

    case "convert_batch": {
      // Emit real progress events so ProgressList shows what it would show.
      cancelled = false;
      const paths = a.paths as string[];
      const target = a.targetFormat as string;
      const out = [];
      for (let i = 0; i < paths.length; i++) {
        const fileName = paths[i].slice(paths[i].lastIndexOf("\\") + 1);
        if (cancelled) {
          emit("conversion-progress", { index: i, total: paths.length, fileName, phase: "cancelled" });
          continue;
        }
        emit("conversion-progress", { index: i, total: paths.length, fileName, phase: "started" });
        await latency(700);
        emit("conversion-progress", { index: i, total: paths.length, fileName, phase: "done" });
        out.push(fx.resultFor(paths[i], target));
      }
      return out as unknown as T;
    }

    case "run_tool": {
      // An empty array is "nothing selected", same as absent: the workspace
      // shows a sample file when opened without a drop, so run against it.
      const given = (a.paths as string[] | undefined) ?? [];
      const paths = given.length > 0 ? given : fx.PATHS.slice(0, 1);
      const id=String(a.id), params=(a.params??{}) as Record<string,string>;
      const target=id==="pdf-text"?"txt":id.startsWith("pdf-")?"pdf":id==="audio-transcribe"||id==="image-ocr"?(params.format??"txt"):id.startsWith("audio-")?"wav":params.format??"png";

      const out = [];
      for (let i = 0; i < paths.length; i++) {
        const fileName = paths[i].slice(paths[i].lastIndexOf("\\") + 1);
        emit("conversion-progress", { index: i, total: paths.length, fileName, phase: "started" });
        for (let step=1; step<=6; step++) { await latency(100); emit("tool-progress", {index:i,total:paths.length,fraction:step/6}); }
        emit("conversion-progress", { index: i, total: paths.length, fileName, phase: "done" });
        out.push(fx.resultFor(paths[i], target));
      }
      return out as unknown as T;
    }

    case "cancel_all":
    case "panic_stop":
      cancelled = true;
      return undefined as T;

    case "undo_last":
      await latency(120);
      return undefined as T;

    case "get_config":
      await latency(80);
      return config as unknown as T;

    case "set_config":
      config = { ...config, ...(a.patch as object) };
      return config as unknown as T;

    case "list_history":
      await latency(140);
      return history as unknown as T;

    case "list_signatures":
      await latency(80);
      return signatures as unknown as T;

    case "add_signature": {
      await latency(200);
      // The harness has no filesystem, so the "upload" produces a drawn mark
      // rather than reading `a.source`. It is deliberately obviously drawn:
      // a fixture that looked like a real scanned signature would be the one
      // thing on this screen indistinguishable from the user's own.
      const made = {
        name: String(a.name).trim() || "untitled",
        path: `/signatures/${String(a.name).trim() || "untitled"}.png`,
        dataUri: drawnSignature(String(a.name)),
      };
      signatures = [...signatures.filter((s) => s.name !== made.name), made].sort((x, y) =>
        x.name.localeCompare(y.name),
      );
      return made as unknown as T;
    }

    case "delete_signature": {
      const had = signatures.length;
      signatures = signatures.filter((s) => s.name !== a.name);
      return (signatures.length !== had) as unknown as T;
    }

    case "save_text_copy": {
      const path = `${String(a.path).replace(/\.[^.]+$/, "")}-edited.${a.extension}`;
      editedText.set(path, String(a.text));
      return path as unknown as T;
    }
    case "save_image_corrections": return `${String(a.path).replace(/\.[^.]+$/, "")}-corrected.png` as unknown as T;
    case "preview_audio": {
      // A clearly synthetic two-second tone for exercising the playback UI.
      const n=32000, buffer=new ArrayBuffer(44+n*2), view=new DataView(buffer);
      const label=(offset:number,value:string)=>{for(let i=0;i<value.length;i++)view.setUint8(offset+i,value.charCodeAt(i));};
      label(0,"RIFF");view.setUint32(4,36+n*2,true);label(8,"WAVEfmt ");view.setUint32(16,16,true);view.setUint16(20,1,true);view.setUint16(22,1,true);view.setUint32(24,16000,true);view.setUint32(28,32000,true);view.setUint16(32,2,true);view.setUint16(34,16,true);label(36,"data");view.setUint32(40,n*2,true);
      for(let i=0;i<n;i++)view.setInt16(44+i*2,Math.sin(i*2*Math.PI*440/16000)*1200,true);
      let binary="";for(const byte of new Uint8Array(buffer))binary+=String.fromCharCode(byte);
      return `data:audio/wav;base64,${btoa(binary)}` as unknown as T;
    }
    case "read_text_output": {
      if(editedText.has(String(a.path))) return editedText.get(String(a.path)) as unknown as T;
      if(String(a.path).endsWith(".vtt")) return "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n[preview] Synthetic transcript for testing the editor." as unknown as T;
      await latency(120);
      // The harness writes no files, so it answers with a short, obviously
      // synthetic line. It is NOT plausible speech: a page of realistic
      // transcript in a preview is indistinguishable from a real result, which
      // is the one thing this panel must never be.
      return ("[preview] This is the harness, not a transcription. " +
        "Run the real app to transcribe " +
        String(a.path).slice(String(a.path).lastIndexOf("\\") + 1) +
        ".") as unknown as T;
    }

    case "audio_peaks": {
      await latency(260);
      // The harness has no decoder and no real recording, so it MEASURES its
      // own stand-in rather than inventing a shape: the fixture waveform is
      // generated once, here, and every reader gets the same numbers. It is
      // still synthetic — the difference from what this replaced is that the
      // product no longer generates one, and this one is not labelled as
      // anything but the harness.
      const n = Number(a.buckets ?? 96);
      const out: number[] = [];
      for (let i = 0; i < n; i += 1) {
        const phrase = Math.abs(Math.sin((i / n) * Math.PI * 3.1));
        const swell = 0.55 + 0.45 * Math.sin((i / n) * Math.PI * 1.3);
        out.push(Math.round(Math.min(1, phrase * swell * 0.9 + 0.06) * 255));
      }
      return out as unknown as T;
    }

    case "reveal_receipt":
    case "reveal_output":
      // The harness has no file manager. Logging is the honest stand-in: it
      // proves the row wired up without pretending a window opened.
      console.info("[preview] reveal", a.path);
      return undefined as T;

    case "wipe_history": {
      const n = history.length;
      history = [];
      return n as unknown as T;
    }

    case "list_receipts":
      await latency(120);
      return receipts as unknown as T;

    // A real-looking card so the preview shows the row it will really show.
    case "gpu_device":
      return "NVIDIA GeForce RTX 4050 Laptop GPU" as unknown as T;

    case "receipts_size":
      return receipts.reduce((n, r) => n + r.outputBytes, 0) as unknown as T;

    case "receipts_count":
      return receipts.length as unknown as T;

    case "delete_receipt":
      receipts = receipts.filter((r) => r.id !== a.id);
      return undefined as T;

    case "delete_all_receipts": {
      const n = receipts.length;
      receipts = [];
      return n as unknown as T;
    }

    case "list_ai_features":
      await latency(90);
      return aiFeatures as unknown as T;

    case "download_ai_feature": {
      const f = aiFeatures.find((x) => x.id === a.id);
      if (f) {
        await latency(700);
        f.downloaded = true;
      }
      return undefined as unknown as T;
    }

    case "list_models":
      await latency(120);
      return models as unknown as T;

    case "download_model": {
      // Stream real progress events so the bar reports rather than spins.
      const model = models.find((m) => m.id === a.id);
      const total = model?.sizeBytes ?? 100_000_000;
      for (let received = 0; received < total; received += Math.ceil(total / 24)) {
        emit("model-download-progress", {
          id: a.id,
          receivedBytes: Math.min(received, total),
          totalBytes: total,
          phase: "downloading",
          message: null,
        });
        // Slow enough to actually watch; a real 465 MB fetch is slower still.
        await latency(260);
      }
      emit("model-download-progress", {
        id: a.id,
        receivedBytes: total,
        totalBytes: total,
        phase: "verifying",
        message: null,
      });
      await latency(500);
      models = models.map((m) => (m.id === a.id ? { ...m, downloaded: true } : m));
      return undefined as T;
    }

    case "set_model_tier": {
      // The real command writes the choice to `config.toml`, and the backend
      // then reports it back through `active` on each feature. Returning
      // undefined was enough while the only selector was in Settings, which
      // re-reads nothing; the tool's own tier selector DRAWS the selection
      // from `active`, so a no-op here rendered a segment with nothing ever
      // selected -- a control the harness could not show working.
      const tool = a.tool as string;
      const want = a.tier as string;
      aiFeatures = aiFeatures.map((f) =>
        f.tool === tool ? { ...f, active: f.tier === want } : f,
      );
      return undefined as T;
    }
    case "set_model_enabled":
      models = models.map((m) => (m.id === a.id ? { ...m, enabled: a.enabled as boolean } : m));
      return undefined as T;

    case "delete_model":
      models = models.map((m) =>
        m.id === a.id ? { ...m, downloaded: false, enabled: false } : m,
      );
      return undefined as T;

    case "preview_file": {
      // Real PNG bytes, so the component's <img> path is genuinely exercised.
      await latency(260);
      const path = (a.path as string) ?? "";
      const isPdf = /\.pdf$/i.test(path);
      const pageNo = (a.page as number) ?? 1;
      // `max_px` IS HONOURED. The harness ignored it and answered 760x1074 for
      // every request, which made it impossible to see here whether anything
      // re-rendered at a larger size -- and re-rendering rather than scaling a
      // bitmap is exactly what the signing column's magnification is for. A
      // fixture that discards the argument under test cannot fail.
      const maxPx = (a.maxPx as number | undefined) ?? (a.max_px as number | undefined) ?? 1024;
      const allOps = (a.ops as string[] | undefined) ?? [];
      // Synthetic size pairs only for exercising the browser's size labels.
      const quality = allOps.find(op => op.startsWith("compress:"))?.split(":")[1];
      const sizes = quality ? { originalBytes: 2800000, compressedBytes: Math.round(2800000 * (quality === "lossless" ? .9 : Number(quality)/110)) } : {};

      if (isPdf) {
        // A SINGLE-PAGE DOCUMENT IS A CASE, AND THE HARNESS HAD NONE.
        //
        // Every `.pdf` here reported six pages, so "the preview does not load
        // for a one-page document" could not be reproduced in the harness at
        // all -- the one shape of document most likely to be dropped on a
        // signing or merging flow was the one the fixtures could not produce.
        // A name containing `1p` is one page; everything else keeps six.
        const count = /1p/i.test(path) ? 1 : 6;
        // A4 proportions, scaled so the long edge is `maxPx` -- which is what
        // the real renderer does.
        const pdfH = maxPx;
        const pdfW = Math.round((maxPx * 760) / 1074);
        return {
          dataUri: await applyOps(page(pageNo, count, pdfW, pdfH), allOps),
          width: pdfW,
          height: pdfH,
          pageCount: count,
          page: pageNo,
          // A4 at 72 dpi. The desktop build returns null until oc-pdf reports
          // the MediaBox; the harness supplies it so the label can be seen.
          ...sizes,
          pageWidthPt: 595,
          pageHeightPt: 842,
        } as unknown as T;
      }
      // The render reflects the operations asked for, exactly as a real
      // renderer would: background removal returns the subject alone, and the
      // pixel ops run over the result.
      // THE ID IS `image-remove-background`. Matching on "rmbg" matched
      // nothing, so the harness quietly rendered the untouched photo for the
      // one operation whose whole point is that the photo changes -- the same
      // shape of bug as the real `downscale_png`, which dropped background
      // removal on the floor and previewed the original.
      const cutout = allOps.includes("image-remove-background");
      return {
        dataUri: await applyOps(photo(960, 660, !cutout), allOps),
        width: 960,
        height: 660,
        pageCount: 1,
        page: 1,
        ...sizes,
        pageWidthPt: null,
        pageHeightPt: null,
      } as unknown as T;
    }

    case "list_tools":
      await latency(100);
      return fx.TOOLS as unknown as T;

    default:
      throw new Error(`preview: no fixture for command "${cmd}"`);
  }
}
