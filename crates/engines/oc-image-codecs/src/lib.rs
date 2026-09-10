//! Image optimizers linked only into confined workers.
pub fn jpeg(rgb: &[u8], width: usize, height: usize, quality: u8) -> Result<Vec<u8>, String> {
    let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    encoder.set_size(width, height);
    encoder.set_quality(f32::from(quality));
    encoder.set_progressive_mode();
    let mut encoder = encoder
        .start_compress(Vec::new())
        .map_err(|e| e.to_string())?;
    encoder.write_scanlines(rgb).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}
pub fn png(bytes: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    let mut options = oxipng::Options::from_preset(if quality <= 25 {
        6
    } else if quality <= 65 {
        4
    } else {
        2
    });
    options.timeout = Some(std::time::Duration::from_secs(15));
    let mut out = oxipng::optimize_from_memory(bytes, &options).map_err(|e| e.to_string())?;
    if quality <= 25 {
        // Refine the already optimized representation without changing pixels.
        let mut stronger = oxipng::Options::from_preset(0);
        stronger.deflater = oxipng::Deflater::Zopfli(oxipng::ZopfliOptions {
            iteration_count: std::num::NonZeroU64::new(1).unwrap(),
            ..Default::default()
        });
        stronger.timeout = Some(std::time::Duration::from_secs(15));
        if let Ok(candidate) = oxipng::optimize_from_memory(&out, &stronger) {
            if candidate.len() < out.len() {
                out = candidate;
            }
        }
    }
    Ok(if out.len() < bytes.len() {
        out
    } else {
        bytes.to_vec()
    })
}
