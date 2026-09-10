<script lang="ts">
let {oncapture}:{oncapture:(png:string)=>void}=$props();
let canvas=$state<HTMLCanvasElement|null>(null),drawing=false,hasInk=$state(false),error=$state("");
function point(e:PointerEvent){const r=canvas!.getBoundingClientRect();return {x:(e.clientX-r.left)*canvas!.width/r.width,y:(e.clientY-r.top)*canvas!.height/r.height};}
function start(e:PointerEvent){if(!canvas)return;drawing=true;canvas.setPointerCapture(e.pointerId);const p=point(e),c=canvas.getContext("2d")!;c.beginPath();c.moveTo(p.x,p.y);c.strokeStyle="#17171a";c.lineWidth=4;c.lineCap="round";}
function move(e:PointerEvent){if(!drawing||!canvas)return;const p=point(e),c=canvas.getContext("2d")!;c.lineTo(p.x,p.y);c.stroke();hasInk=true;}
function clear(){canvas?.getContext("2d")?.clearRect(0,0,800,240);hasInk=false;}
async function paste(e:ClipboardEvent){const file=Array.from(e.clipboardData?.files??[]).find(f=>f.type.startsWith("image/"));if(!file)return;e.preventDefault();if(file.size>8*1024*1024){error="That image is too large.";return;}try {const bitmap=await createImageBitmap(file);const c=canvas?.getContext("2d");if(c&&canvas){clear();const scale=Math.min(canvas.width/bitmap.width,canvas.height/bitmap.height);c.drawImage(bitmap,0,0,bitmap.width*scale,bitmap.height*scale);hasInk=true;}bitmap.close();error="";}catch(e){error="That image could not be pasted.";}}
</script>
<div class="capture" onpaste={paste} role="region" aria-label="Draw or paste signature">
<p>Draw here, or paste an image.</p>
<canvas bind:this={canvas} width="800" height="240" aria-label="Signature drawing area" onpointerdown={start} onpointermove={move} onpointerup={()=>drawing=false} onpointercancel={()=>drawing=false}></canvas>
<div><button type="button" onclick={clear}>Clear</button><button type="button" disabled={!hasInk} onclick={()=>canvas&&oncapture(canvas.toDataURL("image/png"))}>Use signature</button></div>
{#if error}<p role="alert">{error}</p>{/if}
</div>
<style>.capture{display:grid;gap:8px}.capture p{margin:0;font-size:var(--text-small-size)}canvas{width:100%;height:auto;background:white;border:1px solid var(--hairline-strong);border-radius:var(--radius-sm);touch-action:none}button{display:inline-flex;align-items:center;justify-content:center;min-height:28px;padding:0 12px;border:0.5px solid var(--hairline);border-radius:var(--radius-full);background:transparent;color:var(--text-secondary);font-size:var(--text-caption-size)}.capture>div{display:flex;gap:8px;justify-content:flex-end}</style>
