# OpenConvert — Growth, SEO/GEO & Site Plan

How people find OpenConvert, what the website is, and how the project sustains itself.

Status: design record, **v0.4** · 2026-08-13
Part 4 of 9 — see [README](README.md).

> **What changed in v0.4.** Three commitments in this document conflicted with what [08-EXECUTION-PLAN](08-EXECUTION-PLAN.md) and [09-THREAT-MODEL](09-THREAT-MODEL.md) actually build, and a commercial plan that spends a component the engineering plan defers is not a plan, it is a wish.
>
> | Was | Now |
> |---|---|
> | Revenue derived from "~3,000 orgs running the **server** build" — a component deferred out of v1 | The derivation runs on the **CLI and desktop** builds, which exist. The server is upside, and [its trigger](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires) is now countable |
> | A paid "signed **kill-switch feed**" — i.e. a polled endpoint the threat model forbade by default | The kill switch is **free and default-on for everyone**, inside the weekly signed manifest ([SR-9](09-THREAT-MODEL.md#5-requirements-to-tests)). What is paid is the **SLA on response time**, not the delivery of the fix |
> | Launch at month 6, from a 25-week build | **Launch at month 11**, from the [44-week build](08-EXECUTION-PLAN.md#3-weeks-544--outcomes). Every phase shifts; the ARR table is re-anchored, not re-forecast |
> | "Modules as the contribution surface" as the scaling answer, behind a trigger that could never fire | The module trigger is now countable, and until it fires **format coverage scales by us, not by contributors** — which is a real constraint, stated in §9 rather than assumed away |

---

## Contents

1. [The channel problem in 2026](#1-the-channel-problem-in-2026)
2. [The channel portfolio](#2-the-channel-portfolio)
3. [GEO — getting cited by AI engines](#3-geo--getting-cited-by-ai-engines)
4. [Comparison & alternative pages](#4-comparison--alternative-pages)
5. [Free tool pages](#5-free-tool-pages)
6. [Content assets](#6-content-assets)
7. [The site plan](#7-the-site-plan)
8. [Launch playbook](#8-launch-playbook)
9. [Community](#9-community)
10. [Monetization](#10-monetization)
11. [Metrics & roadmap](#11-metrics--roadmap)
12. [Risks](#12-risks)

⚠️ marks figures from secondary sources — verify before betting on them.

---

## 1. The channel problem in 2026

The obvious strategy — build "convert X to Y" tool pages and rank for them — is the **most zero-click-exposed asset available**:

| Signal | 2026 |
|---|---|
| Google searches ending with **no click** | ~60% (from 50% in 2019) ⚠️ |
| Queries showing an **AI Overview** | **58%** — up from 12% in 2024 ⚠️ |
| Position-1 CTR when an AI Overview is present | **−34.5%** ⚠️ |
| Average organic CTR, YoY | 1.41% → **0.64%** ⚠️ |
| Organic traffic loss from zero-click (Bain) | 15–25% ⚠️ |

One nuance helps: `heic to jpg` is a **tool** query, not informational. The searcher wants a thing that does the job, and an AI Overview can't convert their file. Tool queries are meaningfully more click-resilient. But it's a depreciating asset on a declining channel, and building only that is a single point of failure.

**And the counter-signal that reframes everything:**

> **AI-search visitors who click convert at ~23× the rate of traditional search visitors** ⚠️, and AI referral traffic is growing fast.

For a product whose conversion event is *a free download*, a 23× intent multiplier is enormous. Someone who asks an assistant *"what's the best offline converter that doesn't upload my files"* and is told **OpenConvert** arrives pre-sold.

The proof the channel is real at all: CloudConvert takes **81.6% of desktop visits from organic search**, iLovePDF reached 150M+ visits/month bootstrapped, FreeConvert grew 380K → 1.5M US organic in five years.

---

## 2. The channel portfolio

Ranked by efficiency — *(intent quality × durability) ÷ effort*.

| # | Channel | Effort | Intent | Zero-click risk | When |
|---|---|---|---|---|---|
| 1 | **GEO / AI citation** | Low–med | **Very high (23×)** | **Immune — it *is* the answer** | **Before launch** |
| 2 | **Alternative / comparison pages** | Low | **Very high (8.4%)** | Low | Phase 2, ~30 pages |
| 3 | **GitHub + docs SEO** | ~zero | Medium | Low | Day one |
| 4 | **The safety question space** | Medium | High | Medium | Flagship content asset |
| 5 | **Free tool pages (WASM)** | **High** | Medium | **High and rising** | Top ~10 only |
| 6 | **Directories & listings** | Very low | Medium | None | One afternoon, permanent |
| 7 | **Format reference** | Med (mostly generated) | Low–med | Medium | Compounding, feeds #1 |
| 8 | **YouTube** | High | Medium | Low | Later, if someone enjoys it |

**Two things to skip:** generic "how to convert X" posts (maximum zero-click exposure, zero differentiation) and paid search (bidding against CloudConvert's budget for traffic that converts to a *free download* is a losing trade).

---

## 3. GEO — getting cited by AI engines

The highest-leverage channel and the least contested. AI already handles an estimated **12–18% of informational searches** ⚠️, and it's the only channel where OpenConvert's differentiation compresses into a citable sentence.

### 3.1 Target prompts — the unit of work is a prompt, not a keyword

```
best offline file converter                    how do I convert files without uploading them
is it safe to use online file converters       open source alternative to CloudConvert
convert HEIC without uploading                 self-hosted document conversion API
local AI OCR that runs offline                 how to sanitize a PDF from an untrusted source
GDPR-compliant document conversion             convert files on-premise for compliance
does converting HEIC to JPG lose quality       what is content disarm and reconstruction
best open source PDF tools                     how to remove metadata before sharing a file
```

### 3.2 What earns citations

| Tactic | Why it works for OpenConvert specifically |
|---|---|
| **Specificity over adjectives** — "background removal in a 5 MB model, ~2 s/image on CPU" beats "fast and lightweight" | The whole story is *numbers*: 207,000 searches, a 500 MB AI stack, ≤80 ms p99 sandbox overhead, 81.6% organic. Unusually citable. |
| **Original data nobody else has** | The converter audit (§6) is the only source for its measurements, so any model answering "do online converters strip your metadata?" has one place to go |
| **Direct-answer structure** — question as heading, answer in two sentences, evidence below; tables and lists extract cleanly | Cheap, and it makes better docs anyway |
| **Third-party corroboration** — LLMs weight independent mentions heavily (Reddit, HN, GitHub, awesome-lists, AlternativeTo) | **This unifies community work with SEO: they are the same effort.** Every genuine Reddit answer is both traffic and a retrieval signal. |
| **Be the clearest documentation in the category** | Required for the product anyway. Zero marginal cost. |

### 3.3 Measurement loop

Weekly, ~30 fixed prompts through ChatGPT, Claude, Perplexity, and Google AI Overviews. Log who is cited and why. That log tells you what to write next and is the only honest measure of whether any of this works.

**Phase-2 goal:** cited by ≥2 major AI engines on the core prompts.

---

## 4. Comparison & alternative pages

Published benchmarks: **8.43% visitor-to-lead for "alternatives" keywords**, **5.45% for "versus"** ⚠️ — against a fraction of a percent for informational content. These searchers are *already looking to switch*, and the queries resist zero-click because people want to compare, not be told.

~30 pages:

| Cluster | Pages |
|---|---|
| **Cloud converters** | CloudConvert · Convertio · Zamzar · FreeConvert · Online-Convert |
| **PDF suites** | iLovePDF · Smallpdf · Adobe Acrobat (open source alternative) · Sejda |
| **Self-hosted** | Gotenberg · ConvertX · Stirling PDF · "self-hosted CloudConvert" |
| **AI point tools** | Upscayl · remove.bg · offline transcription (Otter/Descript) |
| **Security** | Dangerzone · "open source CDR" · OPSWAT / Votiro / Glasswall |
| **Category** | best offline file converter · best privacy file converter · best open source PDF converter · file converter without upload |

**Do these honestly.** Say where the competitor is better — Gotenberg genuinely wins on `html→pdf` maturity today. A fair comparison outranks and outlasts a dishonest one, gets cited by AI (which cross-checks), and doesn't blow up when the vendor reads it.

---

## 5. Free tool pages

Still worth building — as *proof*, not as the growth engine.

**Top 10 only**, chosen for demand × strategic fit:

1. `heic to jpg` (**207,000**) — pure codec, instant, perfect first impression
2. `pdf to word` (**220,000**) — the flagship; OCR + layout quality is the differentiator
3. `image compressor` (**348,000** as "file compressor") — metric-guided compression genuinely beats competitors
4. `background remover` — a 5 MB model running in-browser, feels like magic
5. `mp3 converter` (**263,000**)
6. `image to text (OCR)` — 21 MB model, in-browser
7. `video to mp3` (**64,000**)
8. `png ↔ jpg / webp` cluster
9. `compress pdf` · `merge pdf` · `jpg to pdf`
10. `pdf to excel` — highest commercial intent in the whole list

**The conversion runs in the visitor's browser via WASM.** The page doesn't claim locality — it *demonstrates* it.

**What this needs from the architecture.** `openconvert-core` is pure and compiles to `wasm32` unchanged. `openconvert-run` does not — it is subprocesses and OS sandboxes. So the tool pages pair the core with a thin in-page executor over a handful of pure-Rust engines (image-rs, resvg, Symphonia), and report `Sandboxed { filesystem: BrowserOrigin, network: Csp, … }` — no new isolation variant, because [the profile is a value](03-ARCHITECTURE.md#9-isolation-the-profile-and-the-floor). That is the whole integration, and it is why the tool pages cover images, SVG and audio rather than the full catalogue.

**And the page says what that profile is worth.** `BrowserOrigin` confines what the *page* can reach; it does not confine `image-rs` from the rest of the page's memory, and `Rlimit` in a browser is a memory ceiling, not a wall clock. So the profile scores `Reduced`, never `Full`, and the tool page's confinement badge says `sandboxed · reduced — browser origin` exactly as the desktop app would. A marketing surface that flattered its own isolation would undermine the one claim the whole site exists to make.

**On data.** These pages are the eventual training source for a learned ranker, and the only thing they may record is the **`(from → to)` pair, aggregated** — no file, no name, no size, no hash, no identifier. The page says so in one line, above the fold. A privacy product that quietly instruments its own marketing site has lost the argument it is making.

---

## 6. Content assets

### 6.1 "What your file converter actually did to your file" — the flagship

Take the top 10 online converters, run identical inputs, measure: silent re-encodes, metadata retention, GPS leakage, quality loss, colour shifts. Publish the methodology and the raw data.

Highest-upside piece available: factual, reproducible, it *is* the product's argument, and it's original data no model can source elsewhere. **Be rigorous and fair** — name methods, let readers reproduce. Sloppy competitor-bashing would backfire.

### 6.2 The safety question space

"Are online file converters safe?" is a genuine anxiety with a hard news peg — the **FBI's March 2025 warning** that free converter sites were distributing malware. Nobody owns this question space, it attracts links, it feeds GEO, and OpenConvert is literally the answer.

Cluster: *are online converters safe · what happens to files you upload · do converters keep your files · how to convert sensitive documents · does converting strip EXIF and GPS · can a PDF contain malware · what is CDR.*

### 6.3 Security deep-dives

"How we sandbox media engines on three operating systems." "Why we refuse to ship Ghostscript." "FFmpeg licensing, precisely." This audience becomes the enterprise pipeline.

### 6.4 Small-model engineering

"Background removal in 5 MB." "The complete local AI stack for document conversion is 500 MB." Counter-narrative content travels.

### 6.5 The file-format reference

Not tool pages — an authoritative encyclopedia: what each format is, its history, what it converts to losslessly, what it silently loses. Evergreen, link-attracting, and **the natural retrieval source when an assistant is asked "does converting HEIC to JPG lose quality?"**

Nearly free: the format and route tables already contain the conversion matrix, so most of it generates from data you have — the same data `openconvert routes` and `openconvert formats` print.

---

## 7. The site plan

`openconvert.dev` — static, fast, no trackers. The site must embody the product's claims, because visitors will check.

```
/                          the pitch + live in-browser demo above the fold
/download                  platform detection, checksums, signatures, package managers
/convert/<from>-to-<to>    the 10 WASM tool pages
/formats/<format>          the format reference (generated from the format + route tables)
/vs/<competitor>           the ~30 comparison pages
/security                  threat model · what we don't protect against · security.txt · advisories
/privacy                   what leaves your machine, ever (short enough to read)
/docs                      CLI · recipes · API · self-hosting  (server + modules when they exist)
/enterprise                CDR + sovereign document AI — for CISOs and DPOs
/blog                      the content assets
/models                    every shipped model, its license, size, and what it does
/roadmap                   public, with the non-goals list
```

### Pages that earn their place

| Page | Why it's unusual |
|---|---|
| **`/`** | The demo is **the real thing**, running in WASM. Drop a file on the homepage and watch the plan preview appear with `⇄ stream copy · lossless`. Nobody's landing page proves its claim in three seconds. |
| **`/security`** | Including a **"what we don't protect against"** section. Dangerzone does this and it earns more trust than any marketing claim. Also a procurement artifact. |
| **`/privacy`** | Short enough that people actually read it. The never-collected list, verbatim-identical to the README and the settings screen, so any drift is visible. |
| **`/models`** | Every model with its **license and commercial-use status**. Enterprise compliance teams ask for exactly this, and no competitor can publish it. |
| **`/formats`** | Generated from the format + route tables — hundreds of evergreen pages at near-zero marginal cost, feeding GEO. |

### Technical

Static generation, no client-side framework on content pages, no analytics beyond privacy-respecting first-party (self-hosted Plausible-class). Schema.org `SoftwareApplication` and `FAQPage` markup. Fast because it's a claim about the product.

---

## 8. Launch playbook

### What the data says

| Finding | |
|---|---|
| HN exposure yields ~**121 stars in 24h**, ~189 in 48h, ~**289 in a week** (mean, AI tools) | arXiv 2511.04453 ⚠️ |
| Submit **"Show HN: [name], a [short description]"** at **8–9 AM ET** | ⚠️ |
| For self-hostable tools, **r/selfhosted is essential** | ⚠️ |
| Progress posts to r/selfhosted *before* a Show HN produced **1,200 stars in 3 months** | ⚠️ |
| README should be a **product page**: GIF above the fold | ⚠️ |
| After ~6,000 stars, one project **stopped broadcasting and started 1:1 calls** | ⚠️ |

### Pre-launch — 8 weeks

Build in public on r/selfhosted with substantive posts (*how we sandboxed FFmpeg*, *why your converter is re-encoding files it shouldn't*) — not marketing copy; Reddit detects that instantly. README as product page: one GIF of drag → plan preview → receipt. **Docs finished before launch** — a spike with broken docs converts nobody. Line up 3–5 friendly reviewers. Pre-write the audit content so it can ship in week 2 while attention holds.

### Launch day

| Time | Action |
|---|---|
| T-1 | Docs live, binaries signed and notarized ×3, Docker pushed, package-manager submissions in flight |
| 08:00 ET | **Show HN: OpenConvert, a local file converter that sandboxes every engine and tells you what it did to your file** |
| 08:15 | r/selfhosted — different framing, genuine, asking for feedback |
| 09:00 | Lobsters, Product Hunt; HN comment engagement begins |
| All day | **Answer every comment.** This is the entire job for 48 hours. Ship visible fixes same-day. |
| T+2d | r/privacy, r/opensource, r/degoogle, PrivacyGuides |
| T+1w | `awesome-selfhosted`, `awesome-privacy`, `awesome-rust`; first technical deep-dive |

**Headline discipline:** lead with the mechanism, not adjectives. "Sandboxes every conversion engine" beats "secure and private." HN rewards specificity and punishes marketing.

### Distribution checklist

`winget` · Homebrew (cask + formula) · AUR · Flathub · Snap · Nix · Chocolatey · Scoop · Docker Hub + GHCR · Helm repo · `awesome-selfhosted` · `awesome-privacy` · AlternativeTo · Product Hunt · Slant · Wikipedia comparison tables.

Unglamorous, and the highest-ROI growth work after launch. Every package manager is a permanent, zero-maintenance acquisition channel.

---

## 9. Community

- **License promise in the README, in writing:** the conversion engine is Apache-2.0 and will never be relicensed. Name the HashiCorp/Redis precedent and commit against it. This preempts the #1 objection open-source infrastructure faces in 2026.
- **Public roadmap and an explicit non-goals list.** Non-goals prevent the exhausting "will you add X" treadmill.
- **Contribution, honestly.** v0.3 said "modules as the contribution surface — that's how format coverage scales without the team scaling," while [03-ARCHITECTURE](03-ARCHITECTURE.md) deferred the module system behind a trigger that could not fire: *nobody can want a module to exist for a system with no ABI.* **Until the module system ships, format coverage scales by us.** The contribution surface that actually exists is the one that should be documented: a route-table entry, a format row, and an adapter inside an existing worker crate — a documented path with a template, which is exactly what [weeks 15–17](08-EXECUTION-PLAN.md#3-weeks-544--outcomes) build. **The third external pull request that adds a format adapter is the module system's trigger**, and it is countable, which is the point.
- **Recognize contributors visibly**; module authors listed in-app.
- **At ~5,000 stars, switch modes:** stop broadcasting, start 30-minute calls with everyone who engages deeply. That's where design partners and the first contracts come from.

---

## 10. Monetization

### 10.1 The reframe

**Do not sell file conversion to enterprises.** The reference price is $8/month (CloudConvert), nobody has a "file conversion" budget line, and free good alternatives already exist. Sell into three budgets that *do* exist:

| Budget line | Market | What OpenConvert already is |
|---|---|---|
| **Secure file ingestion (CDR)** | ~$572M–$1.45B (2026) ⚠️, ~15.9% CAGR ⚠️ | Per-engine OS sandboxing with a published, audited boundary *is* most of a CDR product — and **there is no credible open-source CDR product.** Paranoid mode completes it, and is the deferred piece whose named trigger is exactly this buyer. |
| **Sovereign / on-prem document AI** | IDP est. $4.3B–$14.2B (2026) ⚠️ | Local OCR, layout, ASR, translation with a zero egress surface |
| **Conversion API replacement** | CloudConvert-class spend | The headless server |

**Timing:** EU AI Act Article 16 obligations for high-risk systems took effect **2 August 2026**. Combined with GDPR Art. 4(2) (inference *is* processing, so an out-of-region model call is itself a transfer) and the EDPB naming on-premise inference the strongest available mitigation, there is a live, funded scramble toward what OpenConvert does by construction. Receipts are already close to an Article 16 logging artifact.

### 10.2 The paid boundary

> **Everything required to convert a file is free forever, Apache-2.0, ungated, offline, no account.**
> **What is paid is fleet governance and organizational obligation.**

| Always free | Paid |
|---|---|
| Every format, every conversion, every AI tool | **Control plane**: SSO/SCIM, RBAC, fleet policy, audit warehouse |
| Desktop, CLI, MCP — and the headless server when it exists | **Security-response contract**: a contractual **CVE triage and patch SLA**, backported patches, a private mirror, and a named engineer |
| Sandboxing, Paranoid mode, receipts, **and the engine kill switch** | **Attested builds**: reproducible-build attestation — **scoped to a fixed container image and build path**, which is what is actually achievable — SBOM, FIPS packaging |
| Self-hosting at any scale, air-gapped | **Compliance evidence packs**: AI Act docs, model cards, DPIA templates |
| All public modules and models | **Private registry**, curated validated module sets |
| Community support | **Support & services**, migration, training, **IP indemnification** |

This boundary survives rug-pull scrutiny: the licence on the conversion engine never changes, because the business was never built on it.

### 10.3 Pricing hypotheses

Anchors: Rancher **$1,200–1,800/node/yr** · Grafana self-managed **$100–500K/yr** · LiteLLM enterprise **~$30K/yr** · CloudConvert **$8/mo → $200–500/mo** ⚠️.

Note the two-order-of-magnitude gap between the *conversion* anchor and the *infrastructure* anchor. **Which anchor you're compared against is worth more than any pricing tactic.**

> **The kill switch is not a paid feature.** v0.3 listed a "signed kill-switch feed" as an Assured deliverable, which would have meant the free product's protection against an actively exploited engine CVE was to wait until someone happened to update. The revocation list rides inside the weekly signed manifest that every default install already fetches ([SR-9](09-THREAT-MODEL.md#5-requirements-to-tests)), it works from cache with the network off, and it is free forever. **What Assured buys is the contractual clock** — triage < 72 h, patch-or-disable published < 7 days, a named engineer, and a private mirror for air-gapped fleets. Selling the response time is legitimate; selling the ability to be protected at all would have been a dark pattern in a product whose entire pitch is trust.

| Tier | Contents | Hypothesis |
|---|---|---|
| **Community** | Everything, forever — **including the kill switch** | **$0** |
| **Assured** | Contractual security SLA, patch backports, private mirror, named engineer, business-hours support | $15–30K/yr |
| **Secure Ingest (CDR)** | + CDR deployment, gateway integration, validated engine set, attested builds | $2,500–5,000/node/yr, floor ~$40K |
| **Regulated** | + control plane, AI Act evidence pack, air-gapped bundles, indemnification, 24×7 | $90–250K/yr |
| **Services** | Migration, pipeline design, custom modules, training | $1,800–2,500/day |

**Price on worker nodes, not seats or conversions.** Seats don't fit a headless gateway; per-conversion re-anchors you against $0.008 and punishes adoption.

### 10.4 The motion, and its honest handicap

Bottom-up land (an engineer kills a CloudConvert bill) → the security team notices → someone needs an audit log or a patch SLA → **first conversation**. Do not interrupt the free stage: no sign-up walls, no "contact sales" in the docs, no nag banners.

**Zero telemetry means no pipeline visibility.** You will not know who your users are. Legitimate signals: opt-in registry/mirror pulls, docs analytics, GitHub issues from corporate domains, a low-key "for organizations" page, community presence, conference contact. Budget a longer cycle than a telemetry-instrumented competitor. This is a real cost of the principles — pay it knowingly.

**Get five design partners** before building any paid tier: one bank/insurer, one healthcare or government org, one legal/e-discovery shop, one EU enterprise with a live AI Act problem, and one journalism/NGO (free, for the reference).

### 10.5 Compliance gates

| Gate | When | Cost ⚠️ |
|---|---|---|
| security.txt, threat model, SBOM, disclosure policy | Launch | ~free |
| Third-party security audit (sandboxes, capability enforcement, update chain) | Before first paid deal | $25–60K |
| **SOC 2 Type II** | Before enterprise contracts | $20–50K/yr + 6–12 months observation |
| ISO 27001 | EU deals | $15–40K/yr (reuse the ISMS) |
| EU AI Act Art. 16 docs | Now, for EU deals | Internal |
| FedRAMP | — | **$500K+. Do not pursue** — sell through authorized integrators |

Because OpenConvert processes nothing on our infrastructure, **SOC 2 scope is dramatically smaller than for a SaaS vendor** — the audited system is the build/release/registry pipeline, not a data plane. Worth saying out loud in sales conversations.

---

## 11. Metrics & roadmap

| Phase | Months | Growth milestone | Commercial | Target |
|---|---|---|---|---|
| **P1 Launch** | 11–13 | Show HN; security.txt + **threat model** published; GEO prompt loop running | — | **2,000+ stars, 10k downloads** |
| **P2 Depth** | 13–16 | ~30 comparison pages; top-10 WASM tools; converter audit published | **5 design partners** | 10k stars; **cited by ≥2 AI engines** |
| **P3 Trust** | 16–20 | **Third-party audit of `openconvert-sandbox` + `openconvert-os` published**; format reference live | **First 3 Assured contracts**; SOC 2 clock starts | **$50–90K ARR** |
| **P4 Enterprise** | 20–26 | Enterprise content; conference talks | SOC 2 issued; first Secure Ingest deals | **$300–600K ARR** |
| **P5 Scale** | 26–32 | Partner channel | First Regulated contract | **$1–2M ARR** |

*Launch is **month 11**, not month 6. The build is [44 weeks with two engineers](08-EXECUTION-PLAN.md#3-weeks-544--outcomes), ten of them on the security boundary and twelve on the engine workers. v0.3 said 25 weeks and did not state a headcount; that is the number every row of this table used to hang from, so every row moved. **The ARR targets are unchanged and simply arrive five months later** — this is a re-anchoring, not a re-forecast, because nothing about the market changed.*

*Assumes **two engineers through launch**, matching [08 §8](08-EXECUTION-PLAN.md#8-weekly-rhythm), then a third plus a commercial hire from month 13. A solo build is ~70 weeks: divide revenue by ~3 and add six months. Enterprise security cycles run 6–12 months, and the clock starts when someone contacts you — which, with no telemetry, is later than you'd like.*

*The audit in P3 is cheap and meaningful precisely because the boundary is two small crates rather than a whole shell — and because one of them contains no `unsafe` at all, so the auditor can be told exactly where memory safety is a question and where it is not.*

### Financial sketch at month 24

| | Conservative | Base | Upside |
|---|---|---|---|
| Stars | 8K | 20K | 50K |
| Assured (~$22K) | 6 → $132K | 14 → $308K | 25 → $550K |
| Secure Ingest (~$70K) | 1 → $70K | 4 → $280K | 9 → $630K |
| Regulated (~$140K) | 0 | 2 → $280K | 5 → $700K |
| Services | $60K | $150K | $250K |
| Sponsorship / grants | $30K | $80K | $150K |
| **Total ARR** | **~$290K** | **~$1.1M** | **~$2.3M** |

**Where the customers come from** (conservative): 8K stars → ~200K downloads → ~3,000 orgs with **OpenConvert in a scripted or CI pipeline via the CLI** → ~150 that are >100 employees and production-dependent → ~30 reaching a trigger event over two years → **~12 that actually contact you** (⚠️ the load-bearing assumption — with no telemetry they must go looking) → ~8 close.

> v0.3 derived this from "orgs running the **server** build," which [03-ARCHITECTURE §15.3](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires) defers out of v1 — so the conservative case rested on a component the plan does not build. The CLI is the substitute and it is a fair one: `openconvert convert --json` in a CI job is the same bottom-up landing motion, and it is what actually ships. **The server remains genuine upside**, and if its trigger fires early this row gets better, not worse.

**Without the CDR positioning**, the conservative case barely moves (~$293K, mostly support + services) — but the ceiling collapses: base ~$450K, upside ~$800K. **CDR is an upside lever, not a survival lever.**

**Cost side at ~$293K:** security engineer $120–150K (mandatory, see below) + second engineer + SOC 2 $20–50K + audit + infra ≈ **$260–360K**. The conservative column supports two people and its compliance obligations with nothing left over. It's survival — it buys time to learn whether the base case is real.

---

## 12. Risks

| # | Risk | Sev | Mitigation |
|---|---|---|---|
| G1 | **Zero-click erodes the tool-page channel** faster than it compounds | Med–High | The portfolio (§2): lead with GEO and comparison pages; cap WASM pages at ~10 |
| G2 | **GEO is a channel we don't control** — providers change citation behavior without notice or appeal | Med | Never single-source it; the weekly prompt log is the early warning; owned channels (docs, GitHub, packages, community) stay the floor |
| G3 | **No telemetry = no pipeline visibility** | High | Opt-in registry signals, docs analytics, community investment, deliberate conference presence. Accept a longer cycle. |
| G4 | **The support-only model is too slow** to fund the team | High | Build the control plane earlier than feels comfortable; take services revenue without embarrassment; pursue grants (privacy and digital-infrastructure funders are a genuine fit) |
| G5 | **Selling an SLA on third-party CVEs.** You're contractually on the hook for Ghostscript/ImageMagick/FFmpeg advisories in code you don't control and can't schedule. | **High** | Minimize the engine set; **Ghostscript excluded from core and from the SLA**; sandbox tiers downgrade most CVEs from critical to low *for our users* — a sellable claim; the kill switch makes "disable" a valid <7-day response; price it to fund one dedicated engineer (~7 Assured customers) |
| G6 | **Enterprises won't buy from a tiny team** | High | Design-partner references; published third-party audit; reproducible builds as the "if we vanish" answer; integrator channel |
| G7 | **Codec patent exposure** in a commercial context | High | Royalty-free defaults; patented encode only via OS/hardware encoders; **core ships no H.264/AAC encoder at all**; counsel before the first paid contract |
| G8 | **"Rug-pull" suspicion** poisons adoption pre-emptively | Med | Written, permanent licence commitment; a business model that structurally doesn't need to relicense |
| G9 | **Launch flops** — HN is a lottery | Med | Launch is repeatable, not one-shot: pre-build the r/selfhosted audience, hold the audit content as a second spike, conference talks as a third |
| G10 | **Market figures here are syndicated estimates** with wide spread | Med | Validate with five design-partner conversations before betting the roadmap |
| G11 | **Search volumes in [02-FEATURES](02-FEATURES.md) are mostly banded estimates** | Med | One hour in Ahrefs replaces every estimate with measured volume, CPC and difficulty. Do it before Phase-2 page work. |
| G12 | **The build slips past 44 weeks**, pushing launch past month 11 and every phase with it | **High** | The two ⚠️-marked phases ([boundary and breadth](08-EXECUTION-PLAN.md#3-weeks-544--outcomes)) are where the error will land, and both have an early signal: the week-9 Tauri/AppContainer gate and the week-17 "one worker, signed on three platforms, in CI" milestone. **If week 17 slips, cut the format catalogue, not the boundary** — twenty formats sandboxed beats forty unconfined, and it is the same launch story |
| G13 | **The build-in-public audience is promised for week 1 and the product lands at week 44** | Med | Ten weeks of sandboxing content is a *better* pre-launch asset than four, not a worse one — but it must be *the work*, published as it happens, not progress theatre. If there is nothing real to say in a given week, say nothing |

---

## Decisions needed

1. **Adopt the CDR positioning?** Highest-leverage decision here; changes the README, the site, the launch headline, and which conferences you attend. *Validate with 5 buyer conversations first — cheap to test, expensive to reverse.*
2. **Where exactly does the paid boundary sit?** §10.2 proposes it. Commit publicly and early; ambiguity makes communities anxious.
3. **Control plane: proprietary or source-available?** *Recommend source-available* — more consistent with the ethos and materially easier for security-conscious buyers to approve.
4. **Funded or bootstrapped?** VC pressure runs toward exactly the gating and telemetry the product forbids. *Recommend bootstrap + grants; revisit if CDR proves out.*
5. **Commercial hire by month 8–10.** Engineers selling to CISOs part-time is how this stalls at $200K.
6. **Build the WASM tool pages?** *Yes, but top ~10 only, sharing the Rust core compiled to `wasm32`, not before Phase 2.*
