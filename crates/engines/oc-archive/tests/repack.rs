//! `oc-archive`, driven as a real program through the real host.
//!
//! Same discipline as `oc-images`: `CARGO_BIN_EXE_oc-archive` guarantees the
//! binary exists, so nothing here can skip and report green.
//!
//! Half of these are bombs. SR-5 is the reason this worker exists, and a cap
//! nobody has watched refuse something is a cap nobody has tested.

use openconvert_core::facts::Provenance;
use openconvert_core::limits::Limits;
use openconvert_core::policy::WorkerReuse;
use openconvert_run::pool::WorkerPool;
use openconvert_sandbox::argv::EngineBin;
use std::io::Write;

/// Put `oc-archive` where `Worker::start` looks for it, once per process.
fn install_engine_beside_us() -> bool {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        let src = std::path::PathBuf::from(env!("CARGO_BIN_EXE_oc-archive"));
        let mut dst = std::env::current_exe().expect("current exe");
        dst.pop();
        dst.push(format!("oc-archive{}", std::env::consts::EXE_SUFFIX));
        // Copy when the destination is ABSENT OR STALE.
        //
        // Two failure modes, pulling in opposite directions, and both have bitten.
        //
        // Copying unconditionally on every run raced the linker: the destination
        // is `target/<profile>/deps/`, which cargo also links into, and
        // overwriting a file a linker is writing -- or a worker is executing --
        // is a sharing violation on Windows.
        //
        // Never overwriting is worse. It silently pins the test to whatever
        // binary was there first, so a protocol change rebuilds the worker,
        // leaves the stale copy in place, and the tests keep talking to the old
        // one. That is exactly what happened: `Probed` became an enum and these
        // tests failed with a protocol error against a binary from before the
        // change.
        //
        // The timestamp is the discriminator. A rebuilt worker is newer than
        // its copy; an unchanged one is not, so the common case still does no
        // write at all.
        let stale = std::fs::metadata(&dst)
            .and_then(|d| d.modified())
            .ok()
            .zip(std::fs::metadata(&src).and_then(|s| s.modified()).ok())
            .is_none_or(|(dst_time, src_time)| src_time > dst_time);
        if !stale {
            return true;
        }
        std::fs::copy(&src, &dst).is_ok() || dst.exists()
    })
}

/// A zip holding the given `(name, body)` members, stored uncompressed.
fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in members {
        w.start_file(*name, opts).expect("start");
        w.write_all(body).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A zip whose single member compresses enormously — the bomb shape.
fn zip_bomb(uncompressed: usize) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    w.start_file("bomb.bin", opts).expect("start");
    // Zeroes deflate to almost nothing, which is exactly how a bomb is built.
    w.write_all(&vec![0u8; uncompressed]).expect("write");
    w.finish().expect("finish").into_inner()
}

fn run(bytes: Vec<u8>, to: &str, limits: &Limits) -> Result<Vec<u8>, String> {
    assert!(install_engine_beside_us(), "could not place oc-archive");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);
    pool.run(
        EngineBin::Archive,
        Provenance::Untrusted,
        limits,
        bytes,
        to,
        &[],
    )
    .map(|(bytes, _, _)| bytes)
    .map_err(|e| e.to_string())
}

/// Every entry name in a tar, in order.
fn tar_names(bytes: &[u8]) -> Vec<String> {
    let mut a = tar::Archive::new(std::io::Cursor::new(bytes));
    a.entries()
        .expect("entries")
        .map(|e| {
            e.expect("entry")
                .path()
                .expect("path")
                .display()
                .to_string()
        })
        .collect()
}

/// One member's bytes from a tar.
fn tar_body(bytes: &[u8], want: &str) -> Vec<u8> {
    use std::io::Read;
    let mut a = tar::Archive::new(std::io::Cursor::new(bytes));
    for e in a.entries().expect("entries") {
        let mut e = e.expect("entry");
        if e.path().expect("path").display().to_string() == want {
            let mut out = Vec::new();
            e.read_to_end(&mut out).expect("read");
            return out;
        }
    }
    panic!("{want} not found in the tar");
}

// ---------------------------------------------------------------------------
// The conversion
// ---------------------------------------------------------------------------

/// **`Zip → Tar` repacks, and it is Class A.**
///
/// The second worker, running as a real confined process. Every member's bytes
/// come out identical — only the wrapper changed — which is what makes the
/// route's Class A claim true rather than aspirational.
#[test]
fn a_zip_repacks_into_a_tar_with_its_members_intact() {
    let src = zip_of(&[
        ("hello.txt", b"hello world"),
        ("nested/deep.bin", &[0u8, 1, 2, 3, 4, 5]),
    ]);

    let out = run(src, "tar", &Limits::defaults()).expect("repack");

    let names = tar_names(&out);
    assert!(
        names.iter().any(|n| n.ends_with("hello.txt")),
        "hello.txt is missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("deep.bin")),
        "the nested member is missing: {names:?}"
    );

    assert_eq!(
        tar_body(&out, "hello.txt"),
        b"hello world",
        "a Class A repack must not alter a member's bytes"
    );
}

/// The output is a real tar, readable by a reader that never saw the zip.
#[test]
fn the_output_is_a_tar_a_stranger_can_read() {
    let out = run(zip_of(&[("a.txt", b"A")]), "tar", &Limits::defaults()).expect("repack");
    assert!(
        out.len() > 262 && &out[257..262] == b"ustar",
        "the output does not carry tar's signature"
    );
}

/// A format this worker cannot write is refused by name.
#[test]
fn an_unsupported_target_is_refused_clearly() {
    let err = run(zip_of(&[("a.txt", b"A")]), "jpeg", &Limits::defaults())
        .expect_err("oc-archive does not write images");
    assert!(err.contains("jpeg"), "the message should name it: {err}");
}

// ---------------------------------------------------------------------------
// SR-5. The reason this worker exists.
// ---------------------------------------------------------------------------

/// **A zip bomb is refused by the byte total, counted as it unpacks.**
///
/// Spike S23 settled that `expansion_ratio` cannot do this: a legitimate disk
/// image compresses 1029× and the bomb 1028×. What works is the absolute total,
/// and it has to be counted *while* unpacking — a header's declared size is
/// written by whoever built the file.
#[test]
fn a_bomb_is_refused_by_the_total_byte_cap() {
    let tight = Limits {
        archive_total_bytes: 4096,
        ..Limits::defaults()
    };
    // 8 MB of zeroes; the zip itself is a few kilobytes.
    let bomb = zip_bomb(8 * 1024 * 1024);
    assert!(
        bomb.len() < 64 * 1024,
        "the fixture is not bomb-shaped; it is {} bytes",
        bomb.len()
    );

    let err = run(bomb, "tar", &tight).expect_err("8 MB through a 4 KB cap");
    assert!(
        err.contains("4096") && err.contains("total limit"),
        "the message should name the limit it hit: {err}"
    );
}

/// The control: the same bomb passes under a cap that fits it.
///
/// Without this, the test above is satisfied by a worker that refuses every
/// archive.
#[test]
fn the_same_bomb_unpacks_under_a_sufficient_cap() {
    let roomy = Limits {
        archive_total_bytes: 16 * 1024 * 1024,
        ..Limits::defaults()
    };
    let out = run(zip_bomb(8 * 1024 * 1024), "tar", &roomy).expect("8 MB under a 16 MB cap");
    assert!(!out.is_empty());
}

/// **Entry count is capped**, because a bomb is often millions of tiny files
/// rather than one huge one.
#[test]
fn too_many_entries_is_refused() {
    let members: Vec<(String, Vec<u8>)> = (0..200)
        .map(|i| (format!("f{i}.txt"), vec![b'x']))
        .collect();
    let refs: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let src = zip_of(&refs);

    let tight = Limits {
        archive_entries: 100,
        ..Limits::defaults()
    };
    let err = run(src.clone(), "tar", &tight).expect_err("200 entries through a 100 cap");
    assert!(
        err.contains("200") && err.contains("100"),
        "the message should name both counts: {err}"
    );

    // The control, so the cap is not satisfied by refusing everything.
    assert!(run(src, "tar", &Limits::defaults()).is_ok());
}

/// **An archive inside the archive is refused when nesting is capped.**
#[test]
fn a_nested_archive_is_refused_at_depth_one() {
    let inner = zip_of(&[("inner.txt", b"inner")]);
    let outer = zip_of(&[("payload.zip", &inner)]);

    let shallow = Limits {
        archive_depth: 1,
        ..Limits::defaults()
    };
    let err = run(outer.clone(), "tar", &shallow).expect_err("nesting is capped at 1");
    assert!(
        err.contains("payload.zip") && err.contains("nesting"),
        "the message should name the member and the reason: {err}"
    );

    // The control: the default depth allows it.
    assert!(run(outer, "tar", &Limits::defaults()).is_ok());
}

/// **A member that escapes the archive is refused, not repaired.**
///
/// `../../etc/passwd` is the traversal shape, and the name is written by
/// whoever built the zip. Sanitising it would mean deciding what the author
/// meant; refusing says the file is malformed, which it is.
#[test]
fn a_traversing_member_name_is_refused() {
    // `zip`'s writer normalises some names, so this writes the raw bytes of a
    // traversal that a hostile packer would emit.
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("../escape.txt", opts).expect("start");
    w.write_all(b"nope").expect("write");
    let src = w.finish().expect("finish").into_inner();

    match run(src, "tar", &Limits::defaults()) {
        Err(e) => assert!(
            e.contains("escapes"),
            "refused, but not for the traversal: {e}"
        ),
        // If the zip crate normalised the name on the way in, the traversal
        // never reached us -- which is a pass for the product and worth saying
        // out loud rather than asserting a refusal that could not happen.
        Ok(out) => {
            let names = tar_names(&out);
            assert!(
                names.iter().all(|n| !n.contains("..")),
                "a traversing name survived into the output: {names:?}"
            );
        }
    }
}

/// Garbage in, structured refusal out — the worker does not crash.
#[test]
fn malformed_archive_bytes_produce_a_refusal_not_a_crash() {
    let err = run(
        b"PK\x03\x04 and then nonsense".to_vec(),
        "tar",
        &Limits::defaults(),
    )
    .expect_err("that is not a readable archive");
    assert!(!err.is_empty());
}

// ---------------------------------------------------------------------------
// Phase 2 detection, in the shape an archive actually has
// ---------------------------------------------------------------------------

/// **An archive reports an entry count, not a frame count.**
///
/// `Probed` was a flat image struct — width, height, alpha, frames — so this
/// worker had nowhere to put a member count and rode it in `frames` with the
/// dimensions left at zero. It worked, and a field named for animation frames
/// carrying a member total is a fact nobody reading a receipt could interpret.
///
/// `Properties::Archive` had existed in the core the whole time. The gap was on
/// the wire, which is the half a worker can reach.
#[test]
fn a_probe_reports_the_archive_shape_with_its_entry_count() {
    assert!(install_engine_beside_us(), "could not place oc-archive");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    let src = zip_of(&[("a.txt", b"A"), ("b.txt", b"B"), ("c/d.txt", b"D")]);
    let props = pool
        .probe(
            EngineBin::Archive,
            Provenance::Untrusted,
            &Limits::defaults(),
            &src,
        )
        .expect("probe");

    match props {
        openconvert_core::facts::Properties::Archive {
            entries,
            depth,
            declared_total_bytes,
        } => {
            assert_eq!(entries, 3, "three members were indexed");
            // Both are "not determined by a probe", and saying so beats
            // guessing: reaching either means touching member bodies, which is
            // a decode. The nesting cap is enforced during the conversion,
            // where the bytes are already in hand.
            assert_eq!(depth, 0);
            assert_eq!(declared_total_bytes, None);
        }
        other => panic!("expected an archive shape, got {other:?}"),
    }
}

/// The count comes from the **index**, so a refusal costs no inflation.
///
/// That is the whole reason a probe exists for archives: a file declaring a
/// million members should cost a million index reads to refuse, not a million
/// decompressions.
#[test]
fn the_entry_count_comes_from_the_index_not_from_unpacking() {
    assert!(install_engine_beside_us(), "could not place oc-archive");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    // A bomb: one member, 8 MB when unpacked, a few KB on disk. Probing must
    // answer without inflating any of it.
    let props = pool
        .probe(
            EngineBin::Archive,
            Provenance::Untrusted,
            &Limits::defaults(),
            &zip_bomb(8 * 1024 * 1024),
        )
        .expect("probe");

    assert!(
        matches!(
            props,
            openconvert_core::facts::Properties::Archive { entries: 1, .. }
        ),
        "expected one indexed member, got {props:?}"
    );
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

/// A structurally honest SFNT: a real table directory with real offsets.
///
/// Built here rather than committed, like every other fixture in this file, and
/// verified against the real thing elsewhere — the font module's own output was
/// checked with fontTools against Candara (glyf outlines) and Source Han Sans
/// (CFF), which read both our WOFF and our WOFF2 and found every table
/// byte-identical, and Chromium renders all four files. What this holds is the
/// part that can live in the repo: the round trip is exact and the caps bite.
fn sfnt(flavour: &[u8; 4], tags: &[&[u8; 4]]) -> Vec<u8> {
    let n = tags.len() as u16;
    let entry_selector = (u16::BITS - 1 - n.max(1).leading_zeros()) as u16;
    let search_range = (1_u32 << entry_selector) * 16;

    let mut out = Vec::new();
    out.extend_from_slice(flavour);
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(search_range as u16).to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&((u32::from(n) * 16 - search_range) as u16).to_be_bytes());

    let dir_at = out.len();
    out.resize(dir_at + tags.len() * 16, 0);
    for (i, tag) in tags.iter().enumerate() {
        // Lengths that are not multiples of four, so the padding rules have
        // something to be wrong about.
        let body = vec![i as u8 + 0xA0; 13 + i * 7];
        let offset = out.len();
        out.extend_from_slice(&body);
        out.resize(out.len().div_ceil(4) * 4, 0);

        let e = dir_at + i * 16;
        out[e..e + 4].copy_from_slice(*tag);
        out[e + 4..e + 8].copy_from_slice(&0_u32.to_be_bytes());
        out[e + 8..e + 12].copy_from_slice(&(offset as u32).to_be_bytes());
        out[e + 12..e + 16].copy_from_slice(&(body.len() as u32).to_be_bytes());
    }
    out
}

/// A font goes out to WOFF and comes back byte for byte.
///
/// The whole claim of the font routes is that they are repacks: a Class A label
/// on a conversion that rewrote a `glyf` table would be false in the way this
/// project cares most about.
#[test]
fn a_font_round_trips_through_woff_unchanged() {
    let src = sfnt(
        &[0x00, 0x01, 0x00, 0x00],
        &[b"head", b"glyf", b"loca", b"name", b"OS/2"],
    );

    let woff = run(src.clone(), "woff", &Limits::defaults()).expect("ttf -> woff");
    assert_eq!(&woff[..4], b"wOFF", "the output must be a WOFF");
    assert!(woff.len() < src.len() * 2, "a wrapper, not an expansion");

    let back = run(woff, "ttf", &Limits::defaults()).expect("woff -> ttf");
    assert_eq!(
        back, src,
        "a WOFF round trip must return the original bytes"
    );
}

/// WOFF2 is Brotli over the tables, and the tables go in untransformed.
#[test]
fn a_font_reaches_woff2_and_the_signature_says_so() {
    let src = sfnt(&[0x00, 0x01, 0x00, 0x00], &[b"head", b"glyf", b"loca"]);
    let out = run(src, "woff2", &Limits::defaults()).expect("ttf -> woff2");
    assert_eq!(&out[..4], b"wOF2");
    // The flavour is carried into the header, not re-derived: a WOFF2 that
    // forgot which outlines it holds is one no reader can unwrap.
    assert_eq!(&out[4..8], &[0x00, 0x01, 0x00, 0x00]);
}

/// The route that keeps the outline question honest.
///
/// A WOFF wrapping CFF outlines unwraps to an OTF. Calling it a TTF is the
/// container rename this build refuses to do, so it is refused BY NAME and the
/// message says which route is the right one.
#[test]
fn a_cff_font_is_not_offered_as_a_ttf() {
    let src = sfnt(b"OTTO", &[b"head", b"CFF ", b"name"]);
    let woff = run(src.clone(), "woff", &Limits::defaults()).expect("otf -> woff");

    let err = run(woff.clone(), "ttf", &Limits::defaults()).expect_err("CFF is not a TTF");
    assert!(err.contains("CFF"), "{err}");
    assert!(
        err.contains("otf"),
        "the refusal must name the route that works: {err}"
    );

    assert_eq!(
        run(woff, "otf", &Limits::defaults()).expect("otf is what it holds"),
        src
    );
}

/// SR-5 reaches fonts too: a WOFF table whose zlib stream expands past the cap
/// is refused as it arrives, not after.
#[test]
fn a_font_bomb_is_refused_by_the_same_caps_as_an_archive() {
    // A real font wrapped as WOFF, then converted back under a cap far below
    // its own size.
    let src = sfnt(&[0x00, 0x01, 0x00, 0x00], &[b"head", b"glyf"]);
    let woff = run(src, "woff", &Limits::defaults()).expect("wrap");

    let mut tight = Limits::defaults();
    tight.archive_total_bytes = 8;
    let err = run(woff, "ttf", &tight).expect_err("eight bytes is not enough for a font");
    assert!(
        err.contains("limit"),
        "the refusal must name the cap it crossed: {err}"
    );
}
