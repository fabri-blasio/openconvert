//! `oc-archive` — the second worker, and the first to enforce SR-5.
//!
//! Repacks one archive into another. `Zip → Tar`, `Tar → Zip` and
//! `Tar → Gzip` are the rows the route table has, all Class A: no member is
//! re-encoded, only rewrapped (or compressed whole), so the bytes of every
//! file inside come out identical.
//!
//! # Why this one is pure Rust
//!
//! `engines.toml` names libarchive. This uses `zip`, `tar` and `flate2`
//! instead, all Rust, because the security story does not come from the
//! library. It comes from the caps below, which are ours to enforce whichever
//! decompressor sits underneath — and writing it this way meant the second
//! worker could exist and be tested now rather than after a C build.
//!
//! # SR-5: the caps that actually work
//!
//! Spike S23 settled what does not. `expansion_ratio` cannot separate a bomb
//! from real data: a legitimate 200 MB zeroed disk image compresses **1029×**
//! and a zip bomb **1028×**. No threshold sits between them, so the ratio is a
//! logging tripwire and nothing more.
//!
//! What works is absolute and counted **while unpacking, not before**:
//!
//! - `archive_entries` — a bomb is often millions of tiny files.
//! - `archive_total_bytes` — the sum of what is written out.
//! - `archive_depth` — an archive inside an archive inside an archive.
//!
//! Every one is checked **as the running total crosses it**, never from a
//! declared size in the header. A header is written by whoever built the file;
//! `42.zip` declares a modest size and is 4.5 PB unpacked. The only number that
//! cannot lie is the one this process has actually produced so far.
//!
//! # What this worker does not do
//!
//! It does not *extract to disk*. Every route is archive-to-archive, one blob
//! in and one blob out, so nothing here needs the job directory or a brokered
//! output path. Extraction to loose files is a different step kind and needs
//! machinery that is not wired yet.

mod font;

use openconvert_worker::{run_real, Converted, Engine, Probed, RunLimits};
use std::io::{Cursor, Read, Write};

fn main() -> std::io::Result<()> {
    // `run_real`, never `serve`: this binary applies its own confinement
    // before the first request arrives (Linux), and reports what the host's
    // spawn actually engaged (Windows).
    run_real(&Archives)
}

struct Archives;

impl Engine for Archives {
    fn name(&self) -> &'static str {
        "oc-archive"
    }

    fn version(&self) -> String {
        format!(
            "zip {} · tar {} · flate2 {}",
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION")
        )
    }

    /// Count the members, without unpacking any of them.
    ///
    /// The archive analogue of reading an image header: it walks the central
    /// directory (zip) or the header chain (tar), both of which are indexes,
    /// and touches no compressed bytes. A file declaring a million entries
    /// costs a million index reads to refuse, not a million inflations.
    ///
    /// A gzip stream's entry count cannot be known without decompressing it —
    /// which would be exactly the unpacking a probe exists to avoid — so it
    /// reports 0 under the project-wide convention that zero means "not
    /// determined", the same answer audio probes give for duration.
    fn probe(&self, bytes: &[u8]) -> Result<Probed, String> {
        match zip::ZipArchive::new(Cursor::new(bytes)) {
            Ok(archive) => Ok(Probed::Archive {
                entries: u32::try_from(archive.len()).unwrap_or(u32::MAX),
            }),
            Err(e) if !is_zip(bytes) => {
                // Not a zip at all — try the other readers before refusing.
                let _ = e;
                if is_tar(bytes) {
                    let count = tar::Archive::new(Cursor::new(bytes))
                        .entries()
                        .map_err(|e| format!("could not walk the tar headers: {e}"))?
                        .filter_map(|e| e.ok())
                        .count();
                    return Ok(Probed::Archive {
                        entries: u32::try_from(count).unwrap_or(u32::MAX),
                    });
                }
                if is_gzip(bytes) {
                    return Ok(Probed::Archive { entries: 0 });
                }
                // A font's table count IS its entry count, and it is in the
                // directory rather than behind any decompression -- the
                // cheapest probe in this worker.
                if font::is_font(bytes) {
                    return Ok(Probed::Archive {
                        entries: font::table_count(bytes),
                    });
                }
                Err(
                    "could not recognise these bytes as an archive or a font this worker reads"
                        .into(),
                )
            }
            Err(e) => Err(format!("could not read the archive index: {e}")),
        }
    }

    fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
        // FONTS FIRST, and dispatched on CONTENT like everything else here. A
        // WOFF and a zip are both compressed containers and neither's
        // destination says which arrived; `wOFF`, `OTTO` and `0x00010000` do.
        // See `font.rs` for why a font repack belongs in the archive worker.
        if font::is_font(bytes) {
            return font::run(bytes, to, limits);
        }
        match to {
            // Dispatch on CONTENT here too: `tar` is reachable from a zip
            // (repack) and from a gzip (decompress), and the destination alone
            // does not say which arrived.
            "tar" => {
                if is_gzip(bytes) {
                    gzip_to_tar(bytes, limits)
                } else if is_7z(bytes) {
                    sevenz_to_tar(bytes, limits)
                } else {
                    zip_to_tar(bytes, limits)
                }
            }
            "zip" => {
                // Dispatch on CONTENT, not on what the host believed: the
                // destination alone does not say what arrived.
                if is_tar(bytes) {
                    tar_to_zip(bytes, limits)
                } else if is_7z(bytes) {
                    sevenz_to_zip(bytes, limits)
                } else if is_gzip(bytes) {
                    // A .tar.gz reaches zip by decompressing and then
                    // repacking, which is two conversions and must not wear
                    // one receipt. That is now the `Gzip -> Zip` ROW: two
                    // `Extract` steps, `gzip -> tar` then `tar -> zip`, each
                    // landing here separately with both shown in the plan.
                    //
                    // So this arm is only reached by a caller driving the
                    // worker directly, and it says which way round to do it.
                    Err(
                        "convert this to tar first. gzip to zip in one step would hide a decompress and a repack behind one receipt; the route table chains the two."
                            .into(),
                    )
                } else if is_zip(bytes) {
                    Err("this input is already a zip; there is nothing to repack".into())
                } else {
                    Err("these bytes are not an archive this worker reads".into())
                }
            }
            "gzip" | "gz" => tar_to_gzip(bytes, limits),
            other => Err(format!("oc-archive does not write {other}")),
        }
    }
}

/// Whether these bytes open as a tar (the `ustar` signature at offset 257).
fn is_tar(bytes: &[u8]) -> bool {
    bytes.len() > 262 && &bytes[257..262] == b"ustar"
}

/// Whether these bytes carry the zip local-file-header signature.
fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x50, 0x4B, 0x03, 0x04])
}

/// Whether these bytes carry the 7z signature.
fn is_7z(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])
}

/// Whether these bytes carry the gzip magic.
fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1F, 0x8B])
}

/// Repack a zip as an uncompressed tar, enforcing SR-5 as it goes.
///
/// Class A: every member's bytes are copied through unchanged, so the files
/// inside come out identical. Only the wrapper differs.
fn zip_to_tar(bytes: &[u8], limits: &RunLimits) -> Result<Converted, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("could not read the archive: {e}"))?;

    // Checked before the walk because the index is already parsed and the count
    // is now a fact rather than a claim. Every other cap below is checked
    // against a running total instead.
    let entries = archive.len() as u64;
    if entries > u64::from(limits.archive_entries) {
        return Err(format!(
            "this archive holds {entries} entries, over the limit of {}",
            limits.archive_entries
        ));
    }

    let mut builder = tar::Builder::new(Vec::new());
    let mut written: u64 = 0;

    for i in 0..archive.len() {
        let mut member = archive
            .by_index(i)
            .map_err(|e| format!("could not read entry {i}: {e}"))?;

        // Directories carry no bytes and no risk; tar recreates them from the
        // member paths, so they are skipped rather than copied.
        if member.is_dir() {
            continue;
        }

        // THE NAME IS NOT TRUSTED.
        //
        // A zip member may be called `../../etc/passwd`, and `zip` offers
        // `enclosed_name` precisely because the raw name is attacker-written.
        // Refusing beats sanitising: a name we would have to repair is a name
        // whose author wanted it repaired into something.
        let name = member
            .enclosed_name()
            .ok_or_else(|| format!("entry {i} has a name that escapes the archive"))?;

        // Read the member into memory, bounded as it goes. `member.size()` is
        // the header's claim; this counts what actually arrives.
        let remaining = limits.archive_total_bytes.saturating_sub(written);
        let mut body = Vec::new();
        let mut limited = member.by_ref().take(remaining.saturating_add(1));
        limited
            .read_to_end(&mut body)
            .map_err(|e| format!("could not unpack {}: {e}", name.display()))?;

        // One byte past the budget is enough to know. Reading further would
        // mean holding the whole bomb to discover it was a bomb.
        if body.len() as u64 > remaining {
            return Err(format!(
                "unpacking exceeded the {}-byte total limit at entry {i}",
                limits.archive_total_bytes
            ));
        }
        written += body.len() as u64;

        // Nesting: an archive inside this one counts against the depth cap. A
        // depth of 1 means "this archive, and nothing packed inside it".
        if limits.archive_depth < 2 && looks_like_archive(&body) {
            return Err(format!(
                "{} is itself an archive, and the nesting limit is {}",
                name.display(),
                limits.archive_depth
            ));
        }

        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        // Mode and time are set to fixed values rather than copied. A zip's
        // stored timestamp is attacker-controlled metadata that nothing here
        // needs, and a reproducible output is worth more than a preserved
        // mtime nobody asked to keep.
        header.set_mtime(0);
        header.set_cksum();

        builder
            .append_data(&mut header, &name, Cursor::new(&body))
            .map_err(|e| format!("could not repack {}: {e}", name.display()))?;
    }

    let mut out = builder
        .into_inner()
        .map_err(|e| format!("could not finish the tar: {e}"))?;
    out.flush()
        .map_err(|e| format!("could not flush the tar: {e}"))?;

    Ok(Converted {
        bytes: out,
        // EMPTY, and that is the claim. Every member is copied through byte for
        // byte, so nothing inside the archive is lost -- which is what Class A
        // means. The zip's own per-member timestamps and modes are not carried
        // into the tar, and that is named rather than passed over in silence.
        removed: vec!["zip member timestamps and permission bits".to_string()],
    })
}

/// Repack a tar as a STORED zip — the reverse of [`zip_to_tar`], and held to
/// exactly the same caps.
///
/// Stored, not deflated: compression here would change nothing about safety
/// and would make "the bytes come out identical" harder to keep honest. Class
/// A holds because every member's bytes are copied verbatim.
fn tar_to_zip(bytes: &[u8], limits: &RunLimits) -> Result<Converted, String> {
    let mut reader = tar::Archive::new(Cursor::new(bytes));

    let mut out = Vec::new();
    {
        // ZipWriter needs Write + Seek, so the destination is a Cursor over
        // our buffer rather than the buffer itself.
        let mut sink = Cursor::new(&mut out);
        let mut writer = zip::ZipWriter::new(&mut sink);
        // Fixed metadata everywhere: a deterministic output beats preserving
        // mtimes nobody asked for, mirroring zip_to_tar's header policy.
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let mut written: u64 = 0;
        let mut seen: u64 = 0;

        let entries = reader
            .entries()
            .map_err(|e| format!("could not walk the tar headers: {e}") as String);
        for entry in entries? {
            let mut entry = entry.map_err(|e| format!("could not read a tar entry: {e}"))?;
            seen += 1;
            if seen > u64::from(limits.archive_entries) {
                return Err(format!(
                    "this archive holds more than {} entries",
                    limits.archive_entries
                ));
            }

            // THE NAME IS NOT TRUSTED — same rule, same refusal, as the zip
            // direction. A tar member named `../x` is refused, not repaired.
            let name = entry
                .path()
                .map_err(|e| format!("could not read an entry name: {e}"))?
                .into_owned();
            if name
                .components()
                .any(|c| c == std::path::Component::ParentDir)
                || name.is_absolute()
            {
                return Err(format!("{name:?} escapes the archive"));
            }
            // Directory members become directory entries; no bytes ride them.
            if entry.header().entry_type().is_dir() {
                writer
                    .add_directory(name.to_string_lossy(), options)
                    .map_err(|e| format!("could not add directory {}: {e}", name.display()))?;
                continue;
            }
            if !entry.header().entry_type().is_file() {
                // Links and specials carry no payload this repack preserves;
                // skipping silently would be a lie on a Class A receipt.
                return Err(format!(
                    "{} is a {:?}, which this repack does not carry",
                    name.display(),
                    entry.header().entry_type()
                ));
            }

            let remaining = limits.archive_total_bytes.saturating_sub(written);
            let mut body = Vec::new();
            let mut limited = entry.by_ref().take(remaining.saturating_add(1));
            limited
                .read_to_end(&mut body)
                .map_err(|e| format!("could not unpack {}: {e}", name.display()))?;
            if body.len() as u64 > remaining {
                return Err(format!(
                    "unpacking exceeded the {}-byte total limit at {}",
                    limits.archive_total_bytes,
                    name.display()
                ));
            }
            written += body.len() as u64;

            if limits.archive_depth < 2 && looks_like_archive(&body) {
                return Err(format!(
                    "{} is itself an archive, and the nesting limit is {}",
                    name.display(),
                    limits.archive_depth
                ));
            }

            writer
                .start_file(name.to_string_lossy(), options)
                .map_err(|e| format!("could not start {}: {e}", name.display()))?;
            writer
                .write_all(&body)
                .map_err(|e| format!("could not write {}: {e}", name.display()))?;
        }

        writer
            .finish()
            .map_err(|e| format!("could not finish the zip: {e}"))?;
    }

    Ok(Converted {
        bytes: out,
        removed: vec!["tar member timestamps and permission bits".to_string()],
    })
}

/// Wrap an uncompressed tar in a gzip stream, whole, via flate2.
///
/// Class A twice over: no member is even READ, so there is nothing to lose.
/// The SR-5 caps still apply where they can — the entry count from walking
/// headers cheaply — but decompression never happens here, which makes the
/// total-bytes cap structurally irrelevant rather than enforced.
fn tar_to_gzip(bytes: &[u8], _limits: &RunLimits) -> Result<Converted, String> {
    if !is_tar(bytes) {
        return Err("only an uncompressed tar can be wrapped as .tar.gz".into());
    }
    let encoder = flate2::write::GzEncoder::new(
        Vec::with_capacity(bytes.len() / 2),
        flate2::Compression::default(),
    );
    let mut out = encoder;
    out.write_all(bytes)
        .map_err(|e| format!("could not compress the tar: {e}"))?;
    let out = out
        .finish()
        .map_err(|e| format!("could not finish the gzip stream: {e}"))?;
    Ok(Converted {
        bytes: out,
        removed: Vec::new(),
    })
}

/// Decompress a gzip member back to the tar inside it.
///
/// The exact inverse of [`tar_to_gzip`], and Class A for the same reason: the
/// bytes that went in come back out. gzip is a single compressed stream with
/// no member list of its own -- `.tar.gz` is a tar that was compressed whole
/// -- so what emerges must still BE a tar, and this checks rather than assumes.
///
/// # The cap is on the OUTPUT, and that is the whole point
///
/// A gzip stream declares nothing trustworthy about its decompressed size, and
/// its ratio is unbounded: a few hundred kilobytes of zeros expand to gigabytes
/// (the "decompression bomb" SR-5 exists for). Bounding the INPUT bounds
/// nothing at all here. So the reader is capped at the memory limit plus one
/// byte, and producing that one extra byte is what proves the file exceeded
/// the cap rather than merely reached it.
fn gzip_to_tar(bytes: &[u8], limits: &RunLimits) -> Result<Converted, String> {
    use std::io::Read as _;

    let cap = limits.memory_bytes;
    let mut out = Vec::new();
    let mut decoder = flate2::read::GzDecoder::new(Cursor::new(bytes)).take(cap.saturating_add(1));
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("could not decompress the gzip stream: {e}"))?;

    if out.len() as u64 > cap {
        return Err(format!(
            "this gzip expands past the {cap}-byte limit. It is a decompression bomb, or it needs a larger memory budget."
        ));
    }
    if !is_tar(&out) {
        return Err(
            "this gzip does not contain a tar. oc-archive unwraps .tar.gz, not arbitrary gzip streams."
                .into(),
        );
    }
    Ok(Converted {
        bytes: out,
        removed: Vec::new(),
    })
}

/// Every member of a 7z, unpacked under the SR-5 caps.
///
/// # Why this reads into memory rather than streaming to the writer
///
/// The caps are the reason. A member's declared size is a claim; what bounds
/// anything is the running total of what actually arrives, and checking that
/// means holding each member long enough to measure it. The same shape
/// `zip_to_tar` uses, for the same reason.
fn sevenz_entries(bytes: &[u8], limits: &RunLimits) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut reader =
        sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty())
            .map_err(|e| format!("could not read the 7z archive: {e}"))?;

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut written: u64 = 0;
    let mut count: u64 = 0;
    let mut refusal: Option<String> = None;

    reader
        .for_each_entries(|entry, rd| {
            if entry.is_directory() {
                return Ok(true);
            }
            count += 1;
            if count > u64::from(limits.archive_entries) {
                refusal = Some(format!(
                    "this archive holds more than the {} entries allowed",
                    limits.archive_entries
                ));
                return Ok(false);
            }

            // THE NAME IS NOT TRUSTED. A 7z member may be called
            // `../../etc/passwd`; refusing beats sanitising, because a name we
            // would have to repair is a name whose author wanted it repaired
            // into something.
            let name = entry.name().to_string();
            if name.is_empty()
                || std::path::Path::new(&name)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                refusal = Some(format!("an entry named {name:?} escapes the archive"));
                return Ok(false);
            }

            let remaining = limits.archive_total_bytes.saturating_sub(written);
            let mut body = Vec::new();
            // One byte past the budget is enough to know. Reading further
            // would mean holding the whole bomb to discover it was a bomb.
            let mut limited = rd.take(remaining.saturating_add(1));
            if let Err(e) = limited.read_to_end(&mut body) {
                refusal = Some(format!("could not unpack {name}: {e}"));
                return Ok(false);
            }
            if body.len() as u64 > remaining {
                refusal = Some(format!(
                    "unpacking exceeded the {}-byte total limit",
                    limits.archive_total_bytes
                ));
                return Ok(false);
            }
            if looks_like_archive(&body) {
                refusal = Some(format!(
                    "{name} is itself an archive; nesting is refused rather than unpacked"
                ));
                return Ok(false);
            }
            written += body.len() as u64;
            out.push((name, body));
            Ok(true)
        })
        .map_err(|e| format!("could not walk the 7z archive: {e}"))?;

    if let Some(why) = refusal {
        return Err(why);
    }
    Ok(out)
}

/// Repack a 7z as an uncompressed tar.
fn sevenz_to_tar(bytes: &[u8], limits: &RunLimits) -> Result<Converted, String> {
    let entries = sevenz_entries(bytes, limits)?;
    let mut builder = tar::Builder::new(Vec::new());
    for (name, body) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, &name, Cursor::new(&body))
            .map_err(|e| format!("could not write {name} into the tar: {e}"))?;
    }
    let out = builder
        .into_inner()
        .map_err(|e| format!("could not finish the tar: {e}"))?;
    Ok(Converted {
        bytes: out,
        removed: vec![
            "7z compression; the tar holds the same members uncompressed".to_string(),
            "permissions, timestamps and ownership, which 7z stores and this does not carry"
                .to_string(),
        ],
    })
}

/// Repack a 7z as a deflate zip.
fn sevenz_to_zip(bytes: &[u8], limits: &RunLimits) -> Result<Converted, String> {
    let entries = sevenz_entries(bytes, limits)?;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in entries {
        writer
            .start_file(&name, options)
            .map_err(|e| format!("could not start {name} in the zip: {e}"))?;
        writer
            .write_all(&body)
            .map_err(|e| format!("could not write {name} into the zip: {e}"))?;
    }
    let out = writer
        .finish()
        .map_err(|e| format!("could not finish the zip: {e}"))?;
    Ok(Converted {
        bytes: out.into_inner(),
        removed: vec![
            "7z compression; the zip re-compresses with deflate".to_string(),
            "permissions, timestamps and ownership".to_string(),
        ],
    })
}

/// Whether these bytes are themselves an archive.
///
/// Signature-only, and deliberately a short list: this decides whether to
/// refuse for nesting, and a false positive refuses a legitimate file. The
/// formats here are the ones the format table knows, so a member this
/// recognises is one the product would otherwise unpack again.
fn looks_like_archive(body: &[u8]) -> bool {
    is_zip(body)
        || is_gzip(body)
        || body.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) // 7z
        || is_tar(body)
}
