<script lang="ts">
import {saveImageCorrections,errorText} from "../lib/ipc";
let {path,original,cutout,source}:{path:string;original:string;cutout:string;source:string}=$props();
type Stroke={restore:boolean;radius:number;points:[number,number][]};
let strokes=$state<Stroke[]>([]),future=$state<Stroke[]>([]),restore=$state(false),radius=$state(.025),background=$state("checker"),canvas=$state<HTMLCanvasElement|null>(null),error=$state(""),saved=$state(""),busy=$state(false);
let originalImage:HTMLImageElement,cutoutImage:HTMLImageElement,drawing=false;
$effect(()=>{const a=new Image(),b=new Image();let stale=false;a.onload=()=>{if(!stale){originalImage=a;render();}};b.onload=()=>{if(!stale){cutoutImage=b;render();}};a.src=source;b.src=cutout;return()=>{stale=true;};});
function render(){if(!canvas||!cutoutImage||!originalImage)return;canvas.width=cutoutImage.naturalWidth;canvas.height=cutoutImage.naturalHeight;const c=canvas.getContext("2d")!;c.clearRect(0,0,canvas.width,canvas.height);c.drawImage(cutoutImage,0,0);for(const stroke of strokes){c.save();c.beginPath();const r=stroke.radius*Math.min(canvas.width,canvas.height);for(let i=0;i<stroke.points.length;i++){const p=stroke.points[i],q=stroke.points[Math.max(0,i-1)];const dx=(p[0]-q[0])*canvas.width,dy=(p[1]-q[1])*canvas.height,n=Math.max(1,Math.ceil(Math.hypot(dx,dy)/(r*.5)));for(let j=0;j<=n;j++){const x=q[0]*canvas.width+dx*j/n,y=q[1]*canvas.height+dy*j/n;c.moveTo(x+r,y);c.arc(x,y,r,0,Math.PI*2);}}if(stroke.restore){c.clip();c.clearRect(0,0,canvas.width,canvas.height);c.drawImage(originalImage,0,0,canvas.width,canvas.height);}else{c.globalCompositeOperation="destination-out";c.fill();}c.restore();}}
function point(e:PointerEvent):[number,number]{const r=canvas!.getBoundingClientRect();return [Math.max(0,Math.min(1,(e.clientX-r.left)/r.width)),Math.max(0,Math.min(1,(e.clientY-r.top)/r.height))];}
function start(e:PointerEvent){if(busy||strokes.length>=500||!canvas)return;drawing=true;canvas.setPointerCapture(e.pointerId);future=[];strokes=[...strokes,{restore,radius,points:[point(e)]}];saved="";render();}
function move(e:PointerEvent){if(!drawing)return;const last=strokes.at(-1)!;if(last.points.length>=2000)return;last.points.push(point(e));render();}
function undo(){const last=strokes.at(-1);if(last){future=[...future,last];strokes=strokes.slice(0,-1);render();}}
function redo(){const last=future.at(-1);if(last){strokes=[...strokes,last];future=future.slice(0,-1);render();}}
async function save(){busy=true;error="";try{saved=await saveImageCorrections(path,original,JSON.stringify(strokes));}catch(e){error=errorText(e);}finally{busy=false;}}
</script>
<div class="mask-editor">
<div class="controls"><button type="button" aria-pressed={!restore} onclick={()=>restore=false}>Erase</button><button type="button" aria-pressed={restore} onclick={()=>restore=true}>Restore</button><label>Brush size <input type="range" min="0.005" max="0.15" step="0.005" bind:value={radius}/></label><button type="button" disabled={!strokes.length||busy} onclick={undo}>Undo</button><button type="button" disabled={!future.length||busy} onclick={redo}>Redo</button></div>
<div class="controls" aria-label="Preview background">{#each ["checker","light","dark"] as value}<button type="button" aria-pressed={background===value} onclick={()=>background=value}>{value}</button>{/each}</div>
<div class="canvas" class:checker={background==="checker"} style:background-color={background==="dark"?"#202024":"#fff"}><canvas bind:this={canvas} aria-label="Erase or restore parts of the cutout" onpointerdown={start} onpointermove={move} onpointerup={()=>drawing=false} onpointercancel={()=>drawing=false}></canvas></div>
<button type="button" disabled={!strokes.length||busy} onclick={save}>{busy?"Saving…":"Save corrected copy"}</button>
{#if error}<p role="alert">{error}</p>{/if}{#if saved}<p role="status">Saved {saved}</p>{/if}
</div>
<style>.mask-editor{display:grid;gap:10px;padding:12px;min-width:0}.controls{display:flex;gap:8px;align-items:center;flex-wrap:wrap}.controls label{display:flex;align-items:center;gap:8px}button{padding:6px 12px;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);background:var(--surface-sunken);color:var(--text-primary)}button[aria-pressed=true]{border-color:var(--text-primary)}canvas{display:block;width:100%;height:auto;touch-action:none;cursor:crosshair}.checker{background-image:conic-gradient(#ddd 25%,transparent 0 50%,#ddd 0 75%,transparent 0);background-size:24px 24px}p{overflow-wrap:anywhere}</style>
