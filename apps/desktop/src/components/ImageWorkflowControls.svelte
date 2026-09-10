<script lang="ts">
import type { Snippet } from "svelte";
const LABELS: Record<string,string> = {"image-upscale":"Upscale","image-invert":"Invert colours","image-greyscale":"Black and white","image-remove-background":"Remove background"};
let {ops,format=$bindable("png"),selected,disabled=false,onchange,onselect,children}: {ops:string[];format?:string;selected:string;disabled?:boolean;onchange:(ops:string[])=>void;onselect:(id:string)=>void;children?:Snippet}=$props();
let dragging=$state<string|null>(null);
function move(id:string,offset:number){const next=[...ops],from=next.indexOf(id),to=from+offset;if(from<0||to<0||to>=next.length)return;next.splice(from,1);next.splice(to,0,id);onchange(next);}
function drop(id:string){if(!dragging||dragging===id)return;const next=ops.filter(v=>v!==dragging);next.splice(next.indexOf(id),0,dragging);dragging=null;onchange(next);}
</script>
<div class="workflow">
<label>Add an edit<select aria-label="Add an edit" {disabled} value="" onchange={e=>{const id=e.currentTarget.value;if(id){onchange([...ops,id]);onselect(id);}e.currentTarget.value="";}}><option value="">Choose an edit…</option>{#each Object.entries(LABELS).filter(([id])=>!ops.includes(id)) as [id,label]}<option value={id}>{label}</option>{/each}</select></label>
<hr/>
<ol aria-label="Edits in execution order">{#each ops as id (id)}
<li draggable={!disabled} ondragstart={()=>dragging=id} ondragend={()=>dragging=null} ondragover={e=>e.preventDefault()} ondrop={e=>{e.preventDefault();if(!disabled)drop(id);}}>
<button type="button" {disabled} class:active={selected===id} aria-expanded={selected===id} onclick={()=>onselect(id)}>{ops.indexOf(id)+1}. {LABELS[id]}</button>
{#if selected===id}<div class="edit-settings">{@render children?.()}<div class="actions"><button type="button" disabled={disabled||ops.indexOf(id)===0} aria-label={`Move ${LABELS[id]} up`} onclick={()=>move(id,-1)}>↑</button><button type="button" disabled={disabled||ops.indexOf(id)===ops.length-1} aria-label={`Move ${LABELS[id]} down`} onclick={()=>move(id,1)}>↓</button></div><button type="button" {disabled} onclick={()=>onchange(ops.filter(v=>v!==id))}>Remove edit</button></div>{/if}
</li>{/each}</ol>
<hr/>
<label>Output format<select bind:value={format} {disabled}><option value="png">PNG</option><option value="jpeg">JPEG</option><option value="webp">WebP</option><option value="avif">AVIF</option></select></label>
</div>
<style>
.workflow{display:grid;gap:12px;padding:12px;min-width:0;font-size:var(--text-small-size)}ol{padding:0;margin:0;list-style:none;display:grid;gap:8px}li{min-width:0}label,.edit-settings{display:grid;gap:8px}.edit-settings{padding:8px 0}button,select{display:flex;align-items:center;box-sizing:border-box;min-width:0;width:100%;min-height:36px;padding:8px 12px;color:var(--text-primary);background:var(--surface-sunken);border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);font:inherit}button{justify-content:center}.active{background:var(--surface-raised)}.edit-settings :global(.tool-settings){padding:0}.actions{display:grid;gap:8px}hr{border:0;border-top:1px solid var(--hairline);width:100%;margin:0}button:disabled{opacity:.45}
</style>
