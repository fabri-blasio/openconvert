//! Real-ESRGAN ×4 super-resolution.
//!
//! # This export has a FIXED 64×64 input, and that decides the whole design
//!
//! Most segmentation models here are fully convolutional and take whatever
//! they are given. This one does not: its graph declares `[1,3,64,64]` in and
//! `[1,3,256,256]` out. So the image is cut into tiles, each tile is run, and
//! the results are stitched back together.
//!
//! Tiling naively produces visible seams. Convolutions near a tile edge see
//! zero-padding instead of the neighbouring pixels, so the outermost few
//! pixels of every tile are wrong in a way that lines up into a grid. The fix
//! is to overlap the input tiles and discard the bad margin from each output —
//! which is what [`OVERLAP`] is for, and why the arithmetic below is fiddlier
//! than "for each tile, paste".

use image::{DynamicImage, GenericImageView, RgbImage};
use ndarray::Array4;

use crate::accelerator::Workload;
use crate::infer::{run_for, run_one};

/// The tile size used when the graph does not fix one.
///
/// A fully-convolutional export declares `[-1, 3, -1, -1]` and will accept any
/// size; something still has to be chosen, and this is a size that divides by
/// 32 (most super-resolution encoders downsample five times) and keeps one
/// tile's working set small enough to be unremarkable.
const DEFAULT_TILE: u32 = 256;

/// Most tiles one image may cost.
///
/// Each is a separate model run, so this is a wall-clock bound rather than a
/// memory one: 4096 tiles of a few tens of milliseconds is already minutes.
/// Refusing with the number named beats appearing to hang.
const MAX_TILES: u32 = 4096;

/// How one model wants to be fed, and what it gives back.
///
/// # These were three `const`s, and that was a trap
///
/// `TILE = 64`, `SCALE = 4` and `OVERLAP = 8` are properties of ONE export --
/// Real-ESRGAN x4, whose graph declares `[1,3,64,64]` in and `[1,3,256,256]`
/// out. Written as constants they read like tuning knobs, and the moment a
/// second model is added they are silently wrong for it.
///
/// Silently is the operative word. A wrong tile size does not fail: it
/// produces a stitched image with a regular grid of seams, which looks like a
/// bad model rather than a bad constant. A wrong scale produces an output
/// canvas of the wrong size, filled correctly in one corner.
///
/// So the numbers come from the graph. `tile` is the input's declared spatial
/// dimension when it has one; `scale` is the ratio of the output's to the
/// input's. A dynamic export declares neither, and is measured instead -- see
/// [`Geometry::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Geometry {
    /// The square tile fed to the model, in source pixels.
    tile: u32,
    /// How much larger the output is than the input.
    scale: u32,
    /// Source pixels of context around each tile, discarded from the output.
    ///
    /// Proportional to the tile rather than fixed: eight pixels of context is
    /// generous around a 64px tile and negligible around a 256px one, and the
    /// receptive-edge damage this exists to hide scales with the model, not
    /// with an absolute number.
    overlap: u32,
}

impl Geometry {
    /// Source pixels each tile actually contributes.
    const fn step(self) -> u32 {
        self.tile - self.overlap * 2
    }

    /// The output side of one tile.
    const fn out_side(self) -> u32 {
        self.tile * self.scale
    }
}

impl Geometry {
    /// Work out how this model wants to be driven.
    ///
    /// # Read it, then measure it
    ///
    /// A fixed export -- Real-ESRGAN x4 declares `[1,3,64,64]` in and
    /// `[1,3,256,256]` out -- answers both questions from its own metadata, at
    /// no cost and before any inference.
    ///
    /// A dynamic one declares `[-1,3,-1,-1]` for both and answers neither. The
    /// tile is then ours to choose, and the SCALE is measured: one tile of the
    /// chosen size is pushed through and the output side divided by the input
    /// side. That costs one small inference at the start of a run that is about
    /// to do hundreds, and it is the only way to know -- a model file does not
    /// otherwise say what factor it enlarges by, and guessing produces a canvas
    /// of the wrong size filled correctly in one corner.
    fn of(session: &mut ort::session::Session) -> Result<Self, String> {
        // Read both declared sides while the borrow is immutable, and finish
        // with it before the probe below needs the session mutably.
        let (declared_in, declared_out) = {
            let side = |dtype: &ort::value::ValueType| -> Option<u32> {
                let ort::value::ValueType::Tensor { shape, .. } = dtype else {
                    return None;
                };
                // NCHW: the last two are spatial, and a fixed square export
                // gives both as the same positive number.
                let dims: Vec<i64> = shape.iter().copied().collect();
                let [.., h, w] = dims.as_slice() else {
                    return None;
                };
                (*h > 0 && h == w).then_some(*h as u32)
            };
            (
                session.inputs().first().and_then(|o| side(o.dtype())),
                session.outputs().first().and_then(|o| side(o.dtype())),
            )
        };

        let tile = declared_in.unwrap_or(DEFAULT_TILE);

        let scale = match (declared_in, declared_out) {
            // Both fixed: the ratio is stated, and a non-integer one is a
            // model this code cannot drive rather than something to round.
            (Some(t), Some(o)) if t > 0 && o.is_multiple_of(t) && o / t > 0 => o / t,
            _ => Self::measure_scale(session, tile)?,
        };

        // An eighth of the tile, floored at 8 -- the value that was correct for
        // the 64px export, which is where the proportion comes from.
        let overlap = (tile / 8).max(8);
        if overlap * 2 >= tile {
            return Err(format!(
                "a {tile}px tile leaves no room for {overlap}px of context on each side"
            ));
        }
        Ok(Self {
            tile,
            scale,
            overlap,
        })
    }

    /// Push one flat tile through and see how big it comes back.
    fn measure_scale(session: &mut ort::session::Session, tile: u32) -> Result<u32, String> {
        let probe = Array4::<f32>::zeros((1, 3, tile as usize, tile as usize));
        let (shape, data) = run_one(session, probe)?;
        let pixels = data.len() / 3;
        let side = (pixels as f64).sqrt().round() as u32;
        if side == 0 || side * side * 3 != data.len() as u32 || !side.is_multiple_of(tile) {
            return Err(format!(
                "this upscaler returned shape {shape:?} for a {tile}x{tile} tile, which is not \
                 a whole-number enlargement of it"
            ));
        }
        Ok(side / tile)
    }
}

/// Upscale an image ×4.
///
/// # Errors
///
/// An image so large it would exceed [`MAX_TILES`], or a model whose output is
/// not the shape this export declares.
pub fn upscale(image: &DynamicImage, model: &[u8]) -> Result<RgbImage, String> {
    // The session comes first now: the tiling geometry is read out of the
    // loaded graph rather than assumed, so it cannot be known before the model
    // is open. See `Geometry`.
    // HEAVY, and this is the measurement the whole feature rests on:
    // 256×192 → 1024×768 took 107.3 s on the CPU and 14.5 s on DirectML.
    // Tiling means one session serves hundreds of calls, so the GPU's
    // session-build cost is amortised to nothing.
    //
    // THE WHOLE TILED RUN IS THE UNIT OF FALLBACK, and it has to be: a tile
    // that exhausts video memory halfway through a two-hundred-tile image
    // cannot be retried on its own, because the tiles before it were stitched
    // by a session that is about to be dropped. Redoing the lot on the
    // processor is slower -- 107 s against 14.5 s, measured -- and it is the
    // difference between a slow answer and none.
    //
    // `Geometry::of` reads the graph, so it is inside too: the tiling depends
    // on the loaded model, not on the provider, but it needs A session and the
    // session belongs to the attempt.
    let (out, _provider) = run_for(model, Workload::Heavy, |session| {
        let geo = Geometry::of(session)?;
        stitch(image, geo, |input| run_one(session, input))
    })?;
    Ok(out)
}

/// Tile, run, and stitch -- with the model behind a closure.
///
/// # Why the model is a parameter
///
/// The arithmetic here is where a seam comes from: `keep` is where a tile
/// belongs, `read` is where it was taken from, and the difference is the margin
/// to skip. Get any of that wrong by a pixel and the output has a regular grid
/// of soft lines through it -- which looks like a mediocre model, not like a
/// bug, and is exactly the failure the per-model geometry could reintroduce.
///
/// A test cannot assert on that with real weights: it would need a model file,
/// and the model's own output is the thing it cannot predict. With the run
/// behind a closure, a test can supply an upscaler whose output IS predictable
/// -- nearest-neighbour, say -- and then assert that the stitched result equals
/// the same transform applied to the whole image. A seam of any size fails it.
fn stitch(
    image: &DynamicImage,
    geo: Geometry,
    mut run_tile: impl FnMut(Array4<f32>) -> Result<(Vec<i64>, Vec<f32>), String>,
) -> Result<RgbImage, String> {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Err("the image declares zero dimensions".into());
    }
    let step = geo.step();

    let tiles_x = w.div_ceil(step);
    let tiles_y = h.div_ceil(step);
    let total = tiles_x.saturating_mul(tiles_y);
    if total > MAX_TILES {
        return Err(format!(
            "upscaling this {w}x{h} image would take {total} model runs, over the limit of \
             {MAX_TILES}; scale it down first"
        ));
    }

    let src = image.to_rgb8();
    let mut out = RgbImage::new(w * geo.scale, h * geo.scale);

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            // Where this tile's KEPT region starts, in source pixels.
            let keep_x = tx * step;
            let keep_y = ty * step;
            // The tile actually fed to the model starts geo.overlap earlier, and
            // is clamped so it never runs off the image — a tile at the right
            // edge slides left rather than shrinking, because the model
            // cannot take a smaller input.
            let read_x = keep_x
                .saturating_sub(geo.overlap)
                .min(w.saturating_sub(geo.tile));
            let read_y = keep_y
                .saturating_sub(geo.overlap)
                .min(h.saturating_sub(geo.tile));

            let mut input = Array4::<f32>::zeros((1, 3, geo.tile as usize, geo.tile as usize));
            for y in 0..geo.tile {
                for x in 0..geo.tile {
                    // Outside the image, clamp to the edge pixel. Zeroes here
                    // would make the border darken, which is the same seam
                    // problem in a different place.
                    let sx = (read_x + x).min(w - 1);
                    let sy = (read_y + y).min(h - 1);
                    let px = src.get_pixel(sx, sy).0;
                    for c in 0..3 {
                        // This export takes plain [0,1], not ImageNet.
                        input[[0, c, y as usize, x as usize]] = f32::from(px[c]) / 255.0;
                    }
                }
            }

            let (shape, data) = run_tile(input)?;
            let side = geo.out_side();
            let expect = (side * side) as usize * 3;
            if data.len() < expect {
                return Err(format!(
                    "the upscaler returned shape {shape:?}, too small for {side}x{side} RGB"
                ));
            }

            // Copy only the region this tile owns, from the middle of its
            // output. `keep_*` is where it belongs; `read_*` is where it came
            // from; the difference is the margin to skip.
            let skip_x = (keep_x - read_x) * geo.scale;
            let skip_y = (keep_y - read_y) * geo.scale;
            let span_x = (step * geo.scale).min(w * geo.scale - keep_x * geo.scale);
            let span_y = (step * geo.scale).min(h * geo.scale - keep_y * geo.scale);
            let plane = (side * side) as usize;
            for y in 0..span_y {
                let oy = skip_y + y;
                if oy >= side {
                    break;
                }
                for x in 0..span_x {
                    let ox = skip_x + x;
                    if ox >= side {
                        break;
                    }
                    let i = (oy * side + ox) as usize;
                    let px = [
                        to_u8(data[i]),
                        to_u8(data[plane + i]),
                        to_u8(data[2 * plane + i]),
                    ];
                    out.put_pixel(
                        keep_x * geo.scale + x,
                        keep_y * geo.scale + y,
                        image::Rgb(px),
                    );
                }
            }
        }
    }
    Ok(out)
}

/// Clamp a model's float output into a byte.
///
/// The network is not bounded to [0,1] and routinely overshoots slightly at
/// hard edges; wrapping instead of clamping turns a bright highlight into a
/// black speck.
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image with structure in both axes, so a seam anywhere shows up.
    ///
    /// A flat or smoothly-graded picture hides stitching errors: copying the
    /// wrong region of a gradient still gives approximately the right value.
    /// This changes sharply at every pixel.
    fn noisy(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            // Coprime multipliers, so the pattern does not repeat on any tile
            // boundary the geometry might choose.
            *px = image::Rgb([
                ((x * 37 + y * 17) % 251) as u8,
                ((x * 11 + y * 43) % 241) as u8,
                ((x * 59 + y * 7) % 233) as u8,
            ]);
        }
        DynamicImage::ImageRgb8(img)
    }

    /// A model that enlarges by nearest-neighbour: predictable, so the stitched
    /// result can be compared against the truth rather than eyeballed.
    fn nearest(scale: u32) -> impl FnMut(Array4<f32>) -> Result<(Vec<i64>, Vec<f32>), String> {
        move |input: Array4<f32>| {
            let tile = input.shape()[2] as u32;
            let side = tile * scale;
            let mut out = vec![0.0_f32; (side * side) as usize * 3];
            let plane = (side * side) as usize;
            for c in 0..3 {
                for y in 0..side {
                    for x in 0..side {
                        let v = input[[0, c, (y / scale) as usize, (x / scale) as usize]];
                        out[c * plane + (y * side + x) as usize] = v;
                    }
                }
            }
            Ok((vec![1, 3, i64::from(side), i64::from(side)], out))
        }
    }

    /// **The stitched image equals the transform applied to the whole image.**
    ///
    /// This is the test the tiling never had. `TILE`, `SCALE` and `OVERLAP`
    /// were constants matching one export; making them per-model is the kind of
    /// change that fails SILENTLY -- a wrong tile size produces a regular grid
    /// of soft seams, which reads as a mediocre model rather than as a bug.
    ///
    /// Asserting the run succeeded would have passed throughout. This asserts
    /// every pixel.
    #[test]
    fn stitching_reproduces_the_transform_exactly() {
        // Sizes deliberately NOT multiples of the step, so the last tile in
        // each direction has to slide back and overlap its neighbour -- which
        // is the case the `keep`/`read`/`skip` arithmetic exists for.
        for (w, h) in [(64_u32, 64_u32), (100, 70), (37, 129), (256, 19)] {
            for geo in [
                Geometry {
                    tile: 64,
                    scale: 4,
                    overlap: 8,
                },
                Geometry {
                    tile: 64,
                    scale: 2,
                    overlap: 8,
                },
                Geometry {
                    tile: 32,
                    scale: 2,
                    overlap: 8,
                },
                Geometry {
                    tile: 128,
                    scale: 2,
                    overlap: 16,
                },
            ] {
                // A tile larger than the image cannot be read from it; that is
                // a caller's problem, not this test's subject.
                if geo.tile > w || geo.tile > h {
                    continue;
                }
                let src = noisy(w, h);
                let got = stitch(&src, geo, nearest(geo.scale)).expect("stitch");

                assert_eq!(got.width(), w * geo.scale);
                assert_eq!(got.height(), h * geo.scale);

                let rgb = src.to_rgb8();
                let mut worst = 0_i32;
                let mut worst_at = (0_u32, 0_u32);
                for y in 0..got.height() {
                    for x in 0..got.width() {
                        let want = rgb.get_pixel(x / geo.scale, y / geo.scale).0;
                        let have = got.get_pixel(x, y).0;
                        for c in 0..3 {
                            let d = i32::from(want[c]) - i32::from(have[c]);
                            if d.abs() > worst {
                                worst = d.abs();
                                worst_at = (x, y);
                            }
                        }
                    }
                }
                assert!(
                    worst <= 1,
                    "{w}x{h} at tile {} scale {}: worst pixel differs by {worst} at {worst_at:?} \
                     -- a seam, or the wrong region copied",
                    geo.tile,
                    geo.scale
                );
            }
        }
    }

    /// The control: a DELIBERATELY wrong geometry is caught.
    ///
    /// Without this the test above could be passing because the comparison is
    /// too loose rather than because the stitching is right.
    #[test]
    fn a_mismatched_scale_does_not_pass_silently() {
        let geo = Geometry {
            tile: 64,
            scale: 4,
            overlap: 8,
        };
        let src = noisy(100, 70);
        // The model enlarges by 2 while the geometry says 4.
        let got = stitch(&src, geo, nearest(2));
        assert!(
            got.is_err(),
            "a model whose output is smaller than the declared scale must be \
             refused, not stitched into a quarter-filled canvas"
        );
    }
}
