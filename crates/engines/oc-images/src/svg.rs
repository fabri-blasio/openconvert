//! SVG rasterisation, through resvg.
//!
//! # Why this is in the sandboxed worker despite being pure Rust
//!
//! Nothing here links a C library, so by the usual rule it could run
//! in-process. It does not, and the reason is the input rather than the
//! parser: SVG is XML, an SVG can reference other documents, and a renderer is
//! a far larger attack surface than a PNG decoder. The worker costs a process
//! launch and buys the same confinement every other untrusted parse gets.
//!
//! # Text is not drawn
//!
//! The `text` and font features are off — see this crate's `Cargo.toml`. A
//! worker has no filesystem, so system fonts cannot be found by construction,
//! and substituting an arbitrary available font would make the same file
//! render differently on two machines. Text is skipped and the conversion
//! discloses it, which is the honest version of a limitation that would
//! otherwise look like a rendering bug.

use resvg::tiny_skia;
use resvg::usvg;

/// The largest side a rasterised SVG may have.
///
/// An SVG is a program, and its declared size is arbitrary: `width="1e6"` is
/// valid and asks for a terabyte of pixels. The pixel budget catches that too,
/// but a dimension cap refuses it before any allocation is attempted.
const MAX_SIDE: u32 = 16_384;

/// What a rasterised SVG produced.
pub struct Raster {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8, row-major, straight (not premultiplied) alpha.
    pub rgba: Vec<u8>,
    /// Whether the document contained text that was not drawn.
    pub had_text: bool,
}

/// Whether these bytes look like SVG.
///
/// SVG has **no fixed signature** — it is XML, and the leading bytes vary with
/// the declaration, the BOM, and any comments before the root element. So this
/// looks for the root tag within a bounded prefix rather than matching magic,
/// which is the same structural test the core sniffer applies.
#[must_use]
pub fn looks_like_svg(bytes: &[u8]) -> bool {
    let window = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(window);
    let lower = text.to_ascii_lowercase();
    lower.contains("<svg")
}

/// Read an SVG's intrinsic size without rendering it.
///
/// # Errors
///
/// A document that does not parse, or one whose declared size is unusable.
pub fn probe_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let tree = parse(bytes)?;
    size_of(&tree)
}

/// Rasterise at the document's intrinsic size.
///
/// # Errors
///
/// A document that does not parse, a size past [`MAX_SIDE`], or an allocation
/// the rasteriser refused.
pub fn rasterise(bytes: &[u8]) -> Result<Raster, String> {
    let tree = parse(bytes)?;
    let (width, height) = size_of(&tree)?;
    let had_text = has_text(&tree);

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("could not allocate a {width}x{height} canvas"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    // tiny-skia works in PREMULTIPLIED alpha and every other path in this
    // worker carries straight RGBA. Handing the premultiplied bytes on would
    // darken every semi-transparent pixel in proportion to how transparent it
    // is — a bug that looks like bad antialiasing rather than a colour-space
    // mistake.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let a = p.alpha();
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let un = |c: u8| ((u16::from(c) * 255) / u16::from(a)).min(255) as u8;
                [un(p.red()), un(p.green()), un(p.blue()), a]
            }
        })
        .collect();

    Ok(Raster {
        width,
        height,
        rgba,
        had_text,
    })
}

fn parse(bytes: &[u8]) -> Result<usvg::Tree, String> {
    // Default options: no filesystem resolution, so an SVG referencing an
    // external file gets nothing rather than reaching for it. That is the
    // behaviour we want and the behaviour the sandbox would enforce anyway.
    usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| format!("this is not an SVG this build can parse: {e}"))
}

fn size_of(tree: &usvg::Tree) -> Result<(u32, u32), String> {
    let size = tree.size();
    let (w, h) = (size.width().ceil() as i64, size.height().ceil() as i64);
    if w <= 0 || h <= 0 {
        return Err("this SVG declares a zero or negative size".into());
    }
    if w > i64::from(MAX_SIDE) || h > i64::from(MAX_SIDE) {
        return Err(format!(
            "this SVG asks to be rendered at {w}x{h}; the limit is {MAX_SIDE} on a side"
        ));
    }
    Ok((w as u32, h as u32))
}

/// Whether any node in the tree is text.
///
/// With the `text` feature off, usvg keeps text nodes in the tree and resvg
/// draws nothing for them. Detecting them is what turns a silent omission into
/// a disclosed one.
fn has_text(tree: &usvg::Tree) -> bool {
    fn walk(group: &usvg::Group) -> bool {
        group.children().iter().any(|node| match node {
            usvg::Node::Text(_) => true,
            usvg::Node::Group(g) => walk(g),
            _ => false,
        })
    }
    walk(tree.root())
}
