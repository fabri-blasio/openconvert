# OpenConvert — Website

What `openconvert.dev` is, what every page does, and the animated sequences that carry the argument.

Status: design record, **v0.2 — product-site revision** · 2026-08-18
Part 10 of 10 — see [README](README.md). Design language: [07-DESIGN-SYSTEM](07-DESIGN-SYSTEM.md). Channel strategy: [04-GROWTH](04-GROWTH.md).

---

## Contents

1. [What v0.1 got wrong](#1-what-v01-got-wrong)
2. [Voice rules](#2-voice-rules)
3. [The three pillars](#3-the-three-pillars)
4. [Sitemap](#4-sitemap)
5. [Page specifications](#5-page-specifications)
6. [Motion](#6-motion)
7. [The six sequences](#7-the-six-sequences)
8. [Visual system](#8-visual-system)
9. [Gates](#9-gates)

---

## The brief

**A product site for a product that works.** Every page is written in the present tense about software that exists, converts files, and runs models on the visitor's machine. Nothing on this site talks about the roadmap, the build, or what is coming next — that belongs in the repository, not in front of someone deciding whether to trust a binary with their contracts.

Three pillars carry the whole site: **safety**, **conversions**, **local AI**. Every page serves exactly one of them, every claim links to the page that proves it, and six animated sequences do the explaining that prose cannot.

| | |
|---|---|
| Routes | 15 |
| Sequences | 6 |
| JavaScript on content pages | 0 KB |
| Third-party requests | 0 |

---

## 1. What v0.1 got wrong

v0.1 produced a site about a *project* rather than a site for a *product*. Naming the three failures precisely is the only way they do not come back.

| Failure | What it produced | The rule that replaces it |
|---|---|---|
| **Status, not product** | A download page reading *"Not yet. And we would rather say so."* An empty demo slot labelled *not yet live*. A changelog of unwritten commits. Every one of those tells a visitor to come back later. | The site describes shipped behaviour in the present indicative. Release state lives in the repository. |
| **Fluff** | Pages explaining the design record to the reader — which document said what, why a section was rewritten, what the author had noticed. Interesting to the author, worthless to the buyer. | Every sentence answers *"what does this do for me"*. Mechanism, number, or cut it. |
| **Wrong pillars** | The site inherited the architecture's internal framing — a pipeline diagram, a purity argument, a crate boundary. Nobody arrives wanting to know about crates. | Safety, conversions, local AI. Architecture appears only where it is the proof of one of those three. |

---

## 2. Voice rules

Enforceable in review, and half of them are greppable ([§9](#9-gates) makes one a build gate). They exist because the failure mode is not writing badly — it is writing accurately about the wrong subject.

### Never appears on a product page

| Token | Also |
|---|---|
| `coming soon` | not yet · planned · in progress · will be · once we · our roadmap |
| `we believe` | we think · our goal is · we aim to · we are working on |
| status | Version disclaimers, beta badges, build progress, commit counts |
| meta | References to the design record, the revision history, or what changed and why |
| adjectives | fast · powerful · seamless · robust · cutting-edge · blazing — replace each with a number |

### Always true of a product page

| Rule | In practice |
|---|---|
| **Tense** | Present indicative. *"Every engine runs confined."* Not *"will run"*. |
| **Subject** | Second person for outcomes, third for mechanisms. *You* get the file; *the sandbox* denies the syscall. |
| **Proof** | Every claim links to the page that demonstrates it. A claim with nowhere to go is marketing. |
| **Numbers** | 21 MB, 80 ms, 8 GB, zero. Specificity is also what gets cited by an assistant. |
| **Mechanism** | Name what does the work. *"Landlock denies the write"*, not *"we protect your files"*. |

> **The one honest exception.** Local AI is off by default and downloaded on request, so [`/ai`](#ai) and [`/download`](#download) must say plainly that the models are a separate opt-in install and roughly what they cost in disk and time. That is a product fact a user needs before they click, not roadmap language — the test is whether the sentence helps someone decide *right now*.

---

## 3. The three pillars

Each pillar has one thing it must prove, one sequence that proves it, and the pages that carry it. A visitor who understands all three understands the product.

### Safety

**Running a converter is running a parser on a hostile file.** Every third-party engine runs in an OS sandbox with no network and no filesystem outside its own job directory. Engines receive open file descriptors, never filenames. Nothing the user already had can be overwritten, because the program contains no truncating write.

- **Must prove:** containment is real and observable, not a claim.
- **Sequences:** [A2 containment](#a2--containment), [A5 nothing overwritten](#a5--nothing-overwritten)
- **Pages:** `/security` · `/privacy` · `/enterprise`

### Conversions

**It tells you what it will do before it does it.** Around 20 formats done correctly, with the plan shown before you commit: which engine, which steps, lossless or lossy, and how long. A container change copies streams in seconds; a real re-encode says so up front instead of surprising you at the end.

- **Must prove:** breadth, plus the honesty about what each conversion costs.
- **Sequences:** [A3 lossless or not](#a3--lossless-or-not), [A6 the receipt](#a6--the-receipt)
- **Pages:** `/convert` · `/convert/<a>-to-<b>` · `/formats/<f>`

### Local AI

**The models run on your laptop, not someone's server.** OCR in 21 MB. Transcription in 74 MB. Background removal in 5 MB. The whole stack is about 500 MB, quantized, CPU-only on 8 GB of RAM — and the network counter stays at zero the entire time it runs.

- **Must prove:** it genuinely runs offline, and it is small enough to believe.
- **Sequence:** [A4 on-device](#a4--on-device)
- **Pages:** `/ai` · `/ai/<capability>` · `/models`

> **The three reinforce each other, and the site should say so out loud.** Local AI is only trustworthy because of the sandbox; the sandbox is only tolerable because conversions stay fast; conversions are only honest because of the receipt. A competitor can copy any one pillar. The combination is the product.

---

## 4. Sitemap

Fifteen routes. Four are generated from data the binary already carries, which is why the format and model references cost nothing to keep accurate.

| Route | Pillar | One job | Source |
|---|---|---|---|
| `/` | all | Convert a file in the browser within 15 seconds of landing. | authored |
| `/convert` | conversions | Find whether it does the conversion you came for. | generated |
| `/convert/<a>-to-<b>` | conversions | Do that conversion, here, without installing anything. | generated |
| `/formats/<f>` | conversions | Explain a format and what leaving it costs. | generated |
| `/ai` | local AI | Prove the models are local, small, and optional. | authored |
| `/ai/<capability>` | local AI | Show one capability working, with its model and its size. | generated |
| `/models` | local AI | List every model, its licence, size and commercial-use status. | `models.toml` |
| `/security` | safety | Show the containment, and publish what it does not cover. | authored |
| `/privacy` | safety | State what leaves the machine, in under 300 words. | generated |
| `/download` | all | Get the right binary, verified, in one command. | release job |
| `/docs` | conversions | Make the CLI, recipes and automation usable without support. | authored |
| `/enterprise` | safety | Reframe from conversion to secure ingestion and sovereign AI. | authored |
| `/vs/<competitor>` | all | Compare honestly, including where they win. | authored |
| `/changelog` | all | Tell an existing user what changed in the release they just got. | releases |
| `/blog` | all | Publish the engineering nobody else can, and earn the citations. | authored |

---

## 5. Page specifications

Each page: what it is for, who arrives, the sections in order, the sequence it carries, and the one action it wants. **The section lists are the build order.**

---

### `/`

| | |
|---|---|
| **Pillar** | All three |
| **Arrives from** | Hacker News, an assistant, a package manager |
| **Sequence** | [A1 the drop](#a1--the-drop) |
| **Action** | Convert a file, then download |

**Job: convert something in the browser before asking for anything.**

1. **Hero.** One sentence: convert any file into anything, on your machine. Under it, a real drop zone — not a screenshot, not a video. The whole body is the target.
2. **The drop, live.** A file dropped here converts in WebAssembly and downloads. No upload, no account, no wait. This is the entire argument and it is above the fold.
3. **What just happened.** Three cards reading off the conversion the visitor just ran: which engine, which class, which sandbox profile. The demo generates the explanation instead of sitting beside it.
4. **Conversions.** A compact matrix of the twenty core formats with a search field, linking to `/convert`. Shows breadth in one screen without a wall of logos.
5. **Safety.** Sequence A2 — the containment box, three denied escapes, conversion completes anyway. One line beneath: engines get descriptors, never filenames. Links to `/security`.
6. **Local AI.** Sequence A4 — a 21 MB model loading and reading a scan with the network counter pinned at zero. Links to `/ai`.
7. **Nothing overwritten.** Sequence A5, two columns. The shortest section on the page and the one people remember.
8. **Install.** Platform-detected one-liner, checksum, and the permanent licence commitment in two sentences.

*Zero-click resilience: the hero converts, so an assistant summarising the page still sends someone who wants to **use** it. Nothing critical is behind the fold or behind script.*

---

### `/convert`

| | |
|---|---|
| **Pillar** | Conversions |
| **Arrives from** | "convert X to Y" searches |
| **Sequence** | None — this page is an index |
| **Action** | Open the tool page |

**Job: answer "does it do the one I need" in under five seconds.**

1. **Search.** A single field matching source, target, alias and operation. Typing `heic` filters instantly. Typing `word` matches DOCX. No dropdowns.
2. **The matrix.** Source formats as rows, target families as columns, each cell carrying its class glyph. Lossless cells are visually quiet; the loud ones are the ones that change your file.
3. **By media.** Images, audio, video, documents, archives, data — each a short list with the highest-demand conversions first and a link to the tool page.
4. **Operations.** Not every job is a format change: compress to a size, strip metadata, extract audio, merge PDFs, OCR a scan. These are what people actually search for.
5. **What it does not convert.** Short, honest, and it prevents the worst first experience — downloading a tool that cannot do the thing.

*Generated from the format and route tables, so it cannot advertise a conversion the binary lacks.*

---

### `/convert/<a>-to-<b>`

| | |
|---|---|
| **Pillar** | Conversions |
| **Arrives from** | The highest-volume queries |
| **Sequence** | [A1](#a1--the-drop), scoped to one pair |
| **Action** | Convert, then download the app |

**Job: do the conversion they searched for, in the tab they searched from.**

1. **Do it now.** Drop zone pre-set to this pair, running in WebAssembly. Working before any prose. Ten of these pages, chosen by demand.
2. **What this conversion costs.** The class, in one sentence. HEIC to JPEG re-encodes; WAV to FLAC does not. Says so before, not after.
3. **The plan it ran.** The actual steps and engine for the file just converted — the desktop app's plan preview, on the web.
4. **Doing it in bulk.** The CLI line for the same conversion across a folder. Converts a curious visitor into an installer.
5. **Questions.** Does it lose quality; is there a size limit; does the file upload. Direct-answer format, two sentences each, because this is what gets extracted.

*The page never uploads. One line above the fold says so, and the browser's own network panel confirms it — an invitation the page makes explicitly.*

---

### `/formats/<f>`

| | |
|---|---|
| **Pillar** | Conversions |
| **Arrives from** | "what is a .heic file" |
| **Sequence** | [A3](#a3--lossless-or-not) on pages where there is a choice |
| **Action** | Continue to a tool page |

**Job: be the reference an assistant quotes when asked about a format.**

1. **What it is.** Two sentences. What the format holds, who made it, why it exists.
2. **Facts.** Extensions, the engine that reads it, how it is detected, whether it runs sandboxed. Detection by content is stated here because it is a safety property.
3. **Converts to.** Every route with its class and steps, and a line on what each one costs.
4. **What you lose leaving it.** The section nobody else writes, and the reason this page gets cited.
5. **From the command line.** Two lines, copyable.

*Where a format shares magic bytes with others — the TIFF-derived RAWs, the ISO-BMFF brands — the page says how it is really identified. That candour is what makes it a reference.*

---

### `/ai`

| | |
|---|---|
| **Pillar** | Local AI |
| **Arrives from** | "offline OCR", "local transcription", "on-premise document AI" |
| **Sequence** | [A4 on-device](#a4--on-device) |
| **Action** | Download, then add the AI pack |

**Job: make "it runs on your laptop" believable to someone who assumes it cannot.**

1. **Sequence A4, immediately.** A model loads, reads a scan, produces text — with a live byte counter at zero. The claim and its proof in one component.
2. **The sizes.** OCR 21 MB. Transcription 74 MB. Background removal 5 MB. Speech detection 2 MB. Document structure 248 MB. Whole stack about 500 MB. The numbers *are* the argument; a table beats any paragraph.
3. **What it runs on.** Four cores, 8 GB, no GPU. Honest per-capability timings, including the slow ones. A capability marked GPU-recommended says so rather than disappointing later.
4. **What each capability does.** Six cards — read text from images, transcribe audio, remove backgrounds, upscale, understand document structure, translate — each linking to its own page.
5. **Inferred versus generated.** The class distinction, applied to AI. OCR reads what is there; upscaling invents pixels. Generated output is never armed by a default, a preset, or a suggestion.
6. **Why local matters.** Three sentences: the file never leaves, it works on a plane, and there is no per-page cost. Then the honest cost: a download, disk space, and slower than a datacentre GPU.
7. **Licences.** Every model permissively licensed and redistributable, linking to `/models`. Procurement asks this first.

*Says plainly that the pack is an optional download and that conversion works fully without it. That is a product fact, not a hedge.*

---

### `/ai/<capability>`

| | |
|---|---|
| **Pillar** | Local AI |
| **Six pages** | `ocr` · `transcribe` · `remove-background` · `upscale` · `document-structure` · `translate` |
| **Sequence** | [A4](#a4--on-device), scoped to one model |
| **Action** | Try it, then download |

**Job: show one capability working and name the model doing the work.**

1. **Try it.** The capability running in-browser on a sample, or on the visitor's own file where the model is small enough to ship to a page.
2. **The model.** Name, size, licence, and what it was trained for. Naming the model is unusual and it is exactly what a technical evaluator wants.
3. **Speed here.** Measured on ordinary hardware, per page or per minute of audio. Not a benchmark chart — one honest number.
4. **Where it is weak.** OCR on handwriting, transcription on heavy accents, upscaling on faces. Publishing the limits is what makes the strengths credible.
5. **Use it in bulk.** The CLI invocation across a folder, with the flag that writes a manifest.

---

### `/security`

| | |
|---|---|
| **Pillar** | Safety |
| **Arrives from** | Hacker News, a CISO, a procurement review |
| **Sequence** | [A2 containment](#a2--containment), [A5 nothing overwritten](#a5--nothing-overwritten) |
| **Action** | Forward it to a colleague |

**Job: be the page a security engineer sends to their team unedited.**

1. **The problem, stated against ourselves.** A converter's job is running a memory-unsafe parser on a hostile file. Moving that local transfers the blast radius to you. Leading with the risk is what earns the rest.
2. **Sequence A2.** The containment box: an engine tries to reach the network, read a key, spawn a process — each denied by name — and the conversion still completes.
3. **The evidence.** Real CVEs in ImageMagick, Ghostscript and FFmpeg, including the one that escaped its own sandbox. Cited, dated, linked. See [09-THREAT-MODEL §2](09-THREAT-MODEL.md#2-evidence-base).
4. **What confines what.** Landlock, seccomp, cgroups; AppContainer, restricted token, Job Object; App Sandbox. Per platform, per mechanism, with what happens when one is unavailable.
5. **Sequence A5.** Nothing you already had is overwritten, and the reason is structural rather than a checkbox.
6. **What we do not protect against.** Published verbatim, no softening — a full chain through the sandbox stack, a compromised OS, side channels, provenance detection failing open. Source: [09-THREAT-MODEL §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against).
7. **Reporting and audit.** Contact, disclosure policy, response times, and the published third-party audit of the boundary.

> **Hard rule:** this page may not state a guarantee stronger than the test backing it. Every claim walks back to a mechanism before it ships.

---

### `/privacy`

| | |
|---|---|
| **Pillar** | Safety |
| **Length** | Under 300 words, hard cap and a build gate |
| **Sequence** | None |
| **Action** | Finish reading it |

**Job: be short enough that people actually read it to the end.**

1. **Your files.** Never leave the machine. There is no endpoint that receives them.
2. **Telemetry.** None. No analytics, no crash pings, no identifier.
3. **The one call.** A weekly signed manifest carrying the engine revocation list. No identifier, no version, no query. Previewable, and switchable off, with the trade stated.
4. **Stored locally.** Config, history, recipes — all yours, all wipeable, none able to weaken a security setting.
5. **This website.** No cookies, no third-party anything. If analytics ever exist they are self-hosted and disclosed in this paragraph.

*Generated from one source shared with the README and the app's settings screen; the build fails if the three disagree.*

---

### `/download`

| | |
|---|---|
| **Pillar** | All three |
| **Sequence** | None — this page is a transaction |
| **Action** | Run one command |

**Job: detected platform, one command, verifiable — above the fold.**

1. **Your platform.** Detected, with the package-manager one-liner first and the direct installer second. Version, size, and date.
2. **Every platform.** Windows, macOS, Linux, both architectures each, with the sandbox mechanisms that engage on each.
3. **Verify it.** Checksum, signature, SBOM, and the reproducible-build instructions. Written by the release job, never by hand — a stale checksum on a security product is worse than none.
4. **The AI pack.** Separate, optional, about 500 MB, added from inside the app or with one CLI command. Conversion works fully without it.
5. **First run.** What it does on launch, including the one weekly network call.

---

### `/docs`

| | |
|---|---|
| **Pillar** | Conversions |
| **Arrives from** | An installed user, or an agent integrating the CLI |
| **Sequence** | None |
| **Action** | Succeed without asking anyone |

**Job: make the CLI learnable in ten minutes and scriptable in twenty.**

1. **Five commands.** `convert`, `plan`, `routes`, `inspect`, `verify`. Each with a real invocation and real output.
2. **Every flag.** Generated from the parser so it cannot drift from the binary.
3. **JSON output.** The schema, with worked examples. This is the page an agent integration reads.
4. **Recipes.** Saving a conversion, sharing it, and what happens when a shared recipe meets a different machine.
5. **Batch and automation.** Folders, resume, manifests, exit codes, and CI usage.
6. **Configuration.** The three layers and the command that prints which layer set a value.

---

### `/enterprise`

| | |
|---|---|
| **Pillar** | Safety |
| **Arrives from** | A CISO, a DPO, a procurement lead |
| **Sequence** | [A2 containment](#a2--containment) |
| **Action** | Start a conversation |

**Job: sell secure ingestion and sovereign document AI, not file conversion.**

1. **The reframe.** Nobody has a file-conversion budget. Secure ingestion and on-premise document AI are funded lines, and the product already is most of both. See [04-GROWTH §10.1](04-GROWTH.md#101-the-reframe).
2. **Ingestion.** Untrusted attachments parsed under per-engine OS isolation with a published, audited boundary — with A2 as the demonstration.
3. **Sovereign AI.** OCR, transcription, extraction and translation with no egress surface at all. The relevant point for anyone whose regulator treats inference as processing.
4. **Evidence pack.** Threat model, audit, SBOM, model licences, reproducible builds, DPIA material — downloadable without a form.
5. **What is paid.** Response-time SLA, fleet control, attested builds, support. Everything needed to convert a file stays free forever, and the boundary is stated plainly.
6. **Talk to us.** One address. No form, no qualification funnel.

---

### `/vs/<competitor>`

| | |
|---|---|
| **Pillar** | All three |
| **~30 pages** | cloud converters · PDF suites · self-hosted · AI point tools · security |
| **Sequence** | [A2](#a2--containment) where isolation is the difference |
| **Action** | Switch |

**Job: win the comparison by being the one that admits where it loses.**

1. **The one-line difference.** Stated in the first sentence, without adjectives.
2. **Feature table.** Real capabilities, marked accurately, with our gaps visible.
3. **Where they are better.** Named specifically. A comparison that never concedes is not read as a comparison.
4. **Where we are.** Usually isolation, receipts, and no upload. Linked to their proofs.
5. **Migrating.** The equivalent command or workflow, so switching is concrete.

*Assistants cross-check comparisons. A fair one gets cited; a dishonest one gets contradicted.*

---

### `/changelog` · `/blog` · `/models`

| | |
|---|---|
| **Pillar** | Supporting |
| **Sequence** | None |
| **Action** | Update, read, verify |

**Job: serve people who already have it, and earn the citations that bring new ones.**

- **`/changelog`** — shipped releases only, newest first. What changed, what broke, what to do about it. Generated from release tags. Security fixes marked and dated, because that is what this page is really for.
- **`/blog`** — the engineering nobody else can publish: sandboxing three operating systems, why Ghostscript is excluded, what ten converters actually do to your file. Original measurement, not opinion.
- **`/models`** — every shipped model with licence, size, hash and commercial-use status, generated from the same table the build gates on. Compliance teams ask for exactly this and no competitor can produce it.

---

## 6. Motion

The app's rule is that motion is functional only and **nothing loops** ([07 §8](07-DESIGN-SYSTEM.md#8-motion)). That is right for software someone operates daily, and wrong for a page where a stranger has to be taught in fifteen seconds. The site adds one tier and constrains it, rather than quietly ignoring the rule.

| Tier | Duration | Use |
|---|---|---|
| `motion-micro` | 120 ms | Hover, focus, chip state |
| `motion-standard` | 220 ms | Panels, rows, list changes |
| `motion-large` | 340 ms | Sheets, page transitions |
| **`motion-narrative`** | **≤ 6 s** | **Web only.** The six sequences below |

One curve throughout: `cubic-bezier(0.32, 0.72, 0, 1)`. Fast start, long settle.

### Five rules for narrative motion

| Rule | Meaning |
|---|---|
| **Once** | Plays on entry, one time. Never loops. Replay is always available and keyboard-reachable. |
| **Teaching** | Must carry information a still cannot. If a still would do, use a still. |
| **Skippable** | Never blocks reading. Nothing beneath it waits for it. |
| **Honest** | A recording is labelled a recording. On a product sold on verifiability, a fake live demo is disqualifying. |
| **Cheap** | Transform, opacity and `stroke-dashoffset` only. The layout-shift budget is zero, not low. |

> **Reduced motion is not "fade instead".** Under `prefers-reduced-motion` every sequence resolves **instantly to its end state**: the containment box shows all four denials already recorded, the route table shows the eliminated row struck through, the AI panel shows the finished text and the zero counter. Nobody loses information by turning motion off.

**Implementation note.** Do not use `requestAnimationFrame` to trigger a reveal. It is throttled in backgrounded and non-compositing tabs, so the content can silently never appear. Append the node, force a synchronous reflow (`void el.offsetWidth`), then add the class.

---

## 7. The six sequences

Each belongs to a pillar and each replaces a paragraph that would not have been believed.

### A1 — the drop

**Home, tool pages · conversions + safety + local AI**

Twelve files in, one keystroke, done. Files land in the drop zone; the right target is already selected with the reason visible (*"you have done this 14 times · sibling .jpg present"*); the plan appears with its classes and the sandbox profile per step; it finishes with a receipt count. Everything the product claims, in one component, before a word of copy.

| | |
|---|---|
| **Teaches** | Zero configuration, an explained guess, and the plan shown before it runs |
| **Duration** | 3.4 s · 90 ms per file, 340 ms per stage |
| **Mechanism** | Nodes appended then revealed by class toggle after a forced reflow; fixed-height stage so nothing shifts |
| **Reduced** | Final state renders immediately, plan and receipt visible |

### A2 — containment

**Home, security, enterprise · safety**

The engine tries to get out, and it does not. A confined engine attempts the four things that would matter, each denied by name, and the conversion completes anyway:

```
connect() → 203.0.113.7:443        EPERM  · no network namespace
open("~/.ssh/id_rsa")              EACCES · outside job directory
fork() → /bin/sh                   EPERM  · seccomp denies execve
write("../../Startup/x.lnk")       EACCES · not a single path component

invoice.pdf → invoice.png · completed · nothing escaped the job directory
```

| | |
|---|---|
| **Teaches** | Confinement is enforced by the OS, not by our checks — and it does not cost the conversion |
| **Duration** | 3.2 s · 620 ms per attempt |
| **Mechanism** | Attribute swap driving colour and the denial text; no layout change |
| **Reduced** | All four denials and the completion render at once |

*Safety is unprovable in prose — every converter claims it. The denials are the product.*

### A3 — lossless or not

**Home, formats, tool pages · conversions**

Same target, opposite answers, and it says which before it starts. Two ordered routes are shown; the requirement resolves; the losing route is struck through with its reason.

| Input | Route 1 `StreamCopy` | Verdict |
|---|---|---|
| `movie.mkv → mp4` | requires codecs are MP4-compatible — H.264 and AAC, both fine | stream copy selected · bit-identical · 1.9 s for 4.2 GB |
| `movie.avi → mp4` | requires codecs are MP4-compatible — DivX and MP3 are not | route 1 eliminated · re-encode selected · about 6 min, and you are told first |

| | |
|---|---|
| **Teaches** | The plan is inspectable, the rejected option is named, and speed follows from the choice |
| **Duration** | 1.4 s · replays on every tab change |
| **Reduced** | Final state immediately; tabs switch instantly |

*No other converter shows the decision, because none of them has one to show.*

### A4 — on-device

**Home, `/ai`, capability pages · local AI**

A 21 MB model reads a page, and nothing leaves. Two numbers carry it — the model size and the byte counter — and both are on screen the whole time. The counter is the component; the text appearing is what makes people look at it.

| | |
|---|---|
| **Teaches** | Local inference is real, small, and offline — the counter never moves |
| **Duration** | 3.6 s · 900 ms load, 380 ms per line |
| **Mechanism** | Progress fill by width transition; output lines revealed in sequence at reserved height |
| **Reduced** | Model ready, all lines shown, counter at zero |

*The hardest claim on the site is that useful AI runs locally. This is the only component that proves it.*

### A5 — nothing overwritten

**Home, security · safety**

Two columns, same archive, opposite outcomes. An archive member named after a file you already have passes every traversal check ever written — correctly, because it is not traversal. On the left the originals are replaced and gone. On the right they are untouched and the new files land beside them with a suffix.

| | |
|---|---|
| **Teaches** | Every create is `O_EXCL`; no truncating open exists in the codebase, so this is structural |
| **Duration** | 3.2 s · 420 ms per file, alternating columns |
| **Mechanism** | Attribute swap driving colour and strike-through; new rows fade in at reserved height |
| **Reduced** | Both end states side by side |

### A6 — the receipt

**Home, formats, security · conversions**

After a conversion completes, the receipt writes itself line by line — engine and version, parameters, class, limits, the sandbox profile that actually engaged, the hash of the exact bytes read. Ordinary JSON, and no competitor produces one.

| | |
|---|---|
| **Teaches** | "What exactly did you do to my file" has a literal answer |
| **Duration** | 2.4 s · 160 ms per line |
| **Mechanism** | Lines revealed in sequence inside a fixed-height `pre`; no reflow |
| **Reduced** | Whole receipt at once |

> **Rule:** always generated from a real run, never hand-written. A receipt on the marketing site that does not match what the binary emits is the one inconsistency this product cannot survive.

---

## 8. Visual system

The site uses the app's tokens so a screenshot sits on a page without a seam ([07 §13](07-DESIGN-SYSTEM.md#13-cli--web-parity)).

### Inherited unchanged

| | |
|---|---|
| **Neutrals** | Near-monochrome. Text `#1D1D1F` on white, inverting to `#EDEDED` on `#0A0A0A`. Never pure black on pure white — it reads as unfinished. |
| **Hierarchy** | Weight, size and opacity. Never hue. |
| **Accents** | Two, both semantic. Amber `#B4690E` means *this changes your file*; red `#C0392B` means *blocked*. Neither is decoration. |
| **Depth** | Hairlines and translucency, not shadows. Dark elevation gets lighter, not darker. |
| **Numerals** | Tabular everywhere. |
| **State** | Glyph, label, border and fill — four mechanisms, so nothing depends on colour. `=` lossless · `≈` lossy · `⌇` inferred · `✦` generated · `⛨` sandboxed. |

### Site-only, and only these three

| | |
|---|---|
| **Brand hue** | Verdigris `#2F6F6A` light, `#6FB3AC` dark. [07 §3.3](07-DESIGN-SYSTEM.md#33-accents--exactly-two) permits a brand hue *on the website and the icon, never in product chrome*. It sits far from both semantic accents in hue, so it can never be mistaken for state. |
| **Type scale** | 16 px body against the app's 13 px: the app is operated daily at arm's length, the site is read once by a stranger. Same faces, same two weights. Sans for prose, mono for every piece of system apparatus — routes, formats, sizes, flags, output. |
| **Motion** | The `motion-narrative` tier from [§6](#6-motion). Web only; it never ships into the app. |

**Layout.** Single column at 68 characters for prose. Demos and tables break wider. Hairline rules instead of cards; space does the grouping.

**Fonts.** Self-hosted and subset, including the five state glyphs — `⌇` (U+2307), `⛨` (U+26E8) and `✦` (U+2726) are almost certainly not in Inter. The app already gates this; the site uses the same glyphs and inherits the same gate. A tofu box on the page that exists to prove the accessibility claim is the worst possible place for one.

---

## 9. Gates

Executable, run on every build. **A budget that is not checked is a preference.**

| Gate | Threshold |
|---|---|
| JavaScript on any content page | 0 KB |
| Homepage JavaScript, conversion island lazy-loaded | ≤ 20 KB |
| Transferred weight per page, gzipped | ≤ 150 KB |
| Cumulative layout shift | 0 |
| Largest contentful paint, 4× CPU throttle | ≤ 1.2 s |
| External network requests at runtime | 0 |
| Words on `/privacy` | ≤ 300 |
| **Banned voice tokens across all product pages** | **0 matches** |
| Every claim on `/security` resolves to a mechanism | manual, pre-release |
| State glyphs resolve in the shipped font, no fallback | pass |
| Contrast, as rendered component pairs | 4.5:1 body · 3:1 non-text |
| Keyboard traversal and axe pass, every template | pass |
| Every sequence complete under `prefers-reduced-motion` | snapshot |
| Animated properties restricted to compositor-only | lint |
| `/privacy` identical to README and app settings | diff |
| `/download` checksums match the release artifacts | diff |

**The banned-token gate is the one that keeps this plan honest.** It greps every page under `/`, `/convert`, `/ai`, `/formats`, `/security` and `/download` for the phrases in [§2](#2-voice-rules) and fails the build on a match. That is what stops v0.1 happening again.

---

## Sources

Colour, type, materials and state encoding inherit from [07-DESIGN-SYSTEM](07-DESIGN-SYSTEM.md); the three site-only divergences are in [§8](#8-visual-system) and nowhere else. Channel priority and the route list come from [04-GROWTH](04-GROWTH.md). `/security` renders from [09-THREAT-MODEL](09-THREAT-MODEL.md). The conversion catalogue and `/formats` generate from the format and route tables described in [03-ARCHITECTURE §6](03-ARCHITECTURE.md#6-routing-without-a-graph-search).
