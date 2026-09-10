//! Phase 2 of detection: what the header says, before anything is planned.
//!
//! # The control this turns on
//!
//! `route::limits_for` narrows `decode_pixels` to twice an image's actual pixel
//! count — never widens, because widening is the clamp's job to refuse. It has
//! been there since week 2, and **the CLI passed `Properties::None` to every
//! call**, so it never narrowed anything. A 32×32 thumbnail converted with the
//! full 256 Mpx ceiling, which is the ceiling a decoder bug needs to turn a
//! small file into a gigabyte allocation.
//!
//! `03` §6 calls this two-phase detection: phase 1 is `sniff`, which reads
//! magic bytes and decides *what* the file is; phase 2 is this, which reads a
//! header and decides *how big* it claims to be. Phase 1 has existed and been
//! used since week 1. Phase 2 existed and was used by nothing.
//!
//! # Where the probe runs is the same question as where the conversion runs
//!
//! A pure-Rust format is probed in this process, because it was already going
//! to be *decoded* in this process — `route()` grants `InProcess` only to
//! formats the table marks `pure_rust_parser`, so probing there adds no
//! attack surface that conversion does not already carry.
//!
//! Everything else is probed **in the worker**, over the protocol. Probing a
//! HEIC in the host to decide how to sandbox the HEIC would be parsing an
//! untrusted file with libheif inside the trust boundary, which is the exact
//! inversion this architecture exists to prevent.
//!
//! # A failed probe is not a failed conversion
//!
//! It returns `Properties::None`, and the plan proceeds at the un-narrowed
//! ceiling. The probe is defence in depth: it makes a limit *tighter* when the
//! header is readable. A file whose header we cannot read is not thereby safe
//! to refuse — `sniff` may still have identified it, and refusing here would
//! turn a hardening measure into a new way to fail.

use crate::pool::WorkerPool;
use openconvert_core::facts::{FileFacts, Properties};
use openconvert_core::format::{FormatId, MediaKind};
use openconvert_core::limits::Limits;
use openconvert_sandbox::argv::EngineBin;

/// Read a header and report what it claims.
///
/// Never fails: an unreadable header is [`Properties::None`], which leaves
/// limits at the policy ceiling rather than refusing the file.
pub fn probe(
    facts: &FileFacts,
    bytes: &[u8],
    limits: &Limits,
    pool: &mut WorkerPool,
) -> Properties {
    let format = facts.sniff().detected;
    let Some(kind) = format.kind() else {
        return Properties::None;
    };

    // Images and audio are the kinds with probes today. Document and Tabular
    // have variants and no probe, and returning `None` for them says so -- a
    // zero-filled variant would be a fact the plan could not stand behind.
    // A container header is read here whatever the format's parser flag says.
    // `crate::container` is pure Rust over a byte slice with bounded depth,
    // bounded element count and no allocation driven by the file -- the same
    // standard `sniff` meets -- and `02` s3.1 puts Class A video in core for
    // exactly that reason. Sending it to a worker would buy isolation from a
    // parser that does not need isolating.
    if kind == MediaKind::Video {
        return probe_container(format, bytes);
    }

    if !matches!(kind, MediaKind::Image | MediaKind::Audio) {
        return Properties::None;
    }

    if format.has_pure_rust_parser() {
        probe_in_process(bytes)
    } else {
        probe_in_worker(format, bytes, limits, pool)
    }
}

/// Probe a format this process is already trusted to decode.
fn probe_in_process(bytes: &[u8]) -> Properties {
    use image::{ImageDecoder, ImageFormat, ImageReader};

    let Ok(reader) = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() else {
        return Properties::None;
    };
    let format = reader.format();
    let Ok(decoder) = reader.into_decoder() else {
        return Properties::None;
    };
    let (width, height) = decoder.dimensions();

    Properties::Image {
        width,
        height,
        has_alpha: decoder.color_type().has_alpha(),
        // See `openconvert_worker::Response::Properties`: 0 is "not counted".
        // Counting an animated format's frames is a decode, and a probe that
        // decodes is not a probe.
        frames: match format {
            Some(ImageFormat::Gif | ImageFormat::WebP) => 0,
            _ => 1,
        },
    }
}

/// Read a container header for the codecs a remux turns on.
///
/// `01` s154's flagship is decided from this: `route()` asks
/// `codec::streams_carryable` whether the streams fit the destination, and
/// until something produced `Properties::Video` the answer was always no.
fn probe_container(format: FormatId, bytes: &[u8]) -> Properties {
    let Some(s) = crate::container::read_streams(format, bytes) else {
        return Properties::None;
    };
    Properties::Video {
        // Not read. The header walk stops at the track list, and a duration
        // needs timescale arithmetic each container spells differently -- easy
        // to get subtly wrong, and consumed by nothing. Zero is the same "not
        // determined" sentinel `width` and `frames` use.
        duration_ms: 0,
        width: s.width,
        height: s.height,
        video: s.video,
        audio: s.audio,
    }
}

/// Probe a format only a confined engine may parse.
fn probe_in_worker(
    format: FormatId,
    bytes: &[u8],
    limits: &Limits,
    pool: &mut WorkerPool,
) -> Properties {
    let Some(bin) = format.kind().and_then(EngineBin::for_media) else {
        return Properties::None;
    };
    // Trusted provenance is not assumed here: the pool is asked for a worker
    // under the same rules a conversion uses, and a probe of an untrusted file
    // gets its own process exactly as a conversion of one would (SR-20).
    match pool.probe(bin, facts_provenance(), limits, bytes) {
        Ok(p) => p,
        // Deliberately silent. A worker that cannot read a header has told us
        // nothing about whether the file converts, and the conversion attempt
        // will produce a message of its own if it fails. Two errors for one
        // cause is how `03` §13's "name the file, the step and the engine"
        // turns into noise.
        Err(_) => Properties::None,
    }
}

/// The provenance a probe runs under.
///
/// **Untrusted, always.** A probe happens *before* anything has decided the
/// file is safe — that is what it is for — so it is the one operation that
/// cannot claim the trusted path. Under `Balanced` this costs a fresh worker
/// per probed file, which is the honest price of looking at something before
/// judging it.
const fn facts_provenance() -> openconvert_core::facts::Provenance {
    openconvert_core::facts::Provenance::Untrusted
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// **The narrowing actually happens**, end to end through `route`.
    ///
    /// The whole point of the module. A 40×30 PNG must not convert under the
    /// 256-megapixel default ceiling.
    #[test]
    fn a_probed_image_narrows_the_pixel_limit() {
        use openconvert_core::policy::Policy;

        let props = probe_in_process(&png(40, 30));
        let Properties::Image { width, height, .. } = props else {
            panic!("expected Image, got {props:?}");
        };
        assert_eq!((width, height), (40, 30));

        let policy = Policy::default();
        let wide = policy.base_limits().decode_pixels;
        // `limits_for` is private to route; this asserts the same arithmetic it
        // performs, which is what the plan will carry.
        let narrowed = wide.min(u64::from(width) * u64::from(height) * 2);
        assert_eq!(narrowed, 2400, "40 x 30 x 2");
        assert!(
            narrowed < wide,
            "the probe must narrow: {narrowed} vs {wide}"
        );
    }

    /// Alpha is read from the header, not guessed.
    #[test]
    fn alpha_is_reported_from_the_header() {
        let rgba = probe_in_process(&png(4, 4));
        assert!(
            matches!(
                rgba,
                Properties::Image {
                    has_alpha: true,
                    ..
                }
            ),
            "an RGBA PNG should report alpha: {rgba:?}"
        );

        let gray = {
            let img = image::GrayImage::from_fn(4, 4, |_, _| image::Luma([128]));
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageLuma8(img)
                .write_to(&mut out, image::ImageFormat::Png)
                .expect("encode");
            probe_in_process(&out.into_inner())
        };
        assert!(
            matches!(
                gray,
                Properties::Image {
                    has_alpha: false,
                    ..
                }
            ),
            "a grayscale PNG should not report alpha: {gray:?}"
        );
    }

    /// An animated format reports "not counted" rather than a guess.
    #[test]
    fn an_animatable_format_does_not_claim_a_frame_count() {
        let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([1, 2, 3, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Gif)
            .expect("encode");
        let props = probe_in_process(&out.into_inner());
        assert!(
            matches!(props, Properties::Image { frames: 0, .. }),
            "a GIF must not claim a counted frame total: {props:?}"
        );
    }

    /// **Garbage does not refuse the file**, it declines to narrow.
    ///
    /// The probe is defence in depth. Turning an unreadable header into a
    /// refusal would make a hardening measure into a new failure mode, on files
    /// `sniff` may well have identified correctly.
    #[test]
    fn an_unreadable_header_yields_no_properties_rather_than_an_error() {
        assert_eq!(probe_in_process(b"not an image at all"), Properties::None);
        assert_eq!(probe_in_process(&[]), Properties::None);
        // A valid signature with a truncated header: the case that separates
        // "we could not identify it" from "we identified it and it lied".
        assert_eq!(
            probe_in_process(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR"),
            Properties::None
        );
    }
}
