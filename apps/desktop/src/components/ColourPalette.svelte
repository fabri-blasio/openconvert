<script lang="ts">
import {untrack} from "svelte";
let {colour}:{colour:{hex:string;rgb:string}|null}=$props();
let recent=$state<{hex:string;rgb:string}[]>([]),error=$state(""),copied=$state("");
let copyToken=0;
$effect(()=>{if(colour)recent=untrack(()=>[colour!,...recent.filter(c=>c.hex!==colour!.hex)].slice(0,8));});
async function copy(value:string){try{await navigator.clipboard.writeText(value);error="";copied=value;const token=++copyToken;setTimeout(()=>{if(token===copyToken)copied="";},1800);}catch{error="Could not copy colour.";}}
</script>
{#if recent.length}<div class="palette"><span>Recent colours</span><div class="swatches">{#each recent as c}<button type="button" style:background={c.hex} aria-label={`Copy ${c.hex}`} title={copied===c.hex ? "Copied" : c.hex} onclick={()=>void copy(c.hex)}>{copied===c.hex ? "✓" : ""}</button>{/each}</div>{#if error}<span role="alert">{error}</span>{/if}</div>{/if}
<style>.palette{display:grid;gap:8px;font-size:var(--text-small-size);min-width:0}.swatches{display:flex;gap:6px;overflow:hidden;flex-wrap:nowrap}.swatches button{color:black;text-shadow:0 0 3px white;flex:0 1 26px;min-width:18px;height:26px;padding:0;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm)}</style>
