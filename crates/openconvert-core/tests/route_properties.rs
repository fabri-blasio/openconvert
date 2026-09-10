//! The properties over `route()`. Pure — no fixtures, no files, no clock.
//!
//! `cargo test -p openconvert-core` runs in under a second and touches nothing.
//! That is not a nicety: it is what makes these runnable on every keystroke,
//! and it is only possible because the core has no I/O.
//!
//! Each test names the invariant or requirement it holds. Several carry a
//! **control** — a test asserting the property is not satisfied vacuously —
//! because a property over an empty set is true and useless, and this project
//! has already shipped one test that passed when its own input was deleted.

use std::collections::BTreeSet;

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::environment::{EngineEntry, Environment};
use openconvert_core::facts::Properties;
use openconvert_core::format::{FormatId, MediaKind, TABLE};
use openconvert_core::isolation::{
    FsConfinement, FsRequirement, Isolation, IsolationFloor, NetConfinement, PrivDrop,
    ResourceEnforcement, SandboxProfile, Strength, SyscallFilter,
};
use openconvert_core::plan::{Class, NoRoute, PlanRequest, StepKind, Warning};
use openconvert_core::policy::Policy;
use openconvert_core::route::{route, Requirement, Route, RouteTable};
use openconvert_core::target::{Operation, Target};

// ---------------------------------------------------------------------------
// Fixtures — profiles, not files
// ---------------------------------------------------------------------------

/// Everything the platform offers.
fn full_profile() -> SandboxProfile {
    SandboxProfile::for_test(
        FsConfinement::Landlock,
        NetConfinement::EmptyNetns,
        SyscallFilter::SeccompBpf,
        ResourceEnforcement::CgroupV2,
        PrivDrop::UserNs,
    )
}

/// The hardened-Debian case: Landlock and seccomp, **no user namespace**.
///
/// This is the machine the whole v0.5 ladder rewrite exists for. v0.4 dropped
/// it to `Minimal` and refused it — the outcome the profile was invented to
/// prevent.
fn reduced_profile() -> SandboxProfile {
    SandboxProfile::for_test(
        FsConfinement::Landlock,
        NetConfinement::SeccompBlock,
        SyscallFilter::SeccompBpf,
        ResourceEnforcement::Rlimit,
        PrivDrop::None,
    )
}

/// Filesystem confined, network **not** deniable.
fn no_network_denial() -> SandboxProfile {
    SandboxProfile::for_test(
        FsConfinement::Landlock,
        NetConfinement::None,
        SyscallFilter::SeccompBpf,
        ResourceEnforcement::Rlimit,
        PrivDrop::None,
    )
}

/// Every profile a prober could plausibly produce.
fn all_profiles() -> Vec<SandboxProfile> {
    vec![
        full_profile(),
        reduced_profile(),
        no_network_denial(),
        SandboxProfile::for_test(
            FsConfinement::None,
            NetConfinement::SeccompBlock,
            SyscallFilter::SeccompBpf,
            ResourceEnforcement::Rlimit,
            PrivDrop::None,
        ),
        SandboxProfile::for_test(
            FsConfinement::None,
            NetConfinement::None,
            SyscallFilter::None,
            ResourceEnforcement::None,
            PrivDrop::None,
        ),
        SandboxProfile::for_test(
            FsConfinement::BrowserOrigin,
            NetConfinement::Csp,
            SyscallFilter::None,
            ResourceEnforcement::Rlimit,
            PrivDrop::None,
        ),
    ]
}

fn engines(available: bool) -> Vec<EngineEntry> {
    [
        "oc-images",
        "oc-pdf",
        "oc-archive",
        "oc-audio",
        "libmp3lame",
        "libopus",
    ]
    .into_iter()
    .map(|name| EngineEntry {
        name,
        memory_safe: false,
        available,
    })
    .collect()
}

fn env_with(profile: SandboxProfile) -> Environment {
    Environment::new(profile, engines(true), RouteTable::v1())
}

/// Every (input, target) pair the table can express, plus the operations.
fn all_requests() -> Vec<PlanRequest> {
    let mut out: Vec<PlanRequest> = RouteTable::v1()
        .all()
        .iter()
        .map(|r| PlanRequest {
            input: r.from,
            target: Target::Format(r.to),
            polyglot: false,
        })
        .collect();
    for op in [Operation::StripMetadata, Operation::Remux] {
        out.push(PlanRequest {
            input: FormatId::Jpeg,
            target: Target::Operation(op),
            polyglot: false,
        });
    }
    out.push(PlanRequest {
        input: FormatId::Unknown,
        target: Target::Format(FormatId::Png),
        polyglot: false,
    });
    out.push(PlanRequest {
        input: FormatId::Png,
        target: Target::Format(FormatId::Pdf),
        polyglot: false,
    });
    out
}

fn video_props() -> Properties {
    Properties::Video {
        duration_ms: 1000,
        width: 1920,
        height: 1080,
        video: Some(VideoCodec::H264),
        audio: Some(AudioCodec::Aac),
    }
}

// ---------------------------------------------------------------------------
// 1 · Every step carries limits — I4, SR-5
// ---------------------------------------------------------------------------

#[test]
fn every_step_has_limits() {
    let env = env_with(full_profile());
    let policy = Policy::default();
    let mut steps_seen = 0;

    for req in all_requests() {
        for step in route(req, video_props(), &policy, &env).steps() {
            // Limits is a non-optional field, so this cannot be absent; what a
            // test can check is that it was never left at something degenerate.
            assert!(
                step.limits.memory_bytes > 0,
                "{req:?} produced a zero memory limit"
            );
            assert!(
                step.limits.decode_pixels > 0,
                "{req:?} produced a zero pixel limit"
            );
            assert!(
                !step.limits.wall_time.is_zero(),
                "{req:?} produced a zero wall time"
            );
            steps_seen += 1;
        }
    }
    assert!(
        steps_seen > 20,
        "only {steps_seen} steps examined — the corpus is too small to mean anything"
    );
}

// ---------------------------------------------------------------------------
// 2 · Every step carries an isolation — I10, SR-2
// ---------------------------------------------------------------------------

#[test]
fn every_step_has_isolation() {
    let env = env_with(full_profile());
    let policy = Policy::default();
    for req in all_requests() {
        for step in route(req, video_props(), &policy, &env).steps() {
            match step.isolation {
                Isolation::InProcess => {}
                Isolation::Sandboxed(p) => assert!(
                    p.denies_network(),
                    "{req:?} produced a sandboxed step whose profile does not deny network"
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3 · No memory-unsafe engine runs in-process — SR-2
// ---------------------------------------------------------------------------

/// The most consequential property in the file.
///
/// A format without a pure-Rust parser must never be `InProcess`. There is
/// exactly one branch in `route()` that produces `InProcess`, guarded by the
/// format table — a second branch is how the v0.4 hole reopens.
#[test]
fn no_unsafe_parser_runs_in_process() {
    let env = env_with(full_profile());
    let policy = Policy::default();

    for req in all_requests() {
        let plan = route(req, video_props(), &policy, &env);
        for step in plan.steps() {
            if matches!(step.isolation, Isolation::InProcess) {
                assert!(
                    req.input.has_pure_rust_parser(),
                    "{:?} has no pure-Rust parser and was routed IN-PROCESS",
                    req.input
                );
            }
        }
    }
}

/// Control for the property above.
///
/// Without this, `no_unsafe_parser_runs_in_process` is satisfied by a `route()`
/// that sandboxes everything — including PNG, which would be correct and
/// catastrophically slow, and which nothing else here would notice.
#[test]
fn pure_rust_formats_do_run_in_process() {
    let env = env_with(full_profile());
    let policy = Policy::default();
    let plan = route(
        PlanRequest {
            input: FormatId::Png,
            target: Target::Format(FormatId::Jpeg),
            polyglot: false,
        },
        Properties::Image {
            width: 800,
            height: 600,
            has_alpha: false,
            frames: 1,
        },
        &policy,
        &env,
    );
    assert!(
        !plan.steps().is_empty(),
        "png -> jpeg produced no steps at all"
    );
    assert!(
        plan.steps()
            .iter()
            .all(|s| matches!(s.isolation, Isolation::InProcess)),
        "png -> jpeg was sandboxed; the in-process fast path is dead"
    );
}

/// And the other half: a C-library format really is sandboxed.
#[test]
fn c_library_formats_are_sandboxed() {
    let env = env_with(full_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Heic,
            target: Target::Format(FormatId::Jpeg),
            polyglot: false,
        },
        Properties::None,
        &Policy::default(),
        &env,
    );
    assert!(!plan.steps().is_empty(), "heic -> jpeg produced no steps");
    assert!(
        plan.steps()
            .iter()
            .all(|s| matches!(s.isolation, Isolation::Sandboxed(_))),
        "HEIC needs libheif and was routed in-process"
    );
}

// ---------------------------------------------------------------------------
// 4 · No default policy arms Class D — I8
// ---------------------------------------------------------------------------

#[test]
fn no_default_policy_arms_class_d() {
    let env = env_with(full_profile());
    let policy = Policy::default();
    assert!(
        policy.max_auto_class() < Class::D,
        "the default ceiling admits Class D"
    );

    for req in all_requests() {
        let plan = route(req, video_props(), &policy, &env);
        for step in plan.steps() {
            assert!(
                step.class <= policy.max_auto_class(),
                "{req:?} armed {:?}",
                step.class
            );
        }
    }
}

/// A route above the ceiling yields **no steps**, not a flagged plan.
///
/// Refusal is the absence of a plan. That is what makes the guarantee real:
/// there is nothing sitting around for a caller to run anyway.
///
/// The subject is a synthetic table, not `v1()`: the ceiling is a property of
/// `route()`, and coupling the test to whichever row of v1 happened to be
/// Class C meant deleting one honest row (Docx→Pdf belongs to the Office
/// module) broke this test without touching the logic it exists to guard.
#[test]
fn above_ceiling_yields_no_steps_and_says_why() {
    let env = Environment::new(
        full_profile(),
        engines(true),
        RouteTable::new(vec![Route {
            from: FormatId::Docx,
            to: FormatId::Pdf,
            steps: &[StepKind::Transcode {
                from: FormatId::Docx,
                to: FormatId::Pdf,
            }],
            class: Class::C,
            requires: &[],
        }]),
    );
    let plan = route(
        PlanRequest {
            input: FormatId::Docx,
            target: Target::Format(FormatId::Pdf),
            polyglot: false,
        },
        Properties::Document {
            pages: 3,
            encrypted: false,
        },
        &Policy::default(),
        &env,
    );
    assert!(
        plan.steps().is_empty(),
        "a Class C route ran under a Class B ceiling"
    );
    assert!(
        plan.warnings().iter().any(|w| matches!(
            w,
            Warning::AboveAutoClass {
                needed: Class::C,
                allowed: Class::B
            }
        )),
        "refused without saying why: {:?}",
        plan.warnings()
    );
}

// ---------------------------------------------------------------------------
// 5 · No profile without network denial ever yields steps — SR-1
// ---------------------------------------------------------------------------

/// SR-1 says "ever", and this is what makes that a true word rather than an
/// aspiration. Asserted over **every** profile the prober can produce, not one.
#[test]
fn no_steps_without_network_denial() {
    let policy = Policy::default();
    let mut refused = 0;

    for profile in all_profiles() {
        if profile.denies_network() {
            continue;
        }
        let env = env_with(profile);
        // Only sandboxed steps are gated by the profile. An InProcess step runs
        // no third-party parser -- I10 and the format table guarantee it -- so
        // there is nothing for a confinement profile to be about. SR-1 binds
        // the in-process path through SR-9's single-call-site scan and
        // `net::no_outbound_during_conversion` instead. 03 section 9.1 scopes
        // this precisely; 09 SR-1 states it unscoped, and the two disagree.
        for req in all_requests()
            .into_iter()
            .filter(|r| !r.input.has_pure_rust_parser())
        {
            let plan = route(req, video_props(), &policy, &env);
            assert!(
                plan.steps().is_empty(),
                "{req:?} produced steps on a machine that cannot deny network"
            );
            assert!(!plan.is_executable());
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "no network-incapable profile was tested — this proves nothing"
    );
}

// ---------------------------------------------------------------------------
// 6 · Below the floor yields no steps — I9
// ---------------------------------------------------------------------------

#[test]
fn below_floor_yields_no_steps() {
    let policy = Policy::default().raise_floor(IsolationFloor {
        filesystem: FsRequirement::Confined,
        strength: Strength::Full,
    });
    // Reduced is a real, working machine -- but not Full. HEIC, not PNG:
    // the floor gates sandboxed steps, and PNG has a pure-Rust parser.
    let env = env_with(reduced_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Heic,
            target: Target::Format(FormatId::Jpeg),
            polyglot: false,
        },
        Properties::None,
        &policy,
        &env,
    );
    assert!(plan.steps().is_empty());
    assert!(plan
        .warnings()
        .iter()
        .any(|w| matches!(w, Warning::BelowFloor(_))));
}

/// The case the v0.5 ladder rewrite exists for.
///
/// A hardened Debian with no unprivileged user namespaces lands on `Reduced`
/// — and **converts**. v0.4 dropped it to `Minimal` and refused, which is the
/// outcome the isolation profile was invented to prevent.
///
/// Note the caveat this test cannot carry: the `Reduced` tier's central claim,
/// that Landlock needs neither privilege nor a namespace, was confirmed on a
/// kernel where `unprivileged_userns_clone` does not exist. The actual hardened
/// case has never been reproduced. See the private Debian validation record.
#[test]
fn reduced_tier_still_converts() {
    let env = env_with(reduced_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Heic,
            target: Target::Format(FormatId::Jpeg),
            polyglot: false,
        },
        Properties::None,
        &Policy::default(),
        &env,
    );
    assert!(
        !plan.steps().is_empty(),
        "the hardened-Debian case was refused again"
    );
    assert!(plan.is_executable());
}

// ---------------------------------------------------------------------------
// 7 · Every in-kind pair has a route or an explicit reason
// ---------------------------------------------------------------------------

/// Scoped to *within* a `MediaKind`, deliberately.
///
/// v0.4 asserted this over every pair in the catalogue, which made adding
/// format 21 owe 40 new table decisions — a cost growing with the square of the
/// catalogue that nobody noticed. Cross-kind pairs fall to a generated reason
/// that still explains itself and costs nothing.
#[test]
fn every_pair_has_a_route_or_a_reason() {
    let env = env_with(full_profile());
    let policy = Policy::default();
    let formats: Vec<FormatId> = TABLE.iter().map(|f| f.id).collect();
    let mut cross_kind_reasons = 0;

    for &from in &formats {
        for &to in &formats {
            if from == to {
                continue;
            }
            let plan = route(
                PlanRequest {
                    input: from,
                    target: Target::Format(to),
                    polyglot: false,
                },
                video_props(),
                &policy,
                &env,
            );
            if plan.steps().is_empty() {
                let explained = plan.warnings().iter().any(|w| w.is_blocking());
                assert!(
                    explained,
                    "{from:?} -> {to:?} yielded neither steps nor a reason"
                );

                if from.kind() != to.kind()
                    && plan
                        .warnings()
                        .iter()
                        .any(|w| matches!(w, Warning::NoRoute(NoRoute::CrossKind)))
                {
                    cross_kind_reasons += 1;
                }
            }
        }
    }
    assert!(
        cross_kind_reasons > 0,
        "no cross-kind pair was explained — the generated reason is dead code"
    );
}

// ---------------------------------------------------------------------------
// 8 · Lossless is preferred, and preference order is not an accident
// ---------------------------------------------------------------------------

/// `MKV → MP4` with compatible codecs must stream-copy, not transcode.
///
/// This single pair is why the table is *ordered* rather than a set. It is also
/// the product's flagship demo, and getting it wrong is a silent quality loss
/// on the conversion most likely to be shown to someone.
#[test]
fn compatible_streams_are_copied_not_reencoded() {
    let env = env_with(full_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Mkv,
            target: Target::Format(FormatId::Mp4),
            polyglot: false,
        },
        Properties::Video {
            duration_ms: 60_000,
            width: 1920,
            height: 1080,
            // H.264 + AAC is the common case the flagship demo uses, and MP4
            // carries both. `route` decides that now; nothing reports a
            // `stream_copyable` boolean, because copyability depends on the
            // destination and a probe does not know one.
            video: Some(VideoCodec::H264),
            audio: Some(AudioCodec::Aac),
        },
        &Policy::default(),
        &env,
    );
    assert_eq!(
        plan.class(),
        Some(Class::A),
        "a stream-copyable remux was not Class A"
    );
}

/// The control: incompatible codecs produce NO plan, with the reason named.
///
/// This used to assert a fall-through to the Class B transcode row. That row
/// is gone deliberately — video transcoding needs the separately downloaded
/// FFmpeg module (`02` §3.2), and routing to an execution that cannot finish
/// is the one dishonesty the availability rule exists to prevent. The honest
/// answer now is a refusal whose reason names codec compatibility, so the user
/// learns the truth (a lossless remux is impossible for THIS file) instead of
/// being promised a re-encode nothing can yet perform.
#[test]
fn incompatible_streams_are_refused_and_say_why() {
    let env = env_with(full_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Mkv,
            target: Target::Format(FormatId::Mp4),
            polyglot: false,
        },
        Properties::Video {
            duration_ms: 60_000,
            width: 1920,
            height: 1080,
            // VP9 does not go into MP4 in any form worth shipping.
            video: Some(VideoCodec::Vp9),
            audio: Some(AudioCodec::Opus),
        },
        &Policy::default(),
        &env,
    );
    assert!(
        plan.steps().is_empty(),
        "an unperformable transcode was routed anyway"
    );
    assert!(
        plan.warnings().iter().any(|w| matches!(
            w,
            Warning::NoRoute(NoRoute::RequirementUnmet {
                requirement: Requirement::StreamsCompatible
            })
        )),
        "the refusal must name codec compatibility: {:?}",
        plan.warnings()
    );
}

// ---------------------------------------------------------------------------
// 9 · A missing engine explains itself
// ---------------------------------------------------------------------------

#[test]
fn missing_engine_is_named_not_silent() {
    let env = Environment::new(full_profile(), engines(false), RouteTable::v1());
    let plan = route(
        PlanRequest {
            input: FormatId::Heic,
            target: Target::Format(FormatId::Jpeg),
            polyglot: false,
        },
        Properties::None,
        &Policy::default(),
        &env,
    );
    assert!(plan.steps().is_empty());
    assert!(
        plan.warnings().iter().any(|w| matches!(
            w,
            Warning::NoRoute(NoRoute::RequirementUnmet {
                requirement: Requirement::Engine("oc-images")
            })
        )),
        "an absent engine produced {:?} rather than naming oc-images",
        plan.warnings()
    );
}

// ---------------------------------------------------------------------------
// 10 · Format-preserving operations are Class A by construction
// ---------------------------------------------------------------------------

/// The week-3 metadata strip. Class A because nothing is decoded — the JPEG's
/// segments are copied except the metadata ones, so the pixels cannot change.
#[test]
fn metadata_strip_is_class_a_and_in_process() {
    let env = env_with(full_profile());
    let plan = route(
        PlanRequest {
            input: FormatId::Jpeg,
            target: Target::Operation(Operation::StripMetadata),
            polyglot: false,
        },
        Properties::Image {
            width: 4000,
            height: 3000,
            has_alpha: false,
            frames: 1,
        },
        &Policy::default(),
        &env,
    );
    assert_eq!(
        plan.class(),
        Some(Class::A),
        "the metadata strip is not lossless"
    );
    assert!(plan.is_executable());
    assert!(
        plan.steps()
            .iter()
            .all(|s| matches!(s.isolation, Isolation::InProcess)),
        "JPEG has a pure-Rust parser; the strip should not need a subprocess"
    );
}

// ---------------------------------------------------------------------------
// Table hygiene
// ---------------------------------------------------------------------------

/// The format table is internally consistent.
///
/// Cheap, and it catches the copy-paste error that a row-based table invites.
#[test]
fn format_table_is_consistent() {
    for f in TABLE {
        assert!(!f.name.is_empty(), "{:?} has no name", f.id);
        assert!(!f.extension.is_empty(), "{:?} has no extension", f.id);
        assert!(
            f.media_type.contains('/'),
            "{:?} has a malformed media type",
            f.id
        );
        assert_eq!(f.id.kind(), Some(f.kind));
        assert_eq!(f.id.name(), f.name);
    }
    for kind in MediaKind::ALL {
        assert!(
            TABLE.iter().any(|f| f.kind == *kind),
            "{kind} has no formats, so its Properties variant is unreachable"
        );
    }
    // Unknown is the absence of a match, never a row.
    assert!(!TABLE.iter().any(|f| f.id == FormatId::Unknown));
    assert!(!FormatId::Unknown.has_pure_rust_parser());
}

/// A sniffer must read far enough for every signature in the table.
///
/// Derived from the data rather than assumed: `tar`'s `ustar` at offset 257 is
/// why a sniffer reading 16 bytes would silently miss it, and why this is
/// computed instead of written down.
#[test]
fn magic_reach_covers_the_deepest_signature() {
    let reach = openconvert_core::format::max_magic_reach();
    assert!(
        reach >= 262,
        "reach {reach} is too shallow for tar's signature at offset 257"
    );
    for f in TABLE {
        for m in f.magic {
            assert!(
                m.offset + m.bytes.len() <= reach,
                "{:?} needs more than {reach} bytes",
                f.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Noise removal returns the format it was given
// ---------------------------------------------------------------------------

/// Everything denoise needs, including the network it runs.
fn denoise_env(with_lame: bool) -> Environment {
    let mut list = engines(true);
    list.push(EngineEntry {
        name: "deepfilternet",
        memory_safe: true,
        available: true,
    });
    if !with_lame {
        list.retain(|e| e.name != "libmp3lame");
    }
    Environment::new(full_profile(), list, RouteTable::v1())
}

fn audio_props() -> Properties {
    Properties::Audio {
        duration_ms: 194_000,
        channels: 2,
        sample_rate: 48_000,
    }
}

fn denoise_plan(input: FormatId, env: &Environment) -> openconvert_core::plan::Plan {
    route(
        PlanRequest {
            input,
            target: Target::Operation(Operation::Denoise),
            polyglot: false,
        },
        audio_props(),
        // Denoise is Class D — a model decides the samples — so the plan is
        // only executable when D is armed, which naming the operation IS.
        &Policy::default().arming(Class::D),
        env,
    )
}

/// **An mp3 that goes in comes back an mp3.**
///
/// The inference step emits samples, so it can only produce WAV. A second step
/// re-encodes to the format the recording arrived in — which is what "remove
/// the noise from this file" means. The user asked for the noise gone, not for
/// a format change.
#[test]
fn denoising_returns_the_format_it_was_given() {
    let env = denoise_env(true);
    for (input, expected) in [
        (FormatId::Mp3, FormatId::Mp3),
        (FormatId::Flac, FormatId::Flac),
        (FormatId::Ogg, FormatId::Ogg),
        // Already there: no re-encode, and no pointless decode-and-encode.
        (FormatId::Wav, FormatId::Wav),
    ] {
        let plan = denoise_plan(input, &env);
        assert!(plan.is_executable(), "{input:?} should denoise");
        assert_eq!(
            plan.output_format(),
            Some(expected),
            "{input:?} denoised to {:?}, not {expected:?}",
            plan.output_format()
        );
    }
}

/// **The name always matches the bytes, encoder or no encoder.**
///
/// This is the property that was broken, and it is broken in a way that
/// survives either outcome: whatever the plan can reach, the file has to be
/// named for it. Without the mp3 encoder the plan ends on WAV — and the output
/// must then be called `.wav`, not `.mp3`.
#[test]
fn a_denoised_file_is_named_for_what_it_contains() {
    for with_encoder in [true, false] {
        let env = denoise_env(with_encoder);
        for input in [FormatId::Mp3, FormatId::Flac, FormatId::Ogg, FormatId::Wav] {
            let plan = denoise_plan(input, &env);
            assert!(
                plan.is_executable(),
                "{input:?} should denoise whether or not the encoder is here"
            );
            let produced = match plan.steps().last().expect("a step").kind {
                StepKind::Transcode { to, .. } | StepKind::Infer { to, .. } => to,
                other => panic!("{input:?} ended on {other:?}"),
            };
            assert_eq!(
                plan.output_format(),
                Some(produced),
                "the name would not match the bytes for {input:?}                  (encoder present: {with_encoder})"
            );
        }
    }
}

#[test]
fn png_and_jpeg_reach_the_avif_worker() {
    let env = env_with(full_profile());
    for input in [FormatId::Png, FormatId::Jpeg] {
        let plan = route(
            PlanRequest {
                input,
                target: Target::Format(FormatId::Avif),
                polyglot: false,
            },
            Properties::Image {
                width: 800,
                height: 600,
                has_alpha: input == FormatId::Png,
                frames: 1,
            },
            &Policy::default(),
            &env,
        );
        assert!(plan.is_executable(), "{input:?} should encode to AVIF");
        assert_eq!(plan.output_format(), Some(FormatId::Avif));
        assert!(matches!(plan.steps()[0].isolation, Isolation::Sandboxed(_)));
    }
}

#[test]
fn pdf_can_chain_through_png_to_every_raster_target() {
    let env = env_with(full_profile());
    for target in [
        FormatId::Png,
        FormatId::Jpeg,
        FormatId::Webp,
        FormatId::Gif,
        FormatId::Bmp,
        FormatId::Tiff,
        FormatId::Avif,
    ] {
        let plan = route(
            PlanRequest {
                input: FormatId::Pdf,
                target: Target::Format(target),
                polyglot: false,
            },
            Properties::Document {
                pages: 2,
                encrypted: false,
            },
            &Policy::default(),
            &env,
        );
        assert!(plan.is_executable(), "PDF should convert to {target:?}");
        assert_eq!(plan.output_format(), Some(target));
        if !matches!(target, FormatId::Png | FormatId::Jpeg) {
            assert_eq!(plan.steps().len(), 2, "{target:?} needs an intermediate");
        }
    }
}

/// Without the encoder, the plan stops at WAV rather than inventing a route.
#[test]
fn denoising_without_an_encoder_stops_at_wav() {
    let plan = denoise_plan(FormatId::Mp3, &denoise_env(false));
    assert!(
        plan.is_executable(),
        "the denoise itself does not depend on the mp3 encoder"
    );
    assert_eq!(
        plan.output_format(),
        Some(FormatId::Wav),
        "with no encoder the plan must end on WAV"
    );
    assert_eq!(
        plan.steps().len(),
        1,
        "and must not carry a step it cannot run"
    );
}

/// WAV in, WAV out — the case the user actually named.
#[test]
fn denoising_a_wav_leaves_it_a_wav() {
    let plan = denoise_plan(FormatId::Wav, &denoise_env(true));
    assert_eq!(plan.output_format(), Some(FormatId::Wav));
    assert_eq!(plan.steps().len(), 1, "no re-encode from WAV to WAV");
}

// ---------------------------------------------------------------------------
// The camera formats reach the models
// ---------------------------------------------------------------------------

/// Everything the image models need, plus the decoder that opens HEIC.
fn infer_env(with_oc_images: bool) -> Environment {
    let mut list = engines(true);
    for name in ["u2netp", "modnet", "realesrgan-x4"] {
        list.push(EngineEntry {
            name,
            memory_safe: true,
            available: true,
        });
    }
    if !with_oc_images {
        list.retain(|e| e.name != "oc-images");
    }
    Environment::new(full_profile(), list, RouteTable::v1())
}

fn image_props() -> Properties {
    Properties::Image {
        width: 4032,
        height: 3024,
        has_alpha: false,
        frames: 1,
    }
}

fn infer_plan(input: FormatId, target: Target, env: &Environment) -> openconvert_core::plan::Plan {
    route(
        PlanRequest {
            input,
            target,
            polyglot: false,
        },
        image_props(),
        &Policy::default().arming(Class::D),
        env,
    )
}

/// **A photograph from a phone is a HEIC, and the models can have it.**
///
/// `oc-ai` decodes through the `image` crate, which does not open HEIC, AVIF
/// or JXL. That was expressed as a flat refusal — "background removal cannot
/// be applied to this file" — which reads as a statement about the picture and
/// is really a statement about which decoder the model sits behind. The file
/// is readable; `oc-images` opens it. A normalising step to PNG goes in front
/// of the inference step and the operation runs.
#[test]
fn camera_formats_are_normalised_before_inference() {
    let env = infer_env(true);

    for (input, target, op) in [
        (
            FormatId::Heic,
            Target::Operation(Operation::RemoveBackground {
                to: FormatId::Png,
                quality: openconvert_core::target::Quality::Standard,
            }),
            "background removal",
        ),
        (
            FormatId::Avif,
            Target::Operation(Operation::Upscale { to: FormatId::Png }),
            "upscaling",
        ),
        (
            FormatId::Jxl,
            Target::Operation(Operation::Upscale { to: FormatId::Png }),
            "upscaling",
        ),
    ] {
        let plan = infer_plan(input, target, &env);
        assert!(
            plan.is_executable(),
            "{input:?} should reach {op} through a normalising step"
        );
        assert_eq!(
            plan.steps().len(),
            2,
            "{input:?} needs a decode step in front of the model"
        );
        assert!(
            matches!(
                plan.steps()[0].kind,
                StepKind::Transcode {
                    to: FormatId::Png,
                    ..
                }
            ),
            "the first step converts {input:?} to PNG, losslessly"
        );
        assert!(
            matches!(plan.steps()[1].kind, StepKind::Infer { .. }),
            "the model runs second, on what the first step produced"
        );
    }
}

/// **A format the model already reads gets no extra step.**
///
/// The control for the test above: if normalisation were unconditional, every
/// PNG would pay for a decode and re-encode it does not need, and the test
/// above would pass for the wrong reason.
#[test]
fn native_formats_are_not_normalised() {
    let env = infer_env(true);
    for input in [FormatId::Png, FormatId::Jpeg, FormatId::Webp] {
        let plan = infer_plan(
            input,
            Target::Operation(Operation::Upscale { to: FormatId::Png }),
            &env,
        );
        assert!(plan.is_executable(), "{input:?} upscales");
        assert_eq!(
            plan.steps().len(),
            1,
            "{input:?} is already readable by the model; no decode step"
        );
    }
}

/// **Without the decoder, the refusal comes back.**
///
/// Normalisation is only possible when something in this build can open the
/// file. Strip `oc-images` and a HEIC is genuinely unreachable again — which
/// is the honest answer, and no longer the answer for every machine.
#[test]
fn a_camera_format_without_a_decoder_is_still_refused() {
    let env = infer_env(false);
    let plan = infer_plan(
        FormatId::Heic,
        Target::Operation(Operation::Upscale { to: FormatId::Png }),
        &env,
    );
    assert!(
        !plan.is_executable(),
        "with no decoder for HEIC there is nothing to normalise with"
    );
}

// ---------------------------------------------------------------------------
// The raster matrix is complete
// ---------------------------------------------------------------------------

/// **Anything we can decode reaches anything we can encode. All 105 pairs.**
///
/// Seventeen pairs were missing and the gaps had no principle behind them:
/// AVIF was reachable from PNG and JPEG but not from WebP, GIF, BMP or TIFF,
/// and a HEIC could become a PNG or a JPEG and nothing else. `webp -> avif` was
/// reported as a bug; it was one of seventeen.
///
/// **This test used to pass while twenty-seven pairs were still missing**, and
/// that is the more useful thing it records. `DECODES` listed nine formats —
/// the ones the round of work that wrote the test happened to touch — so SVG,
/// the five camera-raw formats and PDF were outside the matrix it checked. A
/// test naming its own subject can only ever be as complete as the list, and
/// this list was short by seven sources and twenty-seven pairs.
///
/// The list is now derived from a stated rule rather than from memory: every
/// format `oc-images` has a decoder for, plus PDF, which reaches the same
/// encoders through a render step. Adding a decoder without adding its rows now
/// fails here.
///
/// HEIC, JXL and SVG are deliberately absent as TARGETS — no encoder for the
/// first two, and rasterised pixels cannot become a vector document again — so
/// a route to them could not run, and an unrunnable route is worse than an
/// absent one. That is asserted below.
#[test]
fn every_decodable_raster_reaches_every_encodable_one() {
    /// What `oc-images` can read, plus PDF, which renders into the same set.
    const DECODES: &[FormatId] = &[
        // image-rs, always present.
        FormatId::Png,
        FormatId::Jpeg,
        FormatId::Webp,
        FormatId::Gif,
        FormatId::Bmp,
        FormatId::Tiff,
        // The native decoders.
        FormatId::Avif,
        FormatId::Heic,
        FormatId::Jxl,
        FormatId::Svg,
        // libraw. These are the five that were outside the old list.
        FormatId::Cr2,
        FormatId::Cr3,
        FormatId::Nef,
        FormatId::Arw,
        FormatId::Dng,
        // pdfium renders a page, and `pdf -> png -> x` covers the rest.
        FormatId::Pdf,
    ];
    /// What it can write. No HEIC, no JXL, no SVG.
    const ENCODES: &[FormatId] = &[
        FormatId::Png,
        FormatId::Jpeg,
        FormatId::Webp,
        FormatId::Gif,
        FormatId::Bmp,
        FormatId::Tiff,
        FormatId::Avif,
    ];

    let table = RouteTable::v1();
    let mut missing = Vec::new();
    let mut found = 0_usize;
    for &from in DECODES {
        for &to in ENCODES {
            if from == to {
                continue;
            }
            if table.routes_for(from, to).next().is_none() {
                missing.push(format!("{from:?} -> {to:?}"));
            } else {
                found += 1;
            }
        }
    }
    assert!(
        missing.is_empty(),
        "no route for {} raster pair(s): {missing:?}",
        missing.len()
    );
    // The count is asserted as well as the emptiness, so shrinking `DECODES`
    // to make a failure go away is itself a failure. That is exactly how the
    // twenty-seven survived.
    assert_eq!(
        found, 105,
        "the raster matrix is {found} pairs; it should be 16 sources x 7 targets \
         less the 7 identities = 105"
    );
}

/// The control: formats with no encoder are not offered as destinations.
///
/// Without this, the test above could be satisfied by adding routes to HEIC and
/// JXL that fail at the worker — which is the failure mode it exists to prevent.
#[test]
fn formats_we_cannot_write_are_not_destinations() {
    let table = RouteTable::v1();
    for &to in &[FormatId::Heic, FormatId::Jxl] {
        let sources: Vec<String> = TABLE
            .iter()
            .filter(|f| table.routes_for(f.id, to).next().is_some())
            .map(|f| format!("{:?}", f.id))
            .collect();
        assert!(
            sources.is_empty(),
            "{to:?} has no encoder in this build, but {sources:?} claim to reach it"
        );
    }
}

/// **A route from a format nobody can detect is a route that cannot fire.**
///
/// Detection is content-only — `sniff.rs` says so in its first line — so a
/// format is reachable as a SOURCE in exactly two ways: it has a byte
/// signature, or `sniff::structural` recognises its shape. A row written from
/// anything else compiles, appears in `docs/ROUTES.md`, and can never run.
///
/// That is not hypothetical. Seven rows shipped from `Markdown`, which has no
/// signature and which `structural` never returns: a `.md` file arrives as
/// `Txt`. The generated reference listed `markdown -> pdf` as a capability of
/// this build, and nothing could reach it. The route table's own comment says
/// a route that cannot run is worse than one that is missing; this is the test
/// that makes that enforceable rather than aspirational.
///
/// A format is allowed to be a target-only format — `Markdown` still is, and
/// `Mka` always was — because a target is asked for by NAME. Only sources have
/// to be detectable.
#[test]
fn source_formats_are_reachable_by_detection() {
    // The formats `sniff::structural` can return. It is private, so this list
    // is maintained by hand — deliberately: adding a structural format means
    // adding it here, which is a moment to ask whether the shape test is
    // actually reliable enough to route on.
    const STRUCTURAL: &[FormatId] = &[
        FormatId::Csv,
        FormatId::Json,
        FormatId::Ipynb,
        FormatId::Svg,
        FormatId::PostScript,
        FormatId::Txt,
        // A DXF opens `0` / `SECTION` — two ordinary lines of text, and no
        // signature there ever will be. `sniff::is_dxf` checks the whole
        // four-line opening rather than the word, which is what keeps a
        // document that merely contains "SECTION" out of the CAD reader.
        FormatId::Dxf,
    ];

    let table = RouteTable::v1();
    let mut unreachable: Vec<(&'static str, usize)> = Vec::new();

    for from in table.all().iter().map(|r| r.from).collect::<BTreeSet<_>>() {
        let row = from.row().expect("every routed format has a table row");
        let detectable = !row.magic.is_empty() || STRUCTURAL.contains(&from);
        if !detectable {
            let count = table.all().iter().filter(|r| r.from == from).count();
            unreachable.push((row.name, count));
        }
    }

    assert!(
        unreachable.is_empty(),
        "these formats are the source of routes that can never fire, because \
         nothing detects them: {unreachable:?}. Either give the format a \
         signature, teach `sniff::structural` to recognise it (and add it to \
         STRUCTURAL above), or delete the rows."
    );
}

/// **A format the table never mentions is capability the product does not have.**
///
/// `openconvert formats` and `docs/spec` list every row in `TABLE`, and a reader
/// takes that list as what this build converts. `postscript` was in it with
/// **zero routes in either direction**: detectable, and then refused for every
/// target. That is the format-level version of the dead route
/// `every_declared_document_route_runs` catches, and it deserves the same kind
/// of gate rather than a periodic audit.
///
/// The allowlist is the point. A format may earn a place here, but only by
/// being written down with a reason, which is a different act from nobody
/// noticing.
#[test]
fn every_format_routes_or_is_declared_detect_only() {
    /// Formats worth DETECTING and deliberately not worth ROUTING.
    ///
    /// PostScript: converting it means an interpreter, and Ghostscript escaped
    /// its own `-dSAFER` sandbox in the wild, so it is excluded from this
    /// product by policy rather than by effort. Detection is separate and pays
    /// for itself: A6/SR-4 says the extension is a claim, and a PostScript file
    /// named `.jpg` has to be named as PostScript in the warning and the
    /// refusal. `boundaries.rs` and `openconvert-run`'s `week4.rs` both hold that
    /// behaviour.
    const DETECT_ONLY: &[FormatId] = &[FormatId::PostScript];

    let table = RouteTable::v1();
    let mut orphans: Vec<&'static str> = Vec::new();
    for row in TABLE {
        if DETECT_ONLY.contains(&row.id) {
            continue;
        }
        if !table
            .all()
            .iter()
            .any(|r| r.from == row.id || r.to == row.id)
        {
            orphans.push(row.name);
        }
    }
    assert!(
        orphans.is_empty(),
        "these formats appear in `openconvert formats` and in no route at all:          {orphans:?}. Give each one a route, or add it to DETECT_ONLY above \
         with the reason it is worth detecting and not worth converting."
    );

    // And the converse: an entry here that has since gained routes is a stale
    // apology, and leaving it would hide the next real orphan.
    for id in DETECT_ONLY {
        assert!(
            !table.all().iter().any(|r| r.from == *id || r.to == *id),
            "{id:?} is listed as detect-only and now has routes; take it out \
             of DETECT_ONLY."
        );
    }
}

/// And the converse, so the test above cannot pass by being vacuous.
///
/// If `RouteTable::v1()` ever returned nothing, every assertion about it would
/// hold trivially.
#[test]
fn the_route_table_is_not_empty() {
    assert!(
        RouteTable::v1().all().len() > 100,
        "the route table shrank unexpectedly; the reachability test above \
         would pass vacuously"
    );
}
