<script lang="ts">
import {saveTextCopy,errorText} from "../lib/ipc";
import AudioReview from "./AudioReview.svelte";
let {path,text,audio}:{path:string;text:string;audio?:string}=$props();
let draft=$state(""),saved=$state(""),error=$state(""),busy=$state(false),seek=$state<number|undefined>(undefined);
$effect(()=>{draft=text;saved="";error="";});
const timed=$derived(/\d{2}:\d{2}:\d{2}[.,]\d{3}\s+-->/.test(draft));
const cues=$derived(draft.split(/\r?\n\r?\n/).flatMap(block=>{const match=block.match(/(\d{2}):(\d{2}):(\d{2})[.,](\d{3})\s+-->/);return match?[{at:Number(match[1])*3600+Number(match[2])*60+Number(match[3])+Number(match[4])/1000,label:match[0].replace(/\s+-->$/,"")}]:[];}));
function plain(){return draft.replace(/^WEBVTT[^\n]*\n/,'').split(/\r?\n\r?\n/).map(b=>b.replace(/^\d+\r?\n/,'').replace(/^.*-->.*\r?\n/m,'').trim()).filter(Boolean).join('\n\n');}
async function save(extension:string){busy=true;error="";try{let content=draft;if(extension==="txt"&&timed)content=plain();if(extension!=="txt"){const blocks=draft.replace(/^WEBVTT[^\n]*\n/,'').trim().split(/\r?\n\r?\n/).map(b=>b.replace(/^\d+\r?\n/,'')).filter(b=>b.includes('-->'));content=extension==="vtt"?'WEBVTT\n\n'+blocks.map(b=>b.replace(/(\d{2}:\d{2}:\d{2}),(\d{3})/g,'$1.$2')).join('\n\n'):blocks.map((b,i)=>`${i+1}\n${b.replace(/(\d{2}:\d{2}:\d{2})\.(\d{3})/g,'$1,$2')}`).join('\n\n');}saved=await saveTextCopy(path,content,extension);}catch(e){error=errorText(e);}finally{busy=false;}}
</script>
<div class="text-editor">
{#if audio}<AudioReview path={audio} {seek}/>{/if}
{#if audio&&cues.length}<div class="cues" aria-label="Transcript playback positions">{#each cues as cue}<button type="button" onclick={()=>seek=cue.at}>Play {cue.label}</button>{/each}</div>{/if}
<textarea bind:value={draft} aria-label="Edit extracted text" rows="10" spellcheck="true"></textarea>
<div class="actions"><button type="button" disabled={busy||!draft.trim()} onclick={()=>save("txt")}>Save text copy</button>{#if timed}<button type="button" disabled={busy} onclick={()=>save("srt")}>Export SRT</button><button type="button" disabled={busy} onclick={()=>save("vtt")}>Export VTT</button>{/if}</div>
{#if saved}<p role="status">Saved {saved}</p>{/if}{#if error}<p role="alert">{error}</p>{/if}
</div>
<style>.text-editor{display:grid;gap:10px;min-width:0}textarea{box-sizing:border-box;width:100%;resize:vertical;padding:12px;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);background:var(--surface-base);color:var(--text-primary);font:inherit;white-space:pre-wrap}.actions,.cues{display:flex;gap:8px;flex-wrap:wrap}.cues{max-height:110px;overflow:auto}button{padding:6px 12px;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);background:var(--surface-sunken);color:var(--text-primary)}p{overflow-wrap:anywhere;font-size:var(--text-small-size)}</style>
