# Benign corpus

Ordinary files for round-trip, golden and limit tests. Committed, small, and
boring on purpose.

Nothing here is hostile — the hostile cases are *generated* by
[`../src/evil.rs`](../src/evil.rs) rather than stored, so that the gates
consuming them cannot skip on a pull request from a fork, which is where a
secret-dependent corpus would fail silently and green.

Files arrive as the weeks that need them arrive:

| Week | Needs |
|---|---|
| 3 | a JPEG with EXIF including GPS — the subject of the week-3 metadata strip and the first Class A gate |
| 4 | small PNG / JPEG / WebP / GIF / BMP / TIFF for detection and Class B |
| 4 | a 4 KB PNG declaring 500 megapixels — the pixel-bomb limit case |
| 24 | a zip bomb and a deeply nested archive, both **generated**, not stored |

The pixel bomb and the archive bombs are generated at test time for the same
reason as the evil names: a checked-in bomb is a repository full of things
antivirus software will quarantine on clone.
