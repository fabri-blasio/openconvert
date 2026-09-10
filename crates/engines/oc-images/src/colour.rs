//! ICC colour profile transformation through **lcms2**, linked DYNAMICALLY.
//!
//! lcms2 is MIT-licensed, so the linkage choice is about the same packaging
//! mechanism as `heif.rs` (`lcms2-2.dll` ships beside the worker, one
//! mechanism, one ACL), not a copyleft obligation. Same trust-boundary
//! placement as every other module in this crate: the bytes were confined by
//! the host before they reached us.
//!
//! # Constants read from the header, not guessed
//!
//! The format codes below are MACRO EXPRESSIONS in `lcms2.h`
//! (`TYPE_RGBA_8 = COLORSPACE_SH(PT_RGB)|EXTRA_SH(1)|CHANNELS_SH(3)|BYTES_SH(1)`,
//! with `PT_RGB = 4`, `COLORSPACE_SH(s) = s << 16`, `EXTRA_SH(e) = e << 7`,
//! `CHANNELS_SH(c) = c << 3`, `BYTES_SH(b) = b`). A first draft carried
//! decimal literals (0xA1 / 0x69) that belong to no lcms2 release; spelled as
//! the same expressions the header defines, they evaluate to `0x40099` /
//! `0x40019` â€” and a wrong value here does not fail loudly, it silently
//! reinterprets every pixel's channel order.

#![allow(non_snake_case, dead_code)]

use std::ffi::c_void;

/// An open ICC profile, opaque outside the library.
pub type CmsProfile = *mut c_void;
/// A compiled colour transform, opaque outside the library.
pub type CmsTransform = *mut c_void;

// --- lcms2.h format encoding macros, mirrored verbatim ----------------------
const PT_RGB: i32 = 4;
const fn COLORSPACE_SH(s: i32) -> i32 {
    s << 16
}
const fn EXTRA_SH(e: i32) -> i32 {
    e << 7
}
const fn CHANNELS_SH(c: i32) -> i32 {
    c << 3
}
const fn BYTES_SH(b: i32) -> i32 {
    b
}

/// `TYPE_RGB_8` from lcms2.h â€” packed RGB, 8 bits per sample. Declared for
/// callers holding 3-channel buffers (RAW output); unused while the worker
/// only feeds RGBA planes.
pub const TYPE_RGB_8: i32 = COLORSPACE_SH(PT_RGB) | CHANNELS_SH(3) | BYTES_SH(1);
/// `TYPE_RGBA_8` from lcms2.h â€” packed RGBA, 8 bits per sample.
pub const TYPE_RGBA_8: i32 = COLORSPACE_SH(PT_RGB) | EXTRA_SH(1) | CHANNELS_SH(3) | BYTES_SH(1);
/// Rendering intent 0 (`INTENT_PERCEPTUAL`, lcms2.h).
const INTENT_PERCEPTUAL: i32 = 0;

// The `#[link]` attribute makes the module carry its own linkage for any
// target that compiles it (worker binary once wired, test harness today),
// mirroring raw.rs; see private implementation notes â€” build.rs remains the canonical long-term
// home for these lines, duplicates are harmless. The import library is
// `lcms2.lib`; the DLL it names is `lcms2-2.dll`.
#[link(name = "lcms2")]
extern "C" {
    /// Open an ICC profile from memory; NULL on parse failure. lcms2 copies
    /// the data internally.
    fn cmsOpenProfileFromMem(data: *const u8, size: u32) -> CmsProfile;
    /// Close a profile; exactly once per handle.
    fn cmsCloseProfile(profile: CmsProfile);
    /// Compile source-profile -> destination-profile transform; a NULL output
    /// profile selects lcms2's built-in sRGB. NULL on failure.
    fn cmsCreateTransform(
        input: CmsProfile,
        input_format: i32,
        output: CmsProfile,
        output_format: i32,
        intent: i32,
        flags: u32,
    ) -> CmsTransform;
    /// Free a transform; exactly once per handle.
    fn cmsDeleteTransform(transform: CmsTransform);
    /// Transform `count` pixels in place through the compiled transform.
    fn cmsDoTransform(transform: CmsTransform, input: *const u8, output: *mut u8, count: u32);
    /// lcms2's built-in sRGB profile. Close when no longer needed; the
    /// transform holds its own references once created. (The lcms1-era
    /// spelling `cmsCreate_sRGB` does not exist in lcms2.)
    #[link_name = "cmsCreate_sRGBProfile"]
    fn cms_create_srgb() -> CmsProfile;
}

/// Apply an embedded ICC profile to an RGBA buffer, converting toward sRGB.
///
/// The source profile is read from `icc_data`; the destination is sRGB
/// (lcms2's built-in). Applied IN PLACE: the same pointer is handed over as
/// input and output, which lcms2 supports for its pixel formats. A trailing
/// partial pixel (`rgba.len() % 4`) is left untouched.
///
/// # Errors
///
/// Returns Err if the ICC profile is malformed or the transform cannot be
/// created; the buffer is unmodified in those cases.
pub fn apply_icc_profile(rgba: &mut [u8], icc_data: &[u8]) -> Result<(), String> {
    // cmsOpenProfileFromMem takes a 32-bit length; refuse rather than wrap.
    if icc_data.len() > u32::MAX as usize {
        return Err("embedded ICC profile is larger than 4 GiB".into());
    }
    // SAFETY: icc_data points to valid memory of the given length; lcms2
    // copies internally, so no lifetime escapes this call.
    let src_profile = unsafe { cmsOpenProfileFromMem(icc_data.as_ptr(), icc_data.len() as u32) };
    if src_profile.is_null() {
        return Err("could not parse the embedded ICC profile".into());
    }
    struct ProfileGuard(CmsProfile);
    impl Drop for ProfileGuard {
        fn drop(&mut self) {
            // SAFETY: opened by cmsOpenProfileFromMem, freed exactly once.
            unsafe { cmsCloseProfile(self.0) }
        }
    }
    let _guard = ProfileGuard(src_profile);

    // An EXPLICIT destination profile, not the NULL shorthand: lcms2 rejects
    // RGBA_8 as an output format when the destination is left NULL ("wrong
    // output color space"), which would make every real call fail. Found by
    // the positive-path test below, not by the refusal tests.
    //
    // SAFETY: factory function; closed exactly once by the guard below,
    // after the transform (which holds its own references) exists.
    let dst_profile = unsafe { cms_create_srgb() };
    if dst_profile.is_null() {
        return Err("could not create the sRGB destination profile".into());
    }
    struct DstProfileGuard(CmsProfile);
    impl Drop for DstProfileGuard {
        fn drop(&mut self) {
            // SAFETY: created above, freed exactly once.
            unsafe { cmsCloseProfile(self.0) }
        }
    }
    let _dst_guard = DstProfileGuard(dst_profile);

    // SAFETY: both handles are live.
    let transform = unsafe {
        cmsCreateTransform(
            src_profile,
            TYPE_RGBA_8,
            dst_profile,
            TYPE_RGBA_8,
            INTENT_PERCEPTUAL,
            0,
        )
    };
    if transform.is_null() {
        return Err("could not create colour transform".into());
    }
    struct TransformGuard(CmsTransform);
    impl Drop for TransformGuard {
        fn drop(&mut self) {
            // SAFETY: created by cmsCreateTransform, freed exactly once.
            unsafe { cmsDeleteTransform(self.0) }
        }
    }
    let _transform_guard = TransformGuard(transform);

    // cmsDoTransform counts PIXELS in a u32. Buffers beyond 2^32 pixels
    // (>16 GiB of RGBA) are transformed in chunks so the cast never wraps.
    const MAX_PIXELS_PER_CALL: usize = u32::MAX as usize;
    let pixel_bytes = MAX_PIXELS_PER_CALL.saturating_mul(4);
    for chunk in rgba.chunks_mut(pixel_bytes.max(1)) {
        let count = (chunk.len() / 4) as u32;
        if count == 0 {
            continue; // trailing partial pixel only
        }
        let pixels = chunk.as_mut_ptr();
        // SAFETY: chunk holds count * 4 whole pixels; the transform reads and
        // writes exactly that many, in place, while both guards are alive.
        unsafe { cmsDoTransform(transform, pixels, pixels, count) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;

    type LogFn = Option<unsafe extern "C" fn(*mut c_void, u32, *const c_char)>;

    extern "C" {
        fn cmsSetLogErrorHandler(fn_: LogFn);
    }

    unsafe extern "C" fn log_handler(_ctx: *mut c_void, code: u32, text: *const c_char) {
        let msg = if text.is_null() {
            String::new()
        } else {
            // SAFETY: static NUL-terminated string from lcms2's logger.
            unsafe { std::ffi::CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        };
        eprintln!("lcms2 error {code}: {msg}");
    }

    #[test]
    fn garbage_icc_returns_err_not_crash() {
        let mut pixels = vec![128u8; 16];
        assert!(apply_icc_profile(&mut pixels, b"not an icc file").is_err());
    }

    #[test]
    fn empty_icc_returns_err() {
        let mut pixels = vec![128u8; 16];
        assert!(apply_icc_profile(&mut pixels, &[]).is_err());
    }

    // -- Positive path -------------------------------------------------------
    //
    // The refusal tests above prove nothing about the transform itself. This
    // hand-builds a minimal valid matrix-shaper ICC profile DESCRIBING sRGB
    // (D50-adapted colourants, parametric sRGB response curves) and asserts
    // that transforming toward lcms2's built-in sRGB is a near identity â€”
    // which exercises cmsOpenProfileFromMem, cmsCreateTransform and
    // cmsDoTransform end to end, including the format constants above.

    /// IEEE 754 s15Fixed16, big-endian (ICC is big-endian throughout).
    fn s15f16(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }

    fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut t = b"XYZ ".to_vec();
        t.extend_from_slice(&[0; 4]); // reserved
        t.extend_from_slice(&s15f16(x));
        t.extend_from_slice(&s15f16(y));
        t.extend_from_slice(&s15f16(z));
        t
    }

    /// Parametric curve, ICC function type 3 - the sRGB device-to-PCS
    /// response: Y = c*X for X < d, else Y = (a*X + b)^g, with the standard
    /// sRGB decode constants (g = 2.4, a = 1/1.055, b = 0.0521...,
    /// c = 12.92, d = 0.00313...).
    fn srgb_curve_tag() -> Vec<u8> {
        let mut t = b"para".to_vec();
        t.extend_from_slice(&[0; 4]); // reserved
        t.extend_from_slice(&3u16.to_be_bytes()); // function type
        t.extend_from_slice(&[0; 2]); // reserved
        for p in [
            2.4,
            1.0 / 1.055,
            0.052_132_701_422,
            12.92,
            0.003_130_804_912,
        ] {
            t.extend_from_slice(&s15f16(p));
        }
        t
    }

    /// A minimal but valid monitor-class RGB matrix-shaper profile whose
    /// interpretation is sRGB.
    fn build_srgb_describing_profile() -> Vec<u8> {
        let tags: [(&[u8; 4], Vec<u8>); 7] = [
            (
                b"wtpt",
                xyz_tag(0.964_2, 1.0, 0.824_9), // D50 white point
            ),
            (b"rXYZ", xyz_tag(0.436_074_7, 0.222_504_5, 0.013_932_2)),
            (b"gXYZ", xyz_tag(0.385_064_9, 0.716_878_6, 0.097_104_5)),
            (b"bXYZ", xyz_tag(0.143_080_4, 0.060_616_9, 0.714_173_3)),
            (b"rTRC", srgb_curve_tag()),
            (b"gTRC", srgb_curve_tag()),
            (b"bTRC", srgb_curve_tag()),
        ];

        // Layout: 128-byte header | tag table | tag data (4-byte aligned).
        let table_off = 128;
        let table_size = 4 + tags.len() * 12;
        let mut offset = table_off + table_size;
        let mut entries = Vec::new();
        let mut data = Vec::new();
        for (sig, payload) in tags.iter() {
            entries.extend_from_slice(*sig);
            entries.extend_from_slice(&(offset as u32).to_be_bytes());
            entries.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            offset += payload.len(); // every payload here is a multiple of 4
            data.extend_from_slice(payload);
        }
        let total = table_off + table_size + data.len();

        let mut p = Vec::with_capacity(total);
        p.extend_from_slice(&(total as u32).to_be_bytes()); // profile size
        p.extend_from_slice(b"lcms"); // preferred CMM
        p.extend_from_slice(&0x0210_0000_u32.to_be_bytes()); // version 2.1
        p.extend_from_slice(b"mntr"); // device class: monitor
        p.extend_from_slice(b"RGB "); // data colour space
        p.extend_from_slice(b"XYZ "); // PCS
        p.extend_from_slice(&[0; 12]); // creation datetime
        p.extend_from_slice(b"acsp"); // the signature that makes it an ICC file
        p.extend_from_slice(&[0; 4]); // platform
        p.extend_from_slice(&[0; 4]); // flags
        p.extend_from_slice(&[0; 4]); // manufacturer
        p.extend_from_slice(&[0; 4]); // model
        p.extend_from_slice(&[0; 8]); // attributes
        p.extend_from_slice(&0u32.to_be_bytes()); // rendering intent
        p.extend_from_slice(&s15f16(0.964_2)); // illuminant X
        p.extend_from_slice(&s15f16(1.0)); // illuminant Y
        p.extend_from_slice(&s15f16(0.824_9)); // illuminant Z
        p.extend_from_slice(&[0; 4]); // creator
        p.extend_from_slice(&[0; 16]); // profile ID
        p.extend_from_slice(&[0; 28]); // reserved
        debug_assert_eq!(p.len(), table_off);
        p.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        p.extend_from_slice(&entries);
        p.extend_from_slice(&data);
        debug_assert_eq!(p.len(), total);
        p
    }

    #[test]
    fn builtin_srgb_to_builtin_srgb_is_identity() {
        // Pins the production CALL PATTERN (formats, in-place doTransform)
        // against lcms2's own reference profiles: sRGB -> sRGB must be a
        // perfect round trip. If this drifts, the bug is ours, not a
        // fixture's.
        unsafe {
            cmsSetLogErrorHandler(Some(log_handler));
        }
        let src = unsafe { cms_create_srgb() }; // SAFETY: factory fn, closed below.
        let dst = unsafe { cms_create_srgb() };
        assert!(!src.is_null() && !dst.is_null());

        let t =
            unsafe { cmsCreateTransform(src, TYPE_RGBA_8, dst, TYPE_RGBA_8, INTENT_PERCEPTUAL, 0) };
        assert!(!t.is_null(), "lcms2 refused its own sRGB pair");

        let mut input: Vec<u8> = Vec::new();
        let mut expect: Vec<u8> = Vec::new();
        for i in 0..=255u8 {
            input.extend_from_slice(&[i, i, i, 255]);
            expect.extend_from_slice(&[i, i, i, 255]);
        }
        let mut out = input.clone();
        let ptr = out.as_mut_ptr();
        // SAFETY: out holds 256 pixels of RGBA_8; read/write stay in bounds
        // while the transform is alive.
        unsafe { cmsDoTransform(t, input.as_ptr(), ptr, 256) };
        unsafe {
            cmsDeleteTransform(t);
            cmsCloseProfile(src);
            cmsCloseProfile(dst);
        }
        assert_eq!(out, expect, "sRGB -> sRGB through lcms2 must be exact");
    }

    #[test]
    fn srgb_profile_transform_is_near_identity() {
        unsafe {
            cmsSetLogErrorHandler(Some(log_handler));
        }

        let profile = build_srgb_describing_profile();

        // A grey gradient, opaque: R == G == B stays neutral under a
        // correct sRGB -> sRGB transform.
        let mut pixels = Vec::with_capacity(256 * 4);
        for i in 0..=255u8 {
            pixels.extend_from_slice(&[i, i, i, 255]);
        }
        let before = pixels.clone();

        apply_icc_profile(&mut pixels, &profile)
            .expect("a valid sRGB-describing profile must transform");

        for (old, new) in before.chunks_exact(4).zip(pixels.chunks_exact(4)) {
            for ch in 0..3 {
                assert!(
                    (i32::from(old[ch]) - i32::from(new[ch])).abs() <= 2,
                    "channel {ch} moved too far: {} -> {}",
                    old[ch],
                    new[ch]
                );
            }
            // Alpha must pass through untouched.
            assert_eq!(old[3], new[3], "alpha channel was modified");
        }
    }
}
