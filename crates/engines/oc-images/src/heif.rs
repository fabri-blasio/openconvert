//! HEIC decoding through **libheif**, linked DYNAMICALLY (D19: the LGPL
//! obligation is discharged by construction â€” `heif.dll`/`libde265.dll` ship
//! as replaceable shared libraries, and `engines.toml` gates the linkage).
//!
//! # Where this code sits, and why the `unsafe` here is legal
//!
//! This is an ENGINE crate: the far side of the trust boundary, inside a
//! process that is already confined (`03` Â§5). FFI `unsafe` lives HERE and
//! nowhere else in the host-side story â€” every pointer handed over comes
//! from bytes the host already confined, and every returned buffer is copied
//! out before release, so no C lifetime escapes this module.
//!
//! # The decode contract
//!
//! Bytes in â†’ interleaved RGBA out, one function, no callbacks, no decoder
//! state retained between calls. A hostile file trips libheif's own security
//! limits (it was hardened for untrusted input by design) and returns an
//! error we shape into a user-facing message naming the library version â€”
//! the number CVE advisories are written against.

#![allow(non_snake_case, dead_code)]

use std::ffi::c_char;

// ---------------------------------------------------------------------------
// Raw bindings â€” the exact subset of heif.h this worker uses
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HeifError {
    pub code: i32,
    pub subcode: i32,
    pub message: *const c_char,
}

impl HeifError {
    /// True when the call succeeded (code 0 = "everything fine").
    const fn ok(self) -> bool {
        self.code == 0
    }
}

pub type HeifContext = *mut core::ffi::c_void;
pub type HeifImageHandle = *mut core::ffi::c_void;
pub type HeifImage = *mut core::ffi::c_void;

// Pinned constants from heif_image.h / heif_color.h, read from the header
// rather than guessed: colorspace_RGB is **1** (YCbCr is 0), interleaved RGBA
// is 11, and the interleaved plane channel is 10. The first draft had
// RGB=0/YCbCr=1 backwards and every decode returned "unsupported feature" —
// which reads as file corruption unless the constants are verified against
// the header they come from.
const COLORSPACE_RGB: i32 = 1;
const CHROMA_INTERLEAVED_RGBA: i32 = 11;
const CHANNEL_INTERLEAVED: i32 = 10;

extern "C" {
    fn heif_context_alloc() -> HeifContext;
    fn heif_context_free(ctx: HeifContext);
    fn heif_context_read_from_memory_without_copy(
        ctx: HeifContext,
        mem: *const core::ffi::c_void,
        size: usize,
        options: *const core::ffi::c_void,
    ) -> HeifError;
    fn heif_context_get_primary_image_handle(
        ctx: HeifContext,
        out_handle: *mut HeifImageHandle,
    ) -> HeifError;
    fn heif_image_handle_get_width(handle: HeifImageHandle) -> i32;
    fn heif_image_handle_get_height(handle: HeifImageHandle) -> i32;
    fn heif_image_handle_has_alpha_channel(handle: HeifImageHandle) -> i32;
    fn heif_decode_image(
        handle: HeifImageHandle,
        out_img: *mut HeifImage,
        colorspace: i32,
        chroma: i32,
        options: *const core::ffi::c_void,
    ) -> HeifError;
    fn heif_image_get_plane_readonly(
        img: HeifImage,
        channel: i32,
        out_stride: *mut i32,
    ) -> *const u8;
    fn heif_image_release(img: HeifImage);
    fn heif_image_handle_release(handle: HeifImageHandle);
}

extern "C" {
    /// Quick check whether a decoder for this compression format exists.
    fn heif_have_decoder_for_format(format: i32) -> i32;
    /// Version string like "1.23.1" â€” recorded in receipts, which is why it
    /// matters: it is the number a libheif CVE advisory names.
    fn heif_get_version() -> *const c_char;
}

/// The linked libheif version, for the receipt.
#[must_use]
pub fn linked_version() -> String {
    // SAFETY: returns a pointer to a static NUL-terminated string owned by
    // the library; copied immediately.
    unsafe {
        let p = heif_get_version();
        if p.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Why a HEIC decode failed.
#[derive(Debug, thiserror::Error)]
pub enum HeifError2 {
    /// libheif refused the file.
    #[error("libheif {version} could not decode this HEIC: {message}")]
    Decode {
        /// Linked library version, named for the receipt.
        version: String,
        /// libheif's own message.
        message: String,
    },
    /// Decoded but produced something unusable.
    #[error("libheif produced an unusable image ({0})")]
    BadOutput(&'static str),
}

/// HEVC-in-HEIF compression format code â€” heif_compression_HEVC = 1, read
/// from heif_context.h rather than guessed (the first draft used 4, which is
/// a different enum in a different header, and reported "no decoder").
pub const COMPRESSION_HEVC: i32 = 1;

/// Whether the linked libheif can actually decode HEVC â€” checked at runtime
/// because plugin availability is a property of the installed set, not of
/// our build.
#[must_use]
pub fn has_hevc_decoder() -> bool {
    // SAFETY: pure query; returns nonzero when a decoder exists.
    unsafe { heif_have_decoder_for_format(COMPRESSION_HEVC) != 0 }
}

/// One decoded frame: tightly-packed RGBA, top-left origin.
pub struct DecodedRgba {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Whether the source declared an alpha channel.
    pub has_alpha: bool,
}

/// Read a HEIC from memory and decode the primary image to RGBA.
///
/// Every step checks its error BEFORE touching the next handle, and every
/// acquired resource is released on every exit path â€” the RAII guards below
/// exist because an early return between acquire and release would leak a
/// C allocation on exactly the hostile inputs this worker exists to survive.
///
/// # Errors
///
/// [`HeifError2`] â€” libheif's message passes through verbatim; it is written
/// for end users and is more specific than anything we would invent.
pub fn decode_primary(bytes: &[u8]) -> Result<DecodedRgba, HeifError2> {
    struct Context(HeifContext);
    impl Drop for Context {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: allocated by heif_context_alloc, freed exactly once.
                unsafe { heif_context_free(self.0) };
            }
        }
    }
    struct Handle(HeifImageHandle);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: acquired from get_primary_image_handle, freed once.
                unsafe { heif_image_handle_release(self.0) };
            }
        }
    }
    struct Img(HeifImage);
    impl Drop for Img {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: acquired from heif_decode_image, freed once.
                unsafe { heif_image_release(self.0) };
            }
        }
    }

    let version = linked_version();
    let fail = |m: String| HeifError2::Decode {
        version: version.clone(),
        message: m,
    };

    // SAFETY: allocates a fresh context; freed by the RAII guard.
    let ctx = Context(unsafe { heif_context_alloc() });
    if ctx.0.is_null() {
        return Err(fail("out of memory".into()));
    }

    // SAFETY: `bytes` outlives the READ (libheif copies internally despite
    // the "_without_copy" name referring to avoiding ONE copy; the context
    // keeps no reference after this call returns per the header contract),
    // and the pointer is valid for the length given.
    let err = unsafe {
        heif_context_read_from_memory_without_copy(
            ctx.0,
            bytes.as_ptr().cast(),
            bytes.len(),
            core::ptr::null(),
        )
    };
    if !err.ok() {
        return Err(fail(message_of(err)));
    }

    let mut handle = Handle(core::ptr::null_mut());
    // SAFETY: live context; out-pointer is a live local.
    let err = unsafe { heif_context_get_primary_image_handle(ctx.0, &raw mut handle.0) };
    if !err.ok() {
        return Err(fail(message_of(err)));
    }

    let width = unsafe { heif_image_handle_get_width(handle.0) }.max(0) as u32;
    let height = unsafe { heif_image_handle_get_height(handle.0) }.max(0) as u32;
    let has_alpha = unsafe { heif_image_handle_has_alpha_channel(handle.0) } != 0;

    let mut img = Img(core::ptr::null_mut());
    // SAFETY: live handle; out-pointer is a live local; RGB+interleaved RGBA
    // asks libheif to convert whatever the codec stored into packed RGBA.
    let err = unsafe {
        heif_decode_image(
            handle.0,
            &raw mut img.0,
            COLORSPACE_RGB,
            CHROMA_INTERLEAVED_RGBA,
            core::ptr::null(),
        )
    };
    if !err.ok() {
        return Err(fail(message_of(err)));
    }

    let mut stride: i32 = 0;
    // SAFETY: live image; stride out-pointer is a live local. The plane is
    // owned by the image and released by the guard below.
    let plane =
        unsafe { heif_image_get_plane_readonly(img.0, CHANNEL_INTERLEAVED, &raw mut stride) };
    if plane.is_null() || stride <= 0 {
        return Err(HeifError2::BadOutput("null pixel plane"));
    }
    let stride = stride as usize;
    if width == 0 || height == 0 {
        return Err(HeifError2::BadOutput("zero dimensions"));
    }
    let row_len = width as usize * 4;
    if stride < row_len {
        return Err(HeifError2::BadOutput("stride smaller than a row"));
    }

    // De-plane: copy row by row (stride may exceed row_len for alignment).
    let mut rgba = vec![0u8; row_len * height as usize];
    for (i, row) in rgba.chunks_exact_mut(row_len).enumerate() {
        // SAFETY: rows 0..height of a plane whose stride bounds it; the copy
        // stays inside the plane the image owns until the guard releases it.
        let src = unsafe { plane.add(i * stride) };
        row.copy_from_slice(unsafe { core::slice::from_raw_parts(src, row_len) });
    }

    Ok(DecodedRgba {
        width,
        height,
        rgba,
        has_alpha,
    })
}

/// Extract libheif's message, which the header guarantees is either NULL or a
/// static string valid for the program lifetime.
fn message_of(err: HeifError) -> String {
    if err.message.is_null() {
        format!("error code {}", err.code)
    } else {
        // SAFETY: static NUL-terminated string per heif_error.h contract.
        unsafe { std::ffi::CStr::from_ptr(err.message) }
            .to_string_lossy()
            .into_owned()
    }
}
