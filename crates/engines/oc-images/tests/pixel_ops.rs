//! Invert and greyscale, through the **real worker binary**.
//!
//! These assert on pixels rather than on "it returned something": an operation
//! that silently did nothing would still hand back a valid PNG of the right
//! size, and a test that only checked the bytes decoded would pass while the
//! feature did not exist. That is the exact failure mode the unrecognised-
//! parameter rule exists to prevent, so it is what is checked.

use openconvert_core::limits::Limits;
use openconvert_run::worker_client::Worker;

const TX_IMAGES: &str = env!("CARGO_BIN_EXE_oc-images");

fn start() -> Worker {
    Worker::start_at(TX_IMAGES.as_ref(), "oc-images", &Limits::defaults()).expect("start oc-images")
}

#[test]
fn png_compression_preserves_bit_depth_and_colour_channels() {
    let img = image::DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(16, 16, |x, y| {
        image::Rgb([(x * 3073) as u16, (y * 4093) as u16, (x * y * 127) as u16])
    }));
    let mut encoded = std::io::Cursor::new(Vec::new());
    img.write_to(&mut encoded, image::ImageFormat::Png).unwrap();
    let (out, _) = start()
        .run(
            encoded.into_inner(),
            "png",
            &Limits::defaults(),
            &[("op".into(), "compress".into())],
        )
        .unwrap();
    let decoded = image::load_from_memory(&out).unwrap();
    assert_eq!(decoded.color(), image::ColorType::Rgb16);
    assert_eq!(decoded.as_bytes(), img.as_bytes());
}

#[test]
fn jpeg_compression_encodes_at_the_selected_quality_once() {
    let img = image::RgbImage::from_fn(32, 32, |x, y| {
        image::Rgb([(x * 7) as u8, (y * 5) as u8, (x * y) as u8])
    });
    let mut source = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut source, 98)
        .encode_image(&img)
        .unwrap();
    let decoded = image::load_from_memory(&source).unwrap().to_rgb8();
    let expected = oc_image_codecs::jpeg(
        decoded.as_raw(),
        decoded.width() as usize,
        decoded.height() as usize,
        85,
    )
    .unwrap();
    let (out, _) = start()
        .run(
            source,
            "jpeg",
            &Limits::defaults(),
            &[
                ("op".into(), "compress".into()),
                ("quality".into(), "85".into()),
            ],
        )
        .unwrap();
    assert_eq!(
        out, expected,
        "compression must not pre-encode at the default quality"
    );
}

/// A 2x2 PNG with known, deliberately asymmetric colours.
fn source() -> Vec<u8> {
    let mut img = image::RgbaImage::new(2, 2);
    img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255])); // red
    img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255])); // green
    img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255])); // blue
    img.put_pixel(1, 1, image::Rgba([10, 20, 30, 128])); // partly transparent
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode the fixture");
    out.into_inner()
}

fn decode(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes)
        .expect("the worker returned something that is not an image")
        .to_rgba8()
}

fn run(op: &str) -> Vec<u8> {
    let mut worker = start();
    let (out, _) = worker
        .run(
            source(),
            "png",
            &Limits::defaults(),
            &[("op".to_string(), op.to_string())],
        )
        .unwrap_or_else(|e| panic!("the worker refused {op}: {e}"));
    out
}

/// Every colour channel flips; **alpha does not**.
///
/// Inverting alpha would turn an opaque image transparent, which nobody means
/// by "invert colours" — so it is asserted rather than left to be noticed.
#[test]
fn invert_flips_colour_and_leaves_alpha_alone() {
    let img = decode(&run("invert"));
    assert_eq!(
        img.get_pixel(0, 0).0,
        [0, 255, 255, 255],
        "red becomes cyan"
    );
    assert_eq!(
        img.get_pixel(1, 0).0,
        [255, 0, 255, 255],
        "green becomes magenta"
    );
    assert_eq!(
        img.get_pixel(0, 1).0,
        [255, 255, 0, 255],
        "blue becomes yellow"
    );
    let corner = img.get_pixel(1, 1).0;
    assert_eq!([corner[0], corner[1], corner[2]], [245, 235, 225]);
    assert_eq!(corner[3], 128, "alpha must survive untouched");
}

/// Inverting twice returns the original pixels exactly.
///
/// This is the property that makes the route Class A. If it ever stops holding,
/// the class is wrong and the receipt would be claiming a lossless operation
/// that is not one.
#[test]
fn invert_is_its_own_inverse() {
    let once = run("invert");
    let mut worker = start();
    let (twice, _) = worker
        .run(
            once,
            "png",
            &Limits::defaults(),
            &[("op".to_string(), "invert".to_string())],
        )
        .expect("second invert");
    assert_eq!(
        decode(&twice).into_raw(),
        decode(&source()).into_raw(),
        "invert applied twice must reproduce the original pixels exactly"
    );
}

/// Colour is gone, and the three channels agree.
#[test]
fn greyscale_removes_colour() {
    let img = decode(&run("greyscale"));
    for (x, y, px) in img.enumerate_pixels() {
        assert_eq!(
            px.0[0], px.0[1],
            "pixel {x},{y} still has colour: {:?}",
            px.0
        );
        assert_eq!(px.0[1], px.0[2], "pixel {x},{y} still has colour");
    }
    // Luminance-weighted, not a mean: pure green is much brighter than pure
    // blue. A mean would make both 85.
    let green = img.get_pixel(1, 0).0[0];
    let blue = img.get_pixel(0, 1).0[0];
    assert!(
        green > blue + 40,
        "greyscale should be luminance-weighted; green {green} vs blue {blue}"
    );
    assert_eq!(img.get_pixel(1, 1).0[3], 128, "alpha must survive");
}

/// An operation the engine does not know is refused, not ignored.
#[test]
fn an_unknown_operation_is_refused() {
    let mut worker = start();
    let err = worker
        .run(
            source(),
            "png",
            &Limits::defaults(),
            &[("op".to_string(), "solarise".to_string())],
        )
        .expect_err("an unknown operation must fail rather than pass the image through");
    assert!(
        err.to_string().contains("solarise"),
        "the refusal must name the operation, got {err}"
    );
}

#[test]
fn compression_preserves_formats_and_rejects_cross_format_requests() {
    let mut worker = start();
    let params = [
        ("op".into(), "compress".into()),
        ("quality".into(), "20".into()),
    ];
    assert!(worker
        .run(source(), "avif", &Limits::defaults(), &params)
        .unwrap_err()
        .to_string()
        .contains("preserve"));
    let rgba = decode(&source());
    for (format, target) in [
        (image::ImageFormat::Png, "png"),
        (image::ImageFormat::WebP, "webp"),
        (image::ImageFormat::Bmp, "bmp"),
        (image::ImageFormat::Tiff, "tiff"),
    ] {
        let mut bytes = std::io::Cursor::new(Vec::new());
        rgba.write_to(&mut bytes, format).unwrap();
        let original = decode(bytes.get_ref());
        let (compressed, _) = worker
            .run(bytes.into_inner(), target, &Limits::defaults(), &params)
            .unwrap();
        assert_eq!(image::guess_format(&compressed).unwrap(), format);
        assert_eq!(decode(&compressed), original);
    }
    let (avif, _) = worker
        .run(source(), "avif", &Limits::defaults(), &[])
        .unwrap();
    let (compressed, _) = worker
        .run(avif, "avif", &Limits::defaults(), &params)
        .unwrap();
    assert!(compressed.windows(4).any(|b| b == b"avif"));
    let (png, _) = worker
        .run(compressed, "png", &Limits::defaults(), &[])
        .unwrap();
    assert!(decode(&png).get_pixel(1, 1).0[3] < 240);
}

#[test]
fn compression_preserves_gif_frames_and_unsupported_native_formats() {
    let mut gif = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    8,
                    8,
                    image::Rgba(color),
                )))
                .unwrap();
        }
    }
    let params = [("op".into(), "compress".into())];
    let (out, _) = start()
        .run(gif.clone(), "gif", &Limits::defaults(), &params)
        .unwrap();
    assert_eq!(out, gif);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#.to_vec();
    let (out, _) = start()
        .run(svg.clone(), "svg", &Limits::defaults(), &params)
        .unwrap();
    assert_eq!(out, svg);
}

#[test]
fn tiff_compression_is_lossless_and_preserves_multiple_pages() {
    let params = [("op".into(), "compress".into())];
    let image = image::RgbImage::from_pixel(128, 128, image::Rgb([40, 80, 120]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Tiff)
        .unwrap();
    let source = bytes.into_inner();
    let (out, _) = start()
        .run(source.clone(), "tiff", &Limits::defaults(), &params)
        .unwrap();
    assert!(out.len() < source.len() / 2);
    assert_eq!(decode(&out), decode(&source));
    let mut multipage = std::io::Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut multipage).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(128, 128, image.as_raw())
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(128, 128, image.as_raw())
            .unwrap();
    }
    let original = multipage.into_inner();
    let (out, _) = start()
        .run(original.clone(), "tiff", &Limits::defaults(), &params)
        .unwrap();
    assert_eq!(out, original);
}

#[test]
fn high_png_compression_preserves_transparency_and_pixel_values() {
    let img = image::RgbaImage::from_fn(80, 60, |x, y| {
        image::Rgba([(x * 3) as u8, (y * 4) as u8, 37, ((x + y) * 2) as u8])
    });
    let mut source = std::io::Cursor::new(Vec::new());
    img.write_to(&mut source, image::ImageFormat::Png).unwrap();
    let input = source.into_inner();
    let mut worker = start();
    for quality in ["85", "65", "20"] {
        let (out, _) = worker
            .run(
                input.clone(),
                "png",
                &Limits::defaults(),
                &[
                    ("op".into(), "compress".into()),
                    ("quality".into(), quality.into()),
                ],
            )
            .unwrap();
        assert_eq!(image::guess_format(&out).unwrap(), image::ImageFormat::Png);
        assert_eq!(image::load_from_memory(&out).unwrap().to_rgba8(), img);
    }
}
