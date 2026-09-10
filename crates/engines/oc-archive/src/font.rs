//! Fonts: SFNT, WOFF and WOFF2.
//!
//! # Why this lives in the archive worker
//!
//! Because that is what these formats are. A TTF or an OTF is an **SFNT**: a
//! table directory followed by a pile of tables. WOFF wraps the same tables and
//! compresses each one with zlib; WOFF2 wraps them and compresses all of them
//! together with Brotli. Every conversion here is unpack-and-repack, which is
//! this worker's entire shape, and the SR-5 caps it already enforces land on
//! exactly the right surface — a small Brotli stream expanding to gigabytes is
//! the same bomb a zip can be, and `archive_total_bytes` counts it as it
//! arrives rather than trusting a declared size.
//!
//! # What is NOT here, deliberately
//!
//! **Outline conversion.** `ttf -> otf` and `otf -> ttf` are what every font
//! converter on the web offers, and what they do is convert cubic Bézier
//! outlines to quadratic or back — an approximation of every curve in the font.
//! The other thing done under that name is renaming the container, which ships
//! CFF outlines in a file called `.ttf` and produces something half the world
//! will not render. Neither is a repack, so neither is here, and `route.rs`
//! carries no row for them.
//!
//! **Reading WOFF2.** Writing one is exact: the specification permits a null
//! transform for `glyf` and `loca`, so the tables go in unchanged and Brotli
//! does the work. Reading an arbitrary one is not, because real WOFF2 files use
//! transform 0 — a bespoke point encoding that has to be reconstructed exactly.
//! That is a different size of job, so WOFF2 is a destination only and the
//! route table says so.
//!
//! # What survives
//!
//! Every table, byte for byte, including `name` and the `OS/2` `fsType` bits
//! that state what the recipient is licensed to do with the font. A conversion
//! that dropped those would change what somebody is permitted to do, which is
//! not a rounding error.

use openconvert_worker::{Converted, RunLimits};
use std::io::Write;

/// `wOFF`, the WOFF 1 signature.
const WOFF: &[u8; 4] = b"wOFF";
/// `wOF2`, the WOFF 2 signature.
const WOFF2: &[u8; 4] = b"wOF2";
/// `OTTO`, an SFNT carrying CFF outlines.
const OTTO: &[u8; 4] = b"OTTO";
/// `0x00010000`, an SFNT carrying `glyf` outlines.
const TRUETYPE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
/// Apple's alternative spelling of the same thing.
const TRUE: &[u8; 4] = b"true";

/// A table directory entry, as the SFNT states it.
#[derive(Debug)]
struct Table {
    tag: [u8; 4],
    checksum: u32,
    offset: usize,
    length: usize,
}

/// A parsed font: its flavour and its tables, in directory order.
#[derive(Debug)]
struct Sfnt {
    /// The version at offset 0, carried verbatim so a repack cannot change it.
    flavour: [u8; 4],
    tables: Vec<Table>,
}

/// Whether these bytes open like a font this worker reads.
pub fn is_font(bytes: &[u8]) -> bool {
    matches!(head(bytes), Some(h) if h == TRUETYPE || &h == OTTO || &h == WOFF || &h == TRUE)
}

/// How many tables this font declares, for the probe.
///
/// From the directory, which is an index: a font declaring a million tables
/// costs one `u16` read to refuse, not a million reads. WOFF puts the count at
/// offset 12 and a bare SFNT at offset 4, and 0 is the project's "not
/// determined" answer for anything unreadable.
#[must_use]
pub fn table_count(bytes: &[u8]) -> u32 {
    let at = if is_woff(bytes) { 12 } else { 4 };
    u32::from(be16(bytes, at).unwrap_or(0))
}

/// Whether these bytes are a WOFF (version 1).
pub fn is_woff(bytes: &[u8]) -> bool {
    head(bytes).is_some_and(|h| &h == WOFF)
}

fn head(bytes: &[u8]) -> Option<[u8; 4]> {
    bytes.get(..4)?.try_into().ok()
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Round up to the next multiple of four, which is where SFNT puts every table.
const fn pad4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

// ---------------------------------------------------------------------------
// SFNT
// ---------------------------------------------------------------------------

/// Read a bare SFNT's table directory.
///
/// Nothing inside a table is parsed. This walks the directory, checks each
/// entry points inside the file, and stops — which is the whole reason a font
/// repack is a small attack surface: the hostile part of a font is its glyph
/// programs, and they are copied rather than interpreted.
fn read_sfnt(bytes: &[u8], limits: &RunLimits) -> Result<Sfnt, String> {
    let flavour = head(bytes).ok_or("this file is too short to be a font")?;
    let count = be16(bytes, 4).ok_or("this font has no table directory")? as usize;
    if count == 0 {
        return Err("this font declares no tables".into());
    }
    // An SFNT's table count is a u16, so 65535 is the format's own ceiling and
    // the entry cap is what a caller can lower. Checked before allocating.
    if count as u64 > u64::from(limits.archive_entries) {
        return Err(format!(
            "this font declares {count} tables, over the limit of {}",
            limits.archive_entries
        ));
    }

    let mut tables = Vec::with_capacity(count);
    let mut total: u64 = 0;
    for i in 0..count {
        let at = 12 + i * 16;
        let tag: [u8; 4] = bytes
            .get(at..at + 4)
            .and_then(|t| t.try_into().ok())
            .ok_or("the table directory is truncated")?;
        let checksum = be32(bytes, at + 4).ok_or("the table directory is truncated")?;
        let offset = be32(bytes, at + 8).ok_or("the table directory is truncated")? as usize;
        let length = be32(bytes, at + 12).ok_or("the table directory is truncated")? as usize;

        // THE OFFSETS ARE THE FILE'S CLAIM. A table that runs past the end is
        // refused here rather than producing a short read somewhere downstream.
        let end = offset
            .checked_add(length)
            .ok_or("a table's offset and length overflow")?;
        if end > bytes.len() {
            return Err(format!(
                "the `{}` table runs past the end of the file",
                String::from_utf8_lossy(&tag)
            ));
        }
        total = total.saturating_add(length as u64);
        if total > limits.archive_total_bytes {
            return Err(format!(
                "this font's tables total more than the {}-byte limit",
                limits.archive_total_bytes
            ));
        }
        tables.push(Table {
            tag,
            checksum,
            offset,
            length,
        });
    }
    Ok(Sfnt { flavour, tables })
}

/// Rebuild a bare SFNT from tables, in the order given.
///
/// The search fields (`searchRange`, `entrySelector`, `rangeShift`) are derived
/// rather than copied: they are a function of the table count and nothing else,
/// and a font whose originals disagreed with its own count would otherwise keep
/// the disagreement.
fn write_sfnt(flavour: [u8; 4], tables: &[([u8; 4], u32, Vec<u8>)]) -> Vec<u8> {
    let n = u16::try_from(tables.len()).unwrap_or(u16::MAX);
    let entry_selector = (u16::BITS - 1 - n.max(1).leading_zeros()) as u16;
    let search_range = (1_u32 << entry_selector) * 16;
    let range_shift = u32::from(n) * 16 - search_range;

    let mut out = Vec::new();
    out.extend_from_slice(&flavour);
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(
        &u16::try_from(search_range)
            .unwrap_or(u16::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&u16::try_from(range_shift).unwrap_or(0).to_be_bytes());

    // The directory is written first with placeholder offsets, then patched:
    // an entry's offset is where its table lands, and that is not known until
    // the directory's own size is.
    let dir_at = out.len();
    out.resize(dir_at + tables.len() * 16, 0);
    for (i, (tag, checksum, data)) in tables.iter().enumerate() {
        let offset = out.len();
        out.extend_from_slice(data);
        out.resize(pad4(out.len()), 0);

        let e = dir_at + i * 16;
        out[e..e + 4].copy_from_slice(tag);
        out[e + 4..e + 8].copy_from_slice(&checksum.to_be_bytes());
        out[e + 8..e + 12].copy_from_slice(&(offset as u32).to_be_bytes());
        out[e + 12..e + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
    }
    out
}

/// The name for a flavour, for messages.
fn flavour_name(flavour: [u8; 4]) -> &'static str {
    if &flavour == OTTO {
        "otf"
    } else {
        "ttf"
    }
}

// ---------------------------------------------------------------------------
// WOFF (version 1): zlib per table
// ---------------------------------------------------------------------------

/// Read a WOFF back into a bare SFNT and its flavour.
fn woff_to_sfnt(bytes: &[u8], limits: &RunLimits) -> Result<([u8; 4], Vec<u8>), String> {
    let flavour: [u8; 4] = bytes
        .get(4..8)
        .and_then(|f| f.try_into().ok())
        .ok_or("this WOFF is too short to carry a flavour")?;
    let count = be16(bytes, 12).ok_or("this WOFF has no table directory")? as usize;
    if count == 0 {
        return Err("this WOFF declares no tables".into());
    }
    if count as u64 > u64::from(limits.archive_entries) {
        return Err(format!(
            "this WOFF declares {count} tables, over the limit of {}",
            limits.archive_entries
        ));
    }

    let mut tables: Vec<([u8; 4], u32, Vec<u8>)> = Vec::with_capacity(count);
    let mut written: u64 = 0;
    for i in 0..count {
        let at = 44 + i * 20;
        let tag: [u8; 4] = bytes
            .get(at..at + 4)
            .and_then(|t| t.try_into().ok())
            .ok_or("the WOFF table directory is truncated")?;
        let offset = be32(bytes, at + 4).ok_or("the WOFF table directory is truncated")? as usize;
        let comp = be32(bytes, at + 8).ok_or("the WOFF table directory is truncated")? as usize;
        let orig = be32(bytes, at + 12).ok_or("the WOFF table directory is truncated")? as usize;
        let checksum = be32(bytes, at + 16).ok_or("the WOFF table directory is truncated")?;

        let end = offset
            .checked_add(comp)
            .ok_or("a WOFF table's offset and length overflow")?;
        let raw = bytes
            .get(offset..end)
            .ok_or_else(|| format!("the `{}` table runs past the end", tag_name(tag)))?;

        // THE DECLARED SIZE IS A CLAIM, AND THE CAP IS ON WHAT ARRIVES. A WOFF
        // table is stored when compLength equals origLength and zlib otherwise,
        // and a zlib stream's ratio is unbounded -- the same reason
        // `gzip_to_tar` caps its output rather than its input.
        let budget = limits.archive_total_bytes.saturating_sub(written);
        let data = if comp == orig {
            raw.to_vec()
        } else {
            inflate(raw, budget, tag)?
        };
        if data.len() as u64 > budget {
            return Err(format!(
                "unpacking this WOFF passed the {}-byte total limit at `{}`",
                limits.archive_total_bytes,
                tag_name(tag)
            ));
        }
        // The directory's own claim has to hold too, or the rebuilt font has
        // tables whose lengths nothing agrees on.
        if data.len() != orig {
            return Err(format!(
                "the `{}` table decompressed to {} bytes, and the directory says {orig}",
                tag_name(tag),
                data.len()
            ));
        }
        written += data.len() as u64;
        tables.push((tag, checksum, data));
    }
    Ok((flavour, write_sfnt(flavour, &tables)))
}

/// zlib-decompress one table, bounded.
fn inflate(raw: &[u8], budget: u64, tag: [u8; 4]) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut out = Vec::new();
    // Budget plus one: producing the extra byte is what proves the table
    // EXCEEDED the cap rather than merely reaching it.
    flate2::read::ZlibDecoder::new(raw)
        .take(budget.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| format!("could not decompress the `{}` table: {e}", tag_name(tag)))?;
    Ok(out)
}

fn tag_name(tag: [u8; 4]) -> String {
    String::from_utf8_lossy(&tag).into_owned()
}

/// Wrap an SFNT as a WOFF.
fn sfnt_to_woff(bytes: &[u8], font: &Sfnt) -> Result<Vec<u8>, String> {
    let mut directory = Vec::with_capacity(font.tables.len() * 20);
    let mut blob = Vec::new();
    let mut total_sfnt: usize = 12 + font.tables.len() * 16;
    let body_at = 44 + font.tables.len() * 20;

    for t in &font.tables {
        let raw = bytes
            .get(t.offset..t.offset + t.length)
            .ok_or("a table moved while it was being read")?;
        let packed = deflate(raw);
        // STORED WHEN COMPRESSION DID NOT HELP, which the format expects:
        // compLength == origLength is how a WOFF says "this one is raw", and a
        // table that grew under zlib (a short one, or already-compressed data)
        // would otherwise make the file larger than the font it came from.
        let stored = packed.len() >= raw.len();
        let data: &[u8] = if stored { raw } else { &packed };

        let offset = body_at + blob.len();
        directory.extend_from_slice(&t.tag);
        directory.extend_from_slice(&(offset as u32).to_be_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_be_bytes());
        directory.extend_from_slice(&(t.length as u32).to_be_bytes());
        directory.extend_from_slice(&t.checksum.to_be_bytes());

        blob.extend_from_slice(data);
        blob.resize(pad4(blob.len()), 0);
        total_sfnt += pad4(t.length);
    }

    let mut out = Vec::with_capacity(body_at + blob.len());
    out.extend_from_slice(WOFF);
    out.extend_from_slice(&font.flavour);
    out.extend_from_slice(&((body_at + blob.len()) as u32).to_be_bytes());
    out.extend_from_slice(&(font.tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0_u16.to_be_bytes()); // reserved, must be zero
    out.extend_from_slice(&(total_sfnt as u32).to_be_bytes());
    out.extend_from_slice(&0_u16.to_be_bytes()); // majorVersion
    out.extend_from_slice(&0_u16.to_be_bytes()); // minorVersion
                                                 // No metadata block and no private block: this worker adds neither, and
                                                 // zero offsets are how the format says they are absent.
    out.extend_from_slice(&[0_u8; 20]);
    out.extend_from_slice(&directory);
    out.extend_from_slice(&blob);
    Ok(out)
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    // Writing to a `Vec` cannot fail; the results are unwrapped through
    // `unwrap_or_default`, which turns an impossible error into "stored".
    if e.write_all(raw).is_err() {
        return Vec::new();
    }
    e.finish().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// WOFF2: Brotli over the lot, with the null transform
// ---------------------------------------------------------------------------

/// The 63 tags WOFF2 can name by index, in the order the specification lists
/// them. Index 63 means "an arbitrary four-byte tag follows".
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

/// UIntBase128: seven bits per byte, high bit continuing, big-endian.
fn base128(mut n: u32, out: &mut Vec<u8>) {
    let mut digits = [0_u8; 5];
    let mut used = 0;
    loop {
        digits[used] = (n & 0x7f) as u8;
        used += 1;
        n >>= 7;
        if n == 0 {
            break;
        }
    }
    for i in (0..used).rev() {
        out.push(digits[i] | if i == 0 { 0 } else { 0x80 });
    }
}

/// Wrap an SFNT as a WOFF2, with the null transform.
///
/// # Why the null transform
///
/// The specification defines transform version 3 for `glyf` and `loca` as "no
/// transform", and 0 for every other table as the same thing. So the tables go
/// into the Brotli stream exactly as they came out of the font, and unwrapping
/// gives them back byte for byte. That is what makes this Class A.
///
/// The cost is size, and it is real: on a 242 KB TTF this produces 100 KB where
/// a transforming encoder produces 83 KB. The receipt says so. The alternative
/// is implementing the `glyf` transform — a bespoke point encoding — in both
/// directions, and being approximately right about it is much worse than being
/// exactly right and 20% larger.
fn sfnt_to_woff2(bytes: &[u8], font: &Sfnt) -> Result<Vec<u8>, String> {
    let mut directory = Vec::new();
    let mut blob = Vec::new();
    let mut total_sfnt: usize = 12 + font.tables.len() * 16;

    for t in &font.tables {
        let known = KNOWN_TAGS.iter().position(|k| *k == &t.tag);
        let index = known.unwrap_or(63) as u8;
        // Transform version 3 is the null transform for `glyf` and `loca`; for
        // every other table 0 already means untransformed. Getting this pair
        // backwards makes a reader look for a transformLength that is not there
        // and fail on the next table.
        let transform = if &t.tag == b"glyf" || &t.tag == b"loca" {
            3_u8
        } else {
            0
        };
        directory.push(index | (transform << 6));
        if known.is_none() {
            directory.extend_from_slice(&t.tag);
        }
        base128(t.length as u32, &mut directory);
        // No transformLength: a null transform means the table is not
        // transformed, so the field is absent.

        let raw = bytes
            .get(t.offset..t.offset + t.length)
            .ok_or("a table moved while it was being read")?;
        // UNPADDED IN THE STREAM. The reconstructed SFNT pads each table to a
        // four-byte boundary and `totalSfntSize` counts that padding, but the
        // compressed stream is the table data concatenated with none. Padding
        // it here makes the decompressed length disagree with the header by
        // exactly the padding, and every reader refuses.
        blob.extend_from_slice(raw);
        total_sfnt += pad4(t.length);
    }

    let compressed = brotli_compress(&blob)?;
    let length = 48 + directory.len() + compressed.len();

    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(WOFF2);
    out.extend_from_slice(&font.flavour);
    out.extend_from_slice(&(length as u32).to_be_bytes());
    out.extend_from_slice(&(font.tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0_u16.to_be_bytes()); // reserved
    out.extend_from_slice(&(total_sfnt as u32).to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&0_u16.to_be_bytes()); // majorVersion
    out.extend_from_slice(&0_u16.to_be_bytes()); // minorVersion
    out.extend_from_slice(&[0_u8; 20]); // no metadata, no private block
    out.extend_from_slice(&directory);
    out.extend_from_slice(&compressed);
    Ok(out)
}

fn brotli_compress(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // Quality 11 and a 22-bit window: what every font pipeline uses, and the
    // reason to reach for WOFF2 at all is size.
    let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
    w.write_all(raw)
        .map_err(|e| format!("could not compress the font: {e}"))?;
    drop(w);
    Ok(out)
}

// ---------------------------------------------------------------------------
// The entry point
// ---------------------------------------------------------------------------

/// Convert a font to `to`.
///
/// # Errors
///
/// A message the host shows the user: a malformed font, a cap crossed, or a
/// target whose outline flavour does not match what the file actually holds.
pub fn run(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    // A WOFF is unwrapped first, because everything downstream reads a bare
    // SFNT and the wrapper is the only difference.
    let (owned, flavour) = if is_woff(bytes) {
        let (flavour, sfnt) = woff_to_sfnt(bytes, limits)?;
        (Some(sfnt), flavour)
    } else {
        (
            None,
            head(bytes).ok_or("this file is too short to be a font")?,
        )
    };
    let sfnt: &[u8] = owned.as_deref().unwrap_or(bytes);

    let mut removed = Vec::new();
    match to {
        "ttf" | "otf" => {
            // THE FLAVOUR IS THE FILE'S, NOT THE REQUEST'S. A WOFF wrapping
            // CFF outlines unwraps to an OTF; writing it under the name `.ttf`
            // is the container rename this module refuses to do, and the other
            // route is one word away.
            let actual = flavour_name(flavour);
            if actual != to {
                return Err(format!(
                    "this font holds {} outlines, so it unwraps to {actual}, not to {to}. \
                     Converting between the two outline formats is an approximation of every \
                     curve in the font and this build does not do it; convert it to {actual} \
                     instead.",
                    if actual == "otf" { "CFF" } else { "glyf" }
                ));
            }
            if owned.is_none() {
                return Err(format!(
                    "this file is already {to}; there is nothing to unwrap"
                ));
            }
            enforce(sfnt.len(), limits)?;
            Ok(Converted {
                bytes: sfnt.to_vec(),
                removed,
            })
        }
        "woff" => {
            let font = read_sfnt(sfnt, limits)?;
            let out = sfnt_to_woff(sfnt, &font)?;
            enforce(out.len(), limits)?;
            Ok(Converted {
                bytes: out,
                removed,
            })
        }
        "woff2" => {
            let font = read_sfnt(sfnt, limits)?;
            let out = sfnt_to_woff2(sfnt, &font)?;
            enforce(out.len(), limits)?;
            // NOT A LOSS, AND WORTH SAYING ANYWAY. Every byte of the font is
            // in there; what is missing is the extra compression a transforming
            // encoder would have achieved, and somebody choosing WOFF2 is
            // choosing it for size.
            removed.push(
                "nothing: every table is carried byte for byte. The `glyf` transform is not \
                 applied, so this file is roughly a fifth larger than a transforming encoder \
                 would produce"
                    .to_string(),
            );
            Ok(Converted {
                bytes: out,
                removed,
            })
        }
        other => Err(format!("oc-archive does not write {other}")),
    }
}

fn enforce(len: usize, limits: &RunLimits) -> Result<(), String> {
    if len as u64 > limits.archive_total_bytes {
        return Err(format!(
            "the output is {len} bytes, over the {}-byte limit",
            limits.archive_total_bytes
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> RunLimits {
        RunLimits {
            decode_pixels: u64::MAX,
            memory_bytes: 1 << 30,
            archive_depth: 32,
            archive_entries: 100_000,
            archive_total_bytes: 1 << 30,
            use_gpu: false,
        }
    }

    /// A minimal but structurally honest TrueType font: a real directory, real
    /// offsets, and tables whose contents are distinguishable.
    ///
    /// Built rather than committed, for the reason every fixture in this repo
    /// is: one nobody can read is one nobody can extend.
    fn font(flavour: [u8; 4], tags: &[&[u8; 4]]) -> Vec<u8> {
        let tables: Vec<([u8; 4], u32, Vec<u8>)> = tags
            .iter()
            .enumerate()
            .map(|(i, tag)| {
                // Lengths that are not multiples of four, so the padding rules
                // have something to be wrong about.
                (**tag, 0x1234_5678 + i as u32, vec![i as u8 + 1; 7 + i * 5])
            })
            .collect();
        write_sfnt(flavour, &tables)
    }

    #[test]
    fn a_font_survives_the_woff_round_trip_byte_for_byte() {
        let src = font(TRUETYPE, &[b"head", b"glyf", b"loca", b"name", b"OS/2"]);
        let parsed = read_sfnt(&src, &limits()).expect("read the fixture");
        let woff = sfnt_to_woff(&src, &parsed).expect("wrap");
        assert_eq!(&woff[..4], WOFF);

        let (flavour, back) = woff_to_sfnt(&woff, &limits()).expect("unwrap");
        assert_eq!(flavour, TRUETYPE);
        assert_eq!(back, src, "a WOFF round trip must be byte-identical");
    }

    /// The tables' own bytes, and the fields that say what may be done with the
    /// font, are the two things a repack must never touch.
    #[test]
    fn the_licensing_tables_survive_a_woff2() {
        let src = font(TRUETYPE, &[b"head", b"name", b"OS/2", b"glyf", b"loca"]);
        let parsed = read_sfnt(&src, &limits()).expect("read");
        let woff2 = sfnt_to_woff2(&src, &parsed).expect("wrap");
        assert_eq!(&woff2[..4], WOFF2);

        // The Brotli stream is the tables concatenated, unpadded, in directory
        // order — so decompressing it must give exactly that.
        let compressed = &woff2[48 + directory_len(&parsed)..];
        let mut plain = Vec::new();
        brotli::BrotliDecompress(&mut &compressed[..], &mut plain).expect("decompress");

        let expected: Vec<u8> = parsed
            .tables
            .iter()
            .flat_map(|t| src[t.offset..t.offset + t.length].to_vec())
            .collect();
        assert_eq!(plain, expected, "every table, byte for byte, unpadded");
    }

    /// The directory's size, so the test above can find where Brotli starts.
    fn directory_len(font: &Sfnt) -> usize {
        let mut n = 0;
        for t in &font.tables {
            n += 1;
            if !KNOWN_TAGS.contains(&&t.tag) {
                n += 4;
            }
            let mut b = Vec::new();
            base128(t.length as u32, &mut b);
            n += b.len();
        }
        n
    }

    /// An arbitrary tag costs four extra bytes in the directory and must still
    /// come back.
    #[test]
    fn an_unknown_table_tag_is_carried_by_name() {
        let src = font(TRUETYPE, &[b"head", b"Zznz", b"glyf"]);
        let parsed = read_sfnt(&src, &limits()).expect("read");
        let woff = sfnt_to_woff(&src, &parsed).expect("wrap");
        let (_, back) = woff_to_sfnt(&woff, &limits()).expect("unwrap");
        assert_eq!(back, src);
    }

    /// The refusal that keeps `woff -> ttf` honest.
    #[test]
    fn a_cff_font_will_not_be_called_a_ttf() {
        let src = font(*OTTO, &[b"head", b"CFF ", b"name"]);
        let parsed = read_sfnt(&src, &limits()).expect("read");
        let woff = sfnt_to_woff(&src, &parsed).expect("wrap");

        let Err(err) = run(&woff, "ttf", &limits()) else {
            panic!("CFF outlines are not a TTF");
        };
        assert!(err.contains("CFF"), "{err}");
        assert!(
            err.contains("otf"),
            "the refusal must name the other route: {err}"
        );

        // And the route that IS right returns the font.
        let ok = run(&woff, "otf", &limits()).expect("otf is what it holds");
        assert_eq!(ok.bytes, src);
    }

    /// Every bound is checked against what arrives, never against a declared
    /// size — the rule the rest of this worker already follows.
    #[test]
    fn a_malformed_directory_is_refused_rather_than_read_past() {
        let mut src = font(TRUETYPE, &[b"head", b"glyf"]);
        // Point the first table past the end of the file.
        src[12 + 8..12 + 12].copy_from_slice(&0xFFFF_0000_u32.to_be_bytes());
        let err = read_sfnt(&src, &limits()).expect_err("a table past the end is not readable");
        assert!(err.contains("past the end"), "{err}");

        assert!(
            read_sfnt(b"\x00\x01\x00\x00", &limits()).is_err(),
            "no directory"
        );
        assert!(read_sfnt(&[], &limits()).is_err(), "empty");
    }

    #[test]
    fn the_entry_cap_is_checked_before_anything_is_allocated() {
        let src = font(TRUETYPE, &[b"head", b"glyf", b"name"]);
        let mut tight = limits();
        tight.archive_entries = 2;
        let err = read_sfnt(&src, &tight).expect_err("three tables, cap of two");
        assert!(err.contains("over the limit"), "{err}");
    }
}
