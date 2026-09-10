# Routes

**Generated — do not edit.** `cargo xtask routes` writes this file from
`RouteTable::v1()`, the same table the planner consults, and
`cargo xtask routes --check` fails the build if the two disagree.

Every conversion this build knows how to perform is below. Whether one
runs *on a given machine* is a different question: each route lists what
it requires, and a missing engine or model turns it into a refusal that
names what is absent. `openconvert routes <from> <to>` answers that
locally.

## Fidelity classes

| Class | Meaning |
|---|---|
| **A** | Lossless. The bytes that matter survive exactly. |
| **B** | Lossy, standard. A re-encode; the usual cost of changing format. |
| **C** | Rebuilt. The output is reconstructed rather than translated. |
| **D** | Generative. A model invented content that was not in the input. |

268 routes, across 46 source formats.

## Archives

### 7z

| To | Class | Steps | Requires |
|---|---|---|---|
| gzip | B | 7z → tar → gzip | `oc-archive` |
| tar | B | 7z → tar | `oc-archive` |
| zip | B | 7z → zip | `oc-archive` |

### gzip

| To | Class | Steps | Requires |
|---|---|---|---|
| tar | A | gzip → tar | `oc-archive` |
| zip | A | gzip → tar → zip | `oc-archive` |

### tar

| To | Class | Steps | Requires |
|---|---|---|---|
| gzip | A | tar → gzip | `oc-archive` |
| zip | A | tar → zip | `oc-archive` |

### zip

| To | Class | Steps | Requires |
|---|---|---|---|
| gzip | A | zip → tar → gzip | `oc-archive` |
| tar | A | zip → tar | `oc-archive` |

## Audio

### flac

| To | Class | Steps | Requires |
|---|---|---|---|
| mp3 | B | flac → mp3 | `libmp3lame` |
| ogg | B | flac → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | A | flac → wav | `oc-audio` |

### m4a

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | m4a → flac | `oc-audio` |
| mp3 | B | m4a → mp3 | `libmp3lame` |
| ogg | B | m4a → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | m4a → wav | `oc-audio` |

### mp3

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | mp3 → flac | `oc-audio` |
| mp3 | B | mp3 | `libmp3lame` |
| ogg | B | mp3 → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | mp3 → wav | `oc-audio` |

### ogg

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | ogg → flac | `oc-audio` |
| mp3 | B | ogg → mp3 | `libmp3lame` |
| ogg | B | ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | ogg → wav | `oc-audio` |

### wav

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | A | wav → flac | `oc-audio` |
| mp3 | B | wav → mp3 | `libmp3lame` |
| ogg | B | wav → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |

## Documents

### docx

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | C | docx → pdf → png → avif | `oc-images`, `oc-pdf` |
| bmp | C | docx → pdf → png → bmp | `oc-images`, `oc-pdf` |
| gif | C | docx → pdf → png → gif | `oc-images`, `oc-pdf` |
| html | B | docx → html | `oc-pdf` |
| jpeg | C | docx → pdf → jpeg | `oc-pdf` |
| markdown | B | docx → markdown | `oc-pdf` |
| odt | B | docx → odt | `oc-pdf` |
| pdf | C | docx → pdf | `oc-pdf` |
| png | C | docx → pdf → png | `oc-pdf` |
| tiff | C | docx → pdf → png → tiff | `oc-images`, `oc-pdf` |
| txt | B | docx → txt | `oc-pdf` |
| webp | C | docx → pdf → png → webp | `oc-images`, `oc-pdf` |

### dxf

| To | Class | Steps | Requires |
|---|---|---|---|
| pdf | C | dxf → pdf | `oc-pdf` |
| svg | C | dxf → svg | `oc-pdf` |

### epub

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | C | epub → docx | `oc-pdf` |
| html | C | epub → html | `oc-pdf` |
| markdown | C | epub → markdown | `oc-pdf` |
| odt | C | epub → odt | `oc-pdf` |
| pdf | C | epub → pdf | `oc-pdf` |
| txt | C | epub → txt | `oc-pdf` |

### html

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | C | html → docx | `oc-pdf` |
| markdown | C | html → markdown | `oc-pdf` |
| odt | C | html → odt | `oc-pdf` |
| pdf | C | html → pdf | `oc-pdf` |
| txt | C | html → txt | `oc-pdf` |

### notebook

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | C | notebook → docx | `oc-pdf` |
| html | C | notebook → html | `oc-pdf` |
| markdown | C | notebook → markdown | `oc-pdf` |
| odt | C | notebook → odt | `oc-pdf` |
| pdf | C | notebook → pdf | `oc-pdf` |
| png | C | notebook → pdf → png | `oc-pdf` |
| txt | C | notebook → txt | `oc-pdf` |

### odp

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | C | odp → docx | `oc-pdf` |
| html | C | odp → html | `oc-pdf` |
| markdown | C | odp → markdown | `oc-pdf` |
| odt | C | odp → odt | `oc-pdf` |
| pdf | C | odp → pdf | `oc-pdf` |
| txt | C | odp → txt | `oc-pdf` |

### odt

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | B | odt → docx | `oc-pdf` |
| html | B | odt → html | `oc-pdf` |
| jpeg | C | odt → pdf → jpeg | `oc-pdf` |
| markdown | B | odt → markdown | `oc-pdf` |
| pdf | C | odt → pdf | `oc-pdf` |
| png | C | odt → pdf → png | `oc-pdf` |
| txt | B | odt → txt | `oc-pdf` |

### pdf

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | pdf → png → avif | `oc-images`, `oc-pdf` |
| bmp | B | pdf → png → bmp | `oc-images`, `oc-pdf` |
| docx | C | pdf → docx | `oc-pdf` |
| gif | B | pdf → png → gif | `oc-images`, `oc-pdf` |
| html | B | pdf → html | `oc-pdf` |
| jpeg | B | pdf → jpeg | `oc-pdf` |
| markdown | B | pdf → markdown | `oc-pdf` |
| odt | C | pdf → odt | `oc-pdf` |
| png | B | pdf → png | `oc-pdf` |
| tiff | B | pdf → png → tiff | `oc-images`, `oc-pdf` |
| txt | B | pdf → txt | `oc-pdf` |
| webp | B | pdf → png → webp | `oc-images`, `oc-pdf` |

### pptx

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | C | pptx → docx | `oc-pdf` |
| html | C | pptx → html | `oc-pdf` |
| markdown | C | pptx → markdown | `oc-pdf` |
| odt | C | pptx → odt | `oc-pdf` |
| pdf | C | pptx → pdf | `oc-pdf` |
| txt | C | pptx → txt | `oc-pdf` |

### txt

| To | Class | Steps | Requires |
|---|---|---|---|
| docx | B | txt → docx | `oc-pdf` |
| html | B | txt → html | `oc-pdf` |
| jpeg | C | txt → pdf → jpeg | `oc-pdf` |
| markdown | B | txt → markdown | `oc-pdf` |
| odt | B | txt → odt | `oc-pdf` |
| pdf | C | txt → pdf | `oc-pdf` |
| png | C | txt → pdf → png | `oc-pdf` |

## Fonts

### otf

| To | Class | Steps | Requires |
|---|---|---|---|
| woff | A | otf → woff | `oc-archive` |
| woff2 | A | otf → woff2 | `oc-archive` |

### ttf

| To | Class | Steps | Requires |
|---|---|---|---|
| woff | A | ttf → woff | `oc-archive` |
| woff2 | A | ttf → woff2 | `oc-archive` |

### woff

| To | Class | Steps | Requires |
|---|---|---|---|
| otf | A | woff → otf | `oc-archive` |
| ttf | A | woff → ttf | `oc-archive` |
| woff2 | A | woff → woff2 | `oc-archive` |

## Images

### arw

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | arw → avif | `oc-images` |
| bmp | B | arw → bmp | `oc-images` |
| gif | B | arw → gif | `oc-images` |
| jpeg | B | arw → jpeg | `oc-images` |
| png | B | arw → png | `oc-images` |
| tiff | B | arw → tiff | `oc-images` |
| webp | B | arw → webp | `oc-images` |

### avif

| To | Class | Steps | Requires |
|---|---|---|---|
| bmp | B | avif → bmp | `oc-images` |
| gif | B | avif → gif | `oc-images` |
| jpeg | B | avif → jpeg | `oc-images` |
| png | B | avif → png | `oc-images` |
| tiff | B | avif → tiff | `oc-images` |
| webp | B | avif → webp | `oc-images` |

### bmp

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | bmp → avif | `oc-images` |
| gif | B | bmp → gif | — |
| jpeg | B | bmp → jpeg | — |
| pdf | B | bmp → pdf | — |
| png | A | bmp → png | — |
| tiff | B | bmp → tiff | — |
| txt | D | infer (ocr) | `paddleocr` |
| webp | B | bmp → webp | — |

### cr2

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | cr2 → avif | `oc-images` |
| bmp | B | cr2 → bmp | `oc-images` |
| gif | B | cr2 → gif | `oc-images` |
| jpeg | B | cr2 → jpeg | `oc-images` |
| png | B | cr2 → png | `oc-images` |
| tiff | B | cr2 → tiff | `oc-images` |
| webp | B | cr2 → webp | `oc-images` |

### cr3

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | cr3 → avif | `oc-images` |
| bmp | B | cr3 → bmp | `oc-images` |
| gif | B | cr3 → gif | `oc-images` |
| jpeg | B | cr3 → jpeg | `oc-images` |
| png | B | cr3 → png | `oc-images` |
| tiff | B | cr3 → tiff | `oc-images` |
| webp | B | cr3 → webp | `oc-images` |

### dng

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | dng → avif | `oc-images` |
| bmp | B | dng → bmp | `oc-images` |
| gif | B | dng → gif | `oc-images` |
| jpeg | B | dng → jpeg | `oc-images` |
| png | B | dng → png | `oc-images` |
| tiff | B | dng → tiff | `oc-images` |
| webp | B | dng → webp | `oc-images` |

### gif

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | gif → avif | `oc-images` |
| bmp | B | gif → bmp | — |
| jpeg | B | gif → jpeg | — |
| pdf | B | gif → pdf | — |
| png | B | gif → png | — |
| tiff | B | gif → tiff | — |
| txt | D | infer (ocr) | `paddleocr` |
| webp | B | gif → webp | — |

### heic

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | heic → avif | `oc-images` |
| bmp | B | heic → bmp | `oc-images` |
| gif | B | heic → gif | `oc-images` |
| jpeg | B | heic → jpeg | `oc-images` |
| png | B | heic → png | `oc-images` |
| tiff | B | heic → tiff | `oc-images` |
| webp | B | heic → webp | `oc-images` |

### jpeg

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | jpeg → avif | `oc-images` |
| bmp | B | jpeg → bmp | — |
| gif | B | jpeg → gif | — |
| pdf | B | jpeg → pdf | — |
| png | B | jpeg → png | — |
| tiff | B | jpeg → tiff | — |
| txt | D | infer (ocr) | `paddleocr` |
| webp | B | jpeg → webp | — |

### jxl

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | jxl → avif | `oc-images` |
| bmp | B | jxl → bmp | `oc-images` |
| gif | B | jxl → gif | `oc-images` |
| jpeg | B | jxl → jpeg | `oc-images` |
| png | B | jxl → png | `oc-images` |
| tiff | B | jxl → tiff | `oc-images` |
| webp | B | jxl → webp | `oc-images` |

### nef

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | nef → avif | `oc-images` |
| bmp | B | nef → bmp | `oc-images` |
| gif | B | nef → gif | `oc-images` |
| jpeg | B | nef → jpeg | `oc-images` |
| png | B | nef → png | `oc-images` |
| tiff | B | nef → tiff | `oc-images` |
| webp | B | nef → webp | `oc-images` |

### png

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | png → avif | `oc-images` |
| bmp | B | png → bmp | — |
| gif | B | png → gif | — |
| jpeg | B | png → jpeg | — |
| pdf | B | png → pdf | — |
| tiff | B | png → tiff | — |
| txt | D | infer (ocr) | `paddleocr` |
| webp | B | png → webp | — |

### svg

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | svg → avif | `oc-images` |
| bmp | B | svg → bmp | `oc-images` |
| gif | B | svg → gif | `oc-images` |
| jpeg | B | svg → jpeg | `oc-images` |
| png | B | svg → png | `oc-images` |
| tiff | B | svg → tiff | `oc-images` |
| webp | B | svg → webp | `oc-images` |

### tiff

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | tiff → avif | `oc-images` |
| bmp | B | tiff → bmp | — |
| gif | B | tiff → gif | — |
| jpeg | B | tiff → jpeg | — |
| pdf | B | tiff → pdf | — |
| png | B | tiff → png | — |
| txt | D | infer (ocr) | `paddleocr` |
| webp | B | tiff → webp | — |

### webp

| To | Class | Steps | Requires |
|---|---|---|---|
| avif | B | webp → avif | `oc-images` |
| bmp | B | webp → bmp | — |
| gif | B | webp → gif | — |
| jpeg | B | webp → jpeg | — |
| pdf | B | webp → pdf | — |
| png | B | webp → png | — |
| tiff | B | webp → tiff | — |
| txt | D | infer (ocr) | `paddleocr` |

## Spreadsheets

### ods

| To | Class | Steps | Requires |
|---|---|---|---|
| csv | C | ods → csv | `oc-pdf` |
| json | C | ods → json | `oc-pdf` |
| xlsx | C | ods → xlsx | `oc-pdf` |

### xlsx

| To | Class | Steps | Requires |
|---|---|---|---|
| csv | C | xlsx → csv | `oc-pdf` |
| json | C | xlsx → json | `oc-pdf` |

## Tabular data

### csv

| To | Class | Steps | Requires |
|---|---|---|---|
| json | A | csv → json | — |

### json

| To | Class | Steps | Requires |
|---|---|---|---|
| csv | B | json → csv | — |

## Video

### avi

| To | Class | Steps | Requires |
|---|---|---|---|
| mka | A | stream copy | AudioCompatible |
| mkv | A | stream copy | StreamsCompatible |
| mp4 | A | stream copy | StreamsCompatible |
| webm | A | stream copy | StreamsCompatible |

### mkv

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | mkv → flac | `oc-audio` |
| mka | A | stream copy | AudioCompatible |
| mp3 | B | mkv → mp3 | `libmp3lame` |
| mp4 | A | stream copy | StreamsCompatible |
| mp4 | B | mkv → mp4 | `ffmpeg` |
| ogg | B | mkv → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | mkv → wav | `oc-audio` |
| webm | A | stream copy | StreamsCompatible |
| webm | B | mkv → webm | `ffmpeg` |

### mov

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | mov → flac | `oc-audio` |
| mka | A | stream copy | AudioCompatible |
| mkv | A | stream copy | StreamsCompatible |
| mp3 | B | mov → mp3 | `libmp3lame` |
| mp4 | A | stream copy | StreamsCompatible |
| ogg | B | mov → ogg | `libopus` |
| wav | B | mov → wav | `oc-audio` |
| webm | A | stream copy | StreamsCompatible |

### mp4

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | mp4 → flac | `oc-audio` |
| mka | A | stream copy | AudioCompatible |
| mkv | A | stream copy | StreamsCompatible |
| mkv | B | mp4 → mkv | `ffmpeg` |
| mp3 | B | mp4 → mp3 | `libmp3lame` |
| ogg | B | mp4 → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | mp4 → wav | `oc-audio` |
| webm | A | stream copy | StreamsCompatible |
| webm | B | mp4 → webm | `ffmpeg` |

### webm

| To | Class | Steps | Requires |
|---|---|---|---|
| flac | B | webm → flac | `oc-audio` |
| mka | A | stream copy | AudioCompatible |
| mkv | A | stream copy | — |
| mkv | B | webm → mkv | `ffmpeg` |
| mp3 | B | webm → mp3 | `libmp3lame` |
| mp4 | A | stream copy | StreamsCompatible |
| mp4 | B | webm → mp4 | `ffmpeg` |
| ogg | B | webm → ogg | `libopus` |
| txt | D | infer (transcribe) | `whisper` |
| wav | B | webm → wav | `oc-audio` |

## Read but never written

These formats can be opened and converted *from*, and nothing
converts *to* them. That is a deliberate absence rather than a gap:
a route with no encoder behind it would be a promise the build
cannot keep, and refusing by name beats failing at the last step.

- `heic`
- `jxl`
- `m4a`
- `mov`
- `avi`
- `pptx`
- `odp`
- `epub`
- `ods`
- `dxf`
- `postscript`
- `7z`
- `cr3`
- `cr2`
- `nef`
- `arw`
- `dng`
- `notebook`

