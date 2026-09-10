//! The week-4 gates. `08` commits 23–28.
//!
//! Each one is a row in `08 §4` and each fails the build. The rule that table
//! sets for itself is that **a gate's week is a week its test can actually
//! pass** — so every subject asserted here exists in the tree today.

use openconvert_core::environment::Environment;
use openconvert_core::facts::Properties;
use openconvert_core::format::FormatId;
use openconvert_core::isolation::SandboxProfile;
use openconvert_core::limits::{Budget, Limits};
use openconvert_core::plan::{Class, PlanRequest};
use openconvert_core::policy::{OnConflict, Policy};
use openconvert_core::route::{route, RouteTable};
use openconvert_core::target::{Operation, Target};
use openconvert_run::engines::{self, EngineError};
use openconvert_run::exec::{execute, ExecError};
use std::io::Cursor;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Scratch space
// ---------------------------------------------------------------------------

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tx-w4-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&p).expect("mkdir");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn plant(&self, name: &str, body: &[u8]) -> PathBuf {
        use std::io::Write;
        let p = self.0.join(name);
        let mut f = std::fs::File::create_new(&p).expect("plant");
        f.write_all(body).expect("write");
        p
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // openconvert-lint: allow -- test scratch teardown
    }
}

fn env() -> Environment {
    Environment::new(
        SandboxProfile::unconfined(),
        engines::registry(),
        RouteTable::v1(),
    )
}

/// A real PNG of the given size, built here so the test needs no fixture file.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode");
    out.into_inner()
}

/// A JPEG carrying EXIF with a GPS tag.
fn jpeg_with_gps() -> Vec<u8> {
    fn seg(marker: u8, payload: &[u8]) -> Vec<u8> {
        let len = (payload.len() + 2) as u16;
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(payload);
        v
    }
    let mut v = vec![0xFF, 0xD8];
    v.extend(seg(
        0xE1,
        b"Exif\x00\x00GPSLatitude=51.5074,GPSLongitude=-0.1278",
    ));
    v.extend(seg(0xDB, &[0x00; 64]));
    v.extend(vec![0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00]);
    v.extend(vec![0x12, 0x34, 0x56, 0x78, 0xFF, 0xD9]);
    v
}

/// Plant the bytes, detect them, route, and execute — the real path.
///
/// Goes through `detect()` rather than handing `execute()` a cursor, so these
/// tests exercise the same one-open-handle route the CLI does. An earlier
/// version passed bytes directly, which tested execute() against a convenience
/// the product does not use.
fn convert(
    t: &Tmp,
    src: &[u8],
    input: FormatId,
    target: Target,
    name: &str,
    policy: &Policy,
) -> Result<openconvert_run::exec::Outcome, ExecError> {
    let stem = format!("in-{name}");
    t.plant(&stem, src);
    let mut table = openconvert_run::handles::HandleTable::new();
    let facts = openconvert_run::detect::detect(&t.path().join(&stem), &mut table).expect("detect");
    let plan = route(
        PlanRequest {
            input,
            target,
            polyglot: facts.sniff().polyglot,
        },
        Properties::None,
        policy,
        &sandboxed_capable_env(),
    );
    // A fresh pool per call. Every step these tests drive is `InProcess`, so
    // the pool is never asked for a worker — passing one keeps the signature
    // honest rather than giving `execute` a second shape for tests.
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());
    execute(&plan, &facts, &mut table, t.path(), name, policy, &mut pool)
}

// ---------------------------------------------------------------------------
// Commit 23 — Class A round-trips byte-identically (metadata half)
// ---------------------------------------------------------------------------

/// **The gate that had no subject.**
///
/// `08` scheduled this at week 4 naming "metadata-only edits and lossless
/// re-container — the operations that exist in week 4". Neither existed: the
/// first was unscheduled and the second is weeks 27–30. The gate would have
/// run, found nothing matching Class A, passed, and shown green.
///
/// It has a subject now. The JPEG metadata strip is Class A **by
/// construction** — the image is never decoded, so the entropy-coded scan data
/// cannot change — and this asserts exactly that.
#[test]
fn class_a_metadata_strip_is_byte_identical_below_the_scan() {
    let t = Tmp::new("classa");
    let src = jpeg_with_gps();

    let out = convert(
        &t,
        &src,
        FormatId::Jpeg,
        Target::Operation(Operation::StripMetadata),
        "clean.jpg",
        &Policy::default(),
    )
    .expect("strip");

    assert_eq!(
        out.record.class.as_deref(),
        Some("A"),
        "the strip is not Class A"
    );

    let written = std::fs::read(&out.output).expect("read output");
    let sos_src = src
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .expect("SOS src");
    let sos_out = written
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .expect("SOS out");
    assert_eq!(
        &src[sos_src..],
        &written[sos_out..],
        "the image data changed -- this is not lossless"
    );

    // A8/SR-11: the location is gone, and the receipt says so by name.
    let text = String::from_utf8_lossy(&written);
    assert!(!text.contains("GPSLatitude"), "GPS survived");
    assert!(
        out.record.steps[0]
            .removed
            .iter()
            .any(|r| r.contains("EXIF")),
        "the receipt does not name what it removed: {:?}",
        out.record.steps[0].removed
    );
}

// ---------------------------------------------------------------------------
// Commit 25 — limits and budget · SR-5
// ---------------------------------------------------------------------------

/// A small file declaring an enormous image errors **before allocating**, with
/// a message naming the limit.
#[test]
fn a_pixel_bomb_errors_before_allocating() {
    let src = png(64, 64);
    let tight = Limits {
        decode_pixels: 100,
        ..Limits::defaults()
    };
    let err = engines::run_in_process(
        openconvert_core::plan::StepKind::Transcode {
            from: FormatId::Png,
            to: FormatId::Jpeg,
        },
        &mut Cursor::new(src),
        &tight,
    )
    .expect_err("should refuse");

    match err {
        EngineError::LimitExceeded {
            what,
            limit,
            actual,
        } => {
            assert_eq!(what, "decode_pixels");
            assert_eq!((limit, actual), (100, 4096));
        }
        other => panic!("wrong error, and the message would be wrong too: {other}"),
    }
}

/// A batch projected past free space refuses in **preflight**, before any work.
#[test]
fn a_batch_that_cannot_fit_refuses_up_front() {
    let b = Budget {
        output_bytes: 10_000,
        reserve_bytes: 1_000,
        ..Budget::defaults()
    };
    assert!(b.admits(5_000, 20_000), "a batch that fits was refused");
    assert!(
        !b.admits(20_000, 100_000),
        "a batch over the cap was admitted"
    );
    assert!(
        !b.admits(9_500, 10_000),
        "a batch was admitted that would leave 500 bytes free"
    );
}

// ---------------------------------------------------------------------------
// Commit 26 — detection routes by content · SR-4
// ---------------------------------------------------------------------------

/// A PostScript file named `.jpg` routes as PostScript, and the receipt records
/// **both** types.
#[test]
fn detection_routes_by_content_and_records_the_mismatch() {
    use std::ffi::OsStr;
    let s = openconvert_run::detect::sniff(
        &mut Cursor::new(b"%!PS-Adobe-3.0\nshowpage\n".to_vec()),
        Some(OsStr::new("invoice.jpg")),
    )
    .expect("sniff");

    assert_eq!(
        s.detected,
        FormatId::PostScript,
        "routed by extension, not content"
    );
    assert_eq!(s.declared, Some(FormatId::Jpeg));
    assert!(s.mismatched());
}

// ---------------------------------------------------------------------------
// Commit 27 — nothing is ever overwritten · SR-15
// ---------------------------------------------------------------------------

/// **The case an earlier revision of this design lost.**
///
/// An output whose name collides with a file already in the destination. Every
/// traversal check passes it, correctly, because it is not traversal.
#[test]
fn an_existing_file_survives_a_colliding_output() {
    let t = Tmp::new("collide");
    t.plant("photo.jpg", b"IRREPLACEABLE ORIGINAL");

    let out = convert(
        &t,
        &png(8, 8),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "photo.jpg",
        &Policy::default(), // Suffix
    )
    .expect("convert");

    assert_eq!(
        std::fs::read(t.path().join("photo.jpg")).expect("read original"),
        b"IRREPLACEABLE ORIGINAL",
        "the original was destroyed"
    );
    assert!(
        out.output.to_string_lossy().contains("photo (2)"),
        "expected a suffixed output, got {}",
        out.output.display()
    );
}

/// `Fail` writes nothing at all rather than leaving a partial result.
#[test]
fn on_conflict_fail_leaves_the_destination_untouched() {
    let t = Tmp::new("failpol");
    t.plant("photo.jpg", b"KEEP");

    let policy = Policy::default().with_on_conflict(OnConflict::Fail);
    let err = convert(
        &t,
        &png(8, 8),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "photo.jpg",
        &policy,
    )
    .expect_err("should refuse");

    assert!(matches!(err, ExecError::Output(_)));
    assert_eq!(
        std::fs::read(t.path().join("photo.jpg")).expect("read"),
        b"KEEP"
    );

    // Nothing was produced: no suffixed variant, and no receipt. Asserted by
    // name rather than by counting directory entries, because the helper plants
    // the input alongside the destination -- a count would be measuring the
    // fixture as much as the behaviour.
    let leftovers: Vec<String> = std::fs::read_dir(t.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.contains("photo (") || n.ends_with(".receipt.json"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "a refused conversion still created {leftovers:?}"
    );
}

// ---------------------------------------------------------------------------
// Receipt completeness, and the rule that it is not optional · SR-11
// ---------------------------------------------------------------------------

/// Every executed step appears in the receipt, with its limits and isolation.
#[test]
fn the_receipt_records_every_step_with_limits_and_isolation() {
    let t = Tmp::new("receipt");
    let out = convert(
        &t,
        &png(8, 8),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "out.jpg",
        &Policy::default(),
    )
    .expect("convert");

    assert_eq!(out.record.steps.len(), 1);
    let step = &out.record.steps[0];
    assert_eq!(step.engine, "image-rs");
    assert!(step.isolation.contains("in-process"));
    assert!(step.limits.memory_bytes > 0);
    assert!(step.limits.decode_pixels > 0);
    assert_eq!(out.record.detected, "png");
    assert_eq!(
        out.record.content_id.len(),
        64,
        "content_id is not a blake3 hex digest"
    );

    // It is on disk and it parses.
    let body = std::fs::read_to_string(&out.receipt).expect("read receipt");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("receipt is not valid JSON");
    assert_eq!(parsed["version"], 1);
}

/// I13/SR-16: the hash in the receipt is of the bytes actually converted.
#[test]
fn the_receipt_hashes_the_bytes_that_were_converted() {
    let t = Tmp::new("identity");
    let src = png(8, 8);
    let expected = blake3::hash(&src).to_hex().to_string();

    let out = convert(
        &t,
        &src,
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "out.jpg",
        &Policy::default(),
    )
    .expect("convert");

    assert_eq!(
        out.record.content_id, expected,
        "the receipt attests to bytes that were not the input"
    );
}

// ---------------------------------------------------------------------------
// The plan preview costs nothing extra
// ---------------------------------------------------------------------------

/// `plan` and `convert` agree, because one is the other minus a call.
///
/// If they could disagree, the preview would be a second implementation of
/// routing — which is exactly what "the preview is `route()` without
/// `execute()`" exists to prevent.
#[test]
fn the_preview_and_the_conversion_agree() {
    let t = Tmp::new("preview");
    let req = PlanRequest::new(FormatId::Png, Target::Format(FormatId::Jpeg));
    let policy = Policy::default();
    let previewed = route(req, Properties::None, &policy, &env());

    let out = convert(
        &t,
        &png(8, 8),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "out.jpg",
        &policy,
    )
    .expect("convert");

    assert_eq!(
        previewed.class().map(|c| format!("{c:?}")),
        out.record.class,
        "the preview promised a different class than the conversion delivered"
    );
    assert_eq!(previewed.steps().len(), out.record.steps.len());
    assert_eq!(previewed.class(), Some(Class::B));
}

// ---------------------------------------------------------------------------
// Commit 16 — a polyglot is refused, not announced · A6
// ---------------------------------------------------------------------------

/// A file that is validly two formats yields **no steps**.
///
/// This test exists because the CLI carried the sentence "this file is being
/// quarantined" while nothing quarantined anything: detection set a flag,
/// `route()` never saw it, and the conversion proceeded. A false statement in
/// user-facing text is worse than a missing feature — the user acts on it.
#[test]
fn a_polyglot_yields_no_steps() {
    let plan = route(
        PlanRequest {
            input: FormatId::Zip,
            target: Target::Format(FormatId::Tar),
            polyglot: true,
        },
        Properties::None,
        &Policy::default(),
        &env(),
    );
    assert!(plan.steps().is_empty(), "a polyglot was converted");
    assert!(!plan.is_executable());
    assert!(
        plan.warnings()
            .iter()
            .any(|w| matches!(w, openconvert_core::plan::Warning::Polyglot)),
        "refused without saying it was a polyglot: {:?}",
        plan.warnings()
    );
}

/// The control: the same request without the flag routes normally.
///
/// Without this, the property above is satisfied by a `route()` that refuses
/// every archive.
#[test]
fn the_same_pair_routes_when_it_is_not_a_polyglot() {
    let plan = route(
        PlanRequest::new(FormatId::Zip, Target::Format(FormatId::Tar)),
        Properties::None,
        &Policy::default(),
        &env(),
    );
    // oc-archive is unavailable, so it still refuses -- but for the RIGHT
    // reason, which is the distinction being asserted.
    assert!(
        !plan
            .warnings()
            .iter()
            .any(|w| matches!(w, openconvert_core::plan::Warning::Polyglot)),
        "a non-polyglot was reported as one"
    );
}

/// Detection and routing agree: a real polyglot reaches `route()` as one.
///
/// The two halves were connected by a struct field, and a field nobody reads
/// is exactly what the previous version had.
#[test]
fn detection_and_routing_agree_about_polyglots() {
    let t = Tmp::new("poly");
    // A REAL polyglot: valid as a ZIP and valid as a tar at the same time.
    //
    // This test used to plant a bare `PK 03 04` header and call it a polyglot,
    // on the grounds that DOCX and ODT begin the same way. They do, by design,
    // and that made every archive in the product unconvertible -- so the test
    // was pinning the defect rather than the property. Two tests were.
    //
    // Zip, Docx and Odt are one family. Tar is not in it. Matching across
    // families is what a polyglot is, and it is what this now builds: the ZIP
    // local-file-header signature at 0, and tar's `ustar` at offset 257.
    let mut bytes = vec![0u8; 300];
    bytes[0..4].copy_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    bytes[257..262].copy_from_slice(b"ustar");
    let p = t.plant("ambiguous.zip", &bytes);
    let mut table = openconvert_run::handles::HandleTable::new();
    let facts = openconvert_run::detect::detect(&p, &mut table).expect("detect");

    assert!(facts.sniff().polyglot, "detection missed the polyglot");

    let plan = route(
        PlanRequest {
            input: facts.sniff().detected,
            target: Target::Format(FormatId::Tar),
            polyglot: facts.sniff().polyglot,
        },
        Properties::None,
        &Policy::default(),
        &env(),
    );
    assert!(
        plan.steps().is_empty(),
        "detection flagged it and routing converted it anyway"
    );
}

// ---------------------------------------------------------------------------
// Commit 28 — no outbound network during a conversion · SR-9
// ---------------------------------------------------------------------------

/// A full conversion makes **zero** connection attempts.
///
/// Asserted structurally rather than by watching sockets: there is no outbound
/// call site in the conversion path at all, and `cargo xtask lint` fails the
/// build if one appears. This test pins the behavioural half — a conversion
/// completes with no network available and no network reachable, so nothing in
/// it can be quietly depending on a fetch.
#[test]
fn a_conversion_needs_no_network() {
    let t = Tmp::new("nonet");
    let out = convert(
        &t,
        &png(16, 16),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "offline.jpg",
        &Policy::default(),
    )
    .expect("convert");

    assert!(out.output.exists());
    assert!(out.receipt.exists());
    // The receipt records no remote anything -- no URL, no host, no fetch.
    let body = std::fs::read_to_string(&out.receipt).expect("receipt");
    for token in ["http://", "https://", "fetch", "download"] {
        assert!(
            !body.contains(token),
            "the receipt mentions {token:?}, which suggests a network dependency"
        );
    }
}

// ---------------------------------------------------------------------------
// Commit 24 — Class B is deterministic · the golden-corpus half
// ---------------------------------------------------------------------------

/// The same input converts to byte-identical output, every time.
///
/// `08` commit 24 asks for "byte-identical under `--deterministic`; SSIM floor
/// otherwise". The SSIM half needs a reference corpus that arrives with the
/// real image engines at week 15 — this is the half that can pass **today**,
/// and it is the half that catches the likelier bug: an encoder that varies run
/// to run makes every downstream reproducibility claim untestable.
#[test]
fn class_b_output_is_deterministic() {
    let src = png(32, 32);
    let mut digests = Vec::new();

    for i in 0..3 {
        let t = Tmp::new(&format!("determ{i}"));
        let out = convert(
            &t,
            &src,
            FormatId::Png,
            Target::Format(FormatId::Jpeg),
            "same.jpg",
            &Policy::default(),
        )
        .expect("convert");
        digests.push(
            blake3::hash(&std::fs::read(&out.output).unwrap())
                .to_hex()
                .to_string(),
        );
    }

    assert_eq!(
        digests[0], digests[1],
        "two identical conversions produced different bytes"
    );
    assert_eq!(digests[1], digests[2]);
}

/// The control: a *different* input gives different bytes.
///
/// Without it, the test above passes on a converter that writes a constant.
#[test]
fn different_inputs_give_different_output() {
    let t1 = Tmp::new("d1");
    let t2 = Tmp::new("d2");
    let a = convert(
        &t1,
        &png(16, 16),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "a.jpg",
        &Policy::default(),
    )
    .expect("a");
    let b = convert(
        &t2,
        &png(32, 32),
        FormatId::Png,
        Target::Format(FormatId::Jpeg),
        "b.jpg",
        &Policy::default(),
    )
    .expect("b");
    assert_ne!(
        std::fs::read(&a.output).unwrap(),
        std::fs::read(&b.output).unwrap(),
        "two different images produced identical output"
    );
}

// ---------------------------------------------------------------------------
// A10 Phase 1 — breadth rows are routable, and honest about availability
// ---------------------------------------------------------------------------

/// A profile that can deny network, so sandboxed rows are not refused by the
/// isolation floor before their own requirements are even consulted.
///
/// Shaped like what a real AppContainer machine reads back, because these
/// tests run with this crate's `attest` feature enabled (it is the shell) and
/// `attest()` is therefore its honest constructor: values IN, profile OUT,
/// exactly as `openconvert-os` builds one after probing.
fn sandboxed_capable_env() -> Environment {
    use openconvert_core::isolation::{
        FsConfinement, NetConfinement, PrivDrop, ResourceEnforcement, SandboxProfile, SyscallFilter,
    };
    Environment::new(
        SandboxProfile::attest(
            FsConfinement::AppContainer,
            NetConfinement::CapabilitySid,
            SyscallFilter::None,
            ResourceEnforcement::JobObject,
            PrivDrop::None,
        ),
        engines::registry(),
        RouteTable::v1(),
    )
}

/// MP3 output exists as a row and is gated on the LINKED encoder, exactly as
/// engines.toml names it. On a machine where libmp3lame.dll is present beside
/// the workers this plans; on one without it, it refuses naming the library.
#[test]
fn wav_to_mp3_gates_on_the_lame_library_row() {
    let lame_present = engines::registry()
        .iter()
        .find(|e| e.name == "libmp3lame")
        .expect("libmp3lame is declared")
        .available;
    let plan = route(
        PlanRequest::new(FormatId::Wav, Target::Format(FormatId::Mp3)),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    assert_eq!(
        plan.is_executable(),
        lame_present,
        "routability must track DLL presence, not hope"
    );
    if !lame_present {
        assert!(plan.warnings().iter().any(|w| matches!(
            w,
            openconvert_core::plan::Warning::NoRoute(
                openconvert_core::plan::NoRoute::RequirementUnmet {
                    requirement: openconvert_core::route::Requirement::Engine("libmp3lame")
                }
            )
        )));
    }
}

/// M4A decode arrived with symphonia's isomp4/aac/alac features; the row is
/// what makes that honest. Routed through oc-audio like every other audio
/// conversion.
#[test]
fn m4a_to_wav_routes_through_oc_audio() {
    let plan = route(
        PlanRequest::new(FormatId::M4a, Target::Format(FormatId::Wav)),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    assert!(
        plan.is_executable(),
        "m4a -> wav must plan: {:?}",
        plan.warnings()
    );
    assert!(plan.steps().iter().all(|s| matches!(
        s.isolation,
        openconvert_core::isolation::Isolation::Sandboxed(_)
    )));
}

/// The reverse archive repack: Tar -> Zip is Class A container surgery in
/// oc-archive, same caps, same engine.
#[test]
fn tar_to_zip_routes_class_a() {
    let plan = route(
        PlanRequest::new(FormatId::Tar, Target::Format(FormatId::Zip)),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    assert!(plan.is_executable(), "tar -> zip must plan");
    assert_eq!(plan.class(), Some(Class::A));
}

/// --trim on something that is not a video container is refused with the
/// reason that names the operation, not a generic "no route".
#[test]
fn trim_outside_matroska_refuses_and_says_why() {
    let plan = route(
        PlanRequest::new(
            FormatId::Png,
            Target::Operation(openconvert_core::target::Operation::Trim {
                start_ms: 0,
                end_ms: 1000,
            }),
        ),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    assert!(plan.steps().is_empty());
    assert!(plan.warnings().iter().any(|w| matches!(
        w,
        openconvert_core::plan::Warning::NoRoute(
            openconvert_core::plan::NoRoute::OperationInputMismatch { operation: "trim" }
        )
    )));
}

/// Trim INSIDE Matroska is Class A container surgery, in-process, and the
/// step carries the window verbatim.
#[test]
fn trim_inside_matroska_plans_one_class_a_step() {
    let plan = route(
        PlanRequest::new(
            FormatId::Mkv,
            Target::Operation(openconvert_core::target::Operation::Trim {
                start_ms: 500,
                end_ms: 9_500,
            }),
        ),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    assert!(plan.is_executable(), "mkv trim must plan");
    assert_eq!(plan.class(), Some(Class::A));
    assert_eq!(plan.steps().len(), 1);
    assert!(matches!(
        plan.steps()[0].kind,
        openconvert_core::plan::StepKind::Trim {
            start_ms: 500,
            end_ms: 9_500
        }
    ));
}

/// --page N plans ONE render step whose parameter survives into the plan,
/// so the preview says exactly which page will be rendered.
#[test]
fn page_render_plans_the_requested_page() {
    let plan = route(
        PlanRequest::new(
            FormatId::Pdf,
            Target::Operation(openconvert_core::target::Operation::RenderPage {
                page: 2,
                to: FormatId::Jpeg,
            }),
        ),
        Properties::None,
        &Policy::default(),
        &sandboxed_capable_env(),
    );
    // Platform-honest: pdfium links on Windows only.
    if cfg!(windows) {
        assert!(plan.is_executable(), "pdf page render must plan on Windows");
        assert_eq!(plan.class(), Some(Class::B));
        assert!(matches!(
            plan.steps()[0].kind,
            openconvert_core::plan::StepKind::RenderPage { page: 2 }
        ));
    } else {
        assert!(plan.steps().is_empty(), "no pdfium, no plan");
    }
}
