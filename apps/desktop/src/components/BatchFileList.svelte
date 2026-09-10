<script lang="ts">
  import { errorText, revealOutput, type ConversionResult } from "../lib/ipc";
  import Icon from "./Icon.svelte";
  import OutputReview from "./OutputReview.svelte";
  import { job } from "../lib/stores/job.svelte";
  let { results, onretry }: { results: ConversionResult[]; onretry?: () => void | Promise<void> } = $props();
  let reviewing = $state<ConversionResult | null>(null), failedOnly = $state(false), retrying = $state(false), error = $state("");
  function baseName(path: string) { return path.split(/[\\/]/).pop() ?? path; }
  const original = $derived.by(() => { const matches = reviewing ? job.lastRequests.flatMap(b => b.paths).filter(path => baseName(path) === reviewing?.fileName) : []; return matches.length === 1 ? matches[0] : undefined; });
  async function retry() { retrying = true; try { await onretry?.(); } finally { retrying = false; } }
  async function showOutput(result: ConversionResult) { try { await revealOutput(result.outputPath); } catch(e) { error=errorText(e); } }
</script>
{#if results.some(r => !r.success)}<div class="result-controls"><label><input type="checkbox" bind:checked={failedOnly}/> Failed only</label>{#if onretry}<button type="button" disabled={retrying} onclick={() => void retry()}>{retrying ? "Retrying…" : "Retry failed"}</button>{/if}</div>{/if}
{#if reviewing}<OutputReview path={reviewing.outputPath} {original} onclose={() => reviewing = null}/>{/if}
{#if error}<p role="alert">{error}</p>{/if}
<ul class="files" aria-label="Files in this batch">{#each results as result,i (i)}{#if !failedOnly || !result.success}<li class:failed={!result.success}>
<span class="name">{result.fileName}</span>
{#if result.success}<div class="actions"><button type="button" aria-label={`Preview ${baseName(result.outputPath)}`} onclick={() => reviewing=result}><Icon name="eye" size={15}/></button><button type="button" aria-label={`Show ${baseName(result.outputPath)} in folder`} onclick={() => void showOutput(result)}><Icon name="folder-open" size={15}/></button></div>{:else}<span class="why">{result.errorMessage ?? "Did not convert"}</span>{/if}
</li>{/if}{/each}</ul>
<style>
.result-controls{display:flex;align-items:center;justify-content:space-between;gap:12px}.result-controls label{display:flex;align-items:center;gap:8px}.files{list-style:none;margin:0;padding:0;display:grid;gap:4px}.files li{display:flex;align-items:center;gap:8px;min-height:var(--row-h);padding:0 12px;font-size:var(--text-small-size);border-radius:var(--radius-md)}.name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.actions{display:flex;gap:4px}.actions button{display:grid;place-items:center;width:28px;height:28px;padding:0;border:1px solid var(--hairline);border-radius:var(--radius-full);color:var(--text-secondary)}.why,p{color:var(--accent-critical);overflow-wrap:anywhere}
</style>
