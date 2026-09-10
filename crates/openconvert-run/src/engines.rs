//! Engine adapters, and the registry that declares what each one needs.
//!
//! **No trait.** There is one engine, and a trait with one implementation is a
//! guess about the second. The `Engine` trait arrives in week 15, when
//! `oc-images` gives it a second implementation and the shape is known rather
//! than imagined.

use crate::strip;
use openconvert_core::environment::EngineEntry;
use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;
use openconvert_core::plan::StepKind;
use std::io::{Read, Seek};

/// The engines this build knows about.
///
/// **`memory_safe` is a data claim**, and `03` Â§7.4 says so rather than
/// pretending otherwise: it is gated by a review rule on this one list, and a
/// property test reads it to assert no memory-unsafe engine is ever
/// `InProcess`. A wrong `true` here defeats SR-2 silently, which is why the
/// list is short and lives in one place.
#[must_use]
pub fn registry() -> Vec<EngineEntry> {
    vec![
        EngineEntry {
            name: "image-rs",
            memory_safe: true,
            available: true,
        },
        // The four worker binaries.
        //
        // `oc-images` is PLATFORM-honest AND DISK-honest, in that order.
        //
        // Platform: on Linux the libheif half is not built yet, so the answer
        // is a flat no. Windows links it.
        //
        // Disk: a Windows build that LINKED libheif still cannot decode a
        // HEIC if the DLL is not where the loader will look. This row used to
        // be a hardcoded `true`, which meant an install missing heif.dll got a
        // planned HEIC -> JPEG that died at the last step, after the user had
        // been told it would work -- the exact failure "available means a plan
        // routed here finishes" exists to prevent, and the failure the two
        // encoder rows below were already checking for.
        //
        // The names here are the LOAD-TIME CLOSURE, not the libraries we call.
        // heif + de265 decode HEIC and raw_r decodes the five camera formats,
        // but `oc-images/build.rs` records that raw_r.dll dies before `main`
        // without lcms2-2.dll and z.dll beside it -- so an install missing
        // either of those has no working worker at all, however many of the
        // three it does have. The condition has to name what the LOADER
        // needs, or it is a different guess dressed as a check.
        //
        // The name is `raw_r.dll`, the reentrant build, because that is what
        // build.rs links (`raw_r.lib`) and copies. `raw.dll` also sits in
        // vcpkg's bin directory and is NOT the one loaded; checking for it
        // would have reported this very machine as unavailable.
        //
        // JXL goes through jxl-oxide, pure Rust and always present, so it is
        // deliberately not part of this condition -- a JXL-only install losing
        // its JXL routes to an unrelated missing DLL would be its own
        // dishonesty, in the opposite direction.
        #[cfg(windows)]
        EngineEntry {
            name: "oc-images",
            memory_safe: false,
            available: IMAGES_DLLS.iter().copied().all(dll_present),
        },
        #[cfg(not(windows))]
        EngineEntry {
            name: "oc-images",
            memory_safe: false,
            available: false,
        },
        // Platform- and disk-honest exactly like oc-images above: on Linux
        // there is no POSIX pdfium link story yet, and on Windows the row is
        // true only when pdfium.dll actually sits beside the worker. It was a
        // hardcoded `true`, and an install without the DLL therefore planned
        // `Pdf -> Png` and failed at the render.
        #[cfg(windows)]
        EngineEntry {
            name: "oc-pdf",
            memory_safe: false,
            available: dll_present("pdfium.dll"),
        },
        #[cfg(not(windows))]
        EngineEntry {
            name: "oc-pdf",
            memory_safe: false,
            available: false,
        },
        // `oc-audio` IS available. Like oc-archive, it is pure Rust today --
        // symphonia decodes, hound writes WAV, flacenc writes FLAC -- and it
        // converts for real through the confined worker, so a plan routed to
        // it finishes. `memory_safe: true` is honest and still does NOT grant
        // in-process: the audio formats' parser flag answers that question,
        // and attacker-supplied media parses behind the boundary whatever
        // today's decoder is written in.
        EngineEntry {
            name: "oc-audio",
            memory_safe: true,
            available: true,
        },
        // The linked MP3/Opus ENCODERS, as rows named exactly like their
        // engines.toml entries. Availability is measured the way the loader
        // will resolve them at run time: each DLL must exist beside the
        // engine binaries (which build.rs copies there when vcpkg supplied
        // it). A row that says "available" without its DLL on disk would let
        // `route()` plan a conversion that dies at the last step -- the exact
        // failure "available means a plan routed here finishes" exists to
        // prevent.
        //
        // `memory_safe: false` is not a slur: both are C libraries reached
        // through FFI inside the confined worker, and the property tests read
        // this flag to keep them out of our address space.
        EngineEntry {
            name: "libmp3lame",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("libmp3lame.dll"),
            #[cfg(not(windows))]
            available: false,
        },
        EngineEntry {
            name: "libopus",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("opus.dll"),
            #[cfg(not(windows))]
            available: false,
        },
        // ---- AI foundation (A10 Phase 3) ----
        //
        // The runtime and every model row are declared, wired through
        // engines.toml / models.toml, and INERT: no adapter has shipped yet,
        // so `available` is false by hand. This is the honest half-way state
        // the registry exists to express -- the plumbing (registry consumer,
        // pinned store, download+verify) exists and is tested, while a route
        // whose execution cannot finish still cannot be planned.
        //
        // THE FIRST ADAPTER HAS LANDED, and `u2netp` below is what that looks
        // like. The rows without adapters stay `false` by hand, which is the
        // same honest half-way state as before -- now with one row that is not
        // in it.
        //
        // Each MODEL row gates on its own artifact rather than on "oc-ai",
        // which is why they are separate entries: a machine with the runtime
        // and one downloaded model can plan that model's work and nothing
        // else, and says so per row.
        EngineEntry {
            name: "oc-ai",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll"),
            #[cfg(not(windows))]
            available: false,
        },
        EngineEntry {
            name: "onnxruntime",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll"),
            #[cfg(not(windows))]
            available: false,
        },
        // Background removal. Three conditions, all of them necessary:
        // the runtime can load, the pinned artifact is in the store, and the
        // user turned it on. Downloading a model is not consent to run it.
        EngineEntry {
            name: "u2netp",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll") && model_ready("u2netp"),
            #[cfg(not(windows))]
            available: false,
        },
        // OCR needs all THREE artifacts. A machine with the detector and no
        // dictionary can plan nothing, and reporting available on two of three
        // would fail at the last step -- which is what this flag exists to
        // prevent.
        // The Video module: a separate download, by design. `02-FEATURES` §3
        // keeps every transcoder out of core, so this is available exactly
        // when the module is installed -- and reports false, loudly, when it
        // is not, rather than planning work that cannot run.
        EngineEntry {
            name: "ffmpeg",
            memory_safe: false,
            available: crate::video::available(),
        },
        EngineEntry {
            name: "deepfilternet",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll")
                && model_ready("deepfilternet")
                && model_ready("deepfilternet-aux"),
            #[cfg(not(windows))]
            available: false,
        },
        EngineEntry {
            name: "realesrgan-x4",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll") && model_ready("realesrgan-x4"),
            #[cfg(not(windows))]
            available: false,
        },
        EngineEntry {
            name: "modnet",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll") && model_ready("modnet"),
            #[cfg(not(windows))]
            available: false,
        },
        // The best background-removal tier. Listed here for the same reason
        // every other model is: `Requirement::Engine` is what a route checks,
        // so a model absent from this list is a model no plan can ever reach --
        // which is what "NO PLAN. Nothing would run." means when the weights
        // are sitting in the store, downloaded and enabled.
        EngineEntry {
            name: "birefnet-lite",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll") && model_ready("birefnet-lite"),
            #[cfg(not(windows))]
            available: false,
        },
        EngineEntry {
            name: "paddleocr",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll")
                && model_ready("paddleocr-det")
                && model_ready("paddleocr-rec")
                && model_ready("paddleocr-dict"),
            #[cfg(not(windows))]
            available: false,
        },
        // Transcription, all three artifacts.
        EngineEntry {
            name: "whisper",
            memory_safe: false,
            #[cfg(windows)]
            available: dll_present("onnxruntime.dll")
                && model_ready("whisper-encoder")
                && model_ready("whisper-decoder")
                && model_ready("whisper-tokenizer")
                && model_ready("silero-vad"),
            #[cfg(not(windows))]
            available: false,
        },
        // `oc-archive` IS available, and this is the first worker where that is
        // true. It repacks Zip -> Tar for real, through the real host, with
        // SR-5's caps enforced -- so a plan routed to it finishes, which is what
        // `available` has to mean.
        //
        // `memory_safe: true` is the honest entry, not an oversight. It links
        // `zip`, `tar` and `flate2`, all pure Rust with no C backend. That makes
        // it the one engine in this list a property test would let run
        // in-process -- and it still does not, because `route()` grants
        // `InProcess` from the FORMAT table's `pure_rust_parser` flag and an
        // archive is not marked pure. Confining it costs a process launch and
        // buys the bomb caps a second enforcement point, which is a trade worth
        // making for attacker-supplied archives.
        //
        // The table's `7z -> Zip` row is NOT implemented: that input needs a
        // dependency this crate does not have. It fails at the engine with a
        // clear, non-retryable message rather than silently, and it is named
        // here so nobody reads `available: true` as covering both rows.
        EngineEntry {
            name: "oc-archive",
            memory_safe: true,
            available: true,
        },
    ]
}

/// Whether an encoder DLL sits where the loader will look for it.
///
/// Workers are resolved against our own installation directory
/// ([`openconvert_sandbox::argv::EngineBin`]), and a DLL dependency loads from
/// the worker's own directory — the same place build.rs copies it. Checking
/// there is therefore not a guess about PATH or vcpkg; it is the exact
/// condition under which the load will succeed.
/// Everything `oc-images` needs ON DISK before a plan routed to it can
/// finish: the two libraries it calls for HEIC, the RAW decoder, and the two
/// that RAW decoder itself loads. Shared by the registry row and the test
/// that asserts it, because two copies of this list is how they come to
/// disagree.
#[cfg(windows)]
const IMAGES_DLLS: [&str; 5] = [
    "heif.dll",
    "libde265.dll",
    "raw_r.dll",
    "lcms2-2.dll",
    "z.dll",
];

/// Whether a model can actually be RUN right now.
///
/// Downloaded is not the same as usable, and neither is enabled. All three
/// have to hold: the row is pinned, the verified artifact is in the
/// content-addressed store, and the user turned it on. Reporting `available`
/// on a downloaded-but-disabled model would let `route()` plan work the user
/// declined.
#[cfg(windows)]
fn model_ready(id: &str) -> bool {
    crate::models::list()
        .into_iter()
        .any(|m| m.id == id && m.downloaded && m.enabled)
}

/// Whether a native library sits where the loader will look for it.
///
/// Through [`crate::engine_dir`] since installed copies exist: a packaged app
/// may keep its engines in a resource directory that is not the one holding the
/// executable, and a check that only ever looked beside the binary reported
/// every engine unavailable on exactly those builds.
#[cfg(windows)]
fn dll_present(name: &str) -> bool {
    crate::engine_dir::locate(name).is_some()
}

/// What an adapter produced.
#[derive(Debug)]
pub struct StepOutput {
    /// The bytes to write.
    pub bytes: Vec<u8>,
    /// What was removed, for the receipt. SR-11 needs the list, not a boolean.
    pub removed: Vec<String>,
    /// The engine that did it, for the receipt.
    pub engine: &'static str,
    /// What the worker **read back** about its own confinement, rendered.
    ///
    /// `None` for an in-process step, which has no worker and no confinement to
    /// report. `Some` replaces the planned profile in the receipt, and the
    /// distinction is I11: `route()` chooses a profile from what the machine
    /// *can* do, and a receipt must record what actually *did*. Rendering the
    /// plan would make every receipt a statement of intent wearing the clothes
    /// of evidence.
    pub confinement: Option<String>,
}

/// Errors an adapter can produce, typed rather than stringly.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The input could not be decoded.
    #[error("{engine} could not decode this {format}: {detail}")]
    Decode {
        /// Which engine.
        engine: &'static str,
        /// What it was told the file is.
        format: FormatId,
        /// The underlying reason.
        detail: String,
    },
    /// A limit was hit before any work was done.
    #[error("{what} exceeds the limit of {limit} ({actual} required)")]
    LimitExceeded {
        /// Which limit.
        what: &'static str,
        /// The ceiling.
        limit: u64,
        /// What the input needed.
        actual: u64,
    },
    /// No adapter handles this step.
    #[error("no in-process adapter for {0:?}")]
    Unsupported(StepKind),
    /// Underlying I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Run one step in our own process.
///
/// Only ever called for a step whose `Isolation` is `InProcess`, which
/// `route()` grants only to formats the table marks `pure_rust_parser`. The
/// caller enforces that; this function does not re-derive it, because two
/// places deciding the same thing is how they drift.
pub fn run_in_process<R: Read + Seek>(
    kind: StepKind,
    src: &mut R,
    limits: &Limits,
) -> Result<StepOutput, EngineError> {
    match kind {
        StepKind::StripMetadata => {
            let mut out = Vec::new();
            let removed = strip::strip_jpeg(src, &mut out)?;
            Ok(StepOutput {
                bytes: out,
                removed: removed.into_iter().map(String::from).collect(),
                engine: "openconvert-strip",
                confinement: None,
            })
        }
        // Image -> PDF: generated here, never parsed. See `pdf` for why that
        // distinction is what makes this safe without a sandbox.
        StepKind::Transcode {
            to: FormatId::Pdf,
            from,
        } => {
            let mut raw = Vec::new();
            src.rewind()?;
            src.read_to_end(&mut raw)?;
            // The step names its own source, and BINDING it beats re-sniffing.
            // Guessing PNG-or-JPEG from the first byte was fine while those
            // were the only two rows, and silently wrong the moment TIFF,
            // WebP, GIF and BMP could reach here — a TIFF would have been
            // handed to the JPEG embedder.
            let (bytes, removed) =
                crate::pdf::wrap_image(from, &raw, limits).map_err(|e| EngineError::Decode {
                    engine: "openconvert-pdf",
                    format: from,
                    detail: e.to_string(),
                })?;
            Ok(StepOutput {
                bytes,
                removed,
                engine: "openconvert-pdf",
                confinement: None,
            })
        }
        // Tabular: pure text both directions, in-process by the table's own
        // flag. The sniff already decided which side of this pair is which;
        // the step's formats say the same thing.
        StepKind::Transcode {
            from: FormatId::Csv,
            to: FormatId::Json,
        } => {
            let mut raw = Vec::new();
            src.rewind()?;
            src.read_to_end(&mut raw)?;
            let (bytes, removed) =
                crate::tabular::csv_to_json(&raw).map_err(|e| EngineError::Decode {
                    engine: "openconvert-tabular",
                    format: FormatId::Csv,
                    detail: e.to_string(),
                })?;
            Ok(StepOutput {
                bytes,
                removed,
                engine: "openconvert-tabular",
                confinement: None,
            })
        }
        StepKind::Transcode {
            from: FormatId::Json,
            to: FormatId::Csv,
        } => {
            let mut raw = Vec::new();
            src.rewind()?;
            src.read_to_end(&mut raw)?;
            let (bytes, removed) =
                crate::tabular::json_to_csv(&raw).map_err(|e| EngineError::Decode {
                    engine: "openconvert-tabular",
                    format: FormatId::Json,
                    detail: e.to_string(),
                })?;
            Ok(StepOutput {
                bytes,
                removed,
                engine: "openconvert-tabular",
                confinement: None,
            })
        }
        StepKind::Transcode { from, to } => transcode(src, from, to, limits),
        other => Err(EngineError::Unsupported(other)),
    }
}

/// Decode and re-encode through `image-rs`.
///
/// # The limit is checked before allocation, not after
///
/// A 4 KB PNG can declare 500 megapixels, which is a ~2 GB allocation **in the
/// same address space as the GUI**. Reading the header first costs nothing and
/// is the whole fix â€” and it produces an actionable error instead of an OOM
/// kill, which is the difference between a message and a crash report.
fn transcode<R: Read + Seek>(
    src: &mut R,
    from: FormatId,
    to: FormatId,
    limits: &Limits,
) -> Result<StepOutput, EngineError> {
    use image::ImageReader;
    use std::io::Cursor;

    // Read once. Both passes below work from the same bytes, which is I13's
    // rule applied inside a single step: the bytes whose header we vetted are
    // the bytes we then decode, with no opportunity for the two to differ.
    let mut raw = Vec::new();
    src.rewind()?;
    src.read_to_end(&mut raw)?;

    // Pass one: header only. Nothing is decoded, nothing is allocated.
    let (w, h) = ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| decode_err(from, &e))?
        .into_dimensions()
        .map_err(|e| decode_err(from, &e))?;

    let pixels = u64::from(w) * u64::from(h);
    if pixels > limits.decode_pixels {
        return Err(EngineError::LimitExceeded {
            what: "decode_pixels",
            limit: limits.decode_pixels,
            actual: pixels,
        });
    }

    // Pass two: decode, handing image-rs its own limits as a second line of
    // defence. Ours is the one that matters -- it ran before any allocation --
    // but a decoder that also refuses cannot be talked into allocating by a
    // malformed intermediate partway through.
    let mut reader = ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| decode_err(from, &e))?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(limits.memory_bytes);
    lim.max_image_width = Some(w);
    lim.max_image_height = Some(h);
    reader.limits(lim);

    let img = reader.decode().map_err(|e| decode_err(from, &e))?;

    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, output_format(to)?)
        .map_err(|e| decode_err(to, &e))?;

    Ok(StepOutput {
        bytes: out.into_inner(),
        // A re-encode drops every metadata segment the source carried, because
        // nothing is copied across. Worth recording rather than leaving the
        // user to infer it from the file size.
        removed: vec!["all source metadata (re-encoded)".to_string()],
        engine: "image-rs",
        confinement: None,
    })
}

fn decode_err(format: FormatId, e: &dyn std::fmt::Display) -> EngineError {
    EngineError::Decode {
        engine: "image-rs",
        format,
        detail: e.to_string(),
    }
}

fn output_format(to: FormatId) -> Result<image::ImageFormat, EngineError> {
    Ok(match to {
        FormatId::Png => image::ImageFormat::Png,
        FormatId::Jpeg => image::ImageFormat::Jpeg,
        FormatId::Webp => image::ImageFormat::WebP,
        FormatId::Gif => image::ImageFormat::Gif,
        FormatId::Bmp => image::ImageFormat::Bmp,
        FormatId::Tiff => image::ImageFormat::Tiff,
        other => {
            return Err(EngineError::Unsupported(StepKind::Transcode {
                from: other,
                to: other,
            }))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::new(w, h);
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode");
        out.into_inner()
    }

    #[test]
    fn png_transcodes_to_jpeg() {
        let src = png(8, 8);
        let out = run_in_process(
            StepKind::Transcode {
                from: FormatId::Png,
                to: FormatId::Jpeg,
            },
            &mut Cursor::new(src),
            &Limits::defaults(),
        )
        .expect("transcode");
        assert_eq!(out.engine, "image-rs");
        assert!(
            out.bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            "output is not a JPEG"
        );
    }

    /// SR-5: the pixel limit is checked **from the header, before allocating**.
    ///
    /// The error names the limit and the actual, so `03` Â§13's message rule is
    /// satisfiable: an OOM kill names nothing.
    #[test]
    fn pixel_limit_refuses_before_decoding() {
        let src = png(64, 64);
        let tight = Limits {
            decode_pixels: 100,
            ..Limits::defaults()
        };
        let err = run_in_process(
            StepKind::Transcode {
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
                assert_eq!(limit, 100);
                assert_eq!(actual, 4096);
            }
            other => panic!("wrong error: {other}"),
        }
    }

    /// The control: the same image passes under the default limit.
    ///
    /// Without this, the test above is satisfied by an adapter that refuses
    /// everything.
    #[test]
    fn the_same_image_passes_under_the_default_limit() {
        let src = png(64, 64);
        run_in_process(
            StepKind::Transcode {
                from: FormatId::Png,
                to: FormatId::Jpeg,
            },
            &mut Cursor::new(src),
            &Limits::defaults(),
        )
        .expect("should succeed under the default limit");
    }

    #[test]
    fn a_corrupt_image_errors_rather_than_panicking() {
        let err = run_in_process(
            StepKind::Transcode {
                from: FormatId::Png,
                to: FormatId::Jpeg,
            },
            &mut Cursor::new(b"\x89PNG\r\n\x1a\nGARBAGE".to_vec()),
            &Limits::defaults(),
        )
        .expect_err("should error");
        assert!(matches!(err, EngineError::Decode { .. }));
    }

    /// Every row says what is true of **that** engine.
    ///
    /// This asserted, for every `tx-*` worker, that it was memory-unsafe and
    /// unavailable -- true of all four when written, and a blanket claim rather
    /// than four facts. `oc-archive` is now pure Rust and does real work, so the
    /// blanket was the first thing to go red when the truth changed. That is the
    /// test working; a per-engine table is what keeps it working next time.
    #[test]
    fn the_registry_declares_memory_safety_honestly() {
        let r = registry();
        let get = |n: &str| {
            r.iter()
                .find(|e| e.name == n)
                .unwrap_or_else(|| panic!("{n} is missing from the registry"))
        };

        // (engine, memory_safe, available, why)
        //
        // oc-images is PLATFORM-honest: Windows links libheif (HEIC decode)
        // and jxl-oxide (pure Rust), so plans routed there finish; Linux
        // builds do not link libheif yet and say so. The two encoder rows are
        // DISK-honest on top of platform-honest: available only when the DLL
        // actually sits where the loader will look, which is why they are
        // asserted against `dll_present` rather than a constant.
        // `cfg!(windows)` is a runtime bool, so the expressions it guarded were
        // still COMPILED everywhere -- and `dll_present` and `IMAGES_DLLS` are
        // `#[cfg(windows)]` items that do not exist elsewhere. This test, and
        // therefore the whole crate's test target, failed to build on Linux.
        // `#[cfg]` on the binding is the version that guards compilation.
        #[cfg(windows)]
        let (lame_available, opus_available, images_available, pdf_available, ort_available) = (
            dll_present("libmp3lame.dll"),
            dll_present("opus.dll"),
            // The worker rows are disk-honest too now, and asserting them
            // against a constant is what let the hardcoded `true` sit here
            // unnoticed.
            IMAGES_DLLS.iter().copied().all(dll_present),
            dll_present("pdfium.dll"),
            dll_present("onnxruntime.dll"),
        );
        // Every native row is `available: false` off Windows, which is what the
        // registry itself declares a few dozen lines above.
        #[cfg(not(windows))]
        let (lame_available, opus_available, images_available, pdf_available, ort_available) =
            (false, false, false, false, false);

        // Same story for the model rows: `model_ready` is `#[cfg(windows)]`, so
        // the assertions below need a shim rather than a direct call. Off
        // Windows every model row is unavailable because the runtime row it
        // depends on is.
        #[cfg(windows)]
        let ready = model_ready;
        #[cfg(not(windows))]
        let ready = |_id: &str| false;
        let expected: Vec<(&str, bool, bool, &str)> = vec![
            ("image-rs", true, true, "pure Rust, in-process"),
            (
                "oc-images",
                false,
                images_available,
                "available exactly when heif/de265/raw sit beside the engine binaries",
            ),
            (
                "oc-pdf",
                false,
                pdf_available,
                "available exactly when pdfium.dll sits beside the engine binaries",
            ),
            (
                "oc-archive",
                true,
                true,
                "zip/tar/flate2, all pure Rust, and it repacks",
            ),
            (
                "oc-audio",
                true,
                true,
                "symphonia/hound/flacenc, pure Rust; decodes and encodes for real",
            ),
            (
                "libmp3lame",
                false,
                lame_available,
                "available exactly when libmp3lame.dll sits beside the engine binaries",
            ),
            (
                "libopus",
                false,
                opus_available,
                "available exactly when opus.dll sits beside the engine binaries",
            ),
            // The runtime rows are DISK-honest now that an adapter exists:
            // background removal runs, so "available" is a question about
            // this machine rather than about the build. The rows below them
            // stay hand-written `false` because their adapters have not
            // shipped, which is the same honesty pointed the other way.
            (
                "oc-ai",
                false,
                ort_available,
                "available exactly when onnxruntime.dll sits beside the engine binaries",
            ),
            (
                "onnxruntime",
                false,
                ort_available,
                "available exactly when onnxruntime.dll sits beside the engine binaries",
            ),
            (
                "u2netp",
                false,
                ort_available && ready("u2netp"),
                "needs the runtime AND a downloaded, enabled model",
            ),
            (
                "ffmpeg",
                false,
                crate::video::available(),
                "available exactly when the Video module is installed",
            ),
            (
                "deepfilternet",
                false,
                ort_available && ready("deepfilternet") && ready("deepfilternet-aux"),
                "needs the runtime AND both enhancement artifacts",
            ),
            (
                "realesrgan-x4",
                false,
                ort_available && ready("realesrgan-x4"),
                "needs the runtime AND a downloaded, enabled model",
            ),
            (
                "modnet",
                false,
                ort_available && ready("modnet"),
                "needs the runtime AND a downloaded, enabled model",
            ),
            (
                "birefnet-lite",
                false,
                ort_available && ready("birefnet-lite"),
                "needs the runtime AND a downloaded, enabled model",
            ),
            (
                "paddleocr",
                false,
                ort_available
                    && ready("paddleocr-det")
                    && ready("paddleocr-rec")
                    && ready("paddleocr-dict"),
                "needs the runtime AND all three OCR artifacts",
            ),
            (
                "whisper",
                false,
                ort_available
                    && ready("whisper-encoder")
                    && ready("whisper-decoder")
                    && ready("whisper-tokenizer")
                    && ready("silero-vad"),
                "needs the runtime AND all four transcription artifacts",
            ),
        ];
        for (name, memory_safe, available, why) in &expected {
            let e = get(name);
            assert_eq!(e.memory_safe, *memory_safe, "{name}: {why}");
            assert_eq!(e.available, *available, "{name}: {why}");
        }
        assert_eq!(
            r.len(),
            expected.len(),
            "an engine was added without a row here"
        );
    }

    /// **A memory-safe engine is still not automatically in-process.**
    ///
    /// `oc-archive` is pure Rust, so `memory_safe` alone would admit it to this
    /// process. It runs confined anyway: `route()` grants `InProcess` from the
    /// FORMAT table's `pure_rust_parser` flag, and archives are not marked pure.
    ///
    /// The two flags answer different questions -- "could this run here safely"
    /// and "should it" -- and collapsing them would put attacker-supplied
    /// archives in the same address space as the GUI to save a process launch.
    #[test]
    fn memory_safe_does_not_by_itself_grant_in_process() {
        use openconvert_core::format::FormatId;
        assert!(
            !FormatId::Zip.has_pure_rust_parser(),
            "if Zip becomes pure_rust_parser, archives run in-process and the \
             bomb caps lose their second enforcement point"
        );
    }
}
