# OpenConvert — site copy

**Every word on the five primary pages, in one place.** Edit this file, hand it
back, and it gets implemented into the components verbatim.

Last synced from the source: 2026-09-06 · `site/src/`

---

## How to use this

- **Edit the text after each `>` or in each list.** Everything quotable is
  marked; anything not marked is a label for you, not copy.
- **`{like this}`** is a number the build fills in from the engine — `{routes}`
  is 212 today because `openconvert routes` printed 212, and `{tools}` is 20
  because `all_tools()` advertises 20. Leave the braces in and the number stays
  correct forever; replace them with a literal and it goes stale on the next
  release. A gate now refuses a build where the generated data and the engine
  disagree, so this is enforced rather than remembered.
- **Links** are written `[text](/path)`. Change the text freely; tell me if you
  want a different destination.
- **Voice gate.** `scripts/gates.mjs` fails the build on 26 phrases — *coming
  soon*, *we believe*, *seamless*, *cutting-edge*, *powerful*, *planned*, *our
  roadmap*, and similar. If a rewrite uses one, I will flag it rather than ship it.
- **Alt text** (§9) is read aloud to screen-reader users and is required by a
  gate. It is not decorative.

---

## 1 · Global chrome

Appears on every page.

### 1.1 Header

| Slot | Text |
|---|---|
| Wordmark | `openconvert` |
| Nav 1 | `Features` |
| Nav 2 | `Community` |
| Nav 3 | `About` |
| Icon | GitHub (no text) |
| Icon | Discord → `/community` (no text) |
| Icon | Theme toggle — label reads `Change the theme: system, light or dark` |
| Button | `Download` |

### 1.2 Footer

**Column 1 — `Product`**
- `Features`
- `Download`
- `Changelog`

**Column 2 — `Trust`**
- `Safety`
- `Privacy`
- `Accessibility`

**Column 3 — `Company`**
- `About`
- `Community`
- `Contact`
- `Source code ↗`

**Column 4 — `Legal`**
- `Terms`
- `Licences`
- `Imprint`

**Bottom line, left**
> © {year} Clura srls · PI 18314321003 · [company details](/imprint)

**Bottom line, right**
> openconvert {version} · Apache-2.0

---

## 2 · Home — `/`

### 2.0 Browser tab and search result

- **Title:** `OpenConvert: convert & modify any file locally`
- **Description:** `A local file converter with sandboxed engines, on-device AI models and a receipt for every job. Open Source & free.`

### 2.1 Hero

**Heading**
> Convert any file on your machine.

**Sub**
> Images, audio, video, documents; {routes} routes across {formats} formats, using locally run AI models where useful.

**Button**
> Download OpenConvert

**Under the app window**
> Web-based mockup with some conversions available. [Download the desktop app](/download) for every feature.

**Inside the app window** *(this is the live demo — it mirrors the desktop app, so changing it makes the site disagree with the product)*
- Drop zone title: `Drop files to convert`
- Drop zone title while dragging: `Release to add files`
- `or click to choose` · `Ctrl+O`
- `Originals are never modified · nothing is ever overwritten`
- Tool workspace prompt: `Open an image file` / `Open a pdf file` / `Open an audio file`
- Tool workspace sub: `Runs on your machine in the desktop app`
- Tool workspace link: `See what it does →`

### 2.2 Bento — section heading

> Built to be the last converter you install.

#### Cell 1 — Local AI models *(the large one)*

- **Eyebrow:** `on-device AI` - remove this
- **Heading:** `Local AI models for advanced conversions`
- **Body:**
  > Optional, off by default. No data is sent anywhere, the model runs on your computer. This allows for state of the art conversions for free.
- **Labels on the diagram:** `Remove background` · `Audio transcript` · `Upscaling` · `Text extraction` · `Audio denoising`

#### Cell 2 — Open source

- **Heading:** `Open source`
- **Body:**
  > Apache-2.0, engines included. No feature is held back for a paid tier, because there is no paid tier to hold it in.
- **Link:** `Every licence →` → `/licences`

#### Cell 3 — Conversions

- **Heading:** `{routes} routes` *(hard-coded today; say the word and I will make it `{routes}`)*
- **Body:**
  > Pick a file, pick a target. The router finds the way.
- **Caption:**
  > {formats} formats, both directions where the route exists.

#### Cell 4 — Offline

- **Heading:** `Completely offline`
- **Body:**
  > Every conversion runs with no network capability at all. The only connection to the web of the app is the AI model download from HuggingFace. --(include links in the features page for the actual models used and where they are downloaded from hugging face)

#### Cell 5 — Batch

- **Heading:** `Drop in as many as you like`
- **Body:**
  > One target filetype for everything, or a different one per row. No practical limit on how many files go in at once.

#### Cell 6 — Sandboxing

- **Heading:** `Every file opens in a cage`
- **Body:**
  > Convert files knowing that your computer will be safe. Parsers are where the exploits live, so each one runs confined by the OS with file descriptors instead of paths. A hostile file has nothing to reach.
- **Link:** `How the confinement works →` → `/safety`

### 2.3 Comparison

**Heading**
> How it compares to other solutions

**Sub**
> Measured against what the most common options

**Columns:** `iLovePDF` (cloud) · `CloudConvert` (cloud) · `Other desktop converters` (desktop) · `OpenConvert` (this app)

| Row | Sub-line | iLovePDF | CloudConvert | Other desktop | OpenConvert |
|---|---|---|---|---|---|
| `Free & open source` | — | ✗ | ✗ | ~ `Some` | ✓ `Apache-2.0` | (i love pdf is 60 eur a year for desktop app)
| `Local AI models` | `Background removal, transcription, upscaling, text extraction all on your computer` | ✗ | ✗ | ✗ | ✓ |
| `Completely offline` | — | ✗ `Uploads` | ✗ `Uploads` | ✓ | ✓ | 
| `No conversion limits` | `Files per day, file size, minutes of processing` | ✗ | ✗ | ~ `Varies` | ✓ |
| `All in one tool` | `Documents, images, audio and video in the same window` | ~ `PDF` | ✓ | ~ `Varies` | ✓ |

> **Before this goes live, please check every cell yourself.** These are claims
> about named companies whose plans change. `~` means "partly" and is there so
> the table can be accurate rather than flattering.

### 2.4 Tools

**Heading**
> The only app you need to manage files.

**Group 1 — `Image editing tools`**
- Remove background: on-device matting, nothing uploaded, different AI models available
- Upscale x2 or x4 without the usual blur using a local AI model
- Invert colours, black and white
- Compress to the quality you pick, without changing the format
- Read the text out of a screenshot or a scan
- Colour picker, sampled from the image itself
- Convert between HEIC, JPEG, PNG, WebP, AVIF, TIFF, JPEG XL, SVG and camera raw

**Group 2 — `PDF tools`**
- Merge documents into one file while keeping hyperlinks, or split one into many
- Keep, drop, reorder, rotate or crop pages
- Add a password, or remove one you know
- Compress without touching a single image
- Sign, with the signature kept on your machine
- Render any page to an image
- Read a scan into searchable text
- PDF to Markdown, HTML, DOCX or ODT

**Group 3 — `Audio tools`**
- Transcribe speech into text you can search
- Remove background noise from a recording
- Convert between FLAC, WAV, MP3, OGG and M4A
- Pull the audio out of a video without re-encoding it

### 2.5 Feature board

**Heading**
> Missing a feature? We'll build it.

**Body**
> You get to decide what we build next. Put your idea on the board, vote on everyone else's, and the ones people actually want move up the list.

**Button:** `Add an idea` · **Link:** `See the whole board →`

**Empty state** *(shown until the GitHub board has entries)*
> **Nobody has asked for anything yet.**
> The board is open and empty. What is missing in openconvert?
> [Open the first idea →]

### 2.6 Get OpenConvert

**Heading**
> Get OpenConvert

**Sub**
> Apache-2.0, and every feature is in the free build.

| Card | Under it |
|---|---|
| `Windows` | `Windows 10 and 11 · x86-64` |
| `macOS` | `Apple Silicon and Intel` |
| `Linux` | `.deb and AppImage` |

Each card ends with `Download →`.

### 2.7 FAQ

**Heading**
> Questions people actually ask.

**Q1 — Is it really free?**
> Yes, and there is no paid tier holding a feature back. OpenConvert is Apache-2.0, so open source & community driven. Support contracts and enterprise licensing fund the work. (include link to a contact form in the about page)

**Q2 — Does anything leave my computer?**
> Your files never leave your computer. Two things reach the network: downloading the AI models you chose and their updates.

**Q3 — What if it converts my file wrong?**
> You still have the original. Every write in the program is exclusive and there is no truncating open anywhere in it, so a conversion cannot replace a file you already had unless you specifically want to. The receipt beside the output names the engine, its version, the parameters and the operation class, so a wrong result can be traced. If this ever happens please submit the receipt on github or discord.

**Q4 — Do I need the AI models?**
> No. Conversion works in full without a single model on disk, and none is installed unless you ask for one. Note that some features are driven by these models though, so they will not be available without them.

**Q5 — How is this different from a website converter?**
> A website converter has your file. That is the entire difference, and it is the reason the FBI issued an advisory about free online converters in March 2025. OpenConvert reads the bytes on your own machine, inside an OS sandbox that has no network capability at all.

**Q6 — Who is behind it, and will it still exist in two years?**
> I'm Fabrizio, an aerospace engineering student and developer in my free time. I made openconvert because i needed it, and I thought of sharing it with everyone else. The licence is the answer to the second half: Apache-2.0 with the whole engine layer in the repository means the software outlives whoever is maintaining it.

---

## 3 · Features — `/features`

### 3.0 Browser tab and search result

- **Title:** `Features — OpenConvert`
- **Description:** `Every conversion openconvert can make, the tools for images, documents, audio and video, the optional on-device models, and the receipt every job writes.`

### 3.1 Hero

**Heading**
> Everything it does.

**Sub**
> {routes} conversions, {tools} tools, optional AI models; all of it on your machine, and every job accounted for.

### 3.2 The conversion picker — `#conversions`

**Heading**
> Can it do what you need?

**Sub**
> Pick your starting filetype. Everything it can become is on the right.

**Inside the picker**
- Left label: `1 · I have a`
- Right label: `2 · and I want a`
- Group names: `Images` · `Documents` · `Audio` · `Video` · `Archives` · `Data`
- Class words beside each target: `lossless` · `lossy` · `rebuilt` · `generated`
- Note under the list:
  > {n} routes from {FORMAT}. Anything not listed here, the binary refuses rather than guesses.

### 3.3 By file type — `#tools`

**Heading**
> The tools we built in

**Sub**
> OpenConvert is more than a converter. It's meant to replace every web query you have ever done about your files

**Group 1 — `Images`**
- Remove the background; on-device matting, nothing uploaded
- Upscale ×2 or ×4 without the usual softness
- Invert colours, or convert to black and white
- Compress to the quality you pick; the format stays what it was
- Read the text out of a screenshot or a scan
- Pick a colour straight out of the image
- HEIC, JPEG, PNG, WebP, AVIF, TIFF, GIF, BMP, JPEG XL, SVG and camera raw

**Group 2 — `PDFs and documents`**
- Merge documents into one file, or split one into several
- Keep the pages you want, or remove the ones you do not
- Reorder pages, rotate them a quarter turn either way, crop by margin
- Add an AES-256 password, or take off one you know
- Compress: the streams a producer left uncompressed, deflated — nothing lossy
- Sign, with the signature kept on your machine
- Render any page to an image at the size you ask for
- PDF, DOCX, ODT, Markdown, HTML, plain text and Jupyter notebooks

**Group 3 — `Audio`**
- Transcribe speech into text you can search
- Remove hiss and background noise from a recording
- FLAC, WAV, MP3, OGG, M4A, Opus and MKA
- Pull the audio out of a video without re-encoding it

**Group 4 — `Video`**
- Remux MKV, MP4 and WebM without touching the streams, in seconds rather than minutes
- Extract the audio track as its own file: WAV, FLAC, MP3, Opus or MKA
- Transcribe what is said in it, straight from the video
- Trim by timecode, and join Matroska files losslessly, from the command line
- Transcoding uses a system FFmpeg when one is installed

### 3.4 The models — `#ai`

**Heading**
> The models, named.

**Sub**
> Optional, off until you download them, and each one shows its size & licence before it is fetched.

**Caption under the screenshot**
> Nothing is installed until this is answered, and the download is the only moment anything is fetched. [Every model's licence →](/licences)

*The four model rows — title, description, size, licence — are generated from
`models.toml` and are not editable here. Today they read:*

| Title | Description |
|---|---|
| Clean up noisy audio | Removes hiss and background noise from speech recordings. |
| Remove image backgrounds | Cuts the subject out of a photo and leaves the rest transparent. |
| Enlarge images | Doubles or quadruples an image's size without the usual blur. |
| Transcribe speech | *(from the registry)* |

### 3.5 Cost — `#cost`

**Heading**
> The $ you save.

**Sub**
> Background removal and file conversion are metered almost everywhere else. With openconvert they are free forever.

**Controls**
- `Files a month` — options `200` · `1,000` · `5,000` · `10,000`
- `What you'd pay per file elsewhere` — options 0.02, 60$ annual subscription just for pdfs, custom

**Results**
- `A cloud tool, per year`
- `OpenConvert, per year` → always `$0`

**Note**
> Per-image and per-minute pricing at the hosted tools changes by plan and by month, this is an estimate based on the most common providers.
### 3.6 Receipts — `#receipts`

- **Eyebrow:** `receipts`
- **Heading:** `Every job leaves a note.`
- **Sub:**
  > Beside every output is a small file saying what happened to yours, can be deactivated in settings.
| Term | Definition |
|---|---|
| `What ran` | The engine and its exact version: oc-images, libheif 1.17.6 |
| `What it did to the file` | Lossless, lossy, rebuilt or generated. |
| `What it was allowed to do` | The sandbox that actually engaged, and the memory and time limits |
| `What it read` | A hash of the exact bytes, so the input can be identified later |
| `What it sent` | The number of network calls. It is zero. |

**Link:** `How the confinement it records actually works →` → `/safety`

### 3.7 Closing

Repeats the **Get OpenConvert** block from §2.6.

---

## 4 · Community — `/community`

### 4.0 Browser tab and search result

- **Title:** `Community — OpenConvert`
- **Description:** `The Discord server, and the board where anyone can add a feature request and everyone can vote on it.`

### 4.1 Discord

**Heading**
> Come and build it with us.

**Sub** *(lives in `board.json`)*
> Questions, conversion problems worth looking at together, and early builds.

**Card, once you give me the invite URL**
- Title: `Join the Discord`
- Sub: `discord.gg`

**Card until then** *(what it says today)*
- Title: `Ask for the Discord invite`
- Sub: `The permanent invite is not published here yet`

### 4.2 The board

**Heading**
> The board

**Sub**
> Anyone can add one. Everyone can vote.

**Button:** `Add an idea`

**Empty state**
> Nobody has raised one yet.
> [Open the first idea →]

**Tier names and their one-liners** *(lives in `board.json`)*

| Tier | Blurb |
|---|---|
| `Shipped` | In a release you can download. |
| `Building` | Someone is on it now. |
| `Next up` | Accepted, not started. |
| `Under consideration` | Open. Most ideas live here. |
| `Not planned` | With a reason, always. |

---

## 5 · About — `/about`

### 5.0 Browser tab and search result

- **Title:** `About — OpenConvert`
- **Description:** `Why OpenConvert exists, who builds it, the Apache-2.0 licence in plain words, and every engine it stands on.`

### 5.1 Why

**Heading**
> A tool you don't have to take on faith.

**Body**
> In March 2025 the FBI warned people away from online file converters; many of them were malware. Do you want your confidential files uploaded on some remote server? We did not, so we built the open source solution we wanted to use.
### 5.2 Nothing escapes your computer

- **Eyebrow:** `Complete privacy`
- **Heading:** `No file escapes your computer`
- **Body:**
  > Every conversion is done locally, no file is ever sent to a server. Optional AI models can be downloaded to unlock advanced tools (Safe, Open source models).
- **Link:** `What each model does →` → `/features#ai`

### 5.3 Who

- **Eyebrow:** `who`
- **Heading:** `Clura srls.`
- **Body:**
  > A small company in Italy, built by Fabrizio Blasio, an aerospace engineering student. We have the goal of providing privacy focused software that solves everyday problems. Company details on the [imprint](/imprint); anything else, [contact](/contact).

### 5.4 Licence

- **Eyebrow:** `licence`
- **Heading:** `Apache-2.0, in plain words.`
- **Bullets:**
  - Use it commercially. Sell things made with it.
  - Modify it, fork it, ship your fork.
  - Patents are granted with the code.
  - No warranty; the licence text is the contract.
- **Body:**
  > The binary is free forever with no feature gates. Revenue comes from what organisations need around running it. SLAs, attested builds, fleet policy. Nothing needed to convert a file is paid, and the engine is never relicensed. If you're an enterprise looking to integrate our software contact us. [Full text →](/licences)

### 5.5 Attribution

- **Eyebrow:** `attribution`
- **Heading:** `Built on their work.`
- **Table columns:** `Engine` · `Domain` · `Licence` · `Link mode`
- **Caption:**
  > Models carry their own registry with hashes and commercial-use status: [the table →](/licences)

*The nine engine rows are a list of facts about third-party licences. Tell me if
one is wrong; do not reword them casually.*

---

## 6 · Download — `/download`

### 6.0 Browser tab and search result

- **Title:** `Install OpenConvert`
- **Description:** `Download OpenConvert for Windows, macOS or Linux. Free forever, Apache-2.0, and nothing you convert ever leaves your machine.`

### 6.1 Hero

**Heading**
> Install OpenConvert.

**Sub**
> Free forever.

**Chips:** `Apache-2.0` · `No account` · `Nothing is uploaded`

### 6.2 One block per platform

Headings are the OS names with their marks: `Windows` · `macOS` · `Linux`.

**Table columns:** `Package` · `Runs on` · `Version` · `Date` · `Size` · *(download button)*

**"Runs on" wording, per package**

| Package | Text |
|---|---|
| Windows `.exe` | x86-64 · installs for you or for every user |
| Windows `.msi` | x86-64 · every user, needs an administrator |
| macOS `.dmg` (Apple Silicon) | Apple Silicon |
| macOS `.dmg` (Intel) | Intel |
| Linux `.deb` | x86-64 · Ubuntu 22.04 LTS and newer |
| Linux `.AppImage` | x86-64 · any distribution with FUSE, no install |

**Button on every row:** `Download`

**Under each table:** `Headless` — then the `curl` line and the package-manager
line, both generated from the release manifest and both with a copy button.

---

## 7 · Pages not covered here

These exist and were not part of the rebuild. Say the word and I will extract
them into this file the same way.

| Route | What it is |
|---|---|
| `/safety` | The containment argument and the five animated sequences |
| `/changelog` | Release notes |
| `/contact` | How to reach us |
| `/privacy` | Under 300 words, gated, generated from `release/privacy.json` |
| `/terms` · `/licences` · `/imprint` · `/accessibility` | Legal and statutory |

---

## 8 · Text that is generated, not written

Changing these means changing the source, not the copy. Listed so you know why
they are missing above.

| Where | Source |
|---|---|
| Route counts, format counts, the whole picker | `openconvert routes` → `src/data/formats.json` |
| Model titles, descriptions, sizes, licences | `models.toml` → `src/data/models.json` |
| Version, date, file names, checksums, install commands | the release job → `src/data/release.json` |
| Every idea and vote on the board | GitHub Discussions → `src/data/board.json` |
| The receipt shown on the homepage | one real `openconvert convert --json` run |

---

## 9 · Alt text

Read aloud to screen-reader users, and required by a build gate. It lives in
`site/tools/shots/scenes.mjs` beside the click path that produces each picture,
so it stays with the screenshot when the screenshot is retaken.

| Screenshot | Alt text |
|---|---|
| `batch-plan` | Five dropped files listed in OpenConvert, each showing its detected format and the target it will convert to — three HEIC photos to JPEG, a file named screenshot.png flagged "The name says PNG and the content says JPEG; routing by content", and a FLAC recording to WAV. |
| `batch-done` | The OpenConvert results screen: "4 converted · 1 failed · 5 total", elapsed time and bytes in and out, each converted file listed with its operation class and duration, and one honest failure — "libheif could not decode this file: unsupported colour profile". |
| `receipt-open` | A conversion receipt expanded inside OpenConvert, listing the engine and its version, the exact parameters, the operation class, the limits and sandbox profile the job ran under, the hash of the bytes read, and the number of network calls made. |
| `remove-bg` | The OpenConvert background-removal workspace: the image on a transparency checkerboard with the subject cut out, a before/after toggle, zoom controls, and the status line "Background removed, now transparent". |
| `ai-features` | The OpenConvert first-run panel headed "Add AI features?", listing four optional on-device models with their size on disk and licence, none of them installed, and a "Not now" button. |
| `settings-models` | The OpenConvert settings screen, showing each on-device model with its size, licence, an enable switch, and a control that deletes it and reclaims the disk space. |

*Eighteen screenshots exist; the six above are the ones that carry an argument.
The rest follow the same pattern.*
