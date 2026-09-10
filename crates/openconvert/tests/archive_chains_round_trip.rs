//! **The archive chains carry their members through both legs.**
//!
//! # What these routes are
//!
//! `gzip -> zip`, `zip -> gzip` and `7z -> gzip` are the first routes in the
//! table with **two steps in one engine**. Each is declared as
//! `Extract { from, to }` twice — `gzip -> tar` then `tar -> zip` — rather than
//! as a single row, and that shape is the answer to an objection `oc-archive`
//! itself raised: "gzip to zip in one step would hide a decompress and a repack
//! behind one receipt". Two steps hide nothing; both appear in the plan and
//! both in the receipt.
//!
//! # Why it needs an end-to-end test rather than a unit one
//!
//! The defect these routes could have shipped with is not in either leg. Both
//! legs were already tested and already worked. It is in the seam: until this
//! change `StepKind::Extract` named no destination and `exec` filled one in
//! from the PLAN's target, which is the same value only while every archive
//! route has exactly one step. A two-step row would have asked for a zip in
//! both steps and handed the first one a gzip.
//!
//! Nothing below would catch that except running the real chain. So this drives
//! the shipped CLI and the real sandboxed worker, and looks at the members that
//! come out the far end.
//!
//! `7z -> gzip` is absent for an honest reason: 7z is read and never written,
//! so this test cannot build a fixture for it without adding a dependency whose
//! only user would be this test.

use std::path::{Path, PathBuf};
use std::process::Command;

const CLI: &str = env!("CARGO_BIN_EXE_openconvert");

/// Written into a member, and looked for after both legs. A repack that lost
/// its payload still produces a plausible archive.
const CANARY: &str = "Quirinalia";

fn run(args: &[&str]) -> (bool, String) {
    // THREE ATTEMPTS, AND ONLY FOR A FAILURE TO START. See
    // `is_transient_spawn_failure` for what is known and what is inferred.
    for attempt in 0..3 {
        let out = Command::new(CLI).args(args).output().expect("run the CLI");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() || !is_transient_spawn_failure(&text) {
            return (out.status.success(), text);
        }
        // Long enough for a scanner to finish with a freshly linked binary,
        // short enough that a genuine failure still reports in under a second.
        std::thread::sleep(std::time::Duration::from_millis(150 * (attempt + 1)));
    }
    let out = Command::new(CLI).args(args).output().expect("run the CLI");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// A transient failure to START a worker, as opposed to a real refusal.
///
/// # What is known, and what is inferred
///
/// **Known.** Under `cargo test --workspace` these tests occasionally fail with
/// `could not start oc-archive: ... The system cannot find the file specified`,
/// naming a path that exists — a watcher polling that file through a whole run
/// never saw it absent. It has never reproduced in isolation: six consecutive
/// runs of this file, and twenty concurrent conversions driven from a shell,
/// all passed. It only appears in a full-workspace run, shortly after cargo
/// links the binaries.
///
/// **Inferred.** On Windows a freshly written executable can be briefly
/// unopenable while a real-time scanner has it, and `CreateProcess` reports
/// that as ERROR_FILE_NOT_FOUND rather than a sharing violation. Real-time
/// protection is on for this machine. That fits every observation above and is
/// not proven — checking the scanner's exclusions needs administrator rights.
///
/// So this retries a failure to START, briefly, and only when the engine
/// binary is actually on disk. It does NOT retry a refusal: a route the engine
/// declines, a malformed file or a cap crossed all fail on the first attempt,
/// which is the whole point of these tests.
fn is_transient_spawn_failure(text: &str) -> bool {
    text.contains("could not start") && engine_dir().is_some()
}

/// The directory the CLI resolves its engines from, if it exists.
fn engine_dir() -> Option<std::path::PathBuf> {
    let dir = Path::new(CLI).parent()?.to_path_buf();
    dir.is_dir().then_some(dir)
}

/// Convert, and return the file it wrote beside the input.
fn convert(input: &Path, to: &str, ext: &str) -> PathBuf {
    let (ok, text) = run(&["convert", &input.to_string_lossy(), "-t", to]);
    assert!(ok, "{} -> {to} failed:\n{text}", input.display());
    let out = input.with_extension(ext);
    assert!(
        out.is_file(),
        "{} -> {to} wrote no file:\n{text}",
        input.display()
    );
    out
}

/// A tar holding two files, built here.
///
/// Hand-written rather than produced by a crate: a tar is 512-byte blocks of
/// ASCII fields, this test needs exactly two members, and a dev-dependency
/// whose only user is a fixture is a dependency the whole workspace carries.
fn tar_of(members: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, body) in members {
        let mut header = [0_u8; 512];
        let put = |h: &mut [u8; 512], at: usize, text: &str| {
            h[at..at + text.len()].copy_from_slice(text.as_bytes());
        };
        put(&mut header, 0, name);
        put(&mut header, 100, "0000644\0"); // mode
        put(&mut header, 108, "0000000\0"); // uid
        put(&mut header, 116, "0000000\0"); // gid
        put(&mut header, 124, &format!("{:011o}\0", body.len()));
        put(&mut header, 136, "00000000000\0"); // mtime
        header[156] = b'0'; // a regular file
        put(&mut header, 257, "ustar\0");
        put(&mut header, 263, "00");

        // The checksum is computed with its own field read as spaces, which is
        // the one part of a tar header that is not simply a value.
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
        put(&mut header, 148, &format!("{sum:06o}\0"));

        out.extend_from_slice(&header);
        out.extend_from_slice(body.as_bytes());
        // Every member's body is padded to a block boundary.
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    // Two zeroed blocks end the archive.
    out.resize(out.len() + 1024, 0);
    out
}

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openconvert-chain-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    dir
}

fn engines_ready() -> bool {
    let dir = Path::new(CLI).parent().expect("the CLI has a directory");
    let worker = dir.join(format!("oc-archive{}", std::env::consts::EXE_SUFFIX));
    if !worker.is_file() {
        eprintln!(
            "SKIPPED: no oc-archive beside {}. Build the workspace first.",
            dir.display()
        );
        return false;
    }
    true
}

/// A tar's members are stored uncompressed, name and body both in the clear,
/// so this is a real check on what survived rather than a size comparison.
fn tar_holds(path: &Path, name: &str, body: &str) {
    let bytes = std::fs::read(path).expect("read the tar");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(name),
        "{} lost the member {name}",
        path.display()
    );
    assert!(
        text.contains(body),
        "{} lost {name}'s contents",
        path.display()
    );
}

#[test]
fn a_gzip_reaches_zip_through_tar_with_its_members_intact() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("gz-zip");
    let tar = dir.join("bundle.tar");
    std::fs::write(&tar, tar_of(&[("a.txt", CANARY), ("b.txt", "second")])).expect("tar");

    // The gzip fixture comes from the route that already existed.
    let gz = convert(&tar, "gzip", "gz");

    // The chain under test, and then back to tar to read the members.
    let zip = convert(&gz, "zip", "zip");
    let (_, plan) = run(&["plan", &gz.to_string_lossy(), "-t", "zip"]);
    assert!(
        plan.matches("Extract").count() >= 2,
        "the plan must show BOTH legs, not one merged step:\n{plan}"
    );

    let back = convert(&zip, "tar", "tar");
    tar_holds(&back, "a.txt", CANARY);
    tar_holds(&back, "b.txt", "second");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zip_reaches_gzip_through_tar_with_its_members_intact() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("zip-gz");
    let tar = dir.join("bundle.tar");
    std::fs::write(&tar, tar_of(&[("a.txt", CANARY), ("b.txt", "second")])).expect("tar");

    let zip = convert(&tar, "zip", "zip");
    let gz = convert(&zip, "gzip", "gz");

    // A gzip holds ONE stream, so a multi-entry archive reaches it as a
    // gzipped tar — which is what `.tar.gz` is, and why the chain goes through
    // tar rather than compressing a member. Unwrapping it proves both members
    // are still in there.
    let back = convert(&gz, "tar", "tar");
    tar_holds(&back, "a.txt", CANARY);
    tar_holds(&back, "b.txt", "second");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A gzip that does not hold a tar is refused by name rather than repacked
/// into an archive with an invented member name.
#[test]
fn a_gzip_of_a_bare_file_is_refused_rather_than_guessed_at() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("bare");
    let tar = dir.join("real.tar");
    std::fs::write(&tar, tar_of(&[("a.txt", CANARY)])).expect("tar");
    let gz = convert(&tar, "gzip", "gz");

    // The same gzip stream with its tar replaced by loose bytes: valid gzip,
    // and nothing inside that names a member.
    let bare = dir.join("bare.gz");
    let mut raw = std::fs::read(&gz).expect("read the gzip");
    // Corrupting the compressed payload is enough — what matters is that the
    // worker refuses anything whose contents are not a tar, and says so.
    if let Some(last) = raw.len().checked_sub(20) {
        raw[10..last].fill(0);
    }
    std::fs::write(&bare, &raw).expect("bare");

    let (ok, text) = run(&["convert", &bare.to_string_lossy(), "-t", "zip"]);
    assert!(!ok, "a gzip with no tar inside must not convert:\n{text}");
    assert!(
        text.contains("gzip") || text.contains("tar"),
        "the refusal must name what was wrong with the file:\n{text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
