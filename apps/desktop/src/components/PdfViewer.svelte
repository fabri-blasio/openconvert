<script lang="ts">
  import { tick } from "svelte";
  import { previewFile, errorText, type FilePreview } from "../lib/ipc";
  let { path, from = 1, to, operations = (_page: number): string[] => [] }: { path: string; from?: number; to?: number; operations?: (page: number) => string[] } = $props();
  let viewport = $state<HTMLDivElement | null>(null), count = $state(0), zoom = $state(1), width = $state(600);
  let ratios = $state<Record<number,string>>({});
  let shots = $state<Record<number,FilePreview>>({}), error = $state("");
  let generation = 0, timer: ReturnType<typeof setTimeout> | undefined;
  const visible = new Set<number>();
  const cache = new Map<string,FilePreview>();
  let pending = false, queued = false;
  function requestRefine() { clearTimeout(timer); timer = setTimeout(() => void refine(),160); }
  async function refine() {
    if (pending) { queued=true; return; }
    pending = true;
    const token = generation, source = path;
    try {
      for (const page of [...visible]) {
        if (token !== generation) break;
        const px = Math.min(3200, Math.max(800,Math.ceil(width*zoom*devicePixelRatio/400)*400));
        if ((shots[page]?.width ?? 0) >= px || (shots[page]?.height ?? 0) >= px) continue;
        const key = `${page}:${px}`;
        const value = cache.get(key) ?? await previewFile(source,page,px,operations(page));
        if (token !== generation) break;
        cache.set(key,value); while(cache.size>16)cache.delete(cache.keys().next().value!);
        shots={...shots,[page]:value};ratios={...ratios,[page]:`${value.width}/${value.height}`};
        if(Object.keys(shots).length>20)shots=Object.fromEntries(Object.entries(shots).filter(([key])=>Number(key)===from || visible.has(Number(key))));
      }
    } catch(e) { if(token===generation)error=errorText(e); }
    finally { pending=false; if(queued){queued=false;requestRefine();} }
  }
  function observe(node:HTMLElement,page:number) {
    const observer=new IntersectionObserver(entries=>{for(const entry of entries){if(entry.isIntersecting)visible.add(page);else visible.delete(page);}requestRefine();},{root:viewport,rootMargin:"250px 0px"});
    observer.observe(node);return {destroy(){observer.disconnect();visible.delete(page);}};
  }
  $effect(()=>{
    const source=path,start=from,end=to,token=++generation;shots={};ratios={};count=0;error="";cache.clear();visible.clear();
    void previewFile(source,start,800,operations(start)).then(value=>{if(token!==generation)return;shots={[start]:value};ratios={[start]:`${value.width}/${value.height}`};count=Math.max(1,Math.min(end ?? value.pageCount,value.pageCount)-start+1);visible.add(start);requestRefine();}).catch(e=>{if(token===generation)error=errorText(e);});
    return()=>{generation++;clearTimeout(timer);};
  });
  async function changeZoom(next:number){
    if(!viewport)return;
    const scroller=viewport,ratio=next/zoom,top=scroller.scrollTop,left=scroller.scrollLeft;
    const edge=scroller.getBoundingClientRect().top;
    const anchor=Array.from(scroller.querySelectorAll<HTMLElement>("figure")).find(node=>node.getBoundingClientRect().bottom>edge);
    const rect=anchor?.getBoundingClientRect(),fraction=rect ? (edge-rect.top)/rect.height : 0;
    zoom=next;await tick();
    // Anchor the same point on the visible page; multiplying scrollTop would also
    // scale the fixed gaps between earlier pages and drift in long documents.
    if(anchor){const nextRect=anchor.getBoundingClientRect();scroller.scrollTop+=nextRect.top+fraction*nextRect.height-edge;}
    else scroller.scrollTop=top*ratio;
    scroller.scrollLeft=left*ratio;requestRefine();
  }
</script>
<div class="pdf-viewer">
  <div class="toolbar"><button type="button" aria-label="Zoom out" disabled={zoom<=.5} onclick={()=>void changeZoom(Math.max(.5,zoom-.25))}>−</button><span>{Math.round(zoom*100)}%</span><button type="button" aria-label="Zoom in" disabled={zoom>=4} onclick={()=>void changeZoom(Math.min(4,zoom+.25))}>+</button><button type="button" onclick={()=>void changeZoom(1)}>Fit</button><span class="count">{count} pages</span></div>
  {#if error}<p role="alert">{error}</p>{/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scroll region needs keyboard focus for arrow and Page Down navigation.) -->
  <div class="scroll" tabindex="0" role="region" aria-label="PDF pages" bind:this={viewport} bind:clientWidth={width}>
    {#if !count && !error}<p role="status">Loading preview…</p>{/if}
    <div class="pages" style:width={`${Math.max(1,width-24)*zoom}px`}>
    {#each Array.from({length:count},(_,i)=>i+from) as page (page)}
      {@const shot=shots[page]}
      <figure use:observe={page} style:aspect-ratio={ratios[page] ?? ratios[from] ?? ".707"}>
        {#if shot}<img src={shot.dataUri} alt={`Page ${page}`} draggable="false"/>{:else}<span>Page {page}</span>{/if}
      </figure>
    {/each}
    </div>
  </div>
</div>
<style>
.pdf-viewer{min-width:0;width:100%;display:flex;flex-direction:column;min-height:0;height:100%;gap:8px}.toolbar{display:flex;align-items:center;gap:8px;flex:none;font-size:var(--text-small-size)}button{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--hairline-strong);background:var(--surface-base);color:var(--text-primary);border-radius:var(--radius-full);min-height:30px;padding:0 12px}.count{margin-left:auto}.scroll{min-width:0;width:100%;overflow:auto;min-height:0;flex:1;background:var(--surface-sunken);border-radius:var(--radius-md);overscroll-behavior:contain}.pages{margin:0 auto;padding:12px 0;display:flex;flex-direction:column;gap:12px;min-height:100%}figure{margin:0;position:relative;background:white;box-shadow:var(--shadow-card);display:grid;place-items:center;flex:none}img{width:100%;height:100%;display:block}p{font-size:var(--text-small-size)}
</style>
