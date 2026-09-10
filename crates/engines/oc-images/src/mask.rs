use image::{DynamicImage, ImageFormat};
use openconvert_worker::{Converted, RunLimits};
use serde::Deserialize;
#[derive(Deserialize)]
pub struct Stroke {
    pub restore: bool,
    pub radius: f32,
    pub points: Vec<[f32; 2]>,
}
pub fn apply(
    cutout: &[u8],
    original: &[u8],
    json: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    if json.len() > 2 << 20 {
        return Err("Too many brush strokes".into());
    }
    let strokes: Vec<Stroke> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if strokes.len() > 500 || strokes.iter().map(|s| s.points.len()).sum::<usize>() > 30_000 {
        return Err("Too many brush points".into());
    }
    let decode = |bytes: &[u8]| -> Result<DynamicImage, String> {
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
        if w as u64 * h as u64 > limits.decode_pixels {
            return Err("Image exceeds the pixel limit".into());
        }
        crate::decode_generic(bytes, w, h).map(|(image, _, _)| image)
    };
    let image = decode(cutout)?;
    let source = decode(original)?;
    let mut image = image.to_rgba8();
    let source = source.to_rgba8();
    if image.dimensions() != source.dimensions() {
        return Err("The original and cutout have different dimensions".into());
    }
    let (w, h) = image.dimensions();
    for stroke in strokes {
        if !stroke.radius.is_finite()
            || !(0.001..=0.2).contains(&stroke.radius)
            || stroke
                .points
                .iter()
                .flatten()
                .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        {
            return Err("Invalid brush coordinates".into());
        }
        let radius = (stroke.radius * w.min(h) as f32).max(1.0);
        for (index, point) in stroke.points.iter().enumerate() {
            let from = if index > 0 {
                stroke.points[index - 1]
            } else {
                *point
            };
            let dx = (point[0] - from[0]) * w as f32;
            let dy = (point[1] - from[1]) * h as f32;
            let steps = ((dx.hypot(dy) / (radius * 0.5)).ceil() as usize).max(1);
            for step in 0..=steps {
                let x = from[0] * w as f32 + dx * step as f32 / steps as f32;
                let y = from[1] * h as f32 + dy * step as f32 / steps as f32;
                for py in (y - radius).max(0.0) as u32..((y + radius).ceil() as u32).min(h) {
                    for px in (x - radius).max(0.0) as u32..((x + radius).ceil() as u32).min(w) {
                        if (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y) <= radius {
                            if stroke.restore {
                                *image.get_pixel_mut(px, py) = *source.get_pixel(px, py);
                            } else {
                                image.get_pixel_mut(px, py)[3] = 0;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(Converted {
        bytes: out.into_inner(),
        removed: vec!["transparency manually corrected with erase/restore strokes".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(alpha: u8) -> Vec<u8> {
        let mut c = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            20,
            20,
            image::Rgba([100, 150, 200, alpha]),
        ))
        .write_to(&mut c, ImageFormat::Png)
        .unwrap();
        c.into_inner()
    }
    #[test]
    fn restore_and_erase_only_brushed_pixels() {
        let limits = RunLimits {
            decode_pixels: 10000,
            memory_bytes: 16 << 20,
            archive_depth: 0,
            archive_entries: 0,
            archive_total_bytes: 0,
            use_gpu: false,
        };
        let json = r#"[{"restore":true,"radius":0.1,"points":[[0.5,0.5]]},{"restore":false,"radius":0.05,"points":[[0.5,0.5]]}]"#;
        let out = apply(&png(0), &png(255), json, &limits).unwrap();
        let image = image::load_from_memory(&out.bytes).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(10, 10)[3], 0);
        assert_eq!(image.get_pixel(11, 10)[3], 255);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert!(apply(
            &png(0),
            &png(255),
            r#"[{"restore":true,"radius":2,"points":[[0.5,0.5]]}]"#,
            &limits
        )
        .is_err());
    }
}
