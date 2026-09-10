//! Camera RAW decoding through **libraw**, linked DYNAMICALLY (same
//! trust-boundary placement as `heif.rs`: copyleft discharged by construction
//! â€” `raw_r.dll` ships as a replaceable shared library beside the worker,
//! and `engines.toml` gates the linkage).
//!
//! # Where this code sits, and why the `unsafe` here is legal
//!
//! This is an ENGINE crate: the far side of the trust boundary, inside a
//! process that is already confined (`03` Â§5). FFI `unsafe` lives HERE and
//! nowhere else in the host-side story â€” every pointer handed over comes from
//! bytes the host already confined, and every returned buffer is copied out
//! before release, so no C lifetime escapes this module.
//!
//! # The decode contract
//!
//! Bytes in â† interleaved RGB8 out (sensors capture no alpha), one function,
//! no callbacks, no decoder state retained between calls. A hostile file trips
//! libraw's own parser and returns an error shaped for a user-facing message
//! naming the library version â€” the number CVE advisories are written against.
//!
//! # Deviations from the brief, verified against the installed headers
//!
//! The first draft of this file invented two functions and mis-signed a third;
//! each binding below cites the line of `libraw.h` / `libraw_const.h` /
//! `libraw_types.h` it was read from (vcpkg libraw 0.22.1):
//!
//! - there are no `libraw_get_image_width/height`; the C API spells them
//!   `libraw_get_iwidth/iheight` (`libraw.h:335â€“341`);
//! - `libraw_close` returns `void`, not `int` (`libraw.h:118`);
//! - `libraw_open_buffer` takes a `size_t`, not an `int64` (`libraw.h:104`);
//! - `libraw_dcraw_make_mem_image` returns a `libraw_processed_image_t*`
//!   whose header carries type/dimensions/size, with an `int *errc`
//!   OUT-parameter â€” not a raw byte pointer with a size OUT-parameter
//!   (`libraw.h:153â€“154`, struct at `libraw_types.h:180â€“186`);
//! - the image that function returns is freed by `libraw_dcraw_clear_mem`,
//!   not `libraw_free_image` (the latter releases the unpacked RAW sensor
//!   buffer held after `unpack`, `libraw.h:121` vs `libraw.h:157`).
//!
//! Guessing these wrong would have linked cleanly and then read pixels out of
//! what is really a struct header â€” which reads as file corruption unless the
//! constants are verified against the header they come from.

#![allow(non_snake_case, dead_code)]

use std::ffi::c_char;

/// An initialised libraw processor, opaque outside the library.
pub type LibRaw = *mut core::ffi::c_void;

// From libraw_const.h:813 â€” LIBRAW_SUCCESS = 0.
const SUCCESS: i32 = 0;
// From libraw_const.h:866 â€” enum LibRaw_image_formats; BITMAP means the
// processed buffer is raw pixels rather than an embedded JPEG/JXL encode.
const IMAGE_BITMAP: u32 = 2;

/// Mirror of `libraw_processed_image_t` (`libraw_types.h:180â€“186`). The C
/// flexible-array tail `data[1]` becomes a one-byte array we never index past
/// what `data_size` bounds-checked reads allow.
#[repr(C)]
pub struct LibRawProcessedImage {
    /// Which `LibRaw_image_formats` this holds â€” [`IMAGE_BITMAP`] here.
    pub image_type: u32,
    /// Pixel height.
    pub height: u16,
    /// Pixel width.
    pub width: u16,
    /// Samples per pixel (3 for sRGB output).
    pub colors: u16,
    /// Bits per sample (8 after the default dcraw pipeline).
    pub bits: u16,
    /// Payload byte count behind `data`.
    pub data_size: u32,
    /// Flexible-array payload start.
    pub data: [u8; 1],
}

// ---------------------------------------------------------------------------
// Raw bindings â€” the exact subset of the LibRaw C API this worker uses
// ---------------------------------------------------------------------------
//
// The `#[link]` attribute makes the module carry its own linkage: any target
// that compiles it (the worker binary once wired, the `tests/native_decoders`
// harness today) emits `-lraw_r` itself instead of depending on a build.rs
// line that does not exist yet (see private implementation notes â€” the build.rs lines remain
// the canonical long-term home; duplicates are harmless).

// Import library for `raw_r.dll` (vcpkg installs `raw_r.lib`; there is no
// `raw.lib`). Thread-safe variant, chosen because it is the only import
// library shipped.
#[link(name = "raw_r")]
extern "C" {
    /// Allocate and initialise a processor (`libraw.h:90`). Returns NULL on
    /// failure; flags 0 selects every default.
    fn libraw_init(flags: u32) -> LibRaw;
    /// Free all resources; call exactly once per handle (`libraw.h:118`).
    fn libraw_close(handle: LibRaw);
    /// Open a RAW from memory (`libraw.h:104`). Returns 0 on success.
    fn libraw_open_buffer(handle: LibRaw, data: *const u8, size: usize) -> i32;
    /// Decode the sensor data (`libraw.h:113`). Returns 0 on success.
    fn libraw_unpack(handle: LibRaw) -> i32;
    /// Demosaic, white-balance, gamma-map toward sRGB (`libraw.h:152`).
    fn libraw_dcraw_process(handle: LibRaw) -> i32;
    /// Processed-image width (`libraw.h:339`).
    fn libraw_get_iwidth(handle: LibRaw) -> i32;
    /// Processed-image height (`libraw.h:341`).
    fn libraw_get_iheight(handle: LibRaw) -> i32;
    /// Materialise the processed image as a heap [`LibRawProcessedImage`]
    /// (`libraw.h:153â€“154`); `errc` receives 0 or a code `libraw_strerror`
    /// explains. NULL return on failure.
    fn libraw_dcraw_make_mem_image(handle: LibRaw, errc: *mut i32) -> *mut LibRawProcessedImage;
    /// Release an image from `make_mem_image` (`libraw.h:157`).
    fn libraw_dcraw_clear_mem(image: *mut LibRawProcessedImage);
    /// Human-readable text for any error code above (`libraw.h:82`); static
    /// string, valid for the program lifetime.
    fn libraw_strerror(errorcode: i32) -> *const c_char;
    /// Version string like "0.22.1" â€” recorded in receipts, which is why it
    /// matters: it is the number a libraw CVE advisory names (`libraw.h:99`).
    fn libraw_version() -> *const c_char;
}

/// The linked libraw version, for the receipt.
#[must_use]
pub fn linked_version() -> String {
    // SAFETY: returns a pointer to a static NUL-terminated string owned by
    // the library; copied immediately.
    unsafe {
        let p = libraw_version();
        if p.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Text for a libraw error code, falling back to the raw number when the
/// library has nothing to say about it.
fn reason_of(code: i32) -> String {
    // SAFETY: static NUL-terminated string per libraw.h's strerror contract.
    unsafe {
        let p = libraw_strerror(code);
        if p.is_null() {
            format!("error code {code}")
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Why a RAW decode failed.
#[derive(Debug, thiserror::Error)]
pub enum RawError {
    /// `open_buffer` failed, or `unpack` could not decode the sensor data.
    #[error("could not read this RAW file: {0}")]
    Open(String),
    /// `dcraw_process` or `make_mem_image` failed.
    #[error("could not process this RAW file: {0}")]
    Process(String),
    /// Decoded but produced something unusable.
    #[error("RAW decoder produced unusable output: {0}")]
    BadOutput(&'static str),
}

/// One decoded RAW image as RGB8 (no alpha; sensors do not capture alpha).
///
/// The default pipeline converts to sRGB at 8 bits per sample and always
/// yields 3 channels; exotic 4-colour CFA outputs are refused by
/// [`decode_raw`] rather than silently reinterpreted (with our parameters â€”
/// sRGB output colour, no four-color RGB â€” they are not expected to occur;
/// the check is the guard, not a supported path).
pub struct DecodedRaw {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 3` bytes, row-major, top-left origin.
    pub rgb: Vec<u8>,
}

/// Decode a camera RAW file to RGB8 using libraw's default pipeline
/// (auto white balance, sRGB output, 8 bits per sample).
///
/// Every step checks its error BEFORE touching the next handle, and both
/// acquired resources (processor, materialised image) are released on every
/// exit path via the RAII guards below â€” an early return between acquire and
/// release would leak a C allocation on exactly the hostile inputs this
/// worker exists to survive.
///
/// # Errors
///
/// [`RawError`] â€” libraw's own messages pass through via `libraw_strerror`;
/// they are written for end users and are more specific than anything we
/// would invent.
pub fn decode_raw(bytes: &[u8]) -> Result<DecodedRaw, RawError> {
    /// Owns the processor until scope end; frees exactly once.
    struct Guard(LibRaw);
    impl Drop for Guard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: allocated by libraw_init, freed exactly once.
                unsafe { libraw_close(self.0) };
            }
        }
    }
    /// Owns a materialised image until scope end; frees exactly once.
    struct MemImg(*mut LibRawProcessedImage);
    impl Drop for MemImg {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: returned by make_mem_image, freed exactly once.
                unsafe { libraw_dcraw_clear_mem(self.0) };
            }
        }
    }

    // SAFETY: allocates processor state; freed by Guard below.
    let handle = unsafe { libraw_init(0) };
    if handle.is_null() {
        return Err(RawError::Open("out of memory".into()));
    }
    let _guard = Guard(handle);

    // SAFETY: `bytes` outlives EVERY use of the handle â€” open, unpack and
    // process all complete inside this function before the guard frees it â€”
    // and the pointer is valid for `bytes.len()` bytes given.
    let r = unsafe { libraw_open_buffer(handle, bytes.as_ptr(), bytes.len()) };
    if r != SUCCESS {
        return Err(RawError::Open(reason_of(r)));
    }

    // SAFETY: live handle opened above.
    let r = unsafe { libraw_unpack(handle) };
    if r != SUCCESS {
        return Err(RawError::Open(reason_of(r)));
    }

    // SAFETY: live handle, unpacked above.
    let r = unsafe { libraw_dcraw_process(handle) };
    if r != SUCCESS {
        return Err(RawError::Process(reason_of(r)));
    }

    // Cross-check dimensions from the processor before asking for the buffer:
    // a header that lies about size should fail HERE, not as an OOB read.
    let iw = unsafe { libraw_get_iwidth(handle) }; // SAFETY: live handle.
    let ih = unsafe { libraw_get_iheight(handle) }; // SAFETY: live handle.
    if iw <= 0 || ih <= 0 {
        return Err(RawError::BadOutput("zero dimensions"));
    }

    let mut errc: i32 = SUCCESS;
    // SAFETY: live handle; `errc` is a live local. The returned image is
    // owned by libraw. The guard is installed BEFORE any error check so the
    // pointer cannot leak on any path, whatever the library returns.
    let img = MemImg(unsafe { libraw_dcraw_make_mem_image(handle, &mut errc) });
    if errc != SUCCESS {
        return Err(RawError::Process(reason_of(errc)));
    }
    if img.0.is_null() {
        return Err(RawError::Process("make_mem_image returned null".into()));
    }

    // SAFETY: non-null image returned by make_mem_image is valid to read.
    let img_ref = unsafe { &*img.0 };
    if img_ref.image_type != IMAGE_BITMAP {
        return Err(RawError::BadOutput(
            "decoder returned an encoded image instead of a pixel bitmap",
        ));
    }
    if img_ref.bits != 8 || img_ref.colors != 3 {
        return Err(RawError::BadOutput(
            "unexpected sample layout (expected 8-bit RGB)",
        ));
    }
    let width = u32::from(img_ref.width);
    let height = u32::from(img_ref.height);
    if width == 0 || height == 0 {
        return Err(RawError::BadOutput("zero dimensions"));
    }
    if u32::from(img_ref.width) != iw.cast_unsigned()
        || u32::from(img_ref.height) != ih.cast_unsigned()
    {
        return Err(RawError::BadOutput(
            "image dimensions disagree with the processor",
        ));
    }

    // Checked because a 32-bit target could not hold 65535 * 65535 * 3.
    let expected = width
        .checked_mul(height)
        .and_then(|p| p.checked_mul(3))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or(RawError::BadOutput("image larger than the address space"))?;
    if (img_ref.data_size as usize) < expected {
        return Err(RawError::BadOutput(
            "pixel buffer shorter than declared dimensions",
        ));
    }

    // SAFETY: `data_size` bytes are valid behind the fixed-size header per
    // the struct contract; we read only the bounds-checked prefix.
    let rgb = unsafe { core::slice::from_raw_parts(img_ref.data.as_ptr(), expected) }.to_vec();

    Ok(DecodedRaw { width, height, rgb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_input_returns_err_not_crash() {
        assert!(decode_raw(&[0xFFu8; 1024]).is_err());
    }

    #[test]
    fn empty_input_returns_err() {
        assert!(decode_raw(&[]).is_err());
    }

    #[test]
    fn truncated_tiff_returns_err() {
        // TIFF magic but truncated body
        assert!(decode_raw(b"II*\x00\x00\x00\x00\x00").is_err());
    }

    #[test]
    fn version_is_reported_for_the_receipt() {
        assert!(!linked_version().is_empty());
    }

    // -- Positive path -------------------------------------------------------
    //
    // The hostile-input tests above prove refusal, not decoding. This builds
    // the smallest file libraw accepts as a RAW â€” a LinearRaw DNG
    // (PhotometricInterpretation 34892, uncompressed 16-bit interleaved RGB)
    // â€” and drives decode_raw all the way through init/open/unpack/process/
    // make_mem_image, which is what actually pins down the #[repr(C)] struct
    // mirror, the dimension cross-check and the payload copy.

    fn le16(v: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn le32(v: u32, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    /// One TIFF directory entry: tag, type (1=BYTE, 3=SHORT, 4=LONG),
    /// count, and the value/offset field packed little-endian.
    fn tiff_entry(tag: u16, typ: u16, count: u32, value: u32, out: &mut Vec<u8>) {
        le16(tag, out);
        le16(typ, out);
        le32(count, out);
        le32(value, out);
    }

    fn build_linear_dng(width: u16, height: u16) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"II*\x00"); // little-endian TIFF magic
        le32(8, &mut d); // IFD0 follows the header

        // Entries must be ascending by tag. BitsPerSample (3 SHORTs) does not
        // fit in the 4-byte value field, so it points at an external array.
        const N: u32 = 10;
        let bps_off: usize = 8 + 2 + N as usize * 12 + 4;
        let data_off = bps_off + 6 + 2; // BPS array + 2-byte pad

        le16(N as u16, &mut d);
        tiff_entry(256, 4, 1, u32::from(width), &mut d); // ImageWidth
        tiff_entry(257, 4, 1, u32::from(height), &mut d); // ImageLength
        tiff_entry(258, 3, 3, bps_off as u32, &mut d); // BitsPerSample -> array
        tiff_entry(259, 3, 1, 1, &mut d); // Compression: none
        tiff_entry(262, 3, 1, 34892, &mut d); // PhotometricInterpretation: LinearRaw
        tiff_entry(273, 4, 1, data_off as u32, &mut d); // StripOffsets
        tiff_entry(277, 3, 1, 3, &mut d); // SamplesPerPixel
        tiff_entry(279, 4, 1, u32::from(width) * u32::from(height) * 6, &mut d); // StripByteCounts
        tiff_entry(284, 3, 1, 1, &mut d); // PlanarConfiguration: chunky
        tiff_entry(50706, 1, 4, u32::from(u16::from_le_bytes([1, 4])), &mut d); // DNGVersion 1.4
        le32(0, &mut d); // no further IFDs

        le16(16, &mut d); // BitsPerSample array: 16, 16, 16
        le16(16, &mut d);
        le16(16, &mut d);
        le16(0, &mut d); // pad to keep the strip 2-byte aligned

        debug_assert_eq!(d.len(), data_off);

        // A deterministic gradient: enough variation that "the pipeline ran"
        // is observable in the output bytes.
        for i in 0..u32::from(width) * u32::from(height) {
            let v = (i % 997 * 65) as u16;
            le16(v, &mut d);
            le16(v, &mut d);
            le16(v, &mut d);
        }
        d
    }

    #[test]
    fn linear_dng_decodes_to_tight_rgb() {
        let (w, h) = (128u16, 96u16);
        let dng = build_linear_dng(w, h);
        let img = decode_raw(&dng).expect("a valid LinearRaw DNG must decode");
        assert_eq!(img.width, u32::from(w));
        assert_eq!(img.height, u32::from(h));
        assert_eq!(img.rgb.len(), usize::from(w) * usize::from(h) * 3);
        // The gradient survived processing: output is not one flat colour.
        assert!(
            img.rgb.iter().any(|&b| b != img.rgb[0]),
            "decoded pixels are unexpectedly uniform"
        );
    }
}
