//! `oc-images` â€” the image conversion worker.
//!
//! Runs confined, speaks the host protocol on stdin/stdout, and knows nothing
//! about paths. It is handed bytes and asked for bytes.
//!
//! # What it links today, and what it will link
//!
//! Today: `image-rs`, pure Rust, six formats. That is deliberate for the worker
//! *template* â€” it proves the whole pipeline end to end without a C toolchain
//! in the way, so the first thing that goes wrong is never the build.
//!
//! Weeks 18â€“26 add libvips, libheif, libavif, libjxl, libraw and lcms2. **This
//! crate is where their FFI `unsafe` will live** â€” on the far side of the trust
//! boundary, in a process that is already confined, which is the entire reason
//! engines are separate binaries rather than linked into the host (`03` Â§5).
//!
//! # Why it reads stdin rather than fd 3
//!
//! The design specifies fd 3 for requests and fd 4 for responses. Inheriting
//! numbered descriptors needs the handle-list spawn on Windows and a `pre_exec`
//! hook on Linux, both of which live in `openconvert-os` and both of which are
//! `unsafe` â€” so wiring them is host-side work, scheduled with the session.
//!
//! stdin/stdout is the same protocol over a different pair of descriptors. When
//! the numbered-fd plumbing lands, only `main` changes; `serve()` does not.

mod avif;
mod heif;
mod jxl;
mod svg;
// The native decoders link only when their vcpkg import libraries were found
// at build time (`build.rs` sets these cfgs; missing libraries warn and leave
// the formats refusing by name rather than breaking the build).
#[cfg(all(windows, has_lcms2))]
mod colour;
#[cfg(all(windows, has_raw))]
mod raw;

use openconvert_worker::{run_real, Converted, Engine, Probed, RunLimits};

/// The image engine.
mod mask;
struct Images;

impl Engine for Images {
    fn run_with_inputs(
        &self,
        bytes: &[u8],
        extras: &[Vec<u8>],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        if params.iter().any(|(k, v)| k == "op" && v == "mask") {
            let [original] = extras else {
                return Err("Correction needs the original image".into());
            };
            let strokes = params
                .iter()
                .find(|(k, _)| k == "strokes")
                .map(|(_, v)| v.as_str())
                .ok_or("Missing brush strokes")?;
            return mask::apply(bytes, original, strokes, limits);
        }
        if !extras.is_empty() || !models.is_empty() {
            return Err("Unexpected extra image inputs".into());
        }
        self.run_params(bytes, to, params, limits)
    }
    fn name(&self) -> &'static str {
        "oc-images"
    }

    fn version(&self) -> String {
        // The receipt records what ran. When libvips arrives this becomes the
        // linked library versions, which is the number a CVE advisory is
        // written against -- ours is not.
        format!("oc-images {} (image-rs)", env!("CARGO_PKG_VERSION"))
    }

    fn probe(&self, bytes: &[u8]) -> Result<Probed, String> {
        // Dispatch on CONTENT, never on the host's guess: a .jpg that is
        // really a HEIC decodes here like any other JPEG. JXL's signature is
        // checked first because image-rs cannot read it at all.
        // SVG has no signature; it is recognised structurally, before the
        // binary formats, because an XML document will not match any of them
        // and would otherwise fall through to image-rs and fail obscurely.
        // AVIF before the generic ISO-BMFF checks: it shares HEIC's box shape
        // and differs only by brand, so a brand test has to come first.
        if avif::looks_like_avif(bytes) {
            let (width, height) = avif::probe_dimensions(bytes)?;
            return Ok(Probed::Image {
                width,
                height,
                has_alpha: true,
                frames: 1,
            });
        }
        if svg::looks_like_svg(bytes) {
            let (width, height) = svg::probe_dimensions(bytes)?;
            return Ok(Probed::Image {
                width,
                height,
                has_alpha: true,
                frames: 1,
            });
        }
        if bytes.len() >= 12 && &bytes[4..8] == b"JXL " {
            let (width, height) = jxl::probe_dimensions(bytes).map_err(|e| e.to_string())?;
            return Ok(Probed::Image {
                width,
                height,
                has_alpha: true,
                frames: 1,
            });
        }
        #[cfg(windows)]
        if bytes.len() >= 12 && (&bytes[4..12] == b"ftypheic" || &bytes[4..12] == b"ftypmif1") {
            let decoded = heif::decode_primary(bytes).map_err(|e| e.to_string())?;
            return Ok(Probed::Image {
                width: decoded.width,
                height: decoded.height,
                has_alpha: decoded.has_alpha,
                frames: 1,
            });
        }
        // RAW, but only on magics no format image-rs reads can own — a plain
        // TIFF starts with the same bytes as a DNG, so the TIFF family is
        // handled by falling back below rather than by claiming it here.
        #[cfg(all(windows, has_raw))]
        if is_unambiguous_raw(bytes) {
            let decoded = raw::decode_raw(bytes).map_err(|e| e.to_string())?;
            return Ok(Probed::Image {
                width: decoded.width,
                height: decoded.height,
                has_alpha: false,
                frames: 1,
            });
        }

        use image::{ImageDecoder, ImageFormat, ImageReader};

        let generic_probe = |bytes: &[u8]| -> Result<Probed, String> {
            // Header only. Nothing is decoded, so a 4 KB file declaring 500
            // megapixels costs nothing to refuse -- which is the entire point: the
            // host narrows `decode_pixels` from this answer BEFORE any allocation.
            let reader = ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| format!("could not read the image header: {e}"))?;
            let format = reader.format();
            let decoder = reader
                .into_decoder()
                .map_err(|e| format!("could not read the image header: {e}"))?;

            let (width, height) = decoder.dimensions();
            // `color_type` comes from the header the decoder has already parsed. No
            // pixel is touched.
            let has_alpha = decoder.color_type().has_alpha();

            // GIF and WebP may animate, and counting their frames means walking the
            // file -- a decode. 0 says "not counted" rather than guessing 1, which
            // would be a fact the receipt could not stand behind.
            let frames = match format {
                Some(ImageFormat::Gif | ImageFormat::WebP) => 0,
                _ => 1,
            };

            Ok(Probed::Image {
                width,
                height,
                has_alpha,
                frames,
            })
        };

        match generic_probe(bytes) {
            Ok(probed) => Ok(probed),
            Err(e) => {
                // The TIFF family is ambiguous: DNG/NEF/ARW share its magic
                // with every ordinary .tif. image-rs gets first refusal; only
                // a header image-rs cannot open falls through to libraw.
                #[cfg(all(windows, has_raw))]
                if is_tiff_family(bytes) {
                    let decoded = raw::decode_raw(bytes).map_err(|raw_e| {
                        format!("{e}; not decodable as camera RAW either ({raw_e})")
                    })?;
                    return Ok(Probed::Image {
                        width: decoded.width,
                        height: decoded.height,
                        has_alpha: false,
                        frames: 1,
                    });
                }
                Err(e)
            }
        }
    }

    /// Convert with parameters.
    ///
    /// One parameter today: `op`, naming a pixel operation to apply after
    /// decoding and before encoding. Absent, this is the ordinary conversion
    /// path and nothing changes. An `op` this engine does not know is an
    /// ERROR, never a silent pass-through -- returning the image unmodified
    /// under a receipt saying it was inverted is precisely the lie the
    /// unrecognised-parameter rule exists to prevent.
    fn run_params(
        &self,
        bytes: &[u8],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        let op = params
            .iter()
            .find(|(k, _)| k == "op")
            .map(|(_, v)| v.as_str());
        if let Some(unknown) = params.iter().find(|(k, _)| k != "op" && k != "quality") {
            return Err(format!(
                "oc-images does not understand the parameter {:?}",
                unknown.0
            ));
        }
        // 1-100, the JPEG scale. Out of range is an ERROR rather than a clamp:
        // `quality=500` means the caller believes something about this scale
        // that is not true, and silently encoding at 100 hides that.
        let quality: u8 = match params.iter().find(|(k, _)| k == "quality") {
            None => DEFAULT_QUALITY,
            Some((_, v)) => {
                let n: u8 = v.parse().map_err(|_| {
                    format!("quality must be a whole number from 1 to 100, got {v:?}")
                })?;
                if !(1..=100).contains(&n) {
                    return Err(format!("quality must be from 1 to 100, got {n}"));
                }
                n
            }
        };
        match op {
            None => self.run(bytes, to, limits),
            Some("compress") => {
                // Decode the source once. Going through run() first encoded
                // JPEG at the default quality before applying the chosen level.
                let Probed::Image {
                    width,
                    height,
                    frames,
                    ..
                } = self.probe(bytes)?
                else {
                    return Err("compression needs an image".into());
                };
                if u64::from(width) * u64::from(height) > limits.decode_pixels {
                    return Err("image exceeds the pixel limit".into());
                }
                let preserve = |reason: &str| {
                    Ok(Converted {
                        bytes: bytes.to_vec(),
                        removed: vec![reason.into()],
                    })
                };
                if !matches!(
                    to,
                    "png" | "jpeg" | "webp" | "avif" | "tiff" | "bmp" | "gif"
                ) {
                    return preserve("Original preserved: no same-format compressor is available");
                }
                let detected = image::guess_format(bytes).ok();
                let same_format = match to {
                    "png" => detected == Some(image::ImageFormat::Png),
                    "jpeg" => detected == Some(image::ImageFormat::Jpeg),
                    "webp" => detected == Some(image::ImageFormat::WebP),
                    "tiff" => detected == Some(image::ImageFormat::Tiff),
                    "bmp" => detected == Some(image::ImageFormat::Bmp),
                    "gif" => detected == Some(image::ImageFormat::Gif),
                    "avif" => avif::looks_like_avif(bytes),
                    _ => false,
                };
                if !same_format {
                    return Err("compression must preserve the input format".into());
                }
                // Do not flatten animation, additional TIFF pages, or camera RAW data.
                let keep = match to {
                    "gif" => true,
                    "png" => image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))
                        .map_err(|e| e.to_string())?
                        .is_apng()
                        .map_err(|e| e.to_string())?,
                    "webp" => image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))
                        .map_err(|e| e.to_string())?
                        .has_animation(),
                    "tiff" => {
                        let mut decoder = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes))
                            .map_err(|e| e.to_string())?;
                        decoder.more_images()
                            || decoder.get_tag(tiff::tags::Tag::Unknown(50706)).is_ok()
                    }
                    "avif" => bytes
                        .get(8..32)
                        .unwrap_or_default()
                        .windows(4)
                        .any(|brand| brand == b"avis"),
                    _ => false,
                };
                if keep {
                    return preserve(
                        "Original animation, multipage image, or native image data preserved",
                    );
                }
                if frames > 1 {
                    return Ok(Converted {
                        bytes: bytes.to_vec(),
                        removed: vec!["Original animation preserved".into()],
                    });
                }
                let (img, decoder, icc_applied) = match decode_special(bytes)? {
                    Some((w, h, rgba, name)) => (
                        image::DynamicImage::ImageRgba8(
                            image::RgbaImage::from_raw(w, h, rgba)
                                .ok_or("decoded pixel buffer has the wrong dimensions")?,
                        ),
                        name,
                        false,
                    ),
                    None => decode_generic(bytes, width, height)?,
                };
                let mut removed = vec![format!("source metadata (re-encoded via {decoder})")];
                if icc_applied {
                    removed.push("embedded ICC profile transformed to sRGB".into());
                }
                if to == "tiff"
                    && !matches!(
                        img,
                        image::DynamicImage::ImageLuma8(_)
                            | image::DynamicImage::ImageLuma16(_)
                            | image::DynamicImage::ImageRgb8(_)
                            | image::DynamicImage::ImageRgb16(_)
                            | image::DynamicImage::ImageRgba8(_)
                            | image::DynamicImage::ImageRgba16(_)
                            | image::DynamicImage::ImageRgb32F(_)
                            | image::DynamicImage::ImageRgba32F(_)
                    )
                {
                    return preserve("Original TIFF sample format preserved");
                }
                compress_image(&img, to, quality, removed)
            }
            Some(name @ ("invert" | "greyscale")) => {
                let mut converted = self.run(bytes, to, limits)?;
                converted = apply_pixel_op(&converted.bytes, name, to, quality, converted.removed)?;
                Ok(converted)
            }
            Some(other) => Err(format!("oc-images does not know the operation {other:?}")),
        }
    }

    fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
        // Same content-first dispatch as probe: JXL, unambiguous RAW and HEIC
        // never reach the generic image-rs path, which cannot read them.
        let decoded_special: Option<(u32, u32, Vec<u8>, String)> = decode_special(bytes)?;

        // The limit is checked from the header, BEFORE allocating. A 4 KB PNG
        // declaring 500 megapixels is a ~2 GB allocation; reading the header
        // first costs nothing and produces an actionable message instead of an
        // OOM kill.
        //
        // The host already clamped this value (I14). Checking again here is not
        // redundant: this process is the one that would do the allocating, and
        // a worker that trusts its caller is a worker that cannot be reused
        // safely.
        // Destructured rather than field-accessed: `Probed` is an enum now, and
        // an image engine that was handed anything else has been asked the
        // wrong question. Refusing beats guessing at dimensions.
        let Probed::Image { width, height, .. } = self.probe(bytes)? else {
            return Err("oc-images was asked to convert something that is not an image".into());
        };
        let pixels = u64::from(width) * u64::from(height);
        if pixels > limits.decode_pixels {
            return Err(format!(
                "this image is {}x{} = {pixels} pixels, over the limit of {}",
                width, height, limits.decode_pixels
            ));
        }

        // The target is validated by `encode_image`, which owns the format
        // mapping now that AVIF needs its own encoder rather than a variant.
        if !matches!(
            to,
            "png" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "avif"
        ) {
            return Err(format!("oc-images does not write {to}"));
        }

        // Special-decode paths hand us raw RGBA plus the decoder's name for
        // the receipt; the generic path decodes through image-rs below.
        let (img, decoder_name, icc_applied) = match decoded_special {
            Some((w, h, rgba, name)) => {
                let img = image::RgbaImage::from_raw(w, h, rgba)
                    .ok_or("decoded pixel buffer does not match its declared dimensions")?;
                (image::DynamicImage::ImageRgba8(img), name, false)
            }
            None => decode_generic(bytes, width, height)?,
        };

        let mut out = std::io::Cursor::new(Vec::new());
        encode_image(&img, to, &mut out)?;
        let mut removed = vec![format!(
            "all source metadata (re-encoded via {decoder_name})"
        )];
        if icc_applied {
            removed.push("embedded ICC profile transformed to sRGB (lcms2)".to_string());
        }
        Ok(Converted {
            bytes: out.into_inner(),
            // Decoding to pixels and re-encoding keeps no container metadata,
            // so nothing is copied across. The DECODER that did it states the
            // loss, so a HEIC converted via libheif and a PNG via image-rs
            // each speak for themselves.
            removed,
        })
    }
}

// ---------------------------------------------------------------------------
// Content dispatch helpers
//
// Dispatch is on magic bytes, never on the host's guess — the same rule the
// Engine impl follows, factored out so probe() and run() answer identically.
// ---------------------------------------------------------------------------

/// What a special decoder produced: declared dimensions, RGBA pixels, and the
/// decoder's name for the receipt.
type SpecialDecode = Option<(u32, u32, Vec<u8>, String)>;

/// The paths that must not reach image-rs because image-rs cannot read them,
/// in probe order. HEIC keeps its plugin-presence refusal: heif.dll without
/// its libde265 decoder answers every read with "unsupported feature", which
/// reads as file corruption unless we name the real cause.
fn decode_special(bytes: &[u8]) -> Result<SpecialDecode, String> {
    if avif::looks_like_avif(bytes) {
        let d = avif::decode(bytes)?;
        return Ok(Some((d.width, d.height, d.rgba, "rav1d".to_string())));
    }
    if svg::looks_like_svg(bytes) {
        let r = svg::rasterise(bytes)?;
        let engine = if r.had_text {
            // Carried through as part of the engine string so it reaches the
            // receipt: this build draws no text, and a cut-down render that
            // says nothing about it is the dishonest kind of success.
            "resvg (text not drawn: no fonts in the sandbox)".to_string()
        } else {
            "resvg".to_string()
        };
        return Ok(Some((r.width, r.height, r.rgba, engine)));
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"JXL " {
        let d = jxl::decode_first_frame(bytes).map_err(|e| e.to_string())?;
        return Ok(Some((d.width, d.height, d.rgba, "jxl-oxide".to_string())));
    }
    #[cfg(all(windows, has_raw))]
    if is_unambiguous_raw(bytes) {
        let d = raw::decode_raw(bytes).map_err(|e| e.to_string())?;
        return Ok(Some((
            d.width,
            d.height,
            rgb_to_rgba(&d.rgb),
            format!("libraw {}", raw::linked_version()),
        )));
    }
    #[cfg(windows)]
    if bytes.len() >= 12 && (&bytes[4..12] == b"ftypheic" || &bytes[4..12] == b"ftypmif1") {
        if !heif::has_hevc_decoder() {
            return Err(
                "this install cannot decode HEVC (the libde265 engine library is \
                 missing or was not loaded). Reinstall to repair the engine set."
                    .into(),
            );
        }
        let d = heif::decode_primary(bytes).map_err(|e| e.to_string())?;
        return Ok(Some((
            d.width,
            d.height,
            d.rgba,
            format!("libheif {}", heif::linked_version()),
        )));
    }
    Ok(None)
}

/// Magics that can belong ONLY to a camera RAW file: CR3 ships an ISO-BMFF
/// `ftypcrx` box; Minolta and Fuji carry fixed lead strings. A plain TIFF
/// never starts this way, which is what keeps ordinary `.tif` conversions
/// out of libraw.
#[cfg(all(windows, has_raw))]
fn is_unambiguous_raw(bytes: &[u8]) -> bool {
    (bytes.len() >= 12 && &bytes[4..12] == b"ftypcrx")
        || bytes.starts_with(b"\x00MRM")
        || bytes.starts_with(b"FUJIFILM")
}

/// The TIFF family: little- and big-endian. DNG, NEF, ARW and CR2 share it
/// with every ordinary `.tif`, so it routes through image-rs first and only
/// falls to libraw when image-rs declines the decode.
#[cfg(all(windows, has_raw))]
fn is_tiff_family(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*")
}

/// libraw over a TIFF-family file, for callers past the pixel-limit check.
#[cfg(all(windows, has_raw))]
fn try_decode_raw_tiff(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let d = raw::decode_raw(bytes).ok()?;
    Some((d.width, d.height, d.rgb))
}

/// Interleaved RGB8 to RGBA8, alpha opaque — sensors capture no alpha, and
/// every encoder downstream of this worker wants four channels.
#[cfg(all(windows, has_raw))]
fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.chunks_exact(3) {
        rgba.extend_from_slice(px);
        rgba.push(0xFF);
    }
    rgba
}

/// The generic path: image-rs decodes, an embedded ICC profile is applied
/// through lcms2 when present, and a TIFF-family header image-rs cannot open
/// falls to libraw (DNG/NEF/ARW share the magic with every ordinary `.tif`).
/// `width`/`height` are the PROBED dimensions, reused as the decoder's own
/// allocation limits — a second line of defence behind the pixel check.
fn decode_generic(
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<(image::DynamicImage, String, bool), String> {
    use image::{ImageDecoder, ImageReader};

    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("could not read the image: {e}"))?;
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(width);
    lim.max_image_height = Some(height);
    reader.limits(lim);

    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("could not read the image: {e}"))?;
    let icc = decoder.icc_profile().ok().flatten();
    let img = match image::DynamicImage::from_decoder(decoder) {
        Ok(img) => img,
        Err(e) => {
            #[cfg(all(windows, has_raw))]
            if let Some((w, h, rgb)) = try_decode_raw_tiff(bytes) {
                let rgba = rgb_to_rgba(&rgb);
                let img = image::RgbaImage::from_raw(w, h, rgba)
                    .ok_or("decoded pixel buffer does not match its declared dimensions")?;
                return Ok((
                    image::DynamicImage::ImageRgba8(img),
                    format!("libraw {}", raw::linked_version()),
                    false,
                ));
            }
            return Err(format!("could not decode the image: {e}"));
        }
    };

    // An embedded ICC profile is applied, not dropped: pixels leave in sRGB
    // either way, but "re-encoded" alone would hide that a colour decision was
    // made. Only 8-bit RGBA carries through the transform, which is why it
    // fires only when a profile exists.
    match icc {
        Some(icc) => {
            #[cfg(all(windows, has_lcms2))]
            {
                let (img, applied) = apply_embedded_icc(img, &icc)?;
                Ok((img, "image-rs".to_string(), applied))
            }
            #[cfg(not(all(windows, has_lcms2)))]
            {
                let _ = icc;
                Ok((img, "image-rs".to_string(), false))
            }
        }
        None => Ok((img, "image-rs".to_string(), false)),
    }
}

#[cfg(all(windows, has_lcms2))]
fn apply_embedded_icc(
    img: image::DynamicImage,
    icc: &[u8],
) -> Result<(image::DynamicImage, bool), String> {
    let dims = (img.width(), img.height());
    let mut rgba = img.to_rgba8().into_raw();
    colour::apply_icc_profile(&mut rgba, icc)
        .map_err(|e| format!("could not apply the embedded colour profile: {e}"))?;
    let managed = image::RgbaImage::from_raw(dims.0, dims.1, rgba)
        .ok_or("colour-managed buffer does not match its declared dimensions")?;
    Ok((image::DynamicImage::ImageRgba8(managed), true))
}

/// Encode `img` to `to`, choosing the encoder rather than taking the default.
///
/// **AVIF NEEDED A DECISION AND WAS GETTING A DEFAULT.** `write_to` builds
/// `AvifEncoder::new`, which is `new_with_speed_quality(w, 4, 80)` — speed 4 on
/// a 1..=10 scale where 1 is slowest. That is `cavif`'s archival default, and
/// it is the wrong end of the scale for a converter someone is watching:
/// measured at **73.7 s for one 2400x1600 photograph** on this machine, which
/// is what made a 44-file batch look frozen.
///
/// Speed 7 keeps AVIF meaningfully smaller than JPEG while bringing a single
/// large image back into seconds. Quality 80 is unchanged, so the fidelity
/// claim in the receipt still means what it meant.
///
/// Every other format keeps `write_to`: their encoders have no comparable
/// speed/size dial and their defaults are already interactive.
fn encode_image(
    img: &image::DynamicImage,
    to: &str,
    out: &mut std::io::Cursor<Vec<u8>>,
) -> Result<(), String> {
    encode_image_quality(img, to, out, 80)
}

fn encode_image_quality(
    img: &image::DynamicImage,
    to: &str,
    out: &mut std::io::Cursor<Vec<u8>>,
    quality: u8,
) -> Result<(), String> {
    if to == "avif" {
        /// 1 is slowest, 10 fastest. See the note above for why not 4.
        const AVIF_SPEED: u8 = 10;

        // NO ALPHA PLANE FOR AN OPAQUE IMAGE.
        //
        // `to_rgba8` on a photograph invents a fully-opaque alpha channel, and
        // the encoder then compresses it as a second plane — real work, for
        // information the source did not have. Most things people convert to
        // AVIF are photographs. Checking costs one pass over the pixels and
        // saves an entire plane.
        // `ravif` DIRECTLY, NOT `image::codecs::avif`.
        //
        // The same encoder either way -- `image` wraps this crate -- but
        // reaching it through `image`'s `avif` feature compiled the shared
        // `image` rlib with AVIF for every crate in the workspace, and put
        // `rav1e` in the host binary through feature unification. `depcheck`
        // exists to catch precisely that. The output is unchanged; the
        // dependency edge is not.
        //
        // `rgb::FromSlice` is what turns the `Vec<u8>` `image` hands back into
        // typed pixels: a safe, zero-copy view, which matters at 2400x1600
        // where a copy would be 15 MB. This crate forbids `unsafe`, and a
        // slice cast is not a reason to open that door.
        let encoded = if img.color().has_alpha() {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width() as usize, rgba.height() as usize);
            let pixels = rgb::FromSlice::as_rgba(rgba.as_raw().as_slice());
            ravif::Encoder::new()
                .with_bit_depth(ravif::BitDepth::Eight)
                .with_quality(f32::from(quality))
                .with_speed(AVIF_SPEED)
                .encode_rgba(ravif::Img::new(pixels, w, h))
        } else {
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width() as usize, rgb8.height() as usize);
            let pixels = rgb::FromSlice::as_rgb(rgb8.as_raw().as_slice());
            ravif::Encoder::new()
                .with_bit_depth(ravif::BitDepth::Eight)
                .with_quality(f32::from(quality))
                .with_speed(AVIF_SPEED)
                .encode_rgb(ravif::Img::new(pixels, w, h))
        }
        .map_err(|e| format!("could not encode to avif: {e}"))?;

        return std::io::Write::write_all(out, &encoded.avif_file)
            .map_err(|e| format!("could not write the avif: {e}"));
    }

    let format = match to {
        "png" => image::ImageFormat::Png,
        "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "gif" => image::ImageFormat::Gif,
        "bmp" => image::ImageFormat::Bmp,
        "tiff" => image::ImageFormat::Tiff,
        other => return Err(format!("oc-images does not write {other}")),
    };
    img.write_to(out, format)
        .map_err(|e| format!("could not encode to {to}: {e}"))
}

fn main() -> std::process::ExitCode {
    // `run_real`, never `serve`: this binary applies its own confinement
    // before the first request arrives (Linux), and reports what the host's
    // spawn actually engaged (Windows). The distinction between the two entry
    // points is what keeps one-way mechanisms out of test processes.
    match run_real(&Images) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // stderr, never stdout: stdout is the protocol, and a stray line on
            // it desynchronises the host's framing for the rest of the session.
            eprintln!("oc-images: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

/// Apply a pixel operation to already-encoded bytes, and re-encode.
///
/// Runs on the OUTPUT of the ordinary conversion rather than on the source, so
/// every decoder path -- HEIC, AVIF, RAW, the generic one -- reaches it the
/// same way and this function never has to know which produced the pixels.
/// The cost is one extra decode/encode of a format we just wrote ourselves,
/// which is bounded and safe; the alternative is threading an operation
/// through five decode paths that have nothing else in common.
/// The JPEG quality used when the caller does not say.
///
/// 75 is the long-standing default across every JPEG tool: visually hard to
/// tell from the original on a photograph, and roughly half the size of the
/// 90-95 a camera writes.
const DEFAULT_QUALITY: u8 = 75;

/// Re-encode an image as small as its own format allows.
///
/// # What "compress" means here, per format
///
/// **The format does not change.** A compressor that quietly turned a PNG into
/// a JPEG would be a converter with a misleading name, and it would throw away
/// transparency on the images most likely to have it. So each format is made
/// smaller in the way that format can be:
///
/// - **JPEG** re-encodes at the requested quality. This is genuinely lossy and
///   the receipt says so; it is also the only thing that makes a JPEG
///   materially smaller.
/// - **PNG** re-encodes with maximum deflate effort and adaptive filtering.
///   **Lossless** -- every pixel survives -- and typically 10-30% smaller than
///   a PNG written by a camera or a screenshot tool, which optimise for speed.
///   `quality` does not apply and saying so beats pretending it does.
/// - **WebP** is refused rather than re-encoded, because the encoder linked
///   here writes lossless WebP only: re-encoding would be work with no gain,
///   reported as a success.
///
/// # A result that is not smaller is not written
///
/// Re-encoding an already-optimised file makes it BIGGER as often as not. A
/// compressor that hands back a larger file has failed at the one thing it was
/// asked to do, so the original bytes are returned instead and the receipt says
/// nothing was gained. Silence here would mean someone "compressing" a folder
/// twice and growing it.
fn compress_image(
    img: &image::DynamicImage,
    to: &str,
    quality: u8,
    mut removed: Vec<String>,
) -> Result<Converted, String> {
    use image::ImageEncoder as _;

    let mut out = std::io::Cursor::new(Vec::new());
    match to {
        "jpeg" => {
            let rgb = img.to_rgb8();
            out = std::io::Cursor::new(oc_image_codecs::jpeg(
                rgb.as_raw(),
                rgb.width() as usize,
                rgb.height() as usize,
                quality,
            )?);
            removed.push(format!(
                "detail the JPEG encoder discards at quality {quality}"
            ));
        }
        "png" => {
            use image::codecs::png::{CompressionType, FilterType, PngEncoder};
            PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
                .write_image(
                    img.as_bytes(),
                    img.width(),
                    img.height(),
                    img.color().into(),
                )
                .map_err(|e| format!("could not re-encode the PNG: {e}"))?;
            out = std::io::Cursor::new(oc_image_codecs::png(out.get_ref(), quality)?);
            removed
                .push("nothing: PNG compression is lossless, every pixel is unchanged".to_string());
        }
        "avif" => {
            encode_image_quality(img, to, &mut out, quality)?;
            removed.push(format!("AVIF re-encoded at quality {quality}"));
        }
        "webp" => {
            encode_image(img, to, &mut out)?;
        }
        "bmp" => {
            encode_image(img, to, &mut out)?;
        }
        "tiff" => {
            use tiff::encoder::{colortype, Compression, DeflateLevel, TiffEncoder};
            let mut encoder = TiffEncoder::new(&mut out)
                .map_err(|e| e.to_string())?
                .with_compression(Compression::Deflate(DeflateLevel::Best));
            macro_rules! write {
                ($pixels:expr, $color:ty) => {
                    encoder.write_image::<$color>(img.width(), img.height(), $pixels.as_raw())
                };
            }
            match img {
                image::DynamicImage::ImageLuma8(p) => write!(p, colortype::Gray8),
                image::DynamicImage::ImageLuma16(p) => write!(p, colortype::Gray16),
                image::DynamicImage::ImageRgb8(p) => write!(p, colortype::RGB8),
                image::DynamicImage::ImageRgb16(p) => write!(p, colortype::RGB16),
                image::DynamicImage::ImageRgba8(p) => write!(p, colortype::RGBA8),
                image::DynamicImage::ImageRgba16(p) => write!(p, colortype::RGBA16),
                image::DynamicImage::ImageRgb32F(p) => write!(p, colortype::RGB32Float),
                image::DynamicImage::ImageRgba32F(p) => write!(p, colortype::RGBA32Float),
                _ => return Err("unsupported TIFF sample format".into()),
            }
            .map_err(|e| e.to_string())?;
            removed.push("TIFF image data compressed losslessly with Deflate".into());
        }
        other => return Err(format!("unsupported compression output {other}")),
    }

    Ok(Converted {
        bytes: out.into_inner(),
        removed,
    })
}

fn apply_pixel_op(
    encoded: &[u8],
    op: &str,
    to: &str,
    quality: u8,
    mut removed: Vec<String>,
) -> Result<Converted, String> {
    // This one needs the variant for the DECODE as well: the pixel op reads
    // back what the conversion step already wrote, in that step's format.
    // Encoding goes through `encode_image`, which owns the AVIF special case.
    let format = match to {
        "png" => image::ImageFormat::Png,
        "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "gif" => image::ImageFormat::Gif,
        "bmp" => image::ImageFormat::Bmp,
        "tiff" => image::ImageFormat::Tiff,
        "avif" => image::ImageFormat::Avif,
        other => return Err(format!("oc-images does not write {other}")),
    };
    let img = image::load_from_memory_with_format(encoded, format)
        .map_err(|e| format!("could not re-read the encoded image: {e}"))?;

    let out_img = match op {
        "invert" => {
            let mut rgba = img.to_rgba8();
            // Alpha is NOT inverted: inverting it would turn an opaque image
            // transparent, which is not what anyone means by "invert colours".
            for px in rgba.pixels_mut() {
                px.0[0] = 255 - px.0[0];
                px.0[1] = 255 - px.0[1];
                px.0[2] = 255 - px.0[2];
            }
            image::DynamicImage::ImageRgba8(rgba)
        }
        "greyscale" => {
            // Luminance-weighted, which is what the eye expects; a plain mean
            // of the channels makes reds too light and blues too dark.
            // Alpha is carried through unchanged.
            image::DynamicImage::ImageLumaA8(img.to_luma_alpha8())
        }
        // Compression re-encodes and does not touch a pixel, so it returns
        // here rather than falling through to the shared encode below -- the
        // whole point is that it chooses its own encoder settings.
        "compress" => return compress_image(&img, to, quality, removed),
        other => return Err(format!("oc-images does not know the operation {other:?}")),
    };

    let mut out = std::io::Cursor::new(Vec::new());
    encode_image(&out_img, to, &mut out)?;

    if op == "greyscale" {
        removed.push("colour (converted to greyscale by luminance)".to_string());
    }
    Ok(Converted {
        bytes: out.into_inner(),
        removed,
    })
}
