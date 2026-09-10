<script lang="ts">
  import PdfViewer from "./PdfViewer.svelte";
  import AudioReview from "./AudioReview.svelte";
  import TextEditor from "./TextEditor.svelte";
  import MaskEditor from "./MaskEditor.svelte";
  import { onMount } from "svelte";
  import { previewFile, readTextOutput, errorText, type FilePreview } from "../lib/ipc";
  let { path, original, embedded = false, onclose }: { path: string; original?: string; embedded?: boolean; onclose: () => void } = $props();
  let page = $state(1), zoom = $state(1), split = $state(50);
  let refine = $state(false);
  let output = $state<FilePreview | null>(null), before = $state<FilePreview | null>(null);
  let text = $state<string | null>(null), error = $state<string | null>(null), loading = $state(false);
  let closeButton = $state<HTMLButtonElement | null>(null);
  const isPdf = $derived(/\.pdf$/i.test(path));
  const isAudio = $derived(/\.(wav|flac|mp3|ogg|opus|aac|m4a)$/i.test(path));
  const isText = $derived(/\.(txt|md|srt|vtt)$/i.test(path));
  $effect(() => {
    const currentPage = page;
    let stale = false;
    loading = true; error = null;
    (async () => {
      try {
        if (isAudio || isPdf) return;
        if (isText) { const value = await readTextOutput(path); if (!stale) text = value; }
        else {
          const value = await previewFile(path,currentPage,1800,[]);
          if (!stale) output = value;
          if (original) { const value = await previewFile(original,currentPage,1800,[]); if (!stale) before = value; }
        }
      } catch (e) { if (!stale) error = errorText(e); }
      finally { if (!stale) loading = false; }
    })();
    return () => { stale = true; };
  });
  onMount(() => { const previous = document.activeElement as HTMLElement | null; closeButton?.focus(); return () => previous?.focus(); });
  function key(event: KeyboardEvent) {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onclose(); }
    if (event.key === "Tab") {
      const nodes = Array.from(closeButton?.closest(".review")?.querySelectorAll<HTMLElement>('button:not(:disabled),input,textarea') ?? []);
      if (event.shiftKey && document.activeElement === nodes[0]) { event.preventDefault(); nodes.at(-1)?.focus(); }
      else if (!event.shiftKey && document.activeElement === nodes.at(-1)) { event.preventDefault(); nodes[0]?.focus(); }
    }
  }
  onMount(() => { window.addEventListener("keydown",key,true); return () => window.removeEventListener("keydown",key,true); });
</script>

<div class="backdrop" class:embedded>
<div class="review" tabindex="-1" role="dialog" aria-modal={!embedded} aria-label="File preview">
  <header><strong>Preview</strong><button bind:this={closeButton} type="button" onclick={onclose}>Close</button></header>
  {#if error}<p role="alert">{error}</p>{/if}
  <div class="viewer" class:pdf={isPdf}>
    {#if isPdf}<PdfViewer {path}/>{:else if refine && output && before && original}<MaskEditor {path} {original} cutout={output.dataUri} source={before.dataUri}/>{:else if isAudio}<AudioReview {path} {original}/>{:else if text !== null}<TextEditor {path} {text} audio={original && /\.(wav|flac|mp3|ogg|opus|aac|m4a|mp4|mkv)$/i.test(original) ? original : undefined}/>{:else if output}
      <div class="image" style:width={`${zoom*100}%`} style:aspect-ratio={`${output.width}/${output.height}`}>
        <img src={output.dataUri} alt="Saved output" />
        {#if before}<img class="original" style:clip-path={`inset(0 ${100-split}% 0 0)`} src={before.dataUri} alt="Original" />{/if}
      </div>
    {/if}
  </div>
  {#if !isPdf}<footer>
    <span role="status">{loading ? "Loading preview…" : before ? "Original left · Saved output right" : "Saved output"}</span>
    {#if before && output && /\.png$/i.test(path)}<button type="button" aria-pressed={refine} onclick={()=>refine=!refine}>{refine?"Compare":"Refine transparency"}</button>{/if}
    {#if before}<input type="range" min="0" max="100" bind:value={split} aria-label="Original and output comparison" />{/if}
    {#if output}<button type="button" disabled={zoom<=1} onclick={()=>zoom=Math.max(1,zoom-.5)} aria-label="Zoom out">−</button><span>{Math.round(zoom*100)}%</span><button type="button" disabled={zoom>=4} onclick={()=>zoom=Math.min(4,zoom+.5)} aria-label="Zoom in">+</button>{/if}
    {#if output && output.pageCount>1}<button type="button" disabled={page<=1 || loading} onclick={()=>page--}>Previous</button><span>{page} / {output.pageCount}</span><button type="button" disabled={page>=output.pageCount || loading} onclick={()=>page++}>Next</button>{/if}
  </footer>{/if}
</div>
</div>
<style>
.backdrop { position: fixed; inset: 0; z-index: 100; background: #0008; padding: 24px; display: grid; place-items: center; }
.review { width: min(1100px,100%); height: 100%; min-height: 0; display: flex; flex-direction: column; background: var(--surface-base); border: 1px solid var(--hairline-strong); border-radius: var(--radius-lg); padding: 16px; gap: 12px; }
header,footer { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; } header { justify-content: space-between; }
button { border-radius: var(--radius-full); min-height:30px; padding: 0 12px; border: 1px solid var(--hairline-strong); background: var(--surface-sunken); color: var(--text-primary); }
.embedded{grid-template-columns:minmax(0,1fr);grid-template-rows:minmax(0,1fr);position:absolute;inset:0;z-index:10;padding:0;background:var(--surface-base);border-radius:var(--radius-lg)}.embedded .review{width:100%;min-width:0;box-sizing:border-box}.viewer { min-width:0; overflow: auto; flex: 1; min-height: 0; background: var(--surface-sunken); }
.viewer.pdf{overflow:hidden}
.image { position: relative; min-width: 100%; } img { display: block; width: 100%; height: 100%; object-fit: contain; } .original { position: absolute; inset: 0; }

footer { font-size: var(--text-small-size); } input { flex: 1; min-width: 80px; }
</style>
