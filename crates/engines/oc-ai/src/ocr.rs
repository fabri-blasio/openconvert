//! PaddleOCR: text detection, then recognition, then plain text out.
//!
//! Three artifacts, because that is what OCR is: a detection network that says
//! WHERE text is, a recognition network that says WHAT it says, and the
//! character dictionary the second one's output indexes into. The registry
//! pins them separately and the step names all three.
//!
//! # The two halves need different pre-processing, and mixing them up is silent
//!
//! Detection normalises with ImageNet mean/std; recognition normalises to
//! [-1, 1]. Both take `[N,3,H,W]` float tensors, so feeding one the other's
//! normalisation produces no error at all — just worse output. They are kept
//! in separate functions here for that reason.

use image::{DynamicImage, GenericImageView, RgbImage};
use ndarray::Array4;
use ort::session::Session;

use crate::accelerator::Workload;
use crate::infer::{run_for, run_one};

/// Longest side the detector sees.
///
/// PP-OCR's detector is fully convolutional, so this is a cost/accuracy knob
/// rather than a fixed requirement. 960 is upstream's default for the mobile
/// model and the size its thresholds were tuned at.
const DET_MAX_SIDE: u32 = 960;

/// The detector's stride. Inputs must be a multiple of it in both dimensions
/// or the network's downsample/upsample path does not line up.
const DET_STRIDE: u32 = 32;

/// Probability above which a pixel is considered text.
const DET_THRESHOLD: f32 = 0.3;

/// Recognition input height, fixed by the model.
const REC_HEIGHT: u32 = 48;

/// Widest recognition input. A very long line is downscaled rather than
/// refused: the alternative is dropping it, and a squashed transcription beats
/// a missing one.
const REC_MAX_WIDTH: u32 = 1600;

/// Smallest box worth recognising, in detector pixels. Below this a "box" is
/// speckle, and running recognition on speckle produces confident nonsense.
const MIN_BOX_SIDE: u32 = 3;

/// A detected text box in ORIGINAL image coordinates.
#[derive(Debug, Clone, Copy)]
struct Box {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// Read text out of an image.
///
/// # Errors
///
/// A message the host shows the user directly.
#[derive(serde::Serialize)]
pub struct Line {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub text: String,
}
pub fn recognise(
    image: &DynamicImage,
    det_model: &[u8],
    rec_model: &[u8],
    charset: &[u8],
) -> Result<String, String> {
    let lines = recognise_layout(image, det_model, rec_model, charset)?;
    let mut text = String::new();
    let mut previous: Option<&Line> = None;
    for line in &lines {
        if let Some(prev) = previous {
            text.push_str(if line.y > prev.y + prev.height + prev.height / 2 {
                "\n\n"
            } else {
                "\n"
            });
        }
        text.push_str(&line.text);
        previous = Some(line);
    }
    Ok(text)
}

pub fn recognise_layout(
    image: &DynamicImage,
    det_model: &[u8],
    rec_model: &[u8],
    charset: &[u8],
) -> Result<Vec<Line>, String> {
    let charset = parse_charset(charset)?;
    let boxes = detect(image, det_model)?;
    if boxes.is_empty() {
        return Ok(Vec::new());
    }

    // The recogniser runs once per detected line -- dozens per page -- so it
    // is heavy however small the file is.
    //
    // THE WHOLE PAGE IS THE UNIT OF FALLBACK, for the reason upscaling's tiles
    // are: a line that fails on the card halfway down a page cannot be retried
    // alone, because the lines before it were read by a session that is about
    // to be dropped. Reading the page again on the processor is slower and it
    // is the difference between a slow answer and none.
    let (lines, _provider) = run_for(rec_model, Workload::Heavy, |rec| {
        let mut lines: Vec<Line> = Vec::new();
        for b in &boxes {
            let crop = image
                .crop_imm(b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0)
                .to_rgb8();
            let text = recognise_one(rec, &crop, &charset)?;
            if !text.trim().is_empty() {
                lines.push(Line {
                    x: b.x0,
                    y: b.y0,
                    width: b.x1 - b.x0,
                    height: b.y1 - b.y0,
                    text,
                });
            }
        }
        Ok(lines)
    })?;
    Ok(lines)
}

/// `blank` + the dictionary + `space`, which is the order PP-OCR's
/// `CTCLabelDecode` builds and therefore the order the model's 6625 output
/// classes are in. Getting it wrong shifts every character by one.
fn parse_charset(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| format!("the character dictionary is not valid UTF-8: {e}"))?;
    let mut out = Vec::with_capacity(text.len() / 3 + 2);
    out.push(String::new()); // index 0 is CTC blank
    for line in text.split('\n') {
        // The file is one character per line; a trailing newline must not
        // become a class.
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out.push(" ".to_string());
    if out.len() < 2 {
        return Err("the character dictionary is empty".into());
    }
    Ok(out)
}

/// Where the text is.
fn detect(image: &DynamicImage, model: &[u8]) -> Result<Vec<Box>, String> {
    let (src_w, src_h) = image.dimensions();
    let (net_w, net_h) = det_size(src_w, src_h);

    let small = image
        .resize_exact(net_w, net_h, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let input = normalise_imagenet(&small);

    // A single run over the whole page, which on a large scan is the biggest
    // tensor this engine ever hands the card.
    let ((shape, probs), _provider) = run_for(model, Workload::Heavy, move |session| {
        run_one(session, input.clone())
    })?;
    // [N, 1, H, W]; the map is the last two dimensions.
    let (map_h, map_w) = match shape.as_slice() {
        [_, _, h, w] => (*h as usize, *w as usize),
        _ => {
            return Err(format!(
                "the detector returned shape {shape:?}, expected [N,1,H,W]"
            ));
        }
    };
    if probs.len() < map_h * map_w {
        return Err("the detector returned fewer values than its own shape".into());
    }

    let mask: Vec<bool> = probs[..map_h * map_w]
        .iter()
        .map(|&p| p > DET_THRESHOLD)
        .collect();
    let regions = connected_boxes(&mask, map_w, map_h);

    // Back to original coordinates. The detector saw a resized image, so every
    // box is scaled by the ratio that resize used — not by a single factor,
    // because width and height were rounded to the stride independently.
    let sx = src_w as f32 / map_w as f32;
    let sy = src_h as f32 / map_h as f32;
    let mut boxes: Vec<Box> = regions
        .into_iter()
        .filter_map(|(x0, y0, x1, y1)| {
            // A small outward pad: DB's probability map hugs the glyphs, and
            // recognition does better with a little air around them. Upstream
            // achieves this by "unclipping" the contour; a fixed pad is the
            // same idea at lower fidelity, and clamped so it cannot leave the
            // image.
            let pad_x = ((x1 - x0) as f32 * 0.10).ceil() as u32 + 2;
            let pad_y = ((y1 - y0) as f32 * 0.25).ceil() as u32 + 2;
            let bx0 = ((x0 as f32 * sx) as u32).saturating_sub(pad_x);
            let by0 = ((y0 as f32 * sy) as u32).saturating_sub(pad_y);
            let bx1 = (((x1 + 1) as f32 * sx) as u32 + pad_x).min(src_w);
            let by1 = (((y1 + 1) as f32 * sy) as u32 + pad_y).min(src_h);
            (bx1 > bx0 + MIN_BOX_SIDE && by1 > by0 + MIN_BOX_SIDE).then_some(Box {
                x0: bx0,
                y0: by0,
                x1: bx1,
                y1: by1,
            })
        })
        .collect();

    // Reading order: top to bottom, then left to right. Boxes whose vertical
    // centres are within half a line height count as the same line, so a
    // two-column row does not come back interleaved by a few pixels of skew.
    boxes.sort_by(|a, b| {
        let ah = (a.y1 - a.y0) as i64;
        let same_line = ((a.y0 as i64 + a.y1 as i64) / 2 - (b.y0 as i64 + b.y1 as i64) / 2).abs()
            < ah.max(1) / 2;
        if same_line {
            a.x0.cmp(&b.x0)
        } else {
            a.y0.cmp(&b.y0)
        }
    });
    Ok(boxes)
}

/// Network input size: longest side capped, both sides a multiple of the
/// stride, and never zero.
fn det_size(w: u32, h: u32) -> (u32, u32) {
    let longest = w.max(h).max(1);
    let scale = if longest > DET_MAX_SIDE {
        DET_MAX_SIDE as f32 / longest as f32
    } else {
        1.0
    };
    let round = |v: u32| {
        let scaled = (v as f32 * scale).round() as u32;
        (scaled.div_ceil(DET_STRIDE) * DET_STRIDE).max(DET_STRIDE)
    };
    (round(w), round(h))
}

/// Axis-aligned bounding boxes of connected true-regions.
///
/// # Why this is not DB's contour walk
///
/// Upstream finds contours, fits a minimum-area rotated rectangle, and expands
/// it by an area/perimeter ratio ("unclip"). That recovers rotated and curved
/// text. This takes connected components and their bounding boxes, which is
/// correct for horizontal text and degrades to a too-large box for rotated
/// text rather than to a wrong one.
///
/// The scan is iterative — an explicit stack, not recursion — because the
/// input is attacker-controlled and a full-page text region would otherwise
/// recurse once per pixel and overflow the stack.
fn connected_boxes(mask: &[bool], w: usize, h: usize) -> Vec<(u32, u32, u32, u32)> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            // Four-connectivity. Eight would merge glyphs across diagonal
            // gaps, which joins adjacent words more often than it helps.
            let mut push = |nx: usize, ny: usize| {
                let j = ny * w + nx;
                if mask[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(x - 1, y);
            }
            if x + 1 < w {
                push(x + 1, y);
            }
            if y > 0 {
                push(x, y - 1);
            }
            if y + 1 < h {
                push(x, y + 1);
            }
        }
        out.push((x0 as u32, y0 as u32, x1 as u32, y1 as u32));
    }
    out
}

/// ImageNet normalisation, CHW — what the DETECTOR expects.
fn normalise_imagenet(img: &RgbImage) -> Array4<f32> {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let (w, h) = img.dimensions();
    let mut a = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for (x, y, px) in img.enumerate_pixels() {
        for c in 0..3 {
            let v = f32::from(px.0[c]) / 255.0;
            a[[0, c, y as usize, x as usize]] = (v - MEAN[c]) / STD[c];
        }
    }
    a
}

/// One cropped line through the recogniser.
fn recognise_one(
    session: &mut Session,
    crop: &RgbImage,
    charset: &[String],
) -> Result<String, String> {
    let (cw, ch) = crop.dimensions();
    if cw == 0 || ch == 0 {
        return Ok(String::new());
    }
    // Aspect-preserving resize to the fixed height. PP-OCR pads to a batch
    // width; with one image per run there is nothing to pad to, so the
    // network simply sees this line's own width.
    let width = ((cw as f32 * REC_HEIGHT as f32 / ch as f32).ceil() as u32)
        .clamp(REC_HEIGHT / 4, REC_MAX_WIDTH);
    let resized = DynamicImage::ImageRgb8(crop.clone())
        .resize_exact(width, REC_HEIGHT, image::imageops::FilterType::Triangle)
        .to_rgb8();

    // Recognition normalises to [-1, 1], NOT ImageNet. See the module note.
    let mut input = Array4::<f32>::zeros((1, 3, REC_HEIGHT as usize, width as usize));
    for (x, y, px) in resized.enumerate_pixels() {
        for c in 0..3 {
            let v = f32::from(px.0[c]) / 255.0;
            input[[0, c, y as usize, x as usize]] = (v - 0.5) / 0.5;
        }
    }

    let (shape, logits) = run_one(session, input)?;
    let (steps, classes) = match shape.as_slice() {
        [_, t, c] => (*t as usize, *c as usize),
        _ => {
            return Err(format!(
                "the recogniser returned shape {shape:?}, expected [N,T,C]"
            ));
        }
    };
    if classes == 0 || logits.len() < steps * classes {
        return Err("the recogniser returned fewer values than its own shape".into());
    }
    Ok(ctc_greedy(
        &logits[..steps * classes],
        steps,
        classes,
        charset,
    ))
}

/// CTC greedy decode: argmax per step, collapse runs, drop blanks.
///
/// The collapse must happen BEFORE blanks are dropped, not after. "aa" is
/// spelled `a,blank,a` and collapsing after removal would turn it into "a" —
/// which is the whole reason the blank class exists.
fn ctc_greedy(logits: &[f32], steps: usize, classes: usize, charset: &[String]) -> String {
    let mut out = String::new();
    let mut prev = usize::MAX;
    for t in 0..steps {
        let row = &logits[t * classes..(t + 1) * classes];
        let mut best = 0usize;
        let mut best_v = f32::MIN;
        for (i, &v) in row.iter().enumerate() {
            if v > best_v {
                best_v = v;
                best = i;
            }
        }
        if best != prev && best != 0 {
            if let Some(ch) = charset.get(best) {
                out.push_str(ch);
            }
        }
        prev = best;
    }
    out
}
