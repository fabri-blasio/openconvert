//! JPEG metadata stripping — **the first Class A operation**.
//!
//! Week 3, and it exists for two reasons at once.
//!
//! 1. **It gives the Class A gate a subject.** The gate "Class A conversions
//!    round-trip byte-identically" was scheduled at week 4 against operations
//!    that do not exist until weeks 15 and 27. A test that passes by testing
//!    nothing reads as coverage on a dashboard while providing none.
//! 2. **It gives A8/SR-11 its first real test.** Metadata leakage — GPS in
//!    EXIF, camera serial, author, edit history — had a written policy and
//!    nothing exercising it.
//!
//! # Why this is trivially lossless
//!
//! A JPEG is a sequence of marker segments. Stripping metadata means copying
//! every segment except the metadata ones. **The image is never decoded**, so
//! there is no encoder, no quantisation table rewrite, no colour conversion,
//! and no opportunity for a pixel to change. That is what makes it Class A by
//! construction rather than by measurement — and cheap to assert.

use std::io::{self, Read, Write};

/// Markers whose payload is metadata rather than image data.
const STRIPPED: &[u8] = &[
    0xE1, // APP1  -- EXIF (GPS, camera model, serial number, timestamps)
    0xE2, // APP2  -- ICC / FlashPix
    0xEC, // APP12 -- Picture Info / Ducky
    0xED, // APP13 -- Photoshop IRB, which carries IPTC authorship
    0xEE, // APP14 -- Adobe
    0xFE, // COM   -- free-text comment
];

/// Markers with no payload length: they stand alone.
fn is_standalone(marker: u8) -> bool {
    marker == 0x01 || (0xD0..=0xD9).contains(&marker)
}

/// Copy `src` to `dst`, dropping metadata segments.
///
/// Returns the markers removed, so the receipt can list **exactly** what was
/// taken out. SR-11 requires that: "metadata removed" without a list is not
/// something a user can verify.
///
/// # Errors
///
/// Fails if the input is not a JPEG, or on any underlying I/O error.
pub fn strip_jpeg<R: Read, W: Write>(src: &mut R, dst: &mut W) -> io::Result<Vec<&'static str>> {
    let mut data = Vec::new();
    src.read_to_end(&mut data)?;

    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a JPEG: missing SOI",
        ));
    }

    let mut removed = Vec::new();
    dst.write_all(&data[0..2])?; // SOI
    let mut i = 2;

    while i + 1 < data.len() {
        if data[i] != 0xFF {
            // Not at a marker boundary. Rather than guess, copy the remainder
            // verbatim: a strip that cannot parse must still not corrupt.
            dst.write_all(&data[i..])?;
            return Ok(removed);
        }
        let marker = data[i + 1];

        if marker == 0xD9 {
            dst.write_all(&data[i..])?; // EOI and anything trailing
            return Ok(removed);
        }
        if marker == 0xDA {
            // Start of scan: entropy-coded image data to the end. Copied
            // untouched -- this is the part that must be bit-identical.
            dst.write_all(&data[i..])?;
            return Ok(removed);
        }
        if is_standalone(marker) {
            dst.write_all(&data[i..i + 2])?;
            i += 2;
            continue;
        }

        if i + 4 > data.len() {
            dst.write_all(&data[i..])?;
            return Ok(removed);
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 || i + 2 + len > data.len() {
            // Truncated or nonsensical length. Copy the rest rather than
            // fabricate a segment boundary. Also the guard that stops a
            // declared length of zero looping forever.
            dst.write_all(&data[i..])?;
            return Ok(removed);
        }

        if STRIPPED.contains(&marker) {
            removed.push(marker_name(marker));
        } else {
            dst.write_all(&data[i..i + 2 + len])?;
        }
        i += 2 + len;
    }
    Ok(removed)
}

fn marker_name(m: u8) -> &'static str {
    match m {
        0xE1 => "APP1 (EXIF, including GPS)",
        0xE2 => "APP2 (ICC/FlashPix)",
        0xEC => "APP12 (Picture Info)",
        0xED => "APP13 (Photoshop/IPTC, including author)",
        0xEE => "APP14 (Adobe)",
        0xFE => "COM (comment)",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let len = (payload.len() + 2) as u16;
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// A JPEG carrying EXIF with GPS, an IPTC author, a comment, a
    /// quantisation table, a frame header, and scan data.
    fn jpeg_with_metadata() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend(segment(0xE1, b"Exif\x00\x00GPSLatitude=51.5074"));
        v.extend(segment(0xED, b"Photoshop3.0 author=Jane Doe"));
        v.extend(segment(0xFE, b"created with SomeApp"));
        v.extend(segment(0xDB, &[0x00; 64])); // quantisation table -- image data
        v.extend(segment(0xC0, &[0x08, 0x00, 0x10, 0x00, 0x10, 0x01])); // SOF0
        v.extend(vec![0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00]); // SOS
        v.extend(vec![0x12, 0x34, 0x56, 0x78]); // entropy-coded data
        v.extend(vec![0xFF, 0xD9]); // EOI
        v
    }

    fn strip(input: &[u8]) -> (Vec<u8>, Vec<&'static str>) {
        let mut out = Vec::new();
        let removed = strip_jpeg(&mut &input[..], &mut out).expect("strip");
        (out, removed)
    }

    /// A8/SR-11: the location really is gone.
    #[test]
    fn gps_and_author_are_removed() {
        let (out, removed) = strip(&jpeg_with_metadata());
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("GPSLatitude"), "GPS survived the strip");
        assert!(
            !text.contains("Jane Doe"),
            "the author name survived the strip"
        );
        assert!(!text.contains("SomeApp"), "the comment survived the strip");
        assert_eq!(
            removed.len(),
            3,
            "the receipt would under-report what was taken: {removed:?}"
        );
    }

    /// Class A: the image data is bit-identical.
    ///
    /// This is the assertion the week-4 gate rests on. The scan segment and
    /// everything after it survive byte for byte, because nothing decoded them.
    #[test]
    fn image_data_is_bit_identical() {
        let input = jpeg_with_metadata();
        let (out, _) = strip(&input);

        let sos_in = input
            .windows(2)
            .position(|w| w == [0xFF, 0xDA])
            .expect("SOS in input");
        let sos_out = out
            .windows(2)
            .position(|w| w == [0xFF, 0xDA])
            .expect("SOS in output");
        assert_eq!(
            &input[sos_in..],
            &out[sos_out..],
            "the entropy-coded image data changed -- this is not Class A"
        );
    }

    /// Non-metadata segments survive.
    ///
    /// The control. Without it, a strip that removed *everything* would satisfy
    /// every assertion above while destroying the image.
    #[test]
    fn image_segments_survive() {
        let (out, _) = strip(&jpeg_with_metadata());
        assert!(
            out.windows(2).any(|w| w == [0xFF, 0xDB]),
            "the quantisation table was removed"
        );
        assert!(
            out.windows(2).any(|w| w == [0xFF, 0xC0]),
            "the frame header was removed"
        );
        assert!(out.starts_with(&[0xFF, 0xD8]), "SOI missing");
    }

    /// Stripping twice changes nothing the second time.
    #[test]
    fn stripping_is_idempotent() {
        let (once, _) = strip(&jpeg_with_metadata());
        let (twice, removed) = strip(&once);
        assert_eq!(once, twice, "a second strip altered the file");
        assert!(
            removed.is_empty(),
            "a second strip claimed to remove {removed:?}"
        );
    }

    /// A JPEG with no metadata is returned unchanged.
    #[test]
    fn a_clean_jpeg_is_untouched() {
        let mut v = vec![0xFF, 0xD8];
        v.extend(segment(0xDB, &[0x00; 64]));
        v.extend(vec![0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00]);
        v.extend(vec![0xAA, 0xBB, 0xFF, 0xD9]);
        let (out, removed) = strip(&v);
        assert_eq!(out, v, "a clean JPEG was modified");
        assert!(removed.is_empty());
    }

    #[test]
    fn non_jpeg_is_refused() {
        let mut out = Vec::new();
        let err = strip_jpeg(&mut &b"\x89PNG\r\n\x1a\n"[..], &mut out).expect_err("should refuse");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// Truncated and malformed input must not panic, and must not corrupt.
    ///
    /// This reads attacker-supplied bytes, so "does not panic" is a security
    /// property rather than a robustness nicety.
    #[test]
    fn malformed_input_never_panics() {
        let full = jpeg_with_metadata();
        for cut in 0..full.len() {
            let mut out = Vec::new();
            let _ = strip_jpeg(&mut &full[..cut], &mut out);
        }
        // A segment claiming a length past the end of the buffer.
        let mut out = Vec::new();
        let _ = strip_jpeg(
            &mut &[0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, 0x41][..],
            &mut out,
        );
        // A declared length of zero, which an unguarded loop would spin on.
        out.clear();
        let _ = strip_jpeg(
            &mut &[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x00, 0x41][..],
            &mut out,
        );
        // A marker byte with nothing after it.
        out.clear();
        let _ = strip_jpeg(&mut &[0xFF, 0xD8, 0xFF][..], &mut out);
    }
}
