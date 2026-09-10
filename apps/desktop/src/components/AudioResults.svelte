<script lang="ts">
  import { onMount } from "svelte";
  import { readTextOutput, errorText, type ConversionResult } from "../lib/ipc";
  import WavePlayer from "./WavePlayer.svelte";
  import Icon from "./Icon.svelte";
  let { results, paths, durations = {}, embedded = false, onclose }: { results: ConversionResult[]; paths: string[]; durations?: Record<string,string>; embedded?: boolean; onclose: () => void } = $props();
  let expanded = $state(new Set<string>()), text = $state<Record<string,string>>({}), error = $state("");
  let dialog = $state<HTMLDivElement | null>(null);
  function source(result:ConversionResult){return paths[results.indexOf(result)] ?? result.outputPath;}
  const isText=(path:string)=>/\.(txt|srt|vtt)$/i.test(path);
  async function load(path:string){if(text[path]!==undefined)return text[path]; const value=await readTextOutput(path);text={...text,[path]:value};return value;}
  async function toggle(path:string){const next=new Set(expanded);if(next.has(path))next.delete(path);else next.add(path);expanded=next;try{await load(path);}catch(e){error=errorText(e);}}
  async function copy(path:string){try{await navigator.clipboard.writeText(await load(path));}catch(e){error=errorText(e);}}
  onMount(()=>{const previous=document.activeElement as HTMLElement|null;dialog?.querySelector<HTMLButtonElement>('button')?.focus();return()=>previous?.focus();});
  function key(e:KeyboardEvent){if(e.key==='Escape'){e.preventDefault();e.stopPropagation();onclose();}if(e.key==='Tab'){const nodes=Array.from(dialog?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')??[]);if(e.shiftKey&&document.activeElement===nodes[0]){e.preventDefault();nodes.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===nodes.at(-1)){e.preventDefault();nodes[0]?.focus();}}}
  onMount(() => { window.addEventListener("keydown",key,true); return () => window.removeEventListener("keydown",key,true); });
</script>

<div class="backdrop" class:embedded><div class="review" bind:this={dialog} role="dialog" aria-modal={!embedded} aria-label="Audio results" tabindex="-1">
<header><strong>Recordings</strong><button type="button" onclick={onclose}>Close</button></header>
{#if error}<p role="alert">{error}</p>{/if}
<div class="recordings">
{#each results.filter(r=>r.success) as result (result.outputPath)}
  <article><div class="recording-head">
    {#if isText(result.outputPath)}<button type="button" class="disclosure" aria-expanded={expanded.has(result.outputPath)} aria-label={`Show text for ${result.fileName}`} onclick={()=>void toggle(result.outputPath)}><span aria-hidden="true">{expanded.has(result.outputPath)?"▾":"▸"}</span><span class="name">{result.fileName}</span></button>{:else}<span class="name">{result.fileName}</span>{/if}
    <span class="duration">{durations[source(result)] ?? ""}</span>
    {#if isText(result.outputPath)}<button class="copy" type="button" aria-label={`Copy text for ${result.fileName}`} onclick={()=>void copy(result.outputPath)}><Icon name="copy" size={16}/></button>{/if}
  </div>
  {#if isText(result.outputPath)}{#if expanded.has(result.outputPath)}<div class="text">{text[result.outputPath] ?? "Loading…"}</div>{/if}
  {:else}<div class="playback"><WavePlayer path={result.outputPath}/></div>{/if}
  </article>
{/each}
</div></div></div>
<style>
.backdrop{position:fixed;inset:0;z-index:110;background:#0008;display:grid;place-items:center;padding:24px}.review{width:min(850px,100%);max-height:100%;box-sizing:border-box;display:flex;flex-direction:column;gap:16px;padding:20px;background:var(--surface-base);border:1px solid var(--hairline-strong);border-radius:var(--radius-lg)}header,.recording-head{display:flex;align-items:center;gap:10px}header{justify-content:space-between}.recordings{overflow:auto;display:grid;align-content:start;flex:0 1 auto;gap:10px}article{background:var(--surface-sunken);border:1px solid var(--hairline);border-radius:var(--radius-md);overflow:hidden}.recording-head{padding:12px}.name{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.disclosure{display:flex;justify-content:flex-start;align-items:center;gap:8px;border:0;padding:0;min-width:0;flex:1;text-align:left;background:transparent}.duration{margin-left:auto;white-space:nowrap;color:var(--text-secondary);font-size:var(--text-caption-size)}button{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--hairline);border-radius:var(--radius-sm);padding:6px 10px;background:transparent;color:var(--text-primary)}.copy{width:28px;height:28px;padding:0;flex:none}.text{padding:0 12px 12px;white-space:pre-wrap;overflow-wrap:anywhere;font-size:var(--text-small-size);line-height:1.5}.playback{padding:0 12px 12px}
.embedded{grid-template-columns:minmax(0,1fr);grid-template-rows:minmax(0,1fr);position:absolute;inset:0;z-index:10;padding:0;background:var(--surface-base);border-radius:var(--radius-lg)}.embedded .review{min-width:0;width:100%;height:100%;max-height:100%}header button{border-radius:var(--radius-full);min-height:30px;padding:0 12px}</style>
