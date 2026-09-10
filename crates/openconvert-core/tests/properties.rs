//! The properties. Pure — no fixtures, no files, no clock.
//!
//! Three of these exist because the v0.6 revision introduced three enforcement
//! claims and tested none of them. That is the defect this project regenerates
//! every round: a feature and its guarantee written in one edit, with the test
//! written in none. They are called out below rather than blended in.

use openconvert_core::facts::Provenance;
use openconvert_core::limits::{LimitCeiling, Limits};
use openconvert_core::policy::{Policy, WorkerReuse};
use openconvert_core::wire::WireLimits;

const ALL_REUSE: [WorkerReuse; 2] = [WorkerReuse::Balanced, WorkerReuse::Isolated];
const ALL_PROV: [Provenance; 2] = [Provenance::Trusted, Provenance::Untrusted];

// ---------------------------------------------------------------------------
// SR-20 — worker_reuse is a one-way ratchet
// ---------------------------------------------------------------------------

/// A policy layer may raise `worker_reuse` and may never lower it.
///
/// This is what lets a managed deployment pin `Isolated` while a user-level
/// config file remains writable by the user. Without it, the strict setting is
/// a suggestion — and the file that would override it is one an attacker who
/// is already the user can edit (09 §8, A16).
///
/// Exhaustive over every ordered pair of settings, which is 4 cases: the
/// enum is small enough that a property test can simply be a total one.
#[test]
fn worker_reuse_never_lowers() {
    for start in ALL_REUSE {
        for requested in ALL_REUSE {
            let p = Policy::default().raise_worker_reuse(start);
            let after = p.raise_worker_reuse(requested).worker_reuse();
            assert!(
                after >= start,
                "layering {requested:?} onto {start:?} produced {after:?}, \
                 which is weaker than where it started"
            );
        }
    }
}

/// Specifically: `Isolated` cannot be undone. Stated separately from the
/// general property because this is the case a reader will want to check by
/// eye, and a general ordering claim is easy to satisfy vacuously.
#[test]
fn isolated_is_a_trapdoor() {
    let pinned = Policy::default().raise_worker_reuse(WorkerReuse::Isolated);
    let attempted = pinned.raise_worker_reuse(WorkerReuse::Balanced);
    assert_eq!(
        attempted.worker_reuse(),
        WorkerReuse::Isolated,
        "a later layer lowered a pinned Isolated setting"
    );
}

// ---------------------------------------------------------------------------
// SR-20 — an untrusted file never shares a worker
// ---------------------------------------------------------------------------

/// No file of untrusted provenance shares a worker process with another file,
/// under any setting.
///
/// This is the mitigation that makes `Balanced` acceptable as a default. The
/// residual it bounds is real and measured: a capability can be revoked from a
/// running worker (verified, including inside an AppContainer), but revocation
/// removes the handle and **not the memory** — so an engine compromised by one
/// file can carry its bytes into the next file in the same batch.
///
/// 09 §8 discloses that residual and names this as its bound. A mitigation
/// named in the threat model and tested nowhere is not a mitigation.
#[test]
fn untrusted_never_shares_a_worker() {
    for reuse in ALL_REUSE {
        for a in ALL_PROV {
            for b in ALL_PROV {
                let shared = reuse.may_share(a, b);
                if a == Provenance::Untrusted || b == Provenance::Untrusted {
                    assert!(
                        !shared,
                        "{reuse:?} allowed an untrusted file to share a worker ({a:?}, {b:?})"
                    );
                }
            }
        }
    }
}

/// `Isolated` shares with nothing at all, including two trusted files.
#[test]
fn isolated_shares_with_nothing() {
    for a in ALL_PROV {
        for b in ALL_PROV {
            assert!(
                !WorkerReuse::Isolated.may_share(a, b),
                "Isolated allowed sharing between {a:?} and {b:?}"
            );
        }
    }
}

/// The control for the two properties above: `Balanced` **does** share between
/// trusted files. Without this, both would pass if `may_share` simply always
/// returned false — and a setting that never reuses anything is not the setting
/// the latency budget depends on.
#[test]
fn balanced_actually_reuses() {
    assert!(
        WorkerReuse::Balanced.may_share(Provenance::Trusted, Provenance::Trusted),
        "Balanced refused to reuse between two trusted files, so the reuse path \
         is dead and the Windows latency gate has no fix"
    );
}

// ---------------------------------------------------------------------------
// SR-5 / I14 — engine facts cannot widen a limit
// ---------------------------------------------------------------------------

/// A value crossing the wire may narrow a limit and may never widen one.
///
/// The trust inversion: `Properties` arrives from a **confined, possibly
/// compromised** engine and flows straight into routing. An engine that reports
/// an enormous width to inflate its own pixel budget is refused at the boundary
/// rather than believed (spike S6).
#[test]
fn engine_facts_cannot_widen_limits() {
    let base = Limits::defaults();
    let ceiling = LimitCeiling::new(base);

    let hostile = WireLimits {
        memory_bytes: u64::MAX,
        decode_pixels: 4_000_000_000,
        archive_depth: u8::MAX,
        archive_entries: u32::MAX,
    };

    let got = hostile.into_limits(&ceiling, base);
    assert_eq!(got.memory_bytes, base.memory_bytes);
    assert_eq!(got.decode_pixels, base.decode_pixels);
    assert_eq!(got.archive_depth, base.archive_depth);
    assert_eq!(got.archive_entries, base.archive_entries);
}

/// Every clamped field, in both directions — including the two `Duration`s.
///
/// **This test exists because mutation testing found the hole.** The wire test
/// above exercises four of the ten fields `clamp` touches, and the two
/// `Duration` fields go through a separate comparison helper that nothing
/// reached. `cargo-mutants` flipped its `<` to `>` and every test still passed.
///
/// That mutant is not cosmetic: with the comparison reversed, `clamp` returns
/// the *larger* duration, so a compromised engine reporting a four-hour
/// `wall_time` would have its request granted by the very function whose job is
/// to refuse it. SR-5 and I14 would both have been false while green.
#[test]
fn clamp_narrows_every_field_including_durations() {
    let base = Limits::defaults();
    let ceiling = LimitCeiling::new(base);

    let mut wider = base;
    wider.wall_time = base.wall_time * 4;
    wider.cpu_time = base.cpu_time * 4;
    wider.memory_bytes = base.memory_bytes * 4;
    wider.output_bytes = base.output_bytes * 4;
    wider.temp_bytes = base.temp_bytes * 4;
    wider.expansion_ratio = base.expansion_ratio.saturating_mul(4);
    wider.decode_pixels = base.decode_pixels * 4;
    wider.archive_depth = u8::MAX;
    wider.archive_entries = base.archive_entries.saturating_mul(4);
    wider.archive_total_bytes = base.archive_total_bytes * 4;

    assert_eq!(
        ceiling.clamp(wider),
        base,
        "a wider request survived the clamp — an engine can widen its own limits"
    );

    let mut narrower = base;
    narrower.wall_time = base.wall_time / 4;
    narrower.cpu_time = base.cpu_time / 4;
    narrower.memory_bytes = base.memory_bytes / 4;
    narrower.output_bytes = base.output_bytes / 4;
    narrower.temp_bytes = base.temp_bytes / 4;
    narrower.expansion_ratio = base.expansion_ratio / 4;
    narrower.decode_pixels = base.decode_pixels / 4;
    narrower.archive_depth = base.archive_depth / 4;
    narrower.archive_entries = base.archive_entries / 4;
    narrower.archive_total_bytes = base.archive_total_bytes / 4;

    assert_eq!(
        ceiling.clamp(narrower),
        narrower,
        "a narrower request was ignored — clamp is not honouring engine limits"
    );

    // Equal in, equal out. Catches a `<=` for `<` swap in the duration helper,
    // which the two assertions above cannot distinguish.
    assert_eq!(ceiling.clamp(base), base);
}

/// An `InputToken` round-trips. Trivial, and untested until a mutant returned
/// a constant from `get()` and nothing noticed — which would have made every
/// token address handle zero.
#[test]
fn input_token_round_trips() {
    use openconvert_core::facts::InputToken;
    for id in [0_u64, 1, 42, u64::MAX] {
        assert_eq!(InputToken::new(id).get(), id);
    }
    assert_ne!(InputToken::new(1), InputToken::new(2));
}

/// The control: an engine reporting *smaller* limits is believed. Without this
/// the property above would pass if `clamp` ignored its input entirely — and
/// narrowing is the whole reason engines report limits at all.
#[test]
fn engine_facts_can_narrow_limits() {
    let base = Limits::defaults();
    let ceiling = LimitCeiling::new(base);

    let modest = WireLimits {
        memory_bytes: 1024,
        decode_pixels: 64,
        archive_depth: 2,
        archive_entries: 8,
    };

    let got = modest.into_limits(&ceiling, base);
    assert_eq!(
        got.memory_bytes, 1024,
        "a narrower memory limit was ignored"
    );
    assert_eq!(got.decode_pixels, 64, "a narrower pixel limit was ignored");
    assert_eq!(got.archive_depth, 2);
    assert_eq!(got.archive_entries, 8);
}

// ---------------------------------------------------------------------------
// The measured defaults
// ---------------------------------------------------------------------------

/// The defaults are the measured ones, and `expansion_ratio` in particular is
/// high enough to be inert as a defence.
///
/// It is pinned by a test because the temptation to "tighten" it is exactly the
/// mistake: a legitimate 200 MB zeroed disk image compresses 1029×, and a zip
/// bomb 1028× (spike S23). Any threshold that catches the bomb refuses disk
/// images, VM images and database dumps. Depth and absolute size do the work.
#[test]
fn measured_defaults_are_what_was_measured() {
    let d = Limits::defaults();
    assert_eq!(
        d.archive_depth, 32,
        "legit nesting reached 8; the bomb was 41"
    );
    assert_eq!(
        d.archive_entries, 100_000,
        "the archivist workload in 01 §7 has 40,000 entries — this cannot be tightened"
    );
    assert!(
        d.expansion_ratio >= 10_000,
        "expansion_ratio is a tripwire, not a control: 1029x legitimate vs 1028x bomb"
    );

    // Exact byte values, not shifts. Mutation testing flipped `1 << 30` to
    // `1 >> 30` and every assertion still passed — leaving a 0-byte memory
    // ceiling that would refuse every conversion, or, read the other way, a
    // number nobody had actually checked.
    assert_eq!(d.memory_bytes, 1_073_741_824, "1 GiB");
    assert_eq!(
        d.decode_pixels, 268_435_456,
        "256 Mpx — about 1 GiB at 4 bytes/px"
    );
    assert_eq!(d.output_bytes, 4_294_967_296, "4 GiB");
    assert_eq!(d.temp_bytes, 8_589_934_592, "8 GiB");
    assert_eq!(d.archive_total_bytes, 8_589_934_592, "8 GiB");
    assert_eq!(
        d.cpu_time,
        d.wall_time * 2,
        "cpu_time is 2x wall_time by design"
    );
}
