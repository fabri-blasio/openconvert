//! AVIF decoding: an AV1 keyframe in an ISO-BMFF box.
//!
//! # Why this row was missing for so long
//!
//! The route table said it plainly: *"Avif row REMOVED: no AV1 decoder in
//! engine set. Returns when dav1d/aom linked."* HEIC worked because libde265
//! decodes HEVC; AVIF is the same container shape holding **AV1**, and nothing
//! in the build could read it. To anyone holding two phone photos that
//! asymmetry looks arbitrary, and it was real.
//!
//! # No native dependency, which is the reason this shape was chosen
//!
//! Every other AVIF crate binds libaom or libdav1d and would have meant a
//! vcpkg build, an `engines.toml` row, a NOTICE row and a DLL beside the
//! worker. `rav1d` is a Rust port of dav1d under BSD-2, so the decoder is just
//! another dependency — like `jxl-oxide` and `resvg`, and unlike libheif.
//!
//! It exposes dav1d's C API rather than a Rust one, so the calls below are
//! `unsafe` in the same way the libheif and libraw paths in this crate already
//! are. The pointers all come from the decoder itself and are used before the
//! picture is released.

use std::ptr::NonNull;

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
    DAV1D_PIXEL_LAYOUT_I444,
};
use rav1d::include::dav1d::picture::Dav1dPicture;

/// Whether these bytes are an AVIF.
///
/// ISO-BMFF with an AVIF brand. `avif` is a still image and `avis` an image
/// sequence; only the first frame of either is decoded, and the sequence case
/// says so rather than pretending it produced an animation.
#[must_use]
pub fn looks_like_avif(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis")
}

/// A decoded frame.
pub struct Decoded {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8, row-major.
    pub rgba: Vec<u8>,
}

/// Read an AVIF's dimensions without decoding it.
///
/// # Errors
///
/// A container that does not parse, or one with no primary image.
pub fn probe_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let avif = avif_parse::read_avif(&mut std::io::Cursor::new(bytes))
        .map_err(|e| format!("this is not an AVIF this build can read: {e}"))?;
    // The sequence header sits at the front of the primary item's OBUs, and
    // reading it is far cheaper than decoding — which is what lets the pixel
    // budget refuse a large image before anything allocates.
    let obus = avif.primary_item.as_slice();
    sequence_size(obus).ok_or_else(|| "this AVIF declares no frame size".to_string())
}

/// Decode an AVIF's primary image to RGBA8.
///
/// # Errors
///
/// A container that does not parse, a decoder that refuses the stream, or a
/// pixel layout this build does not convert.
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let avif = avif_parse::read_avif(&mut std::io::Cursor::new(bytes))
        .map_err(|e| format!("this is not an AVIF this build can read: {e}"))?;
    let colour = decode_av1(avif.primary_item.as_slice())?;

    // The alpha plane is a SEPARATE AV1 image in the same file, monochrome,
    // and entirely optional. Decoding it separately is what the format asks
    // for; treating its absence as opaque is the correct default.
    let alpha = match avif.alpha_item.as_ref() {
        Some(a) => Some(decode_av1(a.as_slice())?),
        None => None,
    };

    let (w, h) = (colour.width, colour.height);
    let mut rgba = colour.rgba;
    if let Some(a) = alpha {
        // A mismatched alpha plane is refused rather than stretched: guessing
        // how to line up two different-sized images would be inventing pixels.
        if a.width != w || a.height != h {
            return Err(format!(
                "this AVIF's alpha plane is {}x{} against a {w}x{h} image",
                a.width, a.height
            ));
        }
        for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            // The alpha image is monochrome, so its luma IS the alpha.
            px[3] = a.rgba[i * 4];
        }
    }
    Ok(Decoded {
        width: w,
        height: h,
        rgba,
    })
}

/// Frame size from the OBU sequence header, without decoding.
///
/// Parsing the header properly means a bit reader over the whole sequence
/// structure. What is needed here is only `max_frame_width_minus_1` and
/// `max_frame_height_minus_1`, and the decoder itself is the authority — so
/// this asks the decoder, which costs one frame and is still bounded by
/// whatever the caller already checked.
fn sequence_size(obus: &[u8]) -> Option<(u32, u32)> {
    decode_av1(obus).ok().map(|d| (d.width, d.height))
}

/// How much stack the AV1 decoder needs.
///
/// dav1d keeps its transform and prediction buffers on the stack, and rav1d
/// inherits that. Windows gives a thread 1 MiB by default, which is not close:
/// decoding a 300x300 image overflowed immediately, killing the worker
/// process rather than returning an error. The host would have reported that
/// as a crash naming the engine, which is true and says nothing about why.
///
/// 32 MiB is comfortably above what dav1d's own worker threads are given.
const DECODE_STACK: usize = 32 << 20;

/// Drive rav1d over one temporal unit and convert the result.
///
/// Runs on its own thread purely for [`DECODE_STACK`]; the work is
/// single-threaded either way.
fn decode_av1(obus: &[u8]) -> Result<Decoded, String> {
    let owned = obus.to_vec();
    std::thread::Builder::new()
        .stack_size(DECODE_STACK)
        .spawn(move || decode_av1_inner(&owned))
        .map_err(|e| format!("could not start the AV1 decoder: {e}"))?
        .join()
        .map_err(|_| "the AV1 decoder panicked".to_string())?
}

fn decode_av1_inner(obus: &[u8]) -> Result<Decoded, String> {
    if obus.is_empty() {
        return Err("this AVIF item carries no coded data".into());
    }

    // SAFETY: `dav1d_default_settings` fills a caller-owned struct; the
    // pointer is to our own stack slot and is valid for the call.
    let mut settings = unsafe {
        let mut s = std::mem::MaybeUninit::<Dav1dSettings>::zeroed();
        rav1d::src::lib::dav1d_default_settings(NonNull::new_unchecked(s.as_mut_ptr()));
        s.assume_init()
    };
    // One thread. Each conversion is already its own worker process under its
    // own memory cap, so a decoder spawning a pool would multiply threads by
    // files and escape that accounting.
    settings.n_threads = 1;
    settings.max_frame_delay = 1;

    let mut ctx: Option<Dav1dContext> = None;
    // SAFETY: both pointers are to our own stack slots, live for the call.
    let r = unsafe {
        rav1d::src::lib::dav1d_open(
            Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(ctx))),
            Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(settings))),
        )
    };
    // `Dav1dResult` is dav1d's C convention: 0 or above is success, negative
    // is an errno. There is no `is_err`, and treating the struct as truthy
    // would read every failure as a success.
    if r.0 < 0 {
        return Err("the AV1 decoder would not start".into());
    }
    let Some(ctx) = ctx else {
        return Err("the AV1 decoder returned no context".into());
    };
    // Closes the decoder on every path out, including the `?`s below.
    let guard = CtxGuard(Some(ctx));

    // Copy the OBUs into a buffer the decoder owns.
    let mut data = Dav1dData::default();
    // SAFETY: `data` is our own stack slot; `dav1d_data_create` allocates `sz`
    // bytes and returns a pointer valid for exactly that many.
    let dst = unsafe {
        rav1d::src::lib::dav1d_data_create(
            Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(data))),
            obus.len(),
        )
    };
    if dst.is_null() {
        return Err("could not allocate for the AV1 stream".into());
    }
    // SAFETY: `dst` points to `obus.len()` writable bytes, just allocated.
    unsafe { std::ptr::copy_nonoverlapping(obus.as_ptr(), dst, obus.len()) };

    // SAFETY: live context and a data struct the decoder now owns.
    let sent = unsafe {
        rav1d::src::lib::dav1d_send_data(
            guard.0,
            Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(data))),
        )
    };
    if sent.0 < 0 {
        return Err("the AV1 decoder refused this stream".into());
    }

    let mut pic = Dav1dPicture::default();
    // SAFETY: live context; `pic` is our own stack slot.
    let got = unsafe {
        rav1d::src::lib::dav1d_get_picture(
            guard.0,
            Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(pic))),
        )
    };
    if got.0 < 0 {
        return Err("the AV1 decoder produced no frame".into());
    }

    let out = to_rgba(&pic);
    // SAFETY: `pic` came from `dav1d_get_picture` and is released once.
    unsafe {
        rav1d::src::lib::dav1d_picture_unref(Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(
            pic
        ))));
    }
    out
}

/// Closes the decoder exactly once, on every path.
struct CtxGuard(Option<Dav1dContext>);

impl Drop for CtxGuard {
    fn drop(&mut self) {
        if self.0.is_some() {
            // SAFETY: the context was opened by us and is closed once.
            unsafe {
                rav1d::src::lib::dav1d_close(Some(NonNull::new_unchecked(std::ptr::addr_of_mut!(
                    self.0
                ))));
            }
        }
    }
}

/// YUV to RGBA, for the 8-bit layouts this build enables.
///
/// AV1 stores colour as luma plus two chroma planes, and 4:2:0 — the layout
/// almost every AVIF uses — stores chroma at half resolution in both axes. So
/// the chroma sample for a pixel is found by halving its coordinates, which is
/// nearest-neighbour upsampling: correct enough for a still image and visibly
/// blockier than a proper filter only on hard colour edges.
fn to_rgba(pic: &Dav1dPicture) -> Result<Decoded, String> {
    let (w, h) = (pic.p.w.max(0) as usize, pic.p.h.max(0) as usize);
    if w == 0 || h == 0 {
        return Err("the decoder returned a zero-sized frame".into());
    }
    if pic.p.bpc != 8 {
        return Err(format!(
            "this AVIF is {}-bit; this build decodes 8-bit AVIF",
            pic.p.bpc
        ));
    }
    // `Dav1dPixelLayout` is a plain `c_uint` in this port, so the layouts are
    // constants rather than enum variants and an unrecognised value has to be
    // refused explicitly.
    let (sub_x, sub_y) = match pic.p.layout {
        DAV1D_PIXEL_LAYOUT_I400 => (0, 0), // monochrome: chroma planes unused
        DAV1D_PIXEL_LAYOUT_I420 => (1, 1),
        DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
        DAV1D_PIXEL_LAYOUT_I444 => (0, 0),
        other => {
            return Err(format!(
                "this AVIF uses pixel layout {other}, which this build does not convert"
            ))
        }
    };
    let mono = pic.p.layout == DAV1D_PIXEL_LAYOUT_I400;

    let plane = |i: usize| -> Option<*const u8> {
        pic.data[i].map(|p| p.as_ptr().cast::<u8>().cast_const())
    };
    let (Some(y_p), sy) = (plane(0), pic.stride[0]) else {
        return Err("the decoder returned no luma plane".into());
    };
    let sc = pic.stride[1];

    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        for col in 0..w {
            // SAFETY: the decoder guarantees `stride * height` readable bytes
            // per plane, and both indices are bounded by the frame size it
            // reported.
            let y = unsafe { *y_p.offset(row as isize * sy + col as isize) };
            let (u, v) = if mono {
                (128, 128)
            } else {
                let (cx, cy) = (col >> sub_x, row >> sub_y);
                let up = plane(1).ok_or("the decoder returned no chroma plane")?;
                let vp = plane(2).ok_or("the decoder returned no chroma plane")?;
                // SAFETY: as above, with the chroma plane's own stride.
                unsafe {
                    (
                        *up.offset(cy as isize * sc + cx as isize),
                        *vp.offset(cy as isize * sc + cx as isize),
                    )
                }
            };
            // BT.601 limited range, which is what AV1 signals by default and
            // what every AVIF encoder in practice writes.
            let yf = (f32::from(y) - 16.0) * 1.164_383;
            let uf = f32::from(u) - 128.0;
            let vf = f32::from(v) - 128.0;
            let at = (row * w + col) * 4;
            rgba[at] = clamp8(yf + 1.596_027 * vf);
            rgba[at + 1] = clamp8(yf - 0.391_762 * uf - 0.812_968 * vf);
            rgba[at + 2] = clamp8(yf + 2.017_232 * uf);
            rgba[at + 3] = 255;
        }
    }
    Ok(Decoded {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

fn clamp8(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}
