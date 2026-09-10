# OpenConvert — Site research

What the competition's **websites** actually do, what the category rewards, and what that means for `openconvert.dev`.

Status: research, **v1.0** · 2026-08-24
Companion to [10-WEBSITE](10-WEBSITE.md), which is the spec. This is the evidence under it.
Product-level competitive analysis lives in [05-RESEARCH §1](05-RESEARCH.md#1-market--competitive-research) and is **not** repeated here — that document compares *products*, this one compares *pages*.

---

## Contents

1. [Why this document exists](#1-why-this-document-exists)
2. [The category, by traffic](#2-the-category-by-traffic)
3. [Site teardowns](#3-site-teardowns)
4. [The trust event](#4-the-trust-event)
5. [The crowded middle](#5-the-crowded-middle)
6. [Patterns that work](#6-patterns-that-work)
7. [Patterns that fail](#7-patterns-that-fail)
8. [What nobody's site does](#8-what-nobodys-site-does)
9. [Recommendations](#9-recommendations)
10. [Sources](#10-sources)

---

## 1. Why this document exists

The site was rebuilt against [10-WEBSITE](10-WEBSITE.md) and judged wrong. Before rebuilding again it is worth knowing what the pages we are competing with actually look like, because the failure is unlikely to have been the token list or the gate table — those are execution details, and they passed. The likelier failure is that the site was designed from our own design record outward rather than from the visitor inward.

**A test worth applying to anything built next.** Every page below was read as a stranger would read it: what is the first thing you can do, how long before you can do it, and what do you have to believe to proceed. That is the frame. Not "does this page describe the architecture accurately".

---

## 2. The category, by traffic

Two numbers reframe the problem.

| Site | Monthly visits | Model |
|---|---|---|
| **iLovePDF** | 163M (Jul 2026) · peaked ~226M (Jan 2026) | Cloud, tool grid, freemium |
| **Smallpdf** | ~40M (Oct 2025) | Cloud, tool grid, freemium |
| **CloudConvert** | not published in sources found | Cloud, single widget, usage-priced |
| **VERT.sh** | not published; **15.4k GitHub stars** | Browser-local, AGPL-3.0 |
| **Stirling-PDF** | self-hosted; **~81k GitHub stars, 25M downloads** | Self-hosted, Docker |

**CloudConvert takes 81.56% of desktop visits from organic search.** That single figure is the category's shape: these are not brands people remember, they are pages people land on from a query and leave. iLovePDF's core audience is India, then Indonesia, then Brazil — a volume-and-ads business, not a considered-purchase one.

**The implication for us is uncomfortable and worth stating plainly.** We are not going to win that game. A signed local binary cannot outrank iLovePDF on `pdf to word` at 163M visits/month, and the strategy in [04-GROWTH](04-GROWTH.md) does not claim otherwise — it treats the tool pages as *proof*, not as the engine. The site's job is therefore **conversion of a small qualified audience**, not capture of a large unqualified one. That is a different site.

> **Stirling-PDF is the more relevant benchmark.** 81k stars and 25M downloads from a self-hosted Docker container with no consumer funnel at all. The audience that installs software to avoid uploading files is real, it is large, and it is reachable through GitHub, Hacker News and word of mouth rather than through `/png-to-jpg`.

---

## 3. Site teardowns

### 3.1 CloudConvert — the well-run incumbent

| | |
|---|---|
| Headline | "Convert Any File" |
| Sub | "Drop a file and pick what to turn it into. CloudConvert handles 200+ formats across documents, images, audio, video, archives and more — straight from your browser." |
| Above the fold | Two-step widget: format pair (defaulting to PDF → DOCX), then a drop zone |
| Formats | "212 formats across 11 categories", tabbed by category with counts |
| Trust | A **Data Security** section: files "processed for the conversion job you request, then removed after processing"; independent ISMS audit; "CloudConvert does not sell customer file data nor mine any data" |
| Footer | About, Security, Blog, Status, Privacy, Terms, Imprint, Contact · "Made in Munich, Germany" |

**What it does well.** The widget is above everything and defaults to the most common pair, so the page is usable before it is read. Format counts are specific per category. The footer names a jurisdiction and a company, which is a trust signal ad-supported converters cannot match.

**Where it is exposed.** Every claim is a promise about their conduct — deletion policy, audit, "does not sell". None of it is checkable by the visitor. The phrase "straight from your browser" describes where the *upload form* is, not where the conversion happens. That gap is the whole opening.

**Worth stealing:** pinnable engine versions (from [05-RESEARCH §1.3](05-RESEARCH.md#13-deep-profiles)) is a reproducibility feature nobody else has, and it is not surfaced on the homepage at all.

### 3.2 Convertio — the volume play

| | |
|---|---|
| Headline | "Convert your files to any format" |
| Claims | "300+ formats supported" · "more than 25600 different conversions" |
| Locality | **"All conversions take place in the cloud"** — stated as a feature |
| Limits | "1 GB maximum file size or Sign Up" |
| Trust | "We delete uploaded files instantly and converted ones after 24 hours"; "privacy is 100% guaranteed" |
| Social proof | Logo carousel (Facebook, Amazon, Microsoft, Google), "4.6" from 31M+ votes |

**"Privacy is 100% guaranteed" next to "all conversions take place in the cloud" is the sentence our entire product is an argument against.** It is not dishonest by their lights — they mean they delete files. It is simply a guarantee of conduct where ours is a guarantee of architecture. Quoting the contrast is more effective than asserting a difference.

### 3.3 iLovePDF — the traffic leader

Tool **grid**, not a single widget: ~6 tools above the fold, categorised (All, Workflows, Organize, Optimize, Convert, Edit, Security, Intelligence). Headline: "Every tool you need to work with PDFs in one place". Trust is delegated to third-party badges in the footer — ISO 27001, HTTPS, PDF Association membership. Premium upsell includes **offline desktop access** as a paid feature. 24 locales.

**Two lessons.** First, at the top of this market the unit is a *task* ("merge PDF"), not a *format pair* — people arrive wanting an outcome. Second, iLovePDF charges for offline. We give it away, and that is a headline, not a footnote.

### 3.4 VERT.sh — our closest analogue

| | |
|---|---|
| Headline | "The file converter you'll love." |
| Key claim | "All image, audio, and document processing is done on your device." |
| Sub-claims | "No file size limit, no ads, and completely open source" |
| Interaction | One drop zone: "Drop or click to convert" |
| Nav | Upload · Convert · Settings · About |
| Footer | Source code, Discord, privacy policy, **commit hash** |
| Licence | AGPL-3.0 · 15.4k stars · Svelte + TypeScript, engines compiled to WASM |
| The asterisk | Video is **not** local on the public instance — it goes to `vertd`, a self-hostable Rust+FFmpeg daemon. The tagline itself carries the asterisk: "fully local\*" |

**What VERT gets right, and we should copy without embarrassment.**

1. **One drop zone, no format pickers before the file.** You do not choose a conversion, you present a file. That is our prediction thesis and they shipped the interaction first.
2. **A commit hash in the footer.** Cheap, and it says "this page corresponds to a build you can read".
3. **The honest asterisk.** They put the limitation in the tagline. It costs them nothing and buys the rest.

**What the Hacker News reception exposes** ([thread](https://news.ycombinator.com/item?id=43663865)) — and this is the most useful single artefact in this document, because it is our audience reacting to our nearest neighbour:

- **Hidden analytics cost them trust immediately.** *"why hide it at the bottom of settings? It just makes me trust you less."* Plausible analytics, disclosed only in a settings pane, read as concealment.
- **Sustainability was read as a threat.** *"I see no way to support that long term, unless they are doing something more."* A free tool with no visible business model invites the assumption that the user is the product.
- **Missing attribution was noticed.** Upstream library credits *"was added in the last day"* — after launch, after comment.
- **AGPL made deployers nervous:** *"be extra-sure that you understand the license before deploying."*
- **WASM's ceiling was raised unprompted:** 2 GB limits, slower than native.
- **The strongest recurring theme was the FBI warning** — commenters brought up malware-injecting converter sites unprompted, as the *reason* to want this.

Every one of those five is a page on our site, or a paragraph on one.

### 3.5 Dangerzone — the security-first sibling

| | |
|---|---|
| Headline | "Dangerzone: Convert potentially dangerous documents into safe PDFs" |
| Tagline | "Take potentially dangerous PDFs, office documents, or images and convert them to safe PDFs." |
| Mechanism | Explained in plain prose: convert to PDF, render to pixels, rebuild. "sanitized in a sandbox with no network access" |
| Sections | Nav → logo + tagline → four feature bullets → news → download → How It Works → footer with signing key |
| Backer | Freedom of the Press Foundation, "a nonprofit organization that protects and defends press freedom" |
| Tone | Casual, emoji, accessible to non-technical readers |

**A correction to an assumption embedded in [10-WEBSITE §5](10-WEBSITE.md#5-page-specifications).** That spec asserts Dangerzone publishes what it does not protect against, and cites it as precedent. **Their homepage does not.** It contains no limitations section at all — the residual-risk discussion (LibreOffice → gVisor → kernel chain) lives in their docs and threat model, not on the marketing page. The technology stack is not named on the homepage either; gVisor and containers are absent.

So publishing our limits on `/security` is not "what Dangerzone does". **It is a genuinely unusual thing to do**, and it should be presented as our choice rather than as an industry norm we are matching. That is a stronger claim, not a weaker one — but the spec's justification for it was factually wrong and would not have survived a reader who checked.

**What Dangerzone gets right:** the tagline states the threat and the outcome in one sentence, and the mechanism is explained without a single piece of jargon. Someone non-technical understands "renders to pixels and rebuilds" immediately.

### 3.6 Ollama — the local-first reference

Not a competitor; the best available model for how to sell "runs on your machine".

- Headline: "Build with open models, on your computer and in the cloud"
- **Extremely sparse.** Short phrases, not paragraphs. Three words carry the privacy argument: *Private. Disconnected. Open.*
- One repeated CTA: **Download**, in header, hero and footer
- Social proof through model pull counts, not testimonials
- Nav is three items: Models, Docs, Pricing

**The lesson is discipline.** Ollama has a far more complex product than a file converter and its homepage has perhaps a fifth of the words the current OpenConvert homepage has. Density reads as anxiety.

### 3.7 Tailscale — the reference for teaching a mechanism

[How Tailscale works](https://tailscale.com/blog/how-tailscale-works) is the best trust artefact in adjacent infrastructure, and worth studying for `/security`:

- **Bottom-up**, in build order: *"Let's go through the entire Tailscale system from bottom to top, the same way we built it."*
- **Seven diagrams, each isolating one concept.** No diagram carries two ideas.
- **Limitations stated as they arise**, not quarantined into a caveats section: *"Some especially cruel networks block UDP entirely"* — followed immediately by what they do about it.
- ~3,500 words. The length itself signals education over persuasion.
- Personal asides and dated references humanise it without costing credibility.

**Applicable directly:** our `/security` page should be one long, ordered, diagrammed explanation in build order, not a set of claim-cards with a limitations table bolted on the end.

---

## 4. The trust event

The single most valuable fact available to this product, and it is external, dated and citable.

**March 2025 — the FBI Denver Field Office warned publicly that free online file-converter tools were being used to deliver malware.**

| | |
|---|---|
| What the tools do | Perform the advertised conversion *and* install malware — `.doc` → `.pdf`, combining `.jpg` images, MP3/MP4 downloaders |
| What is harvested | SSNs, banking credentials, crypto wallet addresses and seed phrases, email addresses and passwords, browser cookies and autofill |
| Outcome | Named as leading to ransomware incidents |
| FBI wording | *"Cyber criminals across the globe are using any type of free document converter or downloader tool"* to inject malware and scrape data |
| FBI recommendation | **Use built-in conversion tools in existing applications** |

**Independently confirmed, not just an advisory.** BleepingComputer (23 March 2025) documented working examples: `docu-flex[.]com` and `pdfixers[.]com`, both serving malicious Windows executables (`DocuFlex.exe`, `Pdfixers.exe`) flagged on VirusTotal. Researcher Will Thomas documented the sites. A separate **Gootloader** campaign ran through Google ads for fake PDF-to-DOCX converters from November 2024 — Gootloader being a loader that pulls banking trojans and infostealers.

**Why this matters more than any feature we have.** The FBI's own recommendation is *"use built-in tools"* — because in March 2025 there was no third option they were willing to name. A signed, reproducible, locally-run, sandboxed converter is that third option. This is the one place where our product is the literal answer to a government advisory, and the site currently does not lead with it.

**Handle with care.** Two failure modes. Citing an FBI warning to sell software is one rhetorical step from fearmongering, and this audience punishes that. And the warning is about *malicious sites*, which is a different threat from *hostile files* — conflating them would be exactly the imprecision the rest of the product refuses. The honest framing is that they are two halves of one problem: you cannot trust the converter, and the converter cannot trust the file.

---

## 5. The crowded middle

> **Revised 2026-08-24, after the positioning was restated as "open source file converter **with AI models**".** The first draft of this section tested locality alone. Locality is the weaker half of that claim; the AI half needs its own audit, and it is a harder audit to pass.

### 5.1 Locality is now a commodity

As of 2026 the browser-local / self-hosted converter cluster includes:

| Project | Shape | Note |
|---|---|---|
| **VERT.sh** | Browser WASM, AGPL-3.0 | 15.4k stars; video offloaded to a daemon |
| **Lokaltools** | Browser WASM | 100+ tools across 8 categories |
| **FLINT** | Browser | Converter + PDF organiser + metadata cleaner + subtitle fixer |
| **HRConvert2** | Self-hosted PHP | 445 formats, built-in OCR **and virus scanning**; "no databases, no cookies, no cache files, no external connections, no analytics" |
| **ConvertX** | Self-hosted Docker | ~1,000 formats via bundled engines |
| **Stirling-PDF** | Self-hosted Docker | PDF only; 81k stars, 25M downloads |
| **ffmpeg.wasm** | Library + playground | The substrate several of the above stand on |

Seven projects, all saying "your files never leave your device". **A headline claim of locality competes on a commodity.**

### 5.2 Every Tier-S model is already shipped by a popular open-source tool

This is the finding that matters, and it is uncomfortable. Testing [02-FEATURES §10.1](02-FEATURES.md#101-tier-s--ships-as-ai-pack-v1) row by row against what already exists:

| Our Tier-S capability | Model we list | Who already ships that model, locally, open source |
|---|---|---|
| OCR | PaddleOCR mobile | **Umi-OCR** (offline, GUI, images + PDF + screenshot), PaddleOCR itself, Tesseract inside **Stirling-PDF** |
| Transcription | Whisper base int8 | **whisper.cpp**, plus ~8 GUI front-ends — Buzz, MacWhisper, VoiceInk, Scriberr, OpenWhispr, Superwhisper… |
| Voice activity detection | Silero VAD | Silero ships it; bundled in most ASR front-ends |
| **Background removal** | **u2netp / BiRefNet-lite** | **rembg — 16k stars, and it uses u2net *and* BiRefNet.** The same weights |
| Upscaling | Real-ESRGAN compact | **Upscayl**, **chaiNNer**, **Video2X**, Real Video Enhancer |
| Narration (TTS) | Kokoro-82M | Widely bundled; the default local TTS of 2025–26 |
| Noise removal | DeepFilterNet | Ships its own CLI, with GUI wrappers |
| **Document structure** | **Granite-Docling-258M** | **IBM Research ships `Docling` around that exact model** |

**There is no proprietary model, no fine-tune and no trained asset in the AI pack.** Every weight is a third-party permissively-licensed artefact a competitor can install this afternoon. That is a deliberate and correct licensing choice — it is what makes the pack redistributable at all — but it means **the AI capability is a curation, not an invention**, and it cannot be positioned as one.

### 5.3 AI document conversion is contested by well-funded teams

One Tier-S row deserves separate warning. `PDF → Markdown/JSON` — marked "★★★ *(rising)* · highest B2B trajectory" in 02-FEATURES — is the most crowded square on the board:

| Tool | Backer | Approach |
|---|---|---|
| **Docling** | IBM Research | Layout model + GraniteDocling 258M VLM; structured `DoclingDocument` for RAG |
| **MarkItDown** | Microsoft | 15+ formats, ~12 s per 100 pages, no GPU |
| **Marker** | Datalab | Surya OCR (650M params), optional `--use_llm` pass |
| **MinerU** · **pdf-craft** · **PyMuPDF4LLM** · **olmOCR** · **DeepSeek-OCR 2** | various | VLM-based single-pass page understanding |

Entering here means fighting IBM, Microsoft and a funded startup on their chosen ground, using **their** model. It is the one Tier-S row whose honest description is "we are late", not "nobody does this".

### 5.4 What is actually unowned

Nothing above is ours. These are:

| # | Unowned | Why nobody has it |
|---|---|---|
| 1 | **One local AI pack across capabilities, inside a general converter** | Everything in §5.2 is a *point* tool. A user wanting OCR + transcription + background removal installs three apps and three model downloaders |
| 2 | **AI as a target format, not a mode** | "Transparent PNG" is a *route* in our table, reached by a model. Everywhere else AI is a separate panel bolted beside conversion |
| 3 | **The inferred/generated split (Class C vs D), enforced** | OCR *reads what is there*; upscaling *invents pixels*. No converter distinguishes them, and ours refuses to arm a generated step from a default, a preset or the prediction engine |
| 4 | **Model licensing published as a table** | Licence, size, hash and **commercial-use status** per model, gated in CI. Procurement asks this first and no competitor can answer it from a file |
| 5 | **A receipt naming which model produced which step** | No converter produces a receipt at all |
| 6 | **Per-engine OS confinement extended to inference** | A hostile ONNX graph is a supply-chain risk; pickle is refused at the file-type layer, before anything opens it |
| 7 | **Non-destructive by construction** | Everyone else opens with truncation |

### 5.5 The regulatory tailwind under item 3

**EU AI Act Article 50 and California SB 942 both require machine-readable disclosure of AI-generated content.** C2PA / Content Credentials is the standard answer, and its ecosystem passed **6,000 members and affiliates in January 2026**.

Our Class C/D split is a **finer** distinction than C2PA's "was AI involved": it is per operation, and it is enforced in the router rather than recorded after the fact. That is a real differentiator with a compliance buyer attached, and it points at an obvious move — **emit C2PA assertions from the receipt.**

Note the irony that makes it valuable. C2PA manifests are routinely **stripped by format conversion**, which is precisely the operation we perform. A converter that *preserves and extends* provenance instead of destroying it is a position nobody in §5.1 or §5.2 is set up to take.

### 5.6 Verdict

**No single feature is unique. The combination is, today. The combination is also the least defensible part of it.**

Bundling eight public models behind one manager is a few months for a competitor who decides to. What is *not* a few months is retrofitting the class taxonomy into a router, the receipt into an executor, `O_EXCL` into every write path, and per-engine sandboxing into a process model. Those are architectural commitments made at commit one, and a competitor adding them later would be rewriting rather than extending.

> **So the durable claim is not "converter with AI".** It is: **the only converter where the AI is accountable** — every model named and licensed in a published table, every operation classed as *read* or *invented*, every step recorded in a receipt, and nothing generative ever armed by default.

---

## 6. Patterns that work

Observed across the set, in rough order of value.

| # | Pattern | Evidence |
|---|---|---|
| 1 | **A file-first interaction above everything.** No format picker before the file. | VERT ("Drop or click to convert"), CloudConvert (widget above the fold) |
| 2 | **Specific counts, per category.** "212 formats across 11 categories" beats "hundreds". | CloudConvert |
| 3 | **One repeated CTA.** Download, three times, nothing competing with it. | Ollama |
| 4 | **Sparse copy.** Short phrases over paragraphs; whitespace as confidence. | Ollama |
| 5 | **A commit hash in the footer.** Ties the page to a readable build. | VERT |
| 6 | **The limitation in the tagline.** "fully local\*" costs nothing and buys everything after it. | VERT |
| 7 | **Mechanism in plain words, no jargon.** "renders each page to pixels, then rebuilds". | Dangerzone |
| 8 | **Bottom-up explanation with one idea per diagram.** | Tailscale |
| 9 | **Third-party validation where it exists.** ISO badges, a named nonprofit, a named company and jurisdiction. | iLovePDF, Dangerzone, CloudConvert |
| 10 | **Tasks, not format pairs, at the top of the funnel.** People want "merge PDF", not "PDF → PDF". | iLovePDF |

---

## 7. Patterns that fail

| # | Anti-pattern | Evidence |
|---|---|---|
| 1 | **Analytics disclosed anywhere but the front.** Reads as concealment even when trivial. | VERT on HN: *"why hide it at the bottom of settings?"* |
| 2 | **No visible business model.** Invites *"unless they are doing something more"*. | VERT on HN |
| 3 | **Attribution added after launch.** Noticed, and read as carelessness about licensing. | VERT on HN |
| 4 | **Guarantees of conduct where architecture is possible.** "We delete files" / "privacy 100% guaranteed" — unfalsifiable. | Convertio |
| 5 | **Claiming a property the mechanism does not deliver.** "straight from your browser" when the file is uploaded. | CloudConvert |
| 6 | **Density.** More words read as less confidence. | Contrast Ollama with every converter site |
| 7 | **Describing the design record to the reader.** Nobody arrives wanting to know which document said what. | Our own v0.1, per [10-WEBSITE §1](10-WEBSITE.md#1-what-v01-got-wrong) |
| 8 | **Status as content.** Roadmaps, "not yet", commit counts on a product page. | Our own v0.1 |

**Items 7 and 8 were already known and were fixed.** Items 1–6 were not on the list, and 6 is the one most likely to have made the last attempt feel wrong: the rebuilt homepage carried nine sections and roughly 1,200 words of prose before the footer, against Ollama's few dozen. Every section was individually defensible. The aggregate was a document, not a landing page.

---

## 8. What nobody's site does

Each of these is available to us, and none of them is being done by anyone in section 3.

| Opportunity | Why it is open |
|---|---|
| **Show the receipt on the marketing page.** Real JSON from a real run. | No competitor produces a receipt at all, so none can show one |
| **Publish the limits.** Not as precedent-following — Dangerzone does not do it on their site — but as a deliberate, unusual choice | It reads as confidence precisely because it is rare |
| **Prove locality instead of asserting it.** "Open your network panel" is an instruction a visitor can follow in five seconds | Every competitor asserts; the browser can verify |
| **Name the FBI advisory and answer it.** | Nobody in the category wants to raise the subject; we are the only one it does not indict |
| **Show a conversion being refused, and why.** A route eliminated with its reason named | Every competitor hides failure; the refusal is the honesty proof |
| **State the business model on the homepage.** | Directly answers the HN objection VERT could not |
| **Attribute upstream engines prominently, with licences and link mode.** | We already generate this; VERT was criticised for the lack of it |

---

## 9. Recommendations

For the rebuild. Ordered by expected effect.

### 9.1 Cut the homepage to roughly a fifth

**Target: under 250 words of prose above the footer, and five sections.** Ollama sells a harder product with less text. The nine-section homepage should become:

1. **Hero + drop zone.** One sentence. The zone is the argument.
2. **What it did.** The receipt from the file just dropped — or a real one if nothing has been.
3. **Three claims, three lines.** Confined · Verifiable · Non-destructive. Each a link, not a paragraph.
4. **Install.** One command, checksum, licence commitment in two sentences.
5. **Footer with a commit hash.**

Everything else moves to the page that owns it. The current homepage tries to be `/security`, `/ai`, `/formats` and `/docs` simultaneously.

### 9.2 Lead with accountable AI — not locality, and not raw capability

Three candidate headlines, tested against §5:

| Candidate | Verdict |
|---|---|
| "Local, open source file converter" | **Dead.** Seven projects already say it (§5.1) |
| "File converter with local AI" | **Weak.** Every model is already shipped by a better-known point tool (§5.2), and one row puts us against IBM and Microsoft (§5.3) |
| **"The only converter that tells you what its models did — and never lets one invent something you did not ask for"** | **Defensible.** Rests on §5.4 items 3–7, none of which anyone else has |

Local execution and the AI pack are the *mechanism*. Accountability is the *claim*. Leading with the mechanism puts the site on a shelf beside rembg, Upscayl and whisper.cpp — each better known than us, and each better at its one job.

### 9.3 Answer the four HN objections explicitly, on the homepage or one click from it

Verbatim from §3.4, because they are the real ones:

| Objection | Where it is answered |
|---|---|
| "Are you tracking me?" | Zero analytics, stated in the footer, not in settings |
| "How is this sustainable?" | The business model, one sentence, on the homepage |
| "What are you built on?" | Attribution page, generated, with licences and link mode |
| "What does the licence mean for me?" | Apache-2.0, and what that permits, said plainly |

### 9.4 Rewrite `/security` as one bottom-up narrative

Tailscale's shape, not a claim grid: start at the file, walk up through detect → route → confine → execute → receipt, one diagram per idea, limitations stated where they arise. The verbatim §8 table stays, but as the end of an argument rather than a bolted-on appendix — and framed as our choice, since §3.5 establishes that it is not an industry norm.

### 9.5 Make the demo prove locality, not just perform it

The instruction *"open your network panel first"* converts a claim into a five-second check the visitor performs themselves. That is worth more than any amount of copy. The existing WASM planning island already supports this — it makes exactly one request, for itself.

### 9.6 Correct the record in 10-WEBSITE

Two factual corrections this research forces:

- **§5 `/security`** asserts Dangerzone publishes what it does not protect against and cites it as precedent. Their site does not. Reframe as a deliberate choice.
- **§4 sitemap** budgets thirty `/vs/<competitor>` pages. Against a category whose leader takes 81% of visits from organic search and where AI answers increasingly intermediate, thirty thin comparison pages is a large investment in the weakest channel. Recommend five, against the projects our audience actually evaluates: VERT, Stirling-PDF, ConvertX, Dangerzone, CloudConvert.

### 9.7 Keep what already works

Not everything should be discarded. Carried forward:

- The generated data layer — routes, CLI transcripts and the receipt exported from the binary. §8 shows this is a capability nobody else has; it should be used more, not less.
- The build gates, including banned tokens and the privacy word cap.
- The zero-JS budget and the 15 KB planning island.
- The design tokens, which are inherited from [07-DESIGN-SYSTEM](07-DESIGN-SYSTEM.md) and are not the problem.

---

## 10. Sources

Retrieved 2026-08-24. Traffic and star counts are point-in-time and move.

**Competitor sites**
- [CloudConvert](https://cloudconvert.com/)
- [Convertio](https://convertio.co/)
- [iLovePDF](https://www.ilovepdf.com/)
- [VERT.sh](https://vert.sh/) · [VERT-sh/VERT on GitHub](https://github.com/VERT-sh/VERT)
- [Dangerzone](https://dangerzone.rocks/)

**Reference sites**
- [Ollama](https://ollama.com/)
- [How Tailscale works](https://tailscale.com/blog/how-tailscale-works)

**The trust event**
- [FBI Denver warns of online file converter scam](https://www.fbi.gov/contact-us/field-offices/denver/news/fbi-denver-warns-of-online-file-converter-scam) — the primary advisory (returns HTTP 403 to automated fetches; content confirmed via the secondary sources below)
- [BleepingComputer — FBI warnings are true, fake file converters do push malware](https://www.bleepingcomputer.com/news/security/fbi-warnings-are-true-fake-file-converters-do-push-malware/), 23 March 2025
- [Dark Reading — FBI: Beware of Document Converter Tools](https://www.darkreading.com/cyberattacks-data-breaches/fbi-document-converter-tools-scam)
- [CSO Online — FBI warns: beware of free online document converter tools](https://www.csoonline.com/article/3853045/fbi-warns-beware-of-free-online-document-converter-tools.html)

**Audience reaction**
- [Hacker News — Open source and self hostable/private file converter](https://news.ycombinator.com/item?id=43663865)

**Category and traffic**
- [Similarweb — cloudconvert.com](https://www.similarweb.com/website/cloudconvert.com/)
- [Semrush — ilovepdf.com overview](https://www.semrush.com/website/ilovepdf.com/overview/)
- [Ahrefs data — ilovepdf.com](https://ahrefstop.com/websites/ilovepdf.com)
- [Marius Hosting — best Docker container converters](https://mariushosting.com/synology-best-docker-container-converters/)
- [Sliplane — 5 ConvertX alternatives](https://sliplane.io/blog/5-awesome-convertx-alternatives)
- [OPSWAT — CDR: 8 best vendors in 2026](https://www.opswat.com/blog/content-disarm-reconstruction-cdr-8-best-vendors-in-2026)
- [MarketsandMarkets — Content Disarm and Reconstruction market](https://www.marketsandmarkets.com/Market-Reports/content-disarm-reconstruction-market-89335390.html)

**AI conversion and local model tools**
- [rembg](https://github.com/danielgatis/rembg) — 16k stars, u2net and BiRefNet
- [Docling (IBM Research)](https://github.com/docling-project/docling) · [MarkItDown (Microsoft)](https://github.com/microsoft/markitdown) · [Marker (Datalab)](https://github.com/datalab-to/marker)
- [Best open-source PDF-to-Markdown tools 2026](https://themenonlab.blog/blog/best-open-source-pdf-to-markdown-tools-2026)
- [MarkItDown vs Docling vs Marker](https://www.danilchenko.dev/posts/markitdown-vs-docling-vs-marker/)
- [Best open source OCR tools & models 2026](https://unstract.com/blog/best-opensource-ocr-tools/)
- [AI transcription: local vs cloud, 8 tools compared](https://screenpipe.com/blog/ai-transcription-local-2026)
- [C2PA and Content Credentials explainer](https://spec.c2pa.org/specifications/specifications/2.4/explainer/Explainer.html) · [C2PA adoption status 2026](https://www.eyesift.com/faq/c2pa-content-credentials-2026-cryptographic-provenance-adoption/)

**Privacy coverage of the category**
- [Practical Web Tools — testing 12 popular converters](https://practicalwebtools.com/blog/online-file-converters-privacy-security-test-2026)
- [Experian — risks of using online file/PDF converters](https://www.experian.com/blogs/ask-experian/risks-of-using-online-file-pdf-converters/)

---

## Appendix — the enterprise line, briefly

Not a website finding, but it surfaced during this research and bears on `/enterprise`.

The **Content Disarm and Reconstruction** market is projected at **USD 0.5B by 2026** (from 0.2B in 2021, ~15.7% CAGR), and is **moderately fragmented — a dozen vendors hold most revenue, none above a quarter share.** Named players: Check Point, Fortinet, Broadcom, OPSWAT, Votiro, Glasswall, Deep Secure, Resec, Odix, Sasa Software, Peraton.

Two 2025 movements worth noting: **Menlo Security acquired Votiro** (February 2025), folding CDR into a secure-browser portfolio; **Glasswall allied with ReversingLabs**, adding 40 billion malware hashes to its decision engine.

**What this means for `/enterprise`.** The budget line exists and is growing, which validates the reframe in [04-GROWTH §10.1](04-GROWTH.md#101-the-reframe). But the incumbents are network-appliance and API vendors selling into email and web gateways — not endpoint tools. Our differentiator against them is not detection depth, which we would lose on; it is **that there is no shared parsing tier at all.** No queue holding other tenants' documents, no appliance to compromise, and an audited boundary published rather than described in an RFP response. That is the sentence the page should open with, and it is not currently the sentence it opens with.
