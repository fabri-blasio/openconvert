# OpenConvert — ML Runbook

How to build the intent engine (L3) and conversion advisor (L4): concepts, process, cost, and decision gates.

Status: design record, **v0.3** · 2026-08-13
Part 6 of 9 — see [README](README.md). Architecture context: [03-ARCHITECTURE §14](03-ARCHITECTURE.md#14-prediction).

> **None of this is in v1.** The shipping prediction engine is five hand-weighted signals in one pure function, with no model of any kind. Everything below — a learned ranker, a bandit, the intent engine, the advisor — is deferred behind observable triggers ([03-ARCHITECTURE §15.3](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires)), and the first of those triggers is *having data*. This document is the runbook for when that day comes, not a plan for now.
>
> **Why a runbook for deferred work exists at all**, when [03 §2](03-ARCHITECTURE.md#2-what-earlier-revisions-cut-and-why-that-still-stands) deletes a module manifest on the grounds that "deferring the implementation but specifying the format is still speculative design." The distinction is what the document commits you to. A manifest *format* is an interface: writing it down fixes a contract that future code must honour, and every hour spent refining it is an hour spent designing for a consumer who does not exist. **This document commits to nothing** — it fixes no type, no file format and no interface, and nothing in the shipping product refers to it. It is a cost estimate and a set of decision gates for a piece of work someone will otherwise badly underestimate, and its single most valuable page is [§8](#8-decision-gates), which mostly says *stop, you probably do not need to train anything.* If it ever starts specifying an artefact the product must accept, it has become the module manifest and should be deleted the same way.
>
> **The learned ranker's trigger was also wrong until v0.3.** It was "website conversion data (~month 6)" — but the tool pages record only aggregated `(from → to)` pairs, deliberately, and you cannot train a *personal-history* ranker on data with no person in it. The trigger is now 10,000 opt-in local history exports or a design partner supplying a corpus, which is a thing that can actually happen.

---

## Contents

1. [Which layer needs what](#1-which-layer-needs-what)
2. [The adaptation spectrum](#2-the-adaptation-spectrum)
3. [How LoRA works](#3-how-lora-works)
4. [Behaviour vs facts](#4-behaviour-vs-facts--the-rule-that-shapes-everything)
5. [The pipeline](#5-the-pipeline)
6. [Cost](#6-cost)
7. [Hardware & tooling](#7-hardware--tooling)
8. [Decision gates](#8-decision-gates)
9. [Hyperparameters](#9-hyperparameters)
10. [Pitfalls](#10-pitfalls)

---

## 1. Which layer needs what

| Layer | Job | Technique | Trained? | Status |
|---|---|---|---|---|
| **L0 Signals** | Predict intent from a drop | **Five hand-weighted signals, `w(n)=n/(n+3)`** | No — and that is the point. See [03-ARCHITECTURE §14](03-ARCHITECTURE.md#14-prediction). | **This is v1.** |
| **L1 Ranker** | The same job, learned | LightGBM / LambdaRank | Yes — but not an LLM | Deferred: needs data |
| **L2 Bandit** | Personalize on-device | Thompson sampling | Online, local, never transmitted | Deferred: needs L1 |
| **L3 Intent engine** | Sentence → plan | Small LLM + grammar | **This runbook** |
| **L4 Advisor** | "Which option, and why?" | **Same model, second adapter** | **This runbook** |

L3 and L4 share one base model. One download, one training pipeline, two LoRA adapters swapped at runtime.

---

## 2. The adaptation spectrum

"Fine-tuning" sits in the middle of a range that runs from *no change at all* to *a new model*. Knowing where each technique sits prevents paying for the wrong one.

| Level | Technique | Weights changed | Persists | Cost |
|---|---|---|---|---|
| 0 | Prompting | No | No — every call | Free |
| 1 | Few-shot in context | No | No — tokens every call | Free but recurring |
| 2 | **RAG / retrieval** | No | No | Free + infrastructure |
| 3 | **Grammar-constrained decoding** | No — masks sampling so invalid output is impossible | At inference | Free |
| 4 | Soft prompts / prefix tuning | No — learns virtual embeddings | Yes | Cheap, largely superseded |
| **5** | **LoRA / adapters** | **Yes — a trained low-rank delta** | **Yes** | **$10–25** |
| 6 | Full fine-tuning | Yes — every parameter | Yes | 10–50× LoRA, catastrophic-forgetting risk |
| 7 | Continued pretraining | Yes, on raw domain text | Yes | Expensive |
| 8 | DPO / ORPO | Yes — trains on *comparisons* | Yes | ≈ LoRA |
| 9 | Distillation | Yes — a new smaller model mimicking a bigger one | Yes | Moderate |

Levels 0–3 change nothing about the model. **OpenConvert uses 2 and 3 always, 5 when measurement justifies it, and 9 only if size blocks adoption.**

---

## 3. How LoRA works

A transformer weight matrix **W** might be 4096×4096 ≈ **16.8M parameters**. Full fine-tuning updates all of them and stores optimizer state for all of them — which is why it needs enormous memory.

LoRA's bet: the *update* ΔW is **low-rank**. So learn two thin matrices instead:

```
     W  (4096 × 4096, FROZEN — no gradients)
     +
  (α/r) · B · A        A: 4096 × r      B: r × 4096      r = 16 or 32
```

At r=16 that's `2 × 4096 × 16 = 131,072` parameters — **0.8% of the original**. **B** initializes to zeros, so ΔW = 0 at step zero and the model starts as exactly the base model.

Because the base gets no gradients, a 4B model fine-tunes comfortably on a 24 GB card.

At inference, either:
- **Merge** — `W' = W + BA`, zero runtime overhead, or
- **Keep separate** — hot-swap adapters at runtime

**OpenConvert uses the second.** One base model, an intent-engine adapter and an advisor adapter, swapped per task.

### The training loop

1. Take `(input, desired output)`
2. Forward pass → predicted next-token probabilities
3. Cross-entropy against the desired tokens → loss
4. Backpropagate → gradients land on **A** and **B** only
5. AdamW step
6. Repeat ~200,000 times

The result is a permanent shift in the output distribution. Ship the adapter and every user gets the behaviour with no prompt.

---

## 4. Behaviour vs facts — the rule that shapes everything

> **Fine-tuning teaches *behaviour*. Retrieval provides *facts*.**

| Good at | Bad at |
|---|---|
| Output structure and format compliance | **Adding facts** — lossy, hallucination-prone, un-updatable |
| Style, register, verbosity | Raising raw reasoning ability |
| Task mappings (fuzzy context → structured constraints) | Anything that changes weekly |
| Making a **small** model excellent at **one** thing | Broad general capability |
| Refusals | |
| Shortening prompts — behaviour baked in means fewer tokens per call | |

This is why format compatibility lives in the **knowledge base**, not the weights. Fine-tune "AVIF has 93% browser support" into a model and it's frozen, unverifiable, and wrong in eight months. In a table it's a one-line edit.

### Division of labour for the advisor

| Concern | Handled by |
|---|---|
| Format facts, compatibility, patent status | **Retrieval** — the format and route tables |
| Valid output structure | **Grammar constraints** — not weights |
| Which conversion is optimal | **The planner** — deterministic |
| "for a client on old Word" → `{compatibility: broad}` | **Fine-tuning** ✓ |
| Explanation tone and length | **Fine-tuning** ✓ |
| Declining DRM removal without inventing a plan | **Fine-tuning** ✓ |

Interpret and narrate. Nothing else. That narrowness is why a ~1B model can do it.

---

## 5. The pipeline

### Stage 0 — Build the eval set *first*

A few hundred hand-written `(context → correct recommendation)` cases, versioned in the repo, **never trained on**. Include adversarial cases: ambiguous context, impossible requests, things that must be refused.

**Do this before generating a single training example.** Otherwise you cannot tell whether anything helped, and you will convince yourself it did.

### Stage 1 — Build the data generator *(the real work)*

Engineering, not ML. The trick that makes this cheap: **generate backwards from the route table.**

```
enumerate:  file properties × user context × candidate actions
                    ↓
compute the correct answer FROM RULES        ← correct by construction
                    ↓
teacher model writes only the LANGUAGE:
   (a) a natural utterance a user might type
   (b) the explanation for the chosen answer
                    ↓
              50k–200k pairs
```

**Zero user data required.** The teacher never decides anything — it phrases.

Deliberately include:
- **Refusal cases** — DRM removal, watermark removal. A model that has never seen these will invent a plan for them.
- **Clarification cases** — context too vague to act on
- **Register variety** — terse, verbose, misspelled, non-English, ambiguous

### Stage 2 — Baseline with no training at all

Prompt-engineer the base model with retrieval; measure against the eval set. **If it passes, stop and ship.** Grammar constraints already guarantee structural validity, so the model only needs to make reasonable *choices* — a much lower bar than open-ended generation.

### Stage 3 — LoRA SFT

Rank 16–32, 2–3 epochs, on Qwen3-4B. Re-measure. Most of the gain lands here — expect it to close consistency, refusals, and explanation register.

### Stage 4 — Distil to ~1B *(only if size blocks adoption)*

A 4B at Q4 is ~2.5 GB, which breaks the *"~500 MB AI stack, CPU-only"* claim. A 1B-class student lands near **600 MB**.

Now you genuinely distil: the fine-tuned 4B becomes the teacher for a 1B student. ⚠️ But try the cheaper thing first — **for narrow, grammar-constrained tasks, directly fine-tuning the 1B on the same synthetic data often matches true distillation** at a fraction of the complexity.

### Stage 5 — Quantize and re-evaluate

Q4_K_M for shipping — but **quantization hurts small models disproportionately.** A 4B at Q4 may be fine where a 1B at Q4 degrades visibly. Test Q8 and Q5_K_M; pick on measured quality, not size alone.

### Stage 5b — Check the inference footprint

Training hardware is a one-off; **inference hardware is every user**. Measure before committing to a size.

| Model | Size (Q4) | RAM ⚠️ | CPU speed ⚠️ | Time to answer* |
|---|---|---|---|---|
| Qwen3-0.6B | ~400 MB | ~0.8–1 GB | 30–50 tok/s | **2–4 s** |
| Distilled ~1B | ~600–700 MB | ~1–1.5 GB | 20–35 tok/s | **4–8 s** |
| Qwen3-4B | ~2.5 GB | ~3–3.5 GB | 6–12 tok/s | **12–25 s** |

\* ~2K tokens of retrieved context + ~120 tokens out, mid-range laptop CPU

CPU generation is **memory-bandwidth-bound** — roughly `tok/s ≈ bandwidth ÷ model size`. Laptop DDR4 gives ~35–40 GB/s usable, DDR5 ~60–70, Apple Silicon 68–150. Size maps almost linearly to latency.

**This is the real argument for distilling.** A 12–25 s wait is a feature nobody uses twice; 4–8 s streamed feels responsive. Latency, not disk footprint, is what justifies stage 4.

**Hardware reality:** Apple Silicon excellent · 2020+ with 8 GB comfortable · 2015–2019 with 8 GB usable but sluggish (~8–15 s) · 4 GB tight · low-end ARM no.

**Do not conflate this with the Tier-S claim.** The *"~500 MB AI stack, CPU-only on 8 GB"* headline covers purpose-built networks — PaddleOCR mobile (21 MB), Whisper base (74 MB), u2netp (5 MB) — which genuinely run anywhere. An LLM roughly doubles or quadruples that stack and belongs in **Tier B as a separate opt-in download**. Blurring the two makes a positioning claim false.

**Three cheap mitigations:** stream the output (perceived latency drops enormously when text starts at 1.5 s) · cache by context hash (advisor questions repeat heavily) · **knowledge base first, model second** — if a rules lookup answers it, never invoke the model.

### Stage 6 — Package

GGUF (or ONNX), signed and hashed into the model registry with a **model card**: training-data provenance, generation date, eval scores, known limitations, licence. The same discipline [applied to third-party models](02-FEATURES.md#11-model-licensing-policy) applies to ours.

---

## 6. Cost

**Compute is a rounding error. The data generator is the investment.**

| Compute | Estimate ⚠️ |
|---|---|
| Teacher generation — 100k examples ≈ 30M output tokens, batched via vLLM on a rented A100 (~$1–2/hr, 8–12 hrs) | $20–40 |
| LoRA SFT — 4B, rank 32, 2 epochs, single A100 or 4090 | $10–25 |
| Distillation to 1B *(optional)* | $30–50 |
| Iteration and failed runs | $50–100 |
| **Total** | **≈ $100–200** |

| Engineering | Estimate |
|---|---|
| Data generator | **2–4 weeks** ← the bulk |
| Eval harness | 1 week |
| Training + iteration | 1–2 weeks (mostly waiting) |
| Packaging + integration | 1 week |
| **Total** | **5–8 weeks of one engineer** (~$15–40K loaded) |

Anyone quoting "distillation costs $200" is quoting the cheapest line item.

---

## 7. Hardware & tooling

| Option | Cost ⚠️ | When |
|---|---|---|
| **Rent** — RunPod / Vast / Lambda | ~$0.35–0.70/hr (4090) · ~$1–2/hr (A100 80GB) · ~$2–3/hr (H100) | Default. Days, not months. |
| **Own** — RTX 4090 24GB / 5090 32GB | ~$1,600–2,500 | Pays back with heavy iteration or more models later |
| **Apple Silicon** via MLX | $0 marginal | Viable for LoRA on ≤4B with high unified memory. Slower. |

A 4B with LoRA fits comfortably in 24 GB. **No H100 is required for any of this.**

### Older GPUs (Pascal / GTX 10-series)

Compute capability 6.1 sits below what the modern stack targets:

| Blocker | Consequence |
|---|---|
| No bf16 (needs Ampere 8.0+) | fp16 or fp32 only |
| **FP16 runs at 1/64 rate on GP104** | Mixed precision saves memory, buys no speed |
| No Flash Attention (FA2 needs 8.0, FA1 needs 7.5) | Much higher activation memory |
| bitsandbytes 4-bit needs ≥7.5 ⚠️; Unsloth needs ≥7.0 | **QLoRA unavailable** — the exact trick that would fit a 4B |

**Verdict:** a **1B** LoRA fits an 8 GB card (~4–5 GB with checkpointing, batch 1, seq 1024) but takes ~20–40 hours where an A100 takes 2–4. A **4B** needs 8 GB for fp16 weights alone — not practical. An 11 GB 1080 Ti makes 4B borderline; an 8 GB 1080 does not.

**Use an old card for development — data generator, script debugging, 1k-example sanity runs — and rent for the real run.** At $10–25 per training run, spending 30 hours of wall-clock to avoid it is a bad trade, especially since you'll retrain three or four times as the data improves. If staying local is a hard requirement, **Qwen3-0.6B** is a legitimate target given how narrow the task is.

| Job | Tool |
|---|---|
| Teacher batch generation | **vLLM** or SGLang — batching is what makes it cheap |
| LoRA SFT | **Unsloth** (most memory-efficient), Axolotl, or LLaMA-Factory |
| Training loop | TRL |
| Quantize + convert | `llama.cpp` → GGUF |
| Constrained decoding | GBNF (llama.cpp) locally; XGrammar server-side |
| Eval | **Custom harness against the frozen set** — generic benchmarks don't measure this task |

---

## 8. Decision gates

Stop at the earliest stage that passes.

| Gate | Pass | Fail |
|---|---|---|
| Prompting + retrieval ≥85% on eval | **Ship it.** No training, no extra download. | → LoRA |
| LoRA-4B ≥85%, size acceptable | **Ship it.** ~2.5 GB, Tier-B optional download. | → diagnose *which* problem |
| **Quality** is the problem | More and better synthetic data — **not a bigger model.** Narrow tasks are data-limited, not capacity-limited. | |
| **Size** is the problem | → distil, or directly fine-tune 1B; target ~600 MB | |

### Success criteria

| Metric | Target |
|---|---|
| Plan validity | **100%** (grammar guarantees it) |
| Plan correctness — exact match on held-out human set | **≥85%** |
| Refusal accuracy on the should-refuse set | **≥95%** |
| Advisor factual accuracy (claims checked against the knowledge base) | **≥98%** — grounding failures are the dangerous class |
| Latency, first token, CPU | <2 s |

---

## 9. Hyperparameters

| Knob | Typical | Effect |
|---|---|---|
| **rank `r`** | 16–32 | Capacity of the delta. Higher = more expressive, bigger adapter, more overfit risk. |
| **alpha `α`** | ≈ 2×r | Scales the delta's influence |
| **target modules** | q/k/v/o projections, optionally MLP | More = stronger, slower, larger |
| **learning rate** | 1e-4 – 2e-4 | ~10× full fine-tuning, because so few parameters are training |
| **epochs** | 2–3 | More overfits fast on narrow data |
| **dropout** | 0.05–0.1 | |
| **QLoRA** | 4-bit frozen base | Lets a 14B+ fine-tune on a 24 GB card |

---

## 10. Pitfalls

| Pitfall | Why it bites |
|---|---|
| **Training before the eval set exists** | You'll have no idea whether it worked — and you'll believe it did |
| **Reaching for a bigger model when quality is poor** | On narrow tasks that is nearly always a *data* problem |
| **Teacher licensing** | Qwen3 (Apache-2.0) and DeepSeek-R1 (MIT) yield redistributable derivatives — but distilling from a **hosted API** is commonly forbidden by provider terms. **Keep the teacher local, on open weights.** |
| **Synthetic-data monoculture** | If every utterance came from one teacher with one prompt, the model learns that teacher's phrasing, not human phrasing. Vary generation prompts aggressively; calibrate on real language later. |
| **Skipping the refusal set** | A model that has never seen *"remove the DRM from this"* will confidently invent a plan for it |
| **Fine-tuning facts in** | They freeze, they drift, they hallucinate, and they can't be patched. Facts go in retrieval. |
| **Quantizing without re-evaluating** | Small models degrade disproportionately at Q4 |
| **Letting the model decide** | The planner decides. The model interprets and narrates. Blur that and the honesty pitch goes with it. |
