# OpenConvert — Vision

**The open source, locally run app to convert any file into anything.**

Status: design record, **v0.4** · 2026-08-13 · License: Apache-2.0
Part 1 of 9 — see [README](README.md) for the full set.

---

## 1. The one-liner

**OpenConvert converts any file into anything — on your machine, with no upload, no account, and no telemetry unless you switch it on — and uses local AI models to do the conversions that plain codecs can't.**

## 2. The problem

Conversion is technically solved and commercially unsolved. The engines exist and are free — libvips, Symphonia, pdfium, LibreOffice, Pandoc, FFmpeg. What doesn't exist is one artifact that is all six of:

| | |
|---|---|
| **Local** | Files never leave the machine, verifiably |
| **Safe** | Because "local" means *you* now run the hostile parser on *your* CPU |
| **Broad** | One tool instead of eleven |
| **Intelligent** | On-device models for what codecs can't do |
| **Modular** | Install only what you need. *(Third-party extension is deferred until **the third external pull request adding a format adapter, or month 12** — an extension ecosystem without enforced capabilities is a malware channel with our signature on it, and until then coverage scales by us. The old trigger, "until a real third-party module wants to exist," could never fire: nobody can want a module for a system with no ABI.)* |
| **Honest** | Every output accounted for: what changed, what was lost, what was inferred |

Every existing product picks two or three.

| Archetype | Example | Where it stops |
|---|---|---|
| Cloud SaaS | CloudConvert, Convertio, Zamzar | Your file is on their server |
| Browser-local | VERT | Great privacy story; video offloads to a server daemon |
| Self-hosted web | ConvertX | 1000+ formats, but it's a service you operate, engines unconfined |
| Domain toolkit | Stirling PDF, HandBrake | Deep in one domain, absent elsewhere |
| Security-first | Dangerzone | Best isolation model in the field; does one thing |
| Local AI point tool | Upscayl, Video2X, Buzz | One capability each, no substrate, four copies of the same models |

## 3. Why now

Four things became true recently that weren't three years ago:

1. **The models got small.** The AI stack covering most AI-required conversion demand is **~500 MB quantized and runs on a CPU-only laptop** — OCR in 21 MB, background removal in 5 MB, transcription in 74 MB.
2. **The licenses got permissive.** Apache-2.0 and MIT models now exist for every core capability, so an open-source app can ship them.
3. **The runtimes converged.** One ONNX file runs on CUDA, DirectML, CoreML, OpenVINO, and NPUs.
4. **Trust collapsed.** The FBI warned in March 2025 that free converter sites were distributing malware. The incumbents are ad-supported websites of unknown provenance.

## 4. The three pillars

### 4.1 The guess is the product

Drop files. **OpenConvert already knows what you want.** One keystroke does it.

This is the feature everything else serves. A prediction from **five signals** — detected file type, your history for that type, whether a sibling output already sits in the folder, the batch shape, and the source folder's category — scored in under 10 ms on a CPU, and always able to explain itself: *"you've done this 14 times."*

Five, not more, because you cannot weight signals you have no data for. A sixth earns its place by measurably improving top-1 acceptance on a frozen benchmark set — see [03-ARCHITECTURE §14](03-ARCHITECTURE.md#14-prediction).

It is deliberately **not an LLM**. It must run before the window finishes animating, on hardware with no GPU, and a model that can't say *why* it guessed would undermine everything else.

### 4.2 Verifiable conversion

Every output carries a **receipt**: engines and versions, exact parameters, an operation class, input and output hashes, and a measured fidelity delta where computable.

Four classes, visible everywhere — in the plan, the UI, the receipt, and the CLI:

| Class | Meaning | Default |
|---|---|---|
| **A — Lossless** | Remux, stream copy, lossless transcode, metadata edit | On |
| **B — Lossy but deterministic** | Re-encode, resize, quantize, tone-map | On, disclosed |
| **C — Inferred** | OCR, transcription, classification — produces *new data*, source untouched | On, reviewable |
| **D — Generative** | Upscale, inpaint, colorize, interpolate — **invents information** | **Off by default** |

**Hard rule:** a Class D operation can never be introduced by a default, a preset, or the intent engine. "Convert" never secretly means "enhance."

No other converter tells you what it did to your file. This is cheap to build and it *is* the trust story.

### 4.3 Hostile input, contained

A converter's job description *is* the attack: take an untrusted file and run a complex, memory-unsafe parser over it. Moving that from a cloud service to a laptop **transfers the blast radius to the user**. So "local" without "sandboxed" is a downgrade, not an upgrade.

The evidence is not theoretical:

| Engine | Reality |
|---|---|
| **ImageMagick** | CVE-2025-57803 — BMP encoder integer overflow, explicitly flagged as dangerous in auto-convert pipelines. CVE-2025-55298 — **crafted filenames** as an attack vector. |
| **Ghostscript** | CVE-2024-29510 — **escaped its own `-dSAFER` sandbox**, exploited in the wild via EPS disguised as JPG, with five siblings from one research effort |
| **FFmpeg** | CVE-2025-1373 — MOV parser use-after-free; a steady annual stream |

So every engine runs confined. Two levels, and the second carries an honest description of what actually engaged:

```
InProcess            pure-Rust parsers, no unsafe in the parse path
                     ← the common formats never leave here, and they still carry hard limits

Sandboxed(profile)   OS-confined subprocess. The profile names the mechanisms that engaged:
                     Linux    Landlock + seccomp-bpf + cgroup v2 + empty netns + userns
                     macOS    App Sandbox + sandbox_init, no network entitlement
                     Windows  AppContainer + restricted token + Job Object + no network SID
```

Five things make this more than a label:

1. **The profile is a value, not a promise** — and a *sealed* one: only the code that probed the machine can construct one, so the line in your receipt is not something any part of the program could have written. On a machine where the full stack is unobtainable the app runs with reduced confinement **and says exactly which mechanism is missing**. A Debian with unprivileged user namespaces disabled still converts, at `Reduced`, with Landlock confining the filesystem — because Landlock needs no namespace. It never silently degrades, and it never refuses when it could have told you the truth instead.
2. **"No network" is not a setting.** The isolation floor has three parts, and the network part has no config key. A machine that cannot deny an engine network access is refused, with the missing mechanism named — because "no engine ever reaches the network" is either true or it is marketing.
3. **Nothing an engine touches is a real path.** Inputs arrive as pre-opened file descriptors with no name at all; outputs are written through parsed single-component names, so an archive member called `../../../.ssh/authorized_keys` cannot be expressed, let alone written.
4. **Nothing you already had is ever overwritten.** An archive member called `.bashrc` is not traversal — it is a legal filename, every traversal check passes it, and most extractors would replace your file. Every file we create is created exclusively; there is no truncating write anywhere in the program.
5. **Every step carries limits and an isolation** — wall time, memory, output bytes, expansion ratio, decode pixels, archive depth — enforced on the in-process path too, because that is where the highest-volume formats run.

Deferred with named triggers: a WASM tier (when third-party modules exist) and a microVM tier with **Paranoid mode**, which rebuilds a document through a bottleneck no exploit survives. Full detail in [09-THREAT-MODEL](09-THREAT-MODEL.md).

## 5. Design principles

1. **The guess is the product.** Zero configuration on the first drop.
2. **One keystroke to done.**
3. **Always show the cost before paying it** — size, time, lossless or not.
4. **Wrong guesses must be cheap**, and correcting the app teaches it.
5. **Never surprise the file.** No silent re-encode, no silent metadata strip, no silent AI.
6. **The original is sacred — and so is every other file you already had.** Non-destructive by default, and nothing we write can replace something that exists.
7. **Safety is a level you turn *up*, never *off* below a floor** — and one part of the floor has no dial at all.
8. **Small core; everything else is a module.** Target under 60 MB installed ⚠️ — an estimate until Week 0 measures it, and we will publish the real number rather than the promised one. *(Two reference points, neither of them the answer: the engine shared-library closure installs at **42.7 MiB** on Debian x86_64, and a worker that really links libvips is **300 KiB** alone or **15 MiB** with the 103 libraries it touches — S12e, S35. One platform, one worker, system copies. A shipped build bundles its own, adds three more workers, the desktop shell and the CLI, and pays signing overhead. Plausible for one platform; tight for three.)*
9. **Everything the GUI does, the CLI does, and the config file describes.**
10. **No accounts. One outbound call, and you can see it and switch it off.** The weekly signed update-and-revocation manifest is the only network request the app makes by default: no identifier, no version, no query, and the literal payload is shown in settings. Everything else — diagnostics, models, engines — is off and opt-in. *We would rather make one honest call than promise none and ship a kill switch that never arrives.*
11. **Boring where it matters.** Class A operations are byte-reproducible. Class B is byte-reproducible **under `--deterministic`**, which pins encoder threading — and reproducible in quality, against a fixed perceptual floor, otherwise. Multi-threaded encoders are not bit-stable across thread counts, so the unqualified version of this promise was false.
12. **Quiet.** No badges, no nags, no upsells. The personality is competence.

## 6. What OpenConvert is not

| Not | Because |
|---|---|
| A cloud service | We ship software; we never operate an endpoint that receives your files |
| An editor (Photoshop, Premiere, Acrobat) | We convert and enhance; we don't author |
| A media library or file manager | It operates on files you bring; it doesn't want to own them |
| A DRM circumvention tool | DRM inputs are refused with a clear message |
| A YouTube downloader | ~6.12M searches/month, and the reason this category smells like malware |
| An AI-first product | It is fully functional with AI uninstalled |

## 7. Who it's for

| Persona | Job | What nobody else gives them |
|---|---|---|
| **Privacy-constrained professional** — lawyer, doctor, journalist, HR | "Convert this without it leaving my laptop" | Verifiable locality + Paranoid mode + real redaction |
| **Creator** — video, podcast, photo | "Smallest file that still looks right, in the format the platform wants" | Platform presets, quality targeting, stems, subtitles, batch |
| **Developer / power user** | "Script it, pipe it, put it in CI, let my agent call it" | CLI, `--json`, watch folders, REST, MCP, version pinning |
| **Knowledge worker** | "PDF → editable; 200 scans → searchable" | Document IR, OCR, table extraction, auto-routing |
| **IT / security admin** | "Let staff convert attachments safely, under policy" | Locked policy, offline bundles, audit log, zero egress |
| **Archivist** *(partly v1)* | "Normalize 40k heterogeneous files, losslessly, with a manifest" | Lossless-first planning, receipts, resumable batches, **one manifest per run**, a job budget so a 40k run cannot fill the disk |

> **What the archivist gets in v1, and what they don't.** Batch, resume, the job budget and `--manifest` land in weeks 31–33. **PDF/A does not:** archival PDF goes through LibreOffice, which is a post-v1 module — so v1 serves image, audio and container normalisation for this persona and not document normalisation. That is worth stating rather than leaving them to discover it, because "losslessly, with a manifest" is a promise the other five personas do not make.

## 8. The promise, stated as a test

Seven things that must all be true at v1, or the project failed. Each maps to a CI gate in [08-EXECUTION-PLAN §4](08-EXECUTION-PLAN.md#4-the-ci-gates) — a promise nothing tests is a slogan, and a promise whose test is scheduled for a week it cannot pass is the same slogan with a date on it.

1. **Drop 12 iPhone photos. Press Enter. Done in 3 seconds, correct format, GPS stripped** — with no configuration and no prior use.
2. **`MKV → MP4` completes in seconds and is bit-identical**, and the UI says `⇄ stream copy · lossless` before you commit.
3. **Turn off the network. Everything still works** except downloading new engines and models and checking for updates — and with the network *on*, a conversion still makes zero connection attempts, in the desktop app as well as the CLI.
4. **Open Activity Monitor. OpenConvert is not in the energy list.**
5. **Every output can answer "what exactly did you do to my file?"** in one click — including which sandbox mechanisms were actually in force, and the hash of the exact bytes that were read.
6. **Drop a hostile file and nothing happens to your machine.** A 4 KB image bomb, a zip bomb, a PostScript file named `.jpg`, and an archive whose members try to escape their folder all fail closed, with an actionable message — and if the app is killed mid-conversion, it leaves nothing behind.
7. **Drop a hostile file and nothing happens to your *other* files either.** An archive whose members are named after things already in the folder leaves every one of them untouched — because that is not an escape, it is a filename, and it is the case a traversal check is designed to let through.

## 9. Business model, in one paragraph

The app is Apache-2.0 with **no feature gating, ever** — every conversion is free forever. Revenue comes from what enterprises need *around* running it: a security-response SLA on the third-party engines, attested builds, compliance evidence packs, a fleet control plane, air-gapped bundles, and support. The enterprise pitch is **not** "file conversion" — it's **secure file ingestion (CDR)** and **sovereign document AI**, which are funded budget lines. Details in [04-GROWTH](04-GROWTH.md).

## 10. The name — decided

# **OpenConvert**

Latin: *a crossing over, a passage, a transition.* Domain `openconvert.dev`.

Tagline: **convert any file into anything.**

`openconvert.dev` and `openfile.dev` to be acquired and **301-redirected**; `convertit.dev` registered defensively only. Full candidate pool and etymology in [07-DESIGN-SYSTEM §14](07-DESIGN-SYSTEM.md#14-naming).

The rest of this section is the record of *why*, kept so the decision doesn't get relitigated.

### Criteria

| | Why it matters |
|---|---|
| **Available** | Domain, trademark, npm/crates.io/PyPI, GitHub org. **This is the gate, not taste.** |
| **CLI-shaped** | `X convert photo.heic` gets typed thousands of times. 3–7 characters is the sweet spot. |
| **Not category-locked** | The enterprise wedge is *secure file ingestion (CDR)* and *sovereign document AI*, not conversion ([04-GROWTH §10.1](04-GROWTH.md#101-the-reframe)). A name that hard-codes "convert" fights that. |
| **Trademarkable** | Descriptive marks are legally weak. |
| **SEO/GEO value** | Containing the head term helps discovery — see [04-GROWTH §3](04-GROWTH.md#3-geo--getting-cited-by-ai-engines). |

### The four that were on the table

| Name | Meaning | Strengths | Costs |
|---|---|---|---|
| **openconvert.dev** | Lat. *a crossing over, passage, transition* | Distinctive and trademarkable · directly on-theme · sounds infrastructural · **not category-locked** — works equally for a converter, a CDR gateway, or a document pipeline · semi-graspable via "transit" | 9 characters is long for a CLI · reads faintly clinical · ⚠️ in ecclesiastical Latin *OpenConvert* denotes the death of a saint (obscure, but real) |
| **openconvert.dev** | descriptive | **Instantly comprehensible, zero explanation** · "open" states the licence position · **excellent SEO/GEO** — contains the category's head term, and an LLM asked for an "open source file converter" surfaces a product named exactly that · impossible to misspell | **Legally weak** — purely descriptive marks get thin protection · crowded prefix (OpenAI, OpenVPN, OpenCV, OpenWrt) · **category-locked**, which fights the CDR positioning · undersells the product: the pitch is *sandboxed, honest, predictive*, not "a converter, but open" · 11 characters |
| **openfile.dev** | descriptive | Shorter (8 ch) and **not category-locked** — works for a converter, a CDR gateway, a pipeline, an inspector · plain in a way that suits the product's personality · genuine resonance with the enterprise wedge, since CDR *is* "how do you safely open a file you don't trust?" | **Takes the costs of a descriptive name without the payoff.** "Open file" is the most generic phrase in computing — every app has that dialog · the query space is saturated by file-extension lookup sites and its searchers want a *viewer*, so there is no ranking benefit to capture · unprotectable · confusable with the OS file dialog in our own documentation |
| **convertit.dev** | descriptive | Shorter than openconvert (9 ch) · keeps the head term, so keeps the SEO/GEO benefit · reads as a *name* rather than a description, which trademark law treats marginally better · friendly, action-oriented; also a real French word (*il convertit*) | ⚠️ **`Convertio` is a direct competitor with 24.9M monthly visits, one character away.** The SEO benefit *inverts* — brand and typo traffic leaks toward the larger incumbent · brand distinction collapses exactly where the positioning depends on it (*"we are not the ad-supported converter sites"*) · confusing similarity in an identical class is the textbook trademark case · secondary collision with **ConvertKit** · category-locked, and without the "open" licence signal |

**How the four compare:**

| | Distinctive | Not category-locked | SEO/GEO payoff | Defensible |
|---|---|---|---|---|
| **openconvert** | ✅ | ✅ | ✗ | ✅ |
| **openfile** | ✗ | ✅ | ✗ | ✗ |
| **openconvert** | ✗ | ✗ | **✅** | ✗ |
| **convertit** | **✗✗** | ✗ | ⚠️ compromised | ✗ |

`openfile` is the only one with no column of its own; `openconvert` at least buys something concrete for what it gives up. **`convertit` is the only candidate whose weakness is actively harmful rather than merely absent** — the others fail to help, that one helps a competitor. Hold it as a defensive registration; don't build on it.

### Also considered (not pursued)

From the fuller Greek/Roman pool — none were availability-checked, since OpenConvert was verified and won on merit:

| Name | Meaning | Note |
|---|---|---|
| **Pyxis** | Gk. a small round box; also the compass-box constellation | *Container* is the literal technical term for MP4/MKV. 5 letters. ⚠️ BD Pyxis in medical dispensing. |
| **Fornax** | Rom. goddess of the furnace; a constellation | The most product-like — sounds like something you deploy |
| **Limen** | Lat. *threshold* | Calmest and most modern; "liminal" makes it graspable |
| **Phanes** | Gk. *the revealer*, "bringer to light" | Exactly what the product does for a file you can't open |

### The strategic question

Descriptive names win early discovery and lose long-term brand and legal ground. Docker isn't "OpenContainer"; Stripe isn't "PayAPI." Against that, OpenAI shows a descriptive name can still become iconic.

**You don't have to choose.** `.dev` registrations cost less than an hour of anyone's time:

> **Brand on a distinctive name; own `openconvert.dev` and `openfile.dev` as redirects.**

That captures the head term's SEO/GEO value at zero cost to the brand, keeps the door open for the CDR positioning where "convert" would be actively wrong, and forecloses a competitor using them. **Redirect rather than building a second site** — splitting content across domains splits authority.

### Why OpenConvert won

Verified available, distinctive enough to defend, on-theme without being obscure, and — decisively — **it survives the pivot from *converter* to *secure ingestion platform*** that the commercial plan depends on. A name containing "convert" cannot be sold to a CISO as an attachment-sanitisation gateway.

**On CLI length:** `openconvert` is nine characters. So is `terraform`, and nobody minds. This was raised as friction earlier and it isn't one — no alias needed.

**Remaining checks before public use:** USPTO/EUIPO search · npm, crates.io, PyPI · GitHub org · a quick look at what "openconvert" returns in the major search engines. ⚠️ Note for the record: in ecclesiastical Latin *OpenConvert* denotes the passing of a saint — obscure, commercially irrelevant, but better known than discovered.
