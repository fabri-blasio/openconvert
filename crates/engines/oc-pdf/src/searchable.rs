use lopdf::{dictionary, Document, Object, Stream};
use openconvert_worker::RunLimits;
use std::collections::BTreeMap;

/// Keep the scan as an image and add invisible, Unicode-mapped OCR glyphs.
pub fn write(png: &[u8], layout: &[u8], limits: &RunLimits) -> Result<Vec<u8>, String> {
    if layout.len() > 4 << 20 {
        return Err("OCR layout exceeds the text limit".into());
    }
    let data: serde_json::Value = serde_json::from_slice(layout).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut image_limits = image::Limits::default();
    image_limits.max_alloc = Some(limits.memory_bytes);
    reader.limits(image_limits);
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if w == 0 || h == 0 || w as u64 * h as u64 > limits.decode_pixels {
        return Err("Scan exceeds the pixel limit".into());
    }
    if data["width"].as_u64() != Some(w as u64) || data["height"].as_u64() != Some(h as u64) {
        return Err("OCR and image dimensions differ".into());
    }
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    for pixel in reader
        .decode()
        .map_err(|e| e.to_string())?
        .to_rgba8()
        .pixels()
    {
        for c in &pixel.0[..3] {
            rgb.push(((*c as u16 * pixel[3] as u16 + 255 * (255 - pixel[3] as u16)) / 255) as u8);
        }
    }
    let lines = data["lines"].as_array().ok_or("Missing OCR lines")?;
    if lines.is_empty() {
        return Err("No text was recognized in this scan".into());
    }
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let mut stream = Stream::new(
        dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>w,"Height"=>h,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},
        rgb,
    );
    stream.compress().map_err(|e| e.to_string())?;
    let image = doc.add_object(stream);
    let chars: Vec<char> = lines
        .iter()
        .filter_map(|l| l["text"].as_str())
        .flat_map(str::chars)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut lookup = BTreeMap::new();
    let mut fonts = lopdf::Dictionary::new();
    for (font_index, group) in chars.chunks(250).enumerate() {
        let name = format!("F{font_index}");
        let mut differences = vec![Object::Integer(1)];
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /OCR def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
        );
        // Invisible base-14 glyphs need no embedded font; ToUnicode maps their
        // codes to the recognized text, including non-Latin characters.
        // PDF CMaps permit at most 100 entries in a bfchar block.
        for (chunk_index, chunk) in group.chunks(100).enumerate() {
            cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (local, ch) in chunk.iter().enumerate() {
                let code = chunk_index * 100 + local + 1;
                differences.push(Object::Name(b"A".to_vec()));
                lookup.insert(*ch, (name.clone(), code as u8));
                let utf16 = ch
                    .encode_utf16(&mut [0; 2])
                    .iter()
                    .map(|u| format!("{u:04X}"))
                    .collect::<String>();
                cmap.push_str(&format!("<{code:02X}> <{utf16}>\n"));
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let font=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica","Encoding"=>dictionary!{"Type"=>"Encoding","Differences"=>differences},"FirstChar"=>1,"LastChar"=>group.len() as i64,"Widths"=>vec![Object::Integer(1000);group.len()],"Resources"=>dictionary!{},"ToUnicode"=>unicode});
        fonts.set(name, font);
    }
    let scale = (14_000.0 / w.max(h) as f32).min(0.75);
    let width = w as f32 * scale;
    let height = h as f32 * scale;
    let mut content = format!("q {width} 0 0 {height} 0 0 cm /Scan Do Q\n");
    for line in lines {
        let text = line["text"].as_str().ok_or("Invalid OCR text")?;
        let x = line["x"].as_f64().ok_or("Invalid OCR x")? as f32;
        let y = line["y"].as_f64().ok_or("Invalid OCR y")? as f32;
        let lw = line["width"].as_f64().ok_or("Invalid OCR width")? as f32;
        let lh = line["height"].as_f64().ok_or("Invalid OCR height")? as f32;
        if ![x, y, lw, lh].iter().all(|n| n.is_finite() && *n >= 0.0)
            || lw == 0.0
            || lh == 0.0
            || x + lw > w as f32 + 1.0
            || y + lh > h as f32 + 1.0
        {
            return Err("Invalid OCR rectangle".into());
        }
        let count = text.chars().count().max(1) as f32;
        content.push_str(&format!(
            "BT 3 Tr {} Tz 1 0 0 1 {} {} Tm\n",
            lw / (count * lh) * 100.0,
            x * scale,
            (h as f32 - y - lh) * scale
        ));
        for ch in text.chars() {
            let (font, code) = &lookup[&ch];
            content.push_str(&format!("/{font} {} Tf <{code:02X}> Tj\n", lh * scale));
        }
        content.push_str("ET\n");
    }
    let content = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),width.into(),height.into()],"Resources"=>dictionary!{"XObject"=>dictionary!{"Scan"=>image},"Font"=>fonts},"Contents"=>content});
    doc.objects.insert(
        pages,
        Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1},
        ),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_has_invisible_unicode_text_and_original_pixels() {
        let limits = RunLimits {
            decode_pixels: 1_000_000,
            memory_bytes: 128 << 20,
            archive_depth: 0,
            archive_entries: 0,
            archive_total_bytes: 0,
            use_gpu: false,
        };
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            400,
            200,
            image::Rgb([230, 240, 250]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let layout=serde_json::to_vec(&serde_json::json!({"width":400,"height":200,"lines":[{"x":10,"y":20,"width":200,"height":30,"text":"Hello café 世界"}]})).unwrap();
        let bytes = write(png.get_ref(), &layout, &limits).unwrap();
        let doc = Document::load_mem(&bytes).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1);
        let content = String::from_utf8(doc.get_page_content(pages[&1])).unwrap();
        assert!(content.contains("3 Tr"));
        assert!(content.contains("/Scan Do"));
        #[cfg(windows)]
        {
            crate::pdfium::init_library();
            let (doc, _) = crate::pdfium::open_document(&bytes).unwrap();
            let chars = crate::pdfium::page_chars(&doc, 0).unwrap();
            let text = chars.iter().map(|c| c.ch).collect::<String>();
            assert!(text.contains("Hello café 世界"), "{text:?}");
        }
    }
}
