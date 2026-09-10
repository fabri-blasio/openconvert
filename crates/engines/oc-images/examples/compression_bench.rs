//! Reproducible codec comparison; synthetic fixtures, not a visual-quality verdict.
//! cargo run --release -p oc-images --example compression_bench
use image::{ImageEncoder, RgbImage};
use std::time::Instant;
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse = a
        .iter()
        .zip(b)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}
fn main() {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    if !paths.is_empty() {
        for path in paths {
            let source = std::fs::read(&path).expect("read PNG fixture");
            let original = image::load_from_memory(&source).unwrap().to_rgba16();
            for q in [85, 65, 20] {
                let started = Instant::now();
                let out = oc_image_codecs::png(&source, q).unwrap();
                assert_eq!(image::load_from_memory(&out).unwrap().to_rgba16(), original);
                println!(
                    "PNG {path} q={q} before={} after={} ms={}",
                    source.len(),
                    out.len(),
                    started.elapsed().as_millis()
                );
            }
        }
        return;
    }
    for kind in 0..4 {
        let image = RgbImage::from_fn(512, 384, |x, y| {
            image::Rgb(match kind {
                0 => [(x / 2) as u8, (y * 2 / 3) as u8, ((x + y) / 4) as u8],
                1 => {
                    let v = if x % 64 < 2 || y % 48 < 2 { 0 } else { 255 };
                    [v, v, v]
                }
                2 => {
                    let n = (x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)).rotate_left(13);
                    [n as u8, (n >> 8) as u8, (n >> 16) as u8]
                }
                _ => {
                    let v = if x > 60 && x < 440 && y % 32 < 10 {
                        40
                    } else {
                        245
                    };
                    [v, v, v]
                }
            })
        });
        let mut source = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut source,
            image::codecs::png::CompressionType::Best,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(image.as_raw(), 512, 384, image::ExtendedColorType::Rgb8)
        .unwrap();
        for q in [85, 65, 25] {
            let t = Instant::now();
            let out = oc_image_codecs::png(&source, q).unwrap();
            assert_eq!(image::load_from_memory(&out).unwrap().to_rgb8(), image);
            println!(
                "PNG fixture={kind} q={q} before={} after={} ms={}",
                source.len(),
                out.len(),
                t.elapsed().as_millis()
            );
        }
        let mut legacy = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut legacy, 75)
            .encode_image(&image)
            .unwrap();
        let target = psnr(
            image.as_raw(),
            image::load_from_memory(&legacy).unwrap().to_rgb8().as_raw(),
        );
        let mut best = None;
        let t = Instant::now();
        for q in 1..=100 {
            let out = oc_image_codecs::jpeg(image.as_raw(), 512, 384, q).unwrap();
            let quality = psnr(
                image.as_raw(),
                image::load_from_memory(&out).unwrap().to_rgb8().as_raw(),
            );
            if quality >= target && best.as_ref().is_none_or(|(len, _, _)| out.len() < *len) {
                best = Some((out.len(), q, quality));
            }
        }
        println!("JPEG fixture={kind} legacy_bytes={} legacy_psnr={target:.2} mozjpeg_matched={best:?} sweep_ms={}",legacy.len(),t.elapsed().as_millis());
    }
}
