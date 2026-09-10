//! ONNX Runtime inference, and the adapters built on it.
//!
//! Compiled only when `build.rs` found the runtime (`has_ort`) on Windows.
//! Everything here runs inside the same AppContainer as the image workers,
//! with the weights arriving on the pipe rather than from disk.

use image::{DynamicImage, GenericImageView, ImageFormat, RgbImage, RgbaImage};
use ndarray::Array4;
use ort::session::Session;

use crate::accelerator::{self, Provider, Workload};
use openconvert_worker::{Converted, RunLimits};
use ort::value::Value;
use std::io::Cursor;

/// How much bigger, in words, from the numbers.
///
/// Whole factors read as words because that is how people say them; anything
/// else is stated as a ratio rather than rounded, since "twice its size" for a
/// 1.8x enlargement is the kind of small untruth a receipt exists to avoid.
fn describe_factor(before: u32, after: u32) -> String {
    if before == 0 {
        return "larger".to_string();
    }
    if after.is_multiple_of(before) {
        return match after / before {
            1 => "the same as".to_string(),
            2 => "twice".to_string(),
            3 => "three times".to_string(),
            4 => "four times".to_string(),
            n => format!("{n} times"),
        };
    }
    format!("{:.1} times", f64::from(after) / f64::from(before))
}

/// The runtime this worker was built against, mirroring the `engines.toml`
/// row. Kept as a constant rather than queried: the C API exposes no version
/// accessor through `ort`'s safe surface, and a receipt naming the wrong
/// runtime would be worse than one naming the declared one.
const ORT_VERSION: &str = "1.20.1";

/// The runtime's own version, for the receipt.
///
/// # Errors
///
/// When the runtime could not be loaded at all — which is the condition the
/// host-side availability flag exists to predict, so it should not be reached
/// in practice.
pub fn runtime_version() -> Result<String, String> {
    // Resolving the shared library is the cheapest thing that proves it is
    // actually there. `commit()` reports whether THIS call installed the
    // environment; either answer means the runtime loaded, and a failure to
    // load surfaces as a panic-free error from the first session instead.
    ort::init().commit();
    Ok(format!("onnxruntime {ORT_VERSION}"))
}

/// Build a session over weights held in memory.
///
/// Single-threaded on purpose. Each conversion already runs in its own worker
/// process under its own Job Object memory cap, so a model spawning its own
/// thread pool would multiply threads by files and escape the accounting the
/// pool does — the parallelism this product wants is across files, not inside
/// one.
pub(crate) fn session_from(model: &[u8]) -> Result<Session, String> {
    Ok(session_for(model, Workload::Light)?.0)
}

/// Build a session on the best provider for this workload, and say which one
/// it was.
///
/// Tries each of [`accelerator::candidates_for`] in order and takes the first
/// that works. The provider is **returned**, not logged and discarded.
///
/// # THIS FALLS BACK ON CREATION ONLY. For the other kind, use [`run_for`].
///
/// A DirectML session can fail two ways and this catches one of them. The
/// provider may be absent from the build — a CPU-only ORT, a machine with no
/// DirectML — and that failure happens *here*, while the session is being
/// built, so this loop sees it and moves to the next candidate.
///
/// The other way is that it registers, compiles the graph, and then runs out
/// of **video** memory — which is not the machine's memory and cannot be
/// raised in Settings. BiRefNet does exactly that on a 4 GB card. **That
/// failure happens at `run()`, after this function has returned**, so nothing
/// here can see it: the session was built without complaint and the caller
/// went on to fail on the GPU with a raw ONNX message.
///
/// The comment that stood here claimed this loop covered both. It never did.
/// [`run_for`] is what covers the second, and a caller whose run can exhaust
/// video memory should use that instead of this.
///
/// # Errors
///
/// Only when every provider fails to BUILD, which means the model itself will
/// not load — CPU is always a candidate and always available.
pub(crate) fn session_for(model: &[u8], workload: Workload) -> Result<(Session, Provider), String> {
    let mut last = String::new();
    for provider in accelerator::candidates_for(workload) {
        match session_on(model, provider) {
            Ok(session) => return Ok((session, provider)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Build a session and do the work on it, moving to the next provider if
/// either the building or the WORK fails.
///
/// # What this exists for
///
/// [`session_for`] falls back when a provider cannot be *created*. The failure
/// that actually reaches users is the other one: DirectML builds a session
/// happily and then exhausts video memory partway through `run()`. The best
/// matting tier does this on a 4 GB card, and upscaling does it on a large
/// image, because both feed the card far more than the graph did at build
/// time. Before this function, that surfaced as raw ONNX Runtime text and the
/// run simply failed — on a machine whose processor could have done it.
///
/// # Any failure on a GPU provider is retried, not just the ones that look
/// like memory
///
/// Sniffing the error string for "out of memory" was the alternative and it is
/// worse: the wording belongs to a vendor's driver and changes between
/// versions, so a mis-guess means the fallback silently stops working with
/// nothing to notice. Retrying on everything costs one wasted CPU attempt when
/// the input was genuinely bad — and that attempt is what turns a DirectML
/// diagnostic into the CPU provider's message, which is the one worth showing
/// anyway.
///
/// Nothing is retried once the run reaches the CPU: it is the last candidate,
/// and a second identical attempt would only double the wait before the same
/// error.
///
/// # `work` MUST return owned data
///
/// Whatever it extracts has to outlive the session it came from, because the
/// session is dropped before the next provider is tried. That is why the
/// callers pull their tensors out inside the closure rather than returning
/// `SessionOutputs`.
///
/// # Errors
///
/// The last provider's error, which is the CPU's whenever the CPU was reached.
pub(crate) fn run_for<T>(
    model: &[u8],
    workload: Workload,
    mut work: impl FnMut(&mut Session) -> Result<T, String>,
) -> Result<(T, Provider), String> {
    accelerator::first_success(
        &accelerator::candidates_for(workload),
        |provider| session_on(model, provider),
        &mut work,
    )
}

/// Build a session on one named provider, refusing rather than falling back.
///
/// # `error_on_failure` is the whole point
///
/// ORT's default is to append CPU behind any provider it could not register and
/// report success. For a service that must stay up, that is right. For a
/// receipt, it is a lie waiting to be written: the session would run on the CPU
/// and every layer above would go on believing a GPU was used. `.error_on_failure()`
/// turns that silence into a refusal this function can act on.
///
/// # Errors
///
/// When the provider is not available in this build or on this machine, or when
/// the bytes are not a model the runtime can load.
pub(crate) fn session_on(model: &[u8], provider: Provider) -> Result<Session, String> {
    let builder = Session::builder()
        .map_err(|e| format!("could not configure the inference session: {e}"))?;

    let builder = match provider {
        Provider::Cpu => builder,
        // NO `cfg` HERE, AND IT NEEDS NONE. This module compiles only under
        // `all(windows, has_ort)`, and `Cargo.toml` gives `ort` the `directml`
        // feature on every Windows target -- so on any build that reaches this
        // line, the provider exists. A `#[cfg(feature = "directml")]` guard
        // was worse than useless: the feature it names is this crate's, which
        // the target section does not set, so the arm compiled out on exactly
        // the builds that could use it.
        Provider::DirectML => builder
            .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
            .map_err(|e| format!("DirectML is not usable here: {e}"))?,
    };

    // SINGLE-THREADED ON THE CPU, AND THE GPU DOES NOT WANT THIS.
    //
    // Each conversion already runs in its own worker under its own Job Object
    // memory cap, so a CPU model spawning its own thread pool would multiply
    // threads by files and escape that accounting -- the parallelism this
    // product wants is across files, not inside one. A GPU session's intra-op
    // threads schedule work onto the device rather than compute it, so pinning
    // them to one would serialise submission for no benefit to the accounting.
    let mut builder = if provider == Provider::Cpu {
        builder
            .with_intra_threads(1)
            .map_err(|e| format!("could not configure the inference session: {e}"))?
    } else {
        builder
    };

    builder.commit_from_memory(model).map_err(|e| {
        format!(
            "these {} bytes are not a model this runtime can load: {e}",
            model.len()
        )
    })
}

/// Decode the input image under the pixel budget.
///
/// Header first, limit second, decode third — the same order every other image
/// path in this project uses, and for the same reason: a small file claiming a
/// huge canvas must cost nothing to refuse.
fn decode_bounded(bytes: &[u8], limits: &RunLimits) -> Result<DynamicImage, String> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("could not read the image header: {e}"))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("could not read the image dimensions: {e}"))?;
    if w == 0 || h == 0 {
        return Err("the image declares zero dimensions".into());
    }
    let pixels = u64::from(w) * u64::from(h);
    if pixels > limits.decode_pixels {
        return Err(format!(
            "this image is {w}x{h} = {pixels} pixels, over the limit of {}",
            limits.decode_pixels
        ));
    }
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("could not read the image: {e}"))?
        .decode()
        .map_err(|e| format!("could not decode the image: {e}"))
}

/// u2netp's fixed input side. The network is fully convolutional but was
/// trained at this size, and feeding it anything else degrades the mask
/// visibly rather than failing loudly.
const U2NET_SIDE: u32 = 320;

/// MODNet's reference size, applied to the SHORT side.
const MODNET_SIDE: u32 = 512;

/// The most either side may reach, so a panorama cannot ask for a tensor the
/// size of the machine's memory once the short side is held at 512.
const MODNET_MAX: u32 = 1024;

/// MODNet's stride. Its encoder downsamples five times, so a dimension that is
/// not a multiple of 32 is padded internally and the matte comes back
/// misaligned against the image it was computed from.
const MODNET_STRIDE: u32 = 32;

/// The input size for a MODNet run, in pixels.
///
/// **The aspect ratio is preserved, and that is the point.**
///
/// MODNet's ONNX declares `[-1, 3, -1, -1]`: every spatial dimension is a
/// symbol, so nothing about a square is the model's requirement -- it was
/// ours. Feeding a 4:3 photograph squashed into 512x512 asks a portrait
/// matting network to find a person who is a third wider than any person it
/// was trained on, and it answers with a torn, displaced blob. On a red disc
/// on a grey field -- about as easy as segmentation gets -- the square input
/// produced a matte shifted a quarter of the frame upward with a hole through
/// it, while u2netp, which really is fixed at 320x320 and really does get a
/// squashed input, returned the disc exactly.
///
/// So: hold the SHORT side at the reference size, keep the ratio, round both
/// to the stride, and cap the long side. The mask comes back at these
/// proportions and is resized to the source from there.
fn modnet_input_size(src_w: u32, src_h: u32) -> (u32, u32) {
    let (w, h) = (src_w.max(1) as f64, src_h.max(1) as f64);
    let scale = f64::from(MODNET_SIDE) / w.min(h);
    // Cap on the long side first, so the cap wins when the two disagree.
    let scale = scale.min(f64::from(MODNET_MAX) / w.max(h));
    let round = |v: f64| -> u32 {
        let n = (v * scale / f64::from(MODNET_STRIDE)).round() as u32;
        n.max(1) * MODNET_STRIDE
    };
    (round(w), round(h))
}

/// Plain [-1, 1] normalisation, CHW — what MODNet expects.
fn normalise_signed(img: &RgbImage) -> Array4<f32> {
    let (w, h) = img.dimensions();
    let mut a = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for (x, y, px) in img.enumerate_pixels() {
        for c in 0..3 {
            a[[0, c, y as usize, x as usize]] = (f32::from(px.0[c]) / 255.0 - 0.5) / 0.5;
        }
    }
    a
}

/// Remove an image's background, returning RGBA PNG.
///
/// # The pipeline, and why each step is where it is
///
/// 1. **Decode under the pixel budget**, before any allocation.
/// 2. **Resize to 320x320** — u2netp's training size.
/// 3. **Normalise.** u2net divides by the image maximum and then applies
///    ImageNet mean/std. Skipping the max-division is the single most common
///    way to get a mask that looks *nearly* right and is systematically wrong
///    on dark images.
/// 4. **Run**, taking output `d0`, the fused mask.
/// 5. **Min-max normalise the mask**, which u2net's own inference script does
///    and without which the alpha is washed out.
/// 6. **Resize the mask back** to the source dimensions and apply it as alpha.
///
/// The output is always PNG: the mask is meaningless without an alpha channel,
/// and JPEG has none. Asking for JPEG is refused rather than silently
/// composited onto a colour nobody chose.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn remove_background(
    bytes: &[u8],
    model: &[u8],
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    remove_background_with(bytes, model, to, limits, Matte::Saliency, Workload::Light)
}

/// Which segmenter produced the mask, and therefore how to finish it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Matte {
    /// u2netp: a saliency map with no fixed range, which must be stretched.
    Saliency,
    /// MODNet: a finished alpha already in [0,1]. Stretching it would push
    /// soft hair edges to fully opaque or fully transparent — the exact
    /// detail the heavier model exists to preserve.
    Alpha,
}

/// Higher-quality matting through MODNet.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn remove_background_quality(
    bytes: &[u8],
    model: &[u8],
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    remove_background_with(bytes, model, to, limits, Matte::Alpha, Workload::Heavy)
}

/// A matte with less than this fraction of the frame opaque is treated as a
/// failure rather than an answer.
///
/// Not zero, because a genuinely tiny subject exists -- a bird in a wide sky is
/// a real photograph and a real thing to cut out. Half a percent of the frame
/// is roughly a 45x45 region of a 12 megapixel image, which is smaller than
/// anything anyone reaches for this tool to extract, and comfortably above the
/// speckle a failed matte leaves behind.
const MATTE_FLOOR: f32 = 0.005;

/// The alpha threshold a pixel must clear to count as part of the subject.
const MATTE_OPAQUE: u8 = 128;

/// Background removal with BiRefNet, falling back the same way the quality
/// tier does.
///
/// # Why it is a third adapter and not a parameter on the other two
///
/// BiRefNet is a different shape of model: fixed 1024x1024 in, one 1024x1024
/// map out, ImageNet normalisation, and an output that may be logits rather
/// than probabilities. u2netp is 320 and saliency-stretched; MODNet is
/// aspect-preserving and already an alpha. Three pre/post pipelines behind one
/// function with a mode flag is how the MODNet aspect-ratio bug survived --
/// the square resize was correct for one branch and silently wrong for the
/// other.
///
/// # Errors
///
/// The model's own error. A degenerate matte is not an error: it falls back,
/// and says so.
pub fn remove_background_birefnet(
    bytes: &[u8],
    model: &[u8],
    fallback: &[u8],
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    if !matches!(to, "png" | "webp") {
        return Err(format!(
            "background removal produces transparency, and {to} has no alpha channel; \
             ask for png"
        ));
    }
    let probe = probe_matting_model(model, bytes, limits)?;
    if probe.opaque + probe.partial > MATTE_FLOOR {
        return Ok(Converted {
            bytes: probe.cut_out,
            removed: vec![
                "the background, replaced with transparency".to_string(),
                "all tags and metadata (not carried across a re-encode)".to_string(),
            ],
        });
    }
    let mut plain = remove_background_with(
        bytes,
        fallback,
        to,
        limits,
        Matte::Saliency,
        Workload::Light,
    )?;
    plain.removed.push(
        "the highest-quality model found no subject in this image, so the standard model \
         produced the cut-out"
            .to_string(),
    );
    Ok(plain)
}

/// What a candidate matting model declared, and what it actually produced.
pub struct MattingProbe {
    /// The input the graph declares, as text.
    pub input_shape: String,
    /// The output the graph declares, as text.
    pub output_shape: String,
    /// Fraction of the matte at least 200/255.
    pub opaque: f32,
    /// Fraction at most 55/255.
    pub transparent: f32,
    /// Everything between: the soft edge, which is the point of a matte.
    pub partial: f32,
    /// The cut-out, as a PNG, so it can be looked at.
    pub cut_out: Vec<u8>,
}

/// Load an unfamiliar matting model, run one image through it, and report.
///
/// **For deciding whether a model row should exist at all.** A registry row is
/// a promise that the artifact works; this is how that promise gets checked
/// before it is written, rather than after someone downloads 224 MB.
///
/// It assumes only the shape every one of these models shares: an image in as
/// NCHW float, a single-channel matte out. The input side is taken from the
/// graph when it fixes one and defaults to 1024 -- BiRefNet's training size --
/// when it does not.
///
/// # Errors
///
/// Anything that stops it loading or running, reported as text.
pub fn probe_matting_model(
    model: &[u8],
    image_bytes: &[u8],
    limits: &RunLimits,
) -> Result<MattingProbe, String> {
    let source = decode_bounded(image_bytes, limits)?;
    let (src_w, src_h) = source.dimensions();

    // READS THE GRAPH, NEVER RUNS IT. This reports the declared input and
    // output shapes so the caller can size its tiles; nothing is fed to the
    // card, so nothing can exhaust it.
    let (mut session, _provider) = session_for(model, Workload::Heavy)?;
    let describe = |dtype: &ort::value::ValueType| -> String {
        match dtype {
            ort::value::ValueType::Tensor { shape, .. } => {
                format!("{:?}", shape.iter().copied().collect::<Vec<i64>>())
            }
            other => format!("{other:?}"),
        }
    };
    let (input_shape, output_shape, side) = {
        let inputs = session.inputs();
        let outputs = session.outputs();
        let input_shape = inputs
            .first()
            .map_or_else(|| "none".into(), |o| describe(o.dtype()));
        let output_shape = outputs
            .first()
            .map_or_else(|| "none".into(), |o| describe(o.dtype()));
        // A fixed square input tells us the size; otherwise BiRefNet's own.
        let side = match inputs.first().map(|o| o.dtype()) {
            Some(ort::value::ValueType::Tensor { shape, .. }) => {
                let dims: Vec<i64> = shape.iter().copied().collect();
                match dims.as_slice() {
                    [.., h, w] if *h > 0 && h == w => *h as u32,
                    _ => 1024,
                }
            }
            _ => 1024,
        };
        (input_shape, output_shape, side)
    };

    // ImageNet normalisation, which is what the BiRefNet family was trained
    // with. A model fed the wrong normalisation still runs and still returns a
    // matte -- a worse one -- so this is worth getting right even in a probe.
    let small = source
        .resize_exact(side, side, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let input = normalise(&small);

    let (shape, data) = run_one(&mut session, input)?;
    let expect = (side * side) as usize;
    if data.len() < expect {
        return Err(format!(
            "the model returned {} values with shape {shape:?}, fewer than the {side}x{side} \
             matte expected",
            data.len()
        ));
    }

    // The last `side*side` values: several BiRefNet exports emit a stack of
    // supervision maps and the FINAL one is the refined matte.
    let flat = &data[data.len() - expect..];
    let alpha: Vec<u8> = flat
        .iter()
        .map(|&v| (sigmoid_if_needed(v).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();

    let mask_img = RgbImage::from_fn(side, side, |x, y| {
        let v = alpha[(y * side + x) as usize];
        image::Rgb([v, v, v])
    });
    let mask_full = DynamicImage::ImageRgb8(mask_img)
        .resize_exact(src_w, src_h, image::imageops::FilterType::Lanczos3)
        .to_luma8();

    let rgba = source.to_rgba8();
    let cut = RgbaImage::from_fn(src_w, src_h, |x, y| {
        let p = rgba.get_pixel(x, y).0;
        image::Rgba([p[0], p[1], p[2], mask_full.get_pixel(x, y).0[0]])
    });

    let total = alpha.len() as f32;
    let opaque = alpha.iter().filter(|&&v| v >= 200).count() as f32 / total;
    let transparent = alpha.iter().filter(|&&v| v <= 55).count() as f32 / total;

    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(cut)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| format!("could not encode the cut-out: {e}"))?;

    Ok(MattingProbe {
        input_shape,
        output_shape,
        opaque,
        transparent,
        partial: 1.0 - opaque - transparent,
        cut_out: out.into_inner(),
    })
}

/// Some exports end in a sigmoid and some do not.
///
/// A value already in [0,1] is a probability; anything outside it is a logit
/// and needs squashing. Guessing wrong turns a good matte into a flat one, and
/// the range is the only signal available without the training code.
fn sigmoid_if_needed(v: f32) -> f32 {
    if (0.0..=1.0).contains(&v) {
        v
    } else {
        1.0 / (1.0 + (-v).exp())
    }
}

/// Background removal at the quality tier, with the standard model as a net.
///
/// **Why this exists.** MODNet is a portrait matting network and is the better
/// model on a person, which is why it is the default whenever it is installed.
/// It is not a general segmenter, and on a subject it was not trained for it
/// can return a matte with essentially nothing in it. That reaches the user as
/// a PNG which opens as an empty transparent rectangle -- a silent, total loss
/// of the thing they asked for, and the reported symptom.
///
/// Falling back is strictly better than either alternative. Refusing would
/// leave a working conversion unavailable because a better model declined.
/// Shipping the empty matte would be shipping nothing and calling it a result.
///
/// The receipt names the model that produced the alpha, and says so
/// explicitly when the fallback ran, so the choice is never invisible.
///
/// # Errors
///
/// The quality model's error, if it fails outright -- a fallback covers an
/// empty ANSWER, not a broken run, and hiding a real failure behind a second
/// model would hide the reason the first one needs fixing.
pub fn remove_background_best(
    bytes: &[u8],
    quality: &[u8],
    fallback: &[u8],
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    let best = remove_background_with(bytes, quality, to, limits, Matte::Alpha, Workload::Heavy)?;
    if matte_coverage(&best.bytes) > MATTE_FLOOR {
        return Ok(best);
    }
    let mut plain = remove_background_with(
        bytes,
        fallback,
        to,
        limits,
        Matte::Saliency,
        Workload::Light,
    )?;
    plain.removed.push(
        "the higher-quality matting model found no subject in this image, so the standard \
         model produced the cut-out"
            .to_string(),
    );
    Ok(plain)
}

/// The fraction of an encoded RGBA image that is opaque enough to be subject.
///
/// Zero when the bytes will not decode: a result nobody can read is not a
/// result to prefer.
fn matte_coverage(encoded: &[u8]) -> f32 {
    let Ok(img) = image::load_from_memory(encoded) else {
        return 0.0;
    };
    let rgba = img.to_rgba8();
    let total = rgba.width() as usize * rgba.height() as usize;
    if total == 0 {
        return 0.0;
    }
    let opaque = rgba.pixels().filter(|p| p.0[3] >= MATTE_OPAQUE).count();
    opaque as f32 / total as f32
}

fn remove_background_with(
    bytes: &[u8],
    model: &[u8],
    to: &str,
    limits: &RunLimits,
    kind: Matte,
    workload: Workload,
) -> Result<Converted, String> {
    if !matches!(to, "png" | "webp") {
        return Err(format!(
            "background removal produces transparency, and {to} has no alpha channel; \
             ask for png"
        ));
    }
    let source = decode_bounded(bytes, limits)?;
    let (src_w, src_h) = source.dimensions();

    // ---- pre-process ----
    //
    // MODNet is fully convolutional and trained around 512; u2netp is fixed at
    // 320. Feeding either the other's size degrades the matte silently, which
    // is why the size travels with the model choice rather than being a
    // constant.
    let (in_w, in_h) = match kind {
        // u2netp's graph is FIXED at 320x320 -- its ONNX declares those
        // dimensions, not symbols -- so the square is the model's requirement
        // and not a choice. Distortion is undone when the mask is resized back.
        Matte::Saliency => (U2NET_SIDE, U2NET_SIDE),
        Matte::Alpha => modnet_input_size(src_w, src_h),
    };
    let small = source
        .resize_exact(in_w, in_h, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let input = match kind {
        Matte::Saliency => normalise(&small),
        Matte::Alpha => normalise_signed(&small),
    };

    // ---- run ----
    //
    // THROUGH `run_for`, WHICH FALLS BACK WHEN THE RUN FAILS, not only when
    // the session cannot be built. This is the case the fallback was written
    // for and did not cover: the best tier is BiRefNet, and BiRefNet exhausts
    // the video memory of a 4 GB card partway through `run()` -- long after
    // `session_for` has returned a session it was perfectly able to build.
    // What a user saw was raw ONNX text, on a machine whose processor could
    // have finished the job in a minute.
    //
    // Everything the mask is made of is extracted INSIDE the closure. The
    // session is dropped before the next provider is tried, so anything still
    // borrowing it could not outlive the attempt.
    let ((mask, mask_w, mask_h), _provider) = run_for(model, workload, move |session| {
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or("this model declares no inputs")?;
        let tensor = Value::from_array(input.clone())
            .map_err(|e| format!("could not build the input tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs![input_name => tensor])
            .map_err(|e| format!("inference failed: {e}"))?;

        // u2net emits seven side outputs; d0 (the first) is the fused one every
        // reference implementation uses. Taking the last would take the coarsest.
        let (_, mask) = outputs
            .iter()
            .next()
            .ok_or("the model produced no output")?;
        let mask = mask
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("the model's output was not the float mask expected: {e}"))?;
        // THE MASK'S SIZE COMES FROM THE MASK, not from what we asked for.
        //
        // This assumed the output was `side x side` and sliced the first
        // `side * side` values out of it. That is true of u2netp, whose graph is
        // fixed; MODNet's is fully dynamic (`[-1, 1, -1, -1]`), so the output
        // matches whatever it was fed -- and once the input stopped being square,
        // reading a square out of it would tear the mask into diagonal bands.
        // Reading the declared shape costs nothing and cannot be wrong.
        let dims: Vec<i64> = mask.0.iter().copied().collect();
        let [.., h, w] = dims.as_slice() else {
            return Err(format!(
                "the model's mask has shape {dims:?}, which has no width or height"
            ));
        };
        let (mask_h, mask_w) = (*h as u32, *w as u32);
        let expected = mask_w as usize * mask_h as usize;
        let mask = mask.1;
        if mask_w == 0 || mask_h == 0 || mask.len() < expected {
            return Err(format!(
                "the model returned {} values for a {mask_w}x{mask_h} mask",
                mask.len()
            ));
        }
        Ok((mask[..expected].to_vec(), mask_w, mask_h))
    })?;

    // ---- post-process ----
    let flat = &mask[..];
    let alpha = match kind {
        Matte::Saliency => min_max_normalise(flat),
        // Already an alpha: clamp and scale, do not stretch.
        Matte::Alpha => flat
            .iter()
            .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect(),
    };
    let mask_img = RgbImage::from_fn(mask_w, mask_h, |x, y| {
        let v = alpha[(y * mask_w + x) as usize];
        image::Rgb([v, v, v])
    });
    let mask_full = DynamicImage::ImageRgb8(mask_img).resize_exact(
        src_w,
        src_h,
        image::imageops::FilterType::Lanczos3,
    );
    let mask_full = mask_full.to_luma8();

    let rgba = source.to_rgba8();
    let cut = RgbaImage::from_fn(src_w, src_h, |x, y| {
        let p = rgba.get_pixel(x, y).0;
        image::Rgba([p[0], p[1], p[2], mask_full.get_pixel(x, y).0[0]])
    });

    let mut out = Cursor::new(Vec::new());
    let format = if to == "webp" {
        ImageFormat::WebP
    } else {
        ImageFormat::Png
    };
    DynamicImage::ImageRgba8(cut)
        .write_to(&mut out, format)
        .map_err(|e| format!("could not encode the result: {e}"))?;

    Ok(Converted {
        bytes: out.into_inner(),
        removed: vec![
            "the background, replaced with transparency".to_string(),
            "all tags and metadata (not carried across a re-encode)".to_string(),
        ],
    })
}

/// u2net's input normalisation: divide by the image maximum, then ImageNet
/// mean/std, in CHW order.
///
/// The max-division comes straight from u2net's `RescaleT`/`ToTensorLab` and is
/// easy to miss because leaving it out still produces a plausible-looking mask.
/// It is the difference between correct and nearly-correct.
fn normalise(img: &RgbImage) -> Array4<f32> {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];

    let max = img
        .pixels()
        .flat_map(|p| p.0.iter().copied())
        .max()
        .unwrap_or(255);
    // An all-black image has a maximum of zero; dividing by it would make
    // every channel NaN and the mask meaningless. 255 leaves the image black,
    // which is the honest answer for an image with nothing in it.
    let max = if max == 0 { 255.0 } else { f32::from(max) };

    let (w, h) = img.dimensions();
    let mut a = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for (x, y, px) in img.enumerate_pixels() {
        for c in 0..3 {
            let v = f32::from(px.0[c]) / max;
            a[[0, c, y as usize, x as usize]] = (v - MEAN[c]) / STD[c];
        }
    }
    a
}

/// Stretch a mask to the full 0-255 range.
///
/// u2net's own inference script does this before saving, and without it the
/// alpha is compressed into whatever range the network happened to emit —
/// typically a washed-out cut-out that looks like a bad model rather than a
/// missing post-processing step.
fn min_max_normalise(mask: &[f32]) -> Vec<u8> {
    let (lo, hi) = mask
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let span = hi - lo;
    // A flat mask carries no information to stretch; scaling by ~0 would turn
    // rounding noise into structure that is not there.
    if !span.is_finite() || span <= f32::EPSILON {
        return vec![0; mask.len()];
    }
    mask.iter()
        .map(|&v| (((v - lo) / span).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

/// Run a single-input, single-output model and return its first output's
/// shape and values.
///
/// Every adapter here has that shape, and each one writing its own
/// input-name lookup and output extraction is three chances to take the wrong
/// output tensor.
///
/// # Errors
///
/// A model with no declared inputs, a run failure, or an output that is not
/// the float tensor the caller expects.
pub(crate) fn run_one(
    session: &mut Session,
    input: ndarray::Array4<f32>,
) -> Result<(Vec<i64>, Vec<f32>), String> {
    let name = session
        .inputs()
        .first()
        .map(|i| i.name().to_string())
        .ok_or("this model declares no inputs")?;
    let tensor =
        Value::from_array(input).map_err(|e| format!("could not build the input tensor: {e}"))?;
    let outputs = session
        .run(ort::inputs![name => tensor])
        .map_err(|e| format!("inference failed: {e}"))?;
    let (_, value) = outputs
        .iter()
        .next()
        .ok_or("the model produced no output")?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("the model's output was not a float tensor: {e}"))?;
    Ok((shape.iter().copied().collect(), data.to_vec()))
}

/// Read an image's text, as UTF-8 plain text.
///
/// Takes three artifacts in the order the step names them: detection network,
/// recognition network, character dictionary.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn ocr_to_layout(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
) -> Result<Converted, String> {
    let [det, rec, dict] = models else {
        return Err("OCR needs its three model artifacts".into());
    };
    let image = decode_bounded(bytes, limits)?;
    let lines = crate::ocr::recognise_layout(&image, det, rec, dict)?;
    let bytes = serde_json::to_vec(
        &serde_json::json!({"width":image.width(),"height":image.height(),"lines":lines}),
    )
    .map_err(|e| e.to_string())?;
    Ok(Converted {
        bytes,
        removed: vec!["OCR text may contain recognition errors".into()],
    })
}

pub fn ocr_to_text(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
) -> Result<Converted, String> {
    let [det, rec, dict] = models else {
        return Err(format!(
            "OCR needs three artifacts (detection, recognition, dictionary); got {}",
            models.len()
        ));
    };
    let image = decode_bounded(bytes, limits)?;
    let text = crate::ocr::recognise(&image, det, rec, dict)?;
    Ok(Converted {
        bytes: text.into_bytes(),
        removed: vec![
            "every pixel: this is a transcription, not a copy of the image".to_string(),
            "layout, fonts, colour and any text the model did not find".to_string(),
        ],
    })
}

/// Decode any audio file this build understands to interleaved f32.
///
/// Returns `(samples, channels, sample_rate)`.
///
/// # Errors
///
/// An unreadable container or a codec this build was not compiled with.
pub fn decode_audio(bytes: &[u8], limits: &RunLimits) -> Result<(Vec<f32>, usize, u32), String> {
    decode_audio_inner(bytes, limits, None)
}

/// Decode straight to mono at `target` Hz, without ever holding the whole
/// interleaved signal.
///
/// **This is what long recordings needed.** The whole-file path collects every
/// sample as interleaved `f32` and hands it to `to_mono_16k`, which allocates
/// the mono copy on top: an hour of 48 kHz stereo is 1.38 GB before the 230 MB
/// the model uses, and `limits.memory_bytes` refused it — the failure reported
/// as "it failed many times on a long recording".
///
/// Resampling as the packets arrive keeps only the result. The arithmetic is
/// identical to the batch path and a test asserts the two agree sample for
/// sample, because a cheaper decode that changes the audio would change the
/// transcript.
///
/// # Errors
///
/// An unreadable container, or a recording longer than the limit allows.
pub fn decode_audio_mono(
    bytes: &[u8],
    limits: &RunLimits,
    target: u32,
) -> Result<Vec<f32>, String> {
    decode_audio_inner(bytes, limits, Some(target)).map(|(samples, _, _)| samples)
}

/// The shared decode. `target` present means downmix and resample on the way
/// through; absent means hand back the interleaved signal as it was decoded.
fn decode_audio_inner(
    bytes: &[u8],
    limits: &RunLimits,
    target_rate: Option<u32>,
) -> Result<(Vec<f32>, usize, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymErr;

    let mss = symphonia::core::io::MediaSourceStream::new(
        Box::new(Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    // No extension hint: identity comes from content, which is SR-4 applied
    // inside the worker too.
    let mut reader = symphonia::default::get_probe()
        .format(
            &symphonia::core::probe::Hint::new(),
            mss,
            &Default::default(),
            &Default::default(),
        )
        .map(|p| p.format)
        .map_err(|e| format!("could not recognise this audio stream: {e}"))?;

    // The first track a decoder can actually be built for — the same rule
    // oc-audio uses, and for the same reason: in a video container the first
    // non-null track is usually video.
    let candidates: Vec<_> = reader
        .tracks()
        .iter()
        .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .map(|t| (t.id, t.codec_params.clone()))
        .collect();
    let mut chosen = None;
    // Kept for the refusal: "no decodable audio track" is true of a 5.1 AAC
    // file and names the wrong problem.
    let mut refusals: Vec<String> = Vec::new();
    for (id, params) in candidates {
        match symphonia::default::get_codecs().make(&params, &DecoderOptions { verify: false }) {
            Ok(d) => {
                chosen = Some((id, params, d));
                break;
            }
            Err(e) => refusals.push(format!("track {id}: {e}")),
        }
    }
    let (track_id, params, mut decoder) = chosen.ok_or_else(|| {
        if refusals.is_empty() {
            "this file contains no audio track at all".to_string()
        } else {
            format!(
                "this file's audio is in a form this build cannot decode ({})",
                refusals.join("; ")
            )
        }
    })?;

    let mut samples: Vec<f32> = Vec::new();
    let mut channels = params.channels.map_or(0, |c| c.count());
    let mut rate = params.sample_rate.unwrap_or(0);
    let mut buf: Option<SampleBuffer<f32>> = None;

    // The resampler runs alongside the decode when the caller asked for mono
    // at a fixed rate. See `decode_audio_mono` for why.
    let mut stream = target_rate.map(crate::mel::StreamResampler::new);

    loop {
        let packet = match reader.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("could not read the audio stream: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("could not decode this audio: {e}"))?;
        let spec = *decoded.spec();
        channels = spec.channels.count();
        rate = spec.rate;
        let b =
            buf.get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        b.copy_interleaved_ref(decoded);

        if let Some(rs) = stream.as_mut() {
            // STREAMING: the packet is downmixed and resampled now, and the
            // interleaved buffer is reused rather than accumulated. The bound
            // is on what is KEPT — the mono result — because that is the only
            // thing that grows with the length of the recording.
            rs.push(b.samples(), channels, rate);
            if rs.len() as u64 * 4 > limits.memory_bytes {
                return Err(format!(
                    "this recording is longer than the {}-byte limit allows",
                    limits.memory_bytes
                ));
            }
            continue;
        }

        // Bounded before the push, not after: a caller that wants the raw
        // interleaved signal gets the whole thing, and a long recording must
        // refuse rather than grow.
        if (samples.len() + b.samples().len()) as u64 * 4 > limits.memory_bytes {
            return Err(format!(
                "decoding this audio would need more than the {}-byte limit",
                limits.memory_bytes
            ));
        }
        samples.extend_from_slice(b.samples());
    }

    if let Some(rs) = stream {
        let mono = rs.finish();
        if mono.is_empty() {
            return Err("this stream contains no decodable audio".into());
        }
        // Already mono, already at the target rate.
        return Ok((mono, 1, target_rate.unwrap_or(rate)));
    }

    if samples.is_empty() {
        return Err("this stream contains no decodable audio".into());
    }
    Ok((samples, channels, rate))
}

/// Transcribe an audio file to plain text.
///
/// Takes three artifacts in the order the step names them: encoder, decoder,
/// tokeniser.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn transcribe_to_text(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
) -> Result<Converted, String> {
    transcribe_to_text_progress(bytes, models, limits, &mut |_, _| {})
}

/// Transcribe and report the number of audio samples processed.
pub fn transcribe_to_text_progress(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Converted, String> {
    transcribe_to_format_progress(bytes, models, limits, false, progress)
}

pub fn transcribe_to_format_progress(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
    timed: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Converted, String> {
    let [encoder, decoder, tokeniser, vad] = models else {
        return Err(format!(
            concat!(
                "transcription needs four artifacts (encoder, decoder, ",
                "tokeniser, voice-activity detector); got {}"
            ),
            models.len()
        ));
    };
    // Decoded straight to what Whisper wants: mono at 16 kHz, resampled as the
    // packets arrive. The interleaved signal is never held, which is what an
    // hour-long recording could not afford.
    let audio = decode_audio_mono(bytes, limits, crate::mel::SAMPLE_RATE)?;
    let text = if timed {
        crate::whisper::transcribe_format(&audio, encoder, decoder, tokeniser, vad, true, progress)?
    } else {
        crate::whisper::transcribe_progress(&audio, encoder, decoder, tokeniser, vad, progress)?
    };
    Ok(Converted {
        bytes: text.into_bytes(),
        removed: vec![
            "the audio itself: this is a transcription, not a copy".to_string(),
            "silent stretches, which were skipped rather than guessed at".to_string(),
            if timed {
                "speaker identity, tone and anything the model misheard"
            } else {
                "speaker identity, timing, tone and anything the model misheard"
            }
            .to_string(),
        ],
    })
}

/// Upscale an image ×4.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn upscale_image(
    bytes: &[u8],
    models: &[Vec<u8>],
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    let [model] = models else {
        return Err(format!("upscaling takes one model, got {}", models.len()));
    };
    let image = decode_bounded(bytes, limits)?;
    let source_w = image.width();
    let out = crate::upscale::upscale(&image, model)?;
    let (w, h) = (out.width(), out.height());
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(out)
        .write_to(&mut buf, encode_format(to)?)
        .map_err(|e| format!("could not encode the result: {e}"))?;
    Ok(Converted {
        bytes: buf.into_inner(),
        removed: vec![
            // THE FACTOR IS MEASURED, NOT ASSERTED. This said "four times its
            // size" whatever the model did, which was true of the one export
            // that existed and becomes a false statement in a receipt the
            // moment a 2x model is added. The output and the input are both in
            // hand here; the ratio is a division.
            format!(
                "nothing was removed; the image is now {w}x{h}, {} its size",
                describe_factor(source_w, w)
            ),
            "detail at the new size is INVENTED by the model, not recovered".to_string(),
            "all tags and metadata (not carried across a re-encode)".to_string(),
        ],
    })
}

/// The `image` format for a target name this build can write.
fn encode_format(to: &str) -> Result<ImageFormat, String> {
    match to {
        "png" => Ok(ImageFormat::Png),
        "jpeg" | "jpg" => Ok(ImageFormat::Jpeg),
        "webp" => Ok(ImageFormat::WebP),
        other => Err(format!("oc-ai does not write {other}")),
    }
}

/// Enhance speech in an audio file, returning 48 kHz mono WAV.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn denoise_audio(
    bytes: &[u8],
    models: &[Vec<u8>],
    limits: &RunLimits,
) -> Result<Converted, String> {
    let [model, aux] = models else {
        return Err(format!(
            "enhancement needs two artifacts (the graph and its constants); got {}",
            models.len()
        ));
    };
    let (samples, channels, rate) = decode_audio(bytes, limits)?;
    let out = crate::denoise::denoise(&samples, channels, rate, model, aux)?;

    // 48 kHz mono, 16-bit: the model's own rate, and mono because enhancement
    // works on one channel. Both are changes to the file, so both are listed.
    let mut buf = Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::new(&mut buf, spec)
            .map_err(|e| format!("could not start the WAV writer: {e}"))?;
        for v in &out {
            let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            w.write_sample(s)
                .map_err(|e| format!("could not write samples: {e}"))?;
        }
        w.finalize()
            .map_err(|e| format!("could not finalise the WAV: {e}"))?;
    }
    Ok(Converted {
        bytes: buf.into_inner(),
        removed: vec![
            "background noise, as the model judged it -- which can take quiet speech with it"
                .to_string(),
            "every channel but the first; the output is mono 48 kHz".to_string(),
            "all tags and metadata (not carried across a re-encode)".to_string(),
        ],
    })
}

/// Diagnostic: per-chunk speech probability for a whole file.
///
/// # Errors
///
/// As the decode and VAD paths.
pub fn vad_probe(bytes: &[u8], model: &[u8], limits: &RunLimits) -> Result<Vec<f32>, String> {
    let (samples, channels, rate) = decode_audio(bytes, limits)?;
    let audio = crate::mel::to_mono_16k(&samples, channels, rate);
    let mut v = crate::vad::Vad::new(model)?;
    v.probabilities(&audio)
}
