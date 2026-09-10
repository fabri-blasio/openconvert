<script lang="ts">
import {previewAudio,errorText} from "../lib/ipc";
let {path,original,seek}:{path:string;original?:string;seek?:number}=$props();
let loaded=$state(true);
let current=$state("output"),src=$state(""),error=$state(""),loading=$state(false),player=$state<HTMLAudioElement|null>(null);
const cache=new Map<string,string>();
let position=0,playing=false;
$effect(()=>{if(!loaded)return;const file=current==="original"&&original?original:path;let stale=false;src="";error="";loading=true;(async()=>{try{let data=cache.get(file);if(!data){data=await previewAudio(file);cache.set(file,data);}if(!stale)src=data;}catch(e){if(!stale)error=errorText(e);}finally{if(!stale)loading=false;}})();return()=>{stale=true;};});
$effect(()=>{const t=seek;if(t!==undefined&&Number.isFinite(t)){position=t;playing=true;loaded=true;if(player){player.currentTime=t;void player.play().catch(()=>{});}}});
function switchTo(value:string){if(player){position=player.currentTime;playing=!player.paused;player.pause();}current=value;}
function ready(){if(player){player.currentTime=Math.min(position,player.duration||position);if(playing)void player.play().catch(()=>{});}}
</script>
<div class="audio-review">
{#if original}<div class="switch"><button type="button" aria-pressed={current==="original"} onclick={()=>switchTo("original")}>Original</button><button type="button" aria-pressed={current==="output"} onclick={()=>switchTo("output")}>Processed</button></div>{/if}
{#if loading}<p role="status">Preparing playback…</p>{:else if error}<p role="alert">{error}</p>{:else if src}<audio bind:this={player} controls {src} onloadedmetadata={ready} aria-label="Recording playback"></audio>{:else}<button type="button" onclick={()=>loaded=true}>Load playback</button>{/if}
</div>
<style>.audio-review{display:grid;gap:8px;padding:12px}.switch{display:flex;gap:8px}audio{width:100%}button{flex:1;padding:6px 12px;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);background:var(--surface-sunken);color:var(--text-primary)}button[aria-pressed=true]{background:var(--surface-raised);border-color:var(--text-secondary)}</style>
