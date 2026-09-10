<script lang="ts">
  import { untrack } from "svelte";
  import { previewAudio, errorText, audioPeaks } from "../lib/ipc";
  import WaveForm from "./WaveForm.svelte";
  let { path, peaks = [] }: { path: string; peaks?: number[] } = $props();
  let player = $state<HTMLAudioElement | null>(null), src = $state(""), loading = $state(false), playing = $state(false), error = $state("");
  let measured = $state<number[]>([]), position = $state(0), duration = $state(0);
  $effect(() => { const file = path; let stale = false; src = ""; position = 0; playing = false; measured = []; duration = 0; if (!untrack(() => peaks.length)) void audioPeaks(file).then(p => { if (!stale) measured = p; }).catch(e => { if (!stale) error = errorText(e); }); return () => { stale = true; player?.pause(); }; });
  async function toggle() {
    if (loading) return;
    if (playing) { player?.pause(); return; }
    error = "";
    if (!src) { loading = true; const file = path; try { const data = await previewAudio(file); if (path === file) src = data; } catch (e) { error = errorText(e); } finally { loading = false; } }
    else await player?.play().catch(e => error = errorText(e));
  }
  function seek(event: MouseEvent) { if (!player || !duration) return; const r = event.currentTarget as HTMLElement; const box = r.getBoundingClientRect(); player.currentTime = Math.max(0, Math.min(duration, (event.clientX-box.left)/box.width*duration)); }
</script>
<div class="wave-player">
  <button type="button" class="wave" aria-label="Seek recording" onclick={seek} style:--position={`${duration ? position/duration*100 : 0}%`}><WaveForm peaks={peaks.length ? peaks : measured} progress={duration ? position/duration : 0} label="Audio waveform"/></button>
  <button type="button" class="play" aria-label={loading ? "Loading audio" : playing ? "Pause recording" : "Play recording"} disabled={loading} onclick={() => void toggle()}>{#if playing}<span aria-hidden="true">Ⅱ</span>{:else}<span aria-hidden="true">▶</span>{/if}</button>
  {#if src}<audio bind:this={player} {src} onloadedmetadata={() => { if(player) { duration=player.duration; void player.play().catch(e=>error=errorText(e)); } }} ontimeupdate={() => position=player?.currentTime ?? 0} onplay={() => playing=true} onpause={() => playing=false} onended={() => playing=false}></audio>{/if}
</div>
{#if error}<p role="alert">{error}</p>{/if}
<style>
.wave-player{--wave-height:40px;display:flex;align-items:center;gap:12px;width:100%;min-width:0}.wave{flex:1;min-width:0;padding:0;border:0;border-radius:4px;background:linear-gradient(to right,var(--fill-quaternary) var(--position),transparent var(--position))}.play{display:grid;place-items:center;flex:none;width:26px;height:26px;border:1px solid var(--hairline);border-radius:var(--radius-full);color:var(--text-secondary);background:transparent}.play span{line-height:1;font-size:13px}p{font-size:var(--text-caption-size);color:var(--accent-critical)}
</style>
