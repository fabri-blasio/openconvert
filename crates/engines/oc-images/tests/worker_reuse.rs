//! `worker_reuse` (D8) and SR-20, driving real processes.
//!
//! # Why reuse is proved by counting, not by timing
//!
//! The obvious test — "the second conversion is faster" — measures a 38 ms
//! difference on a machine also running a compiler, and would be the flakiest
//! assertion in the project. `WorkerPool::idle_count` is exact: a pool that
//! reuses parks one worker and keeps parking the same one, so the count stays
//! at 1 across any number of conversions. A pool that does not reuse either
//! parks nothing (0) or accumulates (n).
//!
//! # Why here
//!
//! `WorkerPool` calls `Worker::start`, which resolves an engine **beside the
//! running executable** and never through `PATH`. Under `cargo test` that is
//! `target/<profile>/deps/`, so the binary is copied there first — which
//! exercises the production resolution path rather than working around it.

use openconvert_core::facts::Provenance;
use openconvert_core::limits::Limits;
use openconvert_core::policy::WorkerReuse;
use openconvert_run::pool::WorkerPool;
use openconvert_sandbox::argv::EngineBin;
use std::path::PathBuf;

/// Put `oc-images` where `Worker::start` looks for it — **once per process**.
///
/// The first version copied on every call and raced with itself: tests run in
/// parallel, all five saw the destination missing, and the ones that lost found
/// the file locked by the winner's copy still in flight. Two of five failed.
///
/// The same shape as the AppContainer race in `worker_client` — a setup step
/// that is idempotent in its *effect* but not in its *execution*. `OnceLock`
/// makes it both.
fn install_engine_beside_us() -> bool {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<bool> = OnceLock::new();

    *INSTALLED.get_or_init(|| {
        let src = PathBuf::from(env!("CARGO_BIN_EXE_oc-images"));
        let mut dst = std::env::current_exe().expect("current exe");
        dst.pop();
        dst.push(format!("oc-images{}", std::env::consts::EXE_SUFFIX));
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

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode");
    out.into_inner()
}

/// **Balanced reuse actually reuses**, and the count proves which process ran.
#[test]
fn balanced_reuses_one_worker_across_trusted_files() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    for i in 0..4 {
        let (bytes, _, confinement) = pool
            .run(
                EngineBin::Images,
                Provenance::Trusted,
                &Limits::defaults(),
                png(16, 16),
                "jpeg",
                &[],
            )
            .expect("convert");
        assert!(
            bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            "run {i} was not a JPEG"
        );
        assert_eq!(
            pool.idle_count(),
            1,
            "after {} conversions the pool holds {} workers -- reuse is not happening",
            i + 1,
            pool.idle_count()
        );

        // **The evidence, not the plan**, and it says different things on
        // different platforms because different things are true.
        //
        // This assertion used to be unconditional and failed on Linux -- where
        // the product was behaving exactly as designed. It then failed the
        // OTHER way, twice, as the platform caught up: first `readback::read_back`
        // returned an empty list off Windows and the receipt said "the worker
        // reported no read-back", which was honest; now the worker applies
        // Landlock + seccomp itself before reading a byte and reports both
        // engaged, which is also honest. The one answer this test refuses is
        // an unearned claim: engagement must name mechanisms this platform
        // actually applies.
        if cfg!(windows) {
            assert!(
                confinement.contains("read back in the worker"),
                "run {i} recorded no read-back: {confinement}"
            );
            assert!(
                confinement.contains("engaged AppContainer")
                    || confinement.contains("AppContainer,")
                    || confinement.contains(", AppContainer"),
                "run {i} did not report the container as engaged: {confinement}"
            );
        } else if cfg!(target_os = "linux") {
            assert!(
                confinement.contains("read back in the worker"),
                "run {i} recorded no read-back: {confinement}"
            );
            // Both mechanisms, in the ENGAGED section specifically. A check
            // for the bare names would also match them in the failures
            // section -- and the first draft of this line asserted against
            // "engaged nothing", which matches its own success text inside
            // "not engaged nothing".
            assert!(
                confinement.contains("engaged Landlock, Seccomp"),
                "run {i} did not report both Linux mechanisms as engaged: {confinement}"
            );
        } else {
            assert!(
                confinement.contains("no read-back"),
                "a platform with no confinement probe claimed one: {confinement}"
            );
        }
    }
}

/// **`Isolated` gets a fresh process every time.**
///
/// The setting the user turns on when they want the 38 ms. It is worth nothing
/// if the pool quietly reuses anyway, and nothing in the type system stops it.
#[test]
fn isolated_never_parks_and_so_never_reuses() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Isolated);

    for _ in 0..3 {
        pool.run(
            EngineBin::Images,
            Provenance::Trusted,
            &Limits::defaults(),
            png(16, 16),
            "png",
            &[],
        )
        .expect("convert");
        assert_eq!(
            pool.idle_count(),
            0,
            "Isolated parked a worker; the next file would land in a used process"
        );
    }
}

/// **SR-20: a worker that touched untrusted input is never parked.**
///
/// The half of the rule that is easy to get wrong. Refusing to reuse *for* an
/// untrusted file while still returning that worker to the pool satisfies the
/// obvious reading and breaks the real one — the next trusted file would then
/// be handed a process that had already run an attacker's bytes.
#[test]
fn an_untrusted_file_leaves_nothing_behind_for_the_next_one() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    // A trusted file first, so there IS something in the pool to misuse.
    pool.run(
        EngineBin::Images,
        Provenance::Trusted,
        &Limits::defaults(),
        png(16, 16),
        "png",
        &[],
    )
    .expect("convert");
    assert_eq!(
        pool.idle_count(),
        1,
        "the trusted file should park a worker"
    );

    // The untrusted one must neither take that worker nor add its own.
    pool.run(
        EngineBin::Images,
        Provenance::Untrusted,
        &Limits::defaults(),
        png(16, 16),
        "png",
        &[],
    )
    .expect("convert");
    assert_eq!(
        pool.idle_count(),
        1,
        "the untrusted file's worker was parked; a later file could land in it"
    );
}

/// A failed conversion does not park the process that failed.
///
/// Its state is not something anyone can vouch for, and the cheapest correct
/// thing to do with it is end it.
#[test]
fn a_failed_conversion_parks_nothing() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    pool.run(
        EngineBin::Images,
        Provenance::Trusted,
        &Limits::defaults(),
        png(16, 16),
        "heic",
        &[],
    )
    .expect_err("oc-images cannot write HEIC");

    assert_eq!(
        pool.idle_count(),
        0,
        "a worker that just failed was returned to the pool"
    );
}

/// Reuse is refused across a different memory cap.
///
/// The Job Object's limit is fixed at spawn — `Limits` travel with each `Run`,
/// the job does not — so a reused worker would silently run the second file
/// under the first file's ceiling. That is I4 broken in the direction that
/// looks like an unexplained failure on a file which converts fine alone.
#[test]
fn a_worker_is_not_reused_under_a_different_memory_cap() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    let small = Limits {
        memory_bytes: 1 << 28,
        ..Limits::defaults()
    };
    let large = Limits {
        memory_bytes: 1 << 29,
        ..Limits::defaults()
    };

    pool.run(
        EngineBin::Images,
        Provenance::Trusted,
        &small,
        png(16, 16),
        "png",
        &[],
    )
    .expect("convert");
    assert_eq!(pool.idle_count(), 1);

    pool.run(
        EngineBin::Images,
        Provenance::Trusted,
        &large,
        png(16, 16),
        "png",
        &[],
    )
    .expect("convert");
    assert_eq!(
        pool.idle_count(),
        2,
        "the larger cap should have started its own worker, leaving both parked"
    );
}

/// **A file larger than one protocol frame.**
///
/// `protocol.rs` says, in its own words, that nothing on this protocol carries
/// file content — "content moves through pre-opened descriptors, which is what
/// keeps this number small enough to be a real bound rather than a formality".
/// `MAX_FRAME_BYTES` is 1 MiB. `Limits::defaults` permits a 4 GiB output and a
/// ~1 GiB decode.
///
/// So: does a 2 MiB image convert?
#[test]
fn an_image_larger_than_one_frame_converts() {
    assert!(install_engine_beside_us(), "could not place oc-images");
    let mut pool = WorkerPool::new(WorkerReuse::Balanced);

    // Random-ish pixels so PNG cannot compress it away.
    let big = {
        let img = image::RgbaImage::from_fn(1024, 1024, |x, y| {
            let n = x
                .wrapping_mul(2654435761)
                .wrapping_add(y.wrapping_mul(40503));
            image::Rgba([(n >> 16) as u8, (n >> 8) as u8, n as u8, 255])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode");
        out.into_inner()
    };
    assert!(
        big.len() > (1 << 20),
        "the fixture must exceed one frame; it is {} bytes",
        big.len()
    );

    let (bytes, _, _) = pool
        .run(
            EngineBin::Images,
            Provenance::Trusted,
            &Limits::defaults(),
            big,
            "jpeg",
            &[],
        )
        .expect("a 2 MiB image is well inside every documented limit");
    assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
}
