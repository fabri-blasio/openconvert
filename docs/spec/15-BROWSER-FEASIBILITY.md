# What the browser sample can actually do

**Measured, not guessed.** Every "yes" below was probed in the live page on
2026-09-07 (Chromium, the engine the in-app browser and the desktop WebView
both use). The probe script is at the bottom so the table can be re-run when a
browser ships something new.

This exists because the homepage sample is the product's first impression and
the one place where a claim is checkable in one click. A sample that pretends
is worse than no sample.

---

## 0. Two bugs found while testing this

Both were live on the homepage and both are fixed.

1. **`fileName` was never declared.** `island.js` runs under `'use strict'`,
   where assigning to an undeclared name throws instead of creating a global.
   `take()` assigns `fileName` on its first line, so **every upload threw a
   `ReferenceError` before anything else ran** — no status, no plan, no visible
   error. This is the "it never gets shown on the next page" report.

2. **The chosen target was read by list position.** `i` is the router's
   *format index* — for PNG, jpeg is `1`, webp is `2`, pdf is `18` — but the
   code fell back to `r.targets[i]`, the *i-th entry of the list*. The wasm
   never emits `target_id`, so that fallback ran every time: picking **jpeg
   produced a `.webp` file**, and picking webp reached for pdf and refused.
   Now taken from `plan.target`, the router's own answer.

---

## 1. Codec support, measured

### Encoding — `canvas.toBlob`

| Asked for | Got back | Usable |
|---|---|---|
| `image/png` | `image/png` | **yes** |
| `image/jpeg` | `image/jpeg` | **yes** |
| `image/webp` | `image/webp` | **yes** |
| `image/avif` | `image/png` | no |
| `image/gif` | `image/png` | no |
| `image/bmp` | `image/png` | no |
| `image/tiff` | `image/png` | no |

> **The hazard that matters.** An unsupported type does **not** throw and does
> **not** return null — it silently returns a **PNG**. Add `avif` to
> `CAN_ENCODE` and the sample will hand the visitor a PNG named `.avif`, with a
> receipt saying it converted to AVIF. That is precisely the failure mode this
> product exists to argue against, and it would be invisible in testing.
> `CAN_ENCODE` must stay an allowlist of the three verified types, and anything
> added to it must be verified by reading the returned blob's `type`.

### Decoding — `<img>`

`png` · `jpeg` · `webp` · `gif` · `bmp` · `svg` all decode. AVIF and HEIC
depend on the visitor's browser and OS; HEIC is Safari-only in practice.

### Platform APIs present

`ImageDecoder` · `OffscreenCanvas` · `CompressionStream` ·
`DecompressionStream` · WebCodecs (`VideoEncoder`).

---

## 2. Conversions

| Route | Feasible in a tab | Cost | Verdict |
|---|---|---|---|
| png · jpeg · webp · gif · bmp · svg **→** png · jpeg · webp | **yes** | 0 KB | **Shipping.** This is what the sample does today. |
| csv ↔ json | **yes** | ~1 KB of JS | **Worth adding.** Pure string work, no dependency, and it is a real route in the table. |
| gzip ↔ tar | partly | 0 KB | `DecompressionStream('gzip')` is native. tar is a header format and needs ~2 KB of hand-written parsing. **Worth adding.** |
| zip → tar, 7z → * | no | 40–300 KB | Needs a real archive library. Out of budget. |
| image → pdf | yes | ~250 KB (`pdf-lib`) | **Blocked by budget**, see §4. |
| avif/jxl/heic/raw → anything | no | decoder is the whole problem | Refuse by name, as now. |
| any → avif/tiff/gif/bmp | **no** | — | No encoder exists in a tab. See the hazard above. |
| audio ↔ audio | no | ~30 MB (ffmpeg.wasm) | Out of the question at any budget. |
| video anything | no | ~30 MB | Same. WebCodecs could decode but cannot mux. |
| pdf → image | no | ~1 MB (pdf.js) | Out of budget. |
| docx/odt/html/markdown/notebook | no | LibreOffice / Pandoc are processes | Never in a tab. |

---

## 3. The twenty tools

| Tool | In a tab? | Why |
|---|---|---|
| **Invert colours** | **yes** | One pass over `ImageData`. ~15 lines, no dependency. |
| **Black and white** | **yes** | Same. |
| **Compress** (image) | **yes** | `toBlob('image/jpeg', q)` — the quality argument is already there. |
| **Colour picker** | **yes** | `getImageData` of one pixel. Already `previewOnly` in the app, so nothing is written either way. |
| Remove background | no | u2netp/MODNet, 11 MB of ONNX + a runtime. |
| Upscale | no | Real-ESRGAN, 64 MB. |
| Read text in image | no | PP-OCRv6, three artifacts. |
| Remove background noise | no | DeepFilterNet, 8 MB. |
| Transcribe to text | no | Whisper, 78 MB. |
| **Merge · Split · Extract · Remove · Reorder · Rotate · Crop** | possible | All seven are page-tree edits `pdf-lib` does, in one ~250 KB dependency. |
| Add password / Remove password | no | `pdf-lib` does not do AES-256 encryption or decryption. |
| Compress (PDF) | marginal | Only re-deflating streams; little gain for the weight. |
| Sign | possible | Same `pdf-lib` dependency. |

**Four tools are free** — invert, black and white, compress, colour picker.
They need no dependency, no model and no new network origin, and all four
operate on formats the tab already decodes. These are the ones worth wiring in.

**Seven PDF tools are one dependency away**, and that dependency is the whole
decision. See below.

---

## 4. What shipped, and what stops the rest

**The recommendation in the first draft of this document was overruled, and
correctly.** It said `pdf-lib` was too heavy to justify. The decision taken was
that a sample which draws seven PDF tools it cannot perform is worth less than
201 KB fetched by the people who actually open one — so the tools run.

### Shipped in the sample

| | |
|---|---|
| **Conversions** | image → PNG · JPEG · WebP (measured encoders only), image → PDF, CSV ↔ JSON |
| **Sources decoded** | PNG · JPEG · WebP · GIF · BMP · SVG · **AVIF** |
| **Image tools** | Invert · Black and white · Compress · Colour picker |
| **PDF tools** | Merge · Split · Extract pages · Remove pages · Reorder · Rotate · Crop |
| **Dimmed, with the reason** | Remove background · Upscale · Read text in image · Remove background noise · Transcribe · Remove password · Add password · Compress (PDF) · Sign |

### Two things the implementation had to get right

**The router is not the limit.** `openconvert-wasm` answers for an environment
with no engines available, so it returns *zero* targets for AVIF — the route
table really does have `avif → png`, but it cannot promise it in-process. This
browser decodes AVIF natively. The sample therefore offers what the browser can
genuinely do, marked apart from the router's own answers and carrying the
encode's own fidelity class rather than one borrowed from a plan nobody made.

**Compression is not conversion, and a tab cannot honour that rule silently.**
The desktop tool keeps the format it was given. `canvas.toBlob` ignores the
quality argument for PNG entirely — there is no lossy PNG — so "compress this
PNG" would either do nothing or hand back a JPEG under a PNG-shaped promise.
The sample asks which format to write, defaults to one that has a quality knob,
and says plainly when PNG was chosen that the number did nothing.

### What still stops the rest

- **`homepage island budget` — 40 KB gzipped**, now 36.0. This is what a
  visitor downloads to read the page, and `pdf-lib` is deliberately not in it.
- **`lazy payload budget` — 240 KB gzipped**, now 201.5. `pdf-lib`, fetched on
  the first PDF tool and never before. A budget so it stays a decision.
- **`zero external requests` and CSP `connect-src 'self'`.** `pdf-lib` is
  vendored into `public/vendor/` and named in the gate with its version. A
  third-party script that is not on that list still fails the build.
- **Models.** Remove background is 11 MB, upscaling 64 MB, Whisper 78 MB. These
  are not budget arguments that could be won; they are the reason the desktop
  app exists.
- **ffmpeg.wasm is ~30 MB**, which ends the audio and video conversation.

## 5. Re-running the probe

Paste into the console on the homepage:

```js
const c = document.createElement('canvas');
c.width = c.height = 4;
c.getContext('2d').fillRect(0, 0, 4, 4);
for (const m of ['image/png','image/jpeg','image/webp','image/avif','image/gif','image/bmp','image/tiff']) {
  const b = await new Promise(r => c.toBlob(r, m));
  console.log(m, '->', b ? b.type : 'null');   // a mismatch means NO encoder
}
```
