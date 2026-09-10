use openconvert_core::policy::Policy;
use openconvert_run::tools::{run_tool_with, ToolRunOptions};
use std::path::Path;

#[test]
fn high_compression_reduces_photo_bytes_and_keeps_dimensions() {
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    openconvert_run::engine_dir::set_resource_dir(profile);
    let dir = std::env::temp_dir().join(format!(
        "openconvert-compression-levels-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let image = image::RgbImage::from_fn(256, 256, |x, y| {
        image::Rgb([x as u8, y as u8, ((x * 7 + y * 11) % 256) as u8])
    });
    let input = dir.join("photo.jpg");
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
        .encode_image(&image)
        .unwrap();
    std::fs::write(&input, &bytes).unwrap();
    let run = |quality: &str| {
        run_tool_with(
            "image-compress",
            std::slice::from_ref(&input),
            &[("quality".into(), quality.into())],
            &Policy::default(),
            ToolRunOptions {
                write_sidecar: false,
            },
        )
        .unwrap()
    };
    let low = run("85");
    let high = run("20");
    let high_bytes = std::fs::read(&high.output).unwrap();
    assert!(high_bytes.len() < std::fs::metadata(&low.output).unwrap().len() as usize);
    assert!(high_bytes.len() < bytes.len() / 2);
    let decoded = image::load_from_memory(&high_bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 256));
    assert_eq!(
        high.output.extension().and_then(|s| s.to_str()),
        Some("jpg")
    );
    assert!(Path::new(&high.output).exists());
}

#[test]
fn saving_compression_keeps_png_at_every_level() {
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    openconvert_run::engine_dir::set_resource_dir(profile);
    let dir = std::env::temp_dir().join(format!(
        "openconvert-png-compression-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("transparent.png");
    let pixels = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([x as u8, y as u8, 128, (x * 4) as u8])
    });
    pixels.save(&input).unwrap();
    let before = std::fs::metadata(&input).unwrap().len();
    for quality in ["85", "65", "20"] {
        let result = run_tool_with(
            "image-compress",
            std::slice::from_ref(&input),
            &[("quality".into(), quality.into())],
            &Policy::default(),
            ToolRunOptions {
                write_sidecar: false,
            },
        )
        .unwrap();
        assert_eq!(result.output.extension().unwrap(), "png");
        let bytes = std::fs::read(&result.output).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(bytes.len() as u64 <= before);
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), pixels);
    }
}

fn compressible_pdf(pages: usize) -> Vec<u8> {
    let kids: String = (0..pages)
        .map(|i| format!("{} 0 R", 3 + 2 * i))
        .collect::<Vec<_>>()
        .join(" ");
    let font = 3 + 2 * pages;

    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    ];
    for i in 0..pages {
        let content = format!(
            "BT\n/F1 24 Tf\n1 0 0 1 72 700 Tm\n(Page {}) Tj\nET\n",
            i + 1
        )
        .repeat(100);
        bodies.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {} 0 R >>",
            4 + 2 * i
        ));
        bodies.push(format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ));
    }
    bodies.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    let mut out = Vec::from(b"%PDF-1.4\n");
    let mut offsets = vec![0u32; bodies.len() + 1];
    for (i, body) in bodies.iter().enumerate() {
        let num = i + 1;
        offsets[num] = out.len() as u32;
        out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for slot in &offsets[1..] {
        out.extend_from_slice(format!("{slot:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out.extend_from_slice(format!("{xref_at}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn pdf_compression_saves_smaller_readable_documents_at_every_level() {
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    openconvert_run::engine_dir::set_resource_dir(profile);
    let dir = std::env::temp_dir().join(format!(
        "openconvert-pdf-compression-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.pdf");
    let source = compressible_pdf(3);
    std::fs::write(&input, &source).unwrap();
    for quality in ["lossless", "85", "45"] {
        let result = run_tool_with(
            "pdf-compress",
            std::slice::from_ref(&input),
            &[("quality".into(), quality.into())],
            &Policy::default(),
            ToolRunOptions {
                write_sidecar: false,
            },
        )
        .unwrap();
        let bytes = std::fs::read(&result.output).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() < source.len() / 2, "PDF must actually shrink");
        let text = run_tool_with(
            "pdf-text",
            std::slice::from_ref(&result.output),
            &[],
            &Policy::default(),
            ToolRunOptions {
                write_sidecar: false,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(text.output).unwrap();
        for page in 1..=3 {
            assert!(
                text.contains(&format!("Page {page}")),
                "missing page {page}"
            );
        }
        eprintln!("PDF {quality}: {} -> {} bytes", source.len(), bytes.len());
    }
}

#[test]
fn png_savings_depend_on_image_content() {
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    openconvert_run::engine_dir::set_resource_dir(profile);
    let dir = std::env::temp_dir().join(format!("openconvert-png-content-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut sizes = Vec::new();
    for noisy in [false, true] {
        let mut seed = 123456789u32;
        let pixels = image::RgbImage::from_fn(128, 128, |_, _| {
            if noisy {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                image::Rgb([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8])
            } else {
                image::Rgb([70, 90, 110])
            }
        });
        let input = dir.join(format!("image-{noisy}.png"));
        pixels.save(&input).unwrap();
        let result = run_tool_with(
            "image-compress",
            std::slice::from_ref(&input),
            &[("quality".into(), "20".into())],
            &Policy::default(),
            ToolRunOptions {
                write_sidecar: false,
            },
        )
        .unwrap();
        let before = std::fs::metadata(&input).unwrap().len();
        let after = std::fs::metadata(&result.output).unwrap().len();
        assert!(after <= before);
        sizes.push((before, after));
        eprintln!("PNG noisy={noisy}: {before} -> {after} bytes");
    }
    assert_ne!(
        sizes[0].1 * sizes[1].0,
        sizes[1].1 * sizes[0].0,
        "savings must not be a fixed ratio"
    );
}
