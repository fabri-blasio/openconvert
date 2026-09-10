//! JPEG XL decoding through **jxl-oxide** -- pure Rust, no C, no `links` key.
//!
//! JXL is the one C-adjacent format that does NOT need the vcpkg toolchain:
//! jxl-oxide is a complete decoder in safe Rust, which is why `JXL -> PNG`
//! ships on every platform while HEIC waits for libheif. Same worker, same
//! protocol, same limits -- only the parser differs.

/// Why a JXL decode failed.
#[derive(Debug, thiserror::Error)]
pub enum JxlError {
    /// The input was not decodable.
    #[error("could not decode this JXL: {0}")]
    Decode(String),
}

/// One decoded frame as RGBA8.
#[allow(dead_code)] // width/height/has_alpha are part of the value's contract
pub struct DecodedRgba {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Whether an alpha channel was present (always true from this path;
    /// jxl-oxide's all-channels render yields RGBA).
    pub has_alpha: bool,
}

/// Decode the first frame of a JXL image to RGBA8.
///
/// Header-only probe support comes free: constructing [`jxl_oxide::JxlImage`]
/// parses the container and headers without rendering frames, so
/// [`probe_dimensions`] costs no pixel work.
///
/// # Errors
///
/// [`JxlError`].
pub fn decode_first_frame(bytes: &[u8]) -> Result<DecodedRgba, JxlError> {
    let image = jxl_oxide::JxlImage::builder()
        .read(std::io::Cursor::new(bytes))
        .map_err(|e| JxlError::Decode(format!("{e}")))?;

    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err(JxlError::Decode(
            "the image declares zero dimensions".into(),
        ));
    }

    let render = image
        .render_frame(0)
        .map_err(|e| JxlError::Decode(format!("{e}")))?;
    let fb = render.image_all_channels();
    let channels = fb.channels();
    if channels != 4 && channels != 3 {
        return Err(JxlError::Decode(format!(
            "unexpected channel count {channels}"
        )));
    }
    let data = fb.buf();
    if data.len() < width as usize * height as usize * channels {
        return Err(JxlError::Decode(
            "decoded buffer shorter than declared".into(),
        ));
    }

    // jxl-oxide renders to f32 samples in [0,1]; scale to u8. Clamping (not
    // rounding-then-wrapping) is the honest mapping for out-of-gamut values
    // the decoder may emit.
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for px in data.chunks_exact(channels) {
        for &sample in &px[..3] {
            rgba.push((sample.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        rgba.push(if channels == 4 {
            (px[3].clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            255
        });
    }

    Ok(DecodedRgba {
        width,
        height,
        rgba,
        has_alpha: true,
    })
}

/// Header-only dimensions -- the probe path. No frame is rendered.
///
/// # Errors
///
/// [`JxlError`].
pub fn probe_dimensions(bytes: &[u8]) -> Result<(u32, u32), JxlError> {
    let image = jxl_oxide::JxlImage::builder()
        .read(std::io::Cursor::new(bytes))
        .map_err(|e| JxlError::Decode(format!("{e}")))?;
    Ok((image.width(), image.height()))
}
