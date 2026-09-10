//! PDF rendering through **pdfium**, linked DYNAMICALLY.
//!
//! Same position as `oc-images/src/heif.rs`: this is an ENGINE crate, the far
//! side of the trust boundary, inside a process that is already confined
//! (`03` §5). FFI `unsafe` lives HERE and nowhere else in the host-side
//! story — every pointer handed over comes from bytes the host already
//! confined, and every returned buffer is copied out before release, so no C
//! lifetime escapes this module.
//!
//! # Where the import library came from
//!
//! The pdfium binary distribution ships only `pdfium.dll`; `build.rs` links
//! against a minimal `pdfium.lib` generated from the DLL exports (`lib.exe
//! /def:pdfium.def`), covering exactly the fifteen functions declared below.
//! At run time the loader finds `pdfium.dll` beside the executable, copied
//! there by `build.rs`.
//!
//! # The decode contract
//!
//! Bytes in → one rendered page of tightly-packed BGRA out, one function, no
//! callbacks, no retained decoder state between calls. A hostile file trips
//! pdfium's own limits (it is Chrome's PDF engine, hardened for untrusted
//! input by design) and comes back as an error shaped for the failure
//! messages in `main.rs`.

// The C names stay C-shaped: these identifiers ARE pdfium's API surface.
#![allow(non_snake_case, non_camel_case_types, dead_code)]

use core::marker::PhantomData;
use std::sync::Once;

// ---------------------------------------------------------------------------
// Raw bindings — the exact subset of fpdfview.h this worker uses
// ---------------------------------------------------------------------------

pub type FPDF_DOCUMENT = *mut core::ffi::c_void;
pub type FPDF_PAGE = *mut core::ffi::c_void;
pub type FPDF_BITMAP = *mut core::ffi::c_void;
/// `FPDF_TEXTPAGE` from fpdf_text.h — a page's text objects, indexed by
/// character. Distinct from `FPDF_PAGE`, and it borrows one: the page must
/// outlive it, which [`page_chars`] enforces by keeping both guards in scope.
pub type FPDF_TEXTPAGE = *mut core::ffi::c_void;

/// `FPDFBitmap_BGRA` from fpdfview.h — read from the header, not guessed.
pub const FPDFBITMAP_FORMAT_BGRA: i32 = 4;
/// `FPDF_ANNOT` from fpdfview.h — render annotations into the bitmap.
pub const RENDER_FLAGS_ANNOT: i32 = 1;
/// Pages larger than this on either axis are scaled down proportionally
/// before a bitmap is allocated, so a hostile MediaBox cannot ask for the
/// machine's whole RAM through an honest-looking one-page PDF.
const MAX_DIMENSION: u32 = 4096;

extern "C" {
    fn FPDF_InitLibrary();
    fn FPDF_DestroyLibrary();
    fn FPDF_LoadMemDocument64(data: *const u8, size: usize, password: *const u8) -> FPDF_DOCUMENT;
    fn FPDF_GetPageCount(doc: FPDF_DOCUMENT) -> i32;
    fn FPDF_LoadPage(doc: FPDF_DOCUMENT, page_index: i32) -> FPDF_PAGE;
    fn FPDF_ClosePage(page: FPDF_PAGE);
    fn FPDF_CloseDocument(doc: FPDF_DOCUMENT);
    fn FPDF_GetPageWidthF(page: FPDF_PAGE) -> f32;
    fn FPDF_GetPageHeightF(page: FPDF_PAGE) -> f32;
    fn FPDF_RenderPageBitmap(
        bitmap: FPDF_BITMAP,
        page: FPDF_PAGE,
        start_x: i32,
        start_y: i32,
        size_x: i32,
        size_y: i32,
        rotate: i32,
        flags: i32,
    );
    fn FPDFBitmap_CreateEx(
        width: i32,
        height: i32,
        format: i32,
        first_scan: *mut u8,
        stride: i32,
    ) -> FPDF_BITMAP;
    fn FPDFBitmap_FillRect(
        bitmap: FPDF_BITMAP,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
        color: u64,
    );
    fn FPDFBitmap_GetBuffer(bitmap: FPDF_BITMAP) -> *mut u8;
    fn FPDFBitmap_GetStride(bitmap: FPDF_BITMAP) -> i32;
    fn FPDFBitmap_Destroy(bitmap: FPDF_BITMAP);
    fn FPDF_GetLastError() -> i32;

    // fpdf_text.h. The subset that answers "what characters are on this page,
    // where, and how big" -- which is everything the layout reconstruction in
    // `text.rs` needs and nothing more.
    fn FPDFText_LoadPage(page: FPDF_PAGE) -> FPDF_TEXTPAGE;
    fn FPDFText_ClosePage(text_page: FPDF_TEXTPAGE);
    fn FPDFText_CountChars(text_page: FPDF_TEXTPAGE) -> i32;
    fn FPDFText_GetUnicode(text_page: FPDF_TEXTPAGE, index: i32) -> u32;
    fn FPDFText_GetCharBox(
        text_page: FPDF_TEXTPAGE,
        index: i32,
        left: *mut f64,
        right: *mut f64,
        bottom: *mut f64,
        top: *mut f64,
    ) -> i32;
    fn FPDFText_GetFontSize(text_page: FPDF_TEXTPAGE, index: i32) -> f64;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a PDF operation failed.
///
/// Every variant carries the [`FPDF_GetLastError`] code captured at the point
/// of failure, because that number is the only explanation pdfium gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfError {
    /// `FPDF_LoadMemDocument64` refused the bytes.
    Load {
        /// `FPDF_GetLastError` after the refusal.
        last_error: i32,
    },
    /// The document opened but reports zero pages.
    Empty,
    /// `FPDF_LoadPage` could not load the requested page.
    Page {
        /// The page index that failed.
        index: i32,
        /// `FPDF_GetLastError` after the refusal.
        last_error: i32,
    },
    /// A bitmap resource failed to materialise.
    Bitmap,
    /// Rendered but produced something unusable.
    BadOutput(&'static str),
}

impl core::fmt::Display for PdfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            PdfError::Load { last_error } => write!(
                f,
                "pdfium refused the document ({}); the file may be corrupt or \
                 password-protected",
                error_name(last_error)
            ),
            PdfError::Empty => write!(f, "the document reports zero pages"),
            PdfError::Page { index, last_error } => write!(
                f,
                "pdfium could not load page {} ({})",
                index + 1,
                error_name(last_error)
            ),
            PdfError::Bitmap => write!(f, "pdfium could not allocate the render bitmap"),
            PdfError::BadOutput(reason) => {
                write!(
                    f,
                    "pdfium produced an unusable rendering surface ({reason})"
                )
            }
        }
    }
}

impl std::error::Error for PdfError {}

/// The `FPDF_ERR_*` name for a code, for messages a user can act on.
fn error_name(code: i32) -> &'static str {
    match code {
        0 => "no error",
        1 => "unknown error",
        2 => "file could not be opened",
        3 => "not in PDF format or corrupted",
        4 => "password required or incorrect",
        5 => "unsupported security scheme",
        6 => "page not found or content error",
        _ => "unrecognised error code",
    }
}

// ---------------------------------------------------------------------------
// RAII guards — every acquired handle released exactly once, on every path
// ---------------------------------------------------------------------------

/// An open PDF document, tied to the bytes it was loaded from.
///
/// fpdfview.h's contract for [`FPDF_LoadMemDocument64`] is explicit: "the
/// memory buffer must remain valid when the document is open" — pdfium
/// RETAINS the caller's pointer and keeps parsing through it lazily on every
/// later call (page count, page load, render). The `'a` here is that
/// sentence made type-checked: a document cannot outlive the slice it
/// parses, so the contract is enforced by the borrow checker instead of
/// trusted to whoever holds the guard.
pub struct PdfDocumentGuard<'a>(FPDF_DOCUMENT, PhantomData<&'a [u8]>);

impl PdfDocumentGuard<'_> {
    /// The raw handle, for calls that take the owning document.
    fn handle(&self) -> FPDF_DOCUMENT {
        self.0
    }
}

impl Drop for PdfDocumentGuard<'_> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: acquired by FPDF_LoadMemDocument64, freed exactly once.
            unsafe { FPDF_CloseDocument(self.0) };
        }
    }
}

/// One loaded page of a document. Dropping closes it.
struct PageGuard(FPDF_PAGE);

impl Drop for PageGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: acquired by FPDF_LoadPage, freed exactly once.
            unsafe { FPDF_ClosePage(self.0) };
        }
    }
}

/// One page's text objects. Dropping releases them.
///
/// **Must be dropped before the `PageGuard` it was loaded from.** fpdf_text.h
/// is explicit that the text page borrows the page, and Rust drops struct
/// fields and locals in declaration order, so the one function that holds both
/// declares the page first and the text page second.
struct TextPageGuard(FPDF_TEXTPAGE);

impl Drop for TextPageGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: acquired by FPDFText_LoadPage, freed exactly once, and
            // before the page it borrows.
            unsafe { FPDFText_ClosePage(self.0) };
        }
    }
}

/// One render-target bitmap. Dropping destroys it.
struct BitmapGuard(FPDF_BITMAP);

impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: acquired by FPDFBitmap_CreateEx, freed exactly once.
            unsafe { FPDFBitmap_Destroy(self.0) };
        }
    }
}

// ---------------------------------------------------------------------------
// Safe wrappers
// ---------------------------------------------------------------------------

/// Initialise pdfium exactly once per process.
///
/// pdfium requires this before any other call; `Once` makes every entry
/// point safe to call without remembering the rule.
pub fn init_library() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: initialises library-global state; documented as callable
        // once per process, which the Once enforces.
        unsafe { FPDF_InitLibrary() };
    });
}

/// One rendered page: tightly-packed BGRA, top-left origin.
pub struct RenderedPage {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` bytes, BGRA order.
    pub bgra: Vec<u8>,
}

/// Read a PDF from memory and report how many pages it has.
///
/// # Errors
///
/// [`PdfError::Load`] when pdfium refuses the bytes (corrupt, not a PDF,
/// password-protected), [`PdfError::Empty`] when it opens to zero pages.
pub fn open_document(bytes: &[u8]) -> Result<(PdfDocumentGuard<'_>, i32), PdfError> {
    if bytes.is_empty() {
        // Fabricated FORMAT code rather than FPDF_GetLastError: the query
        // would return whatever some earlier operation left in pdfium's
        // thread-local error slot, which names nothing about this refusal.
        return Err(PdfError::Load { last_error: 3 });
    }

    // SAFETY: fpdfview.h:451 — "the memory buffer must remain valid when the
    // document is open." pdfium RETAINS this pointer: GetPageCount, LoadPage
    // and RenderPageBitmap all parse through it long after this call
    // returns. The returned guard carries PhantomData<&'a [u8]> over these
    // exact bytes, so the borrow checker pins the document inside the borrow
    // of `bytes`, and Drop runs FPDF_CloseDocument before that borrow can
    // end. An empty password is a bare null.
    let doc = PdfDocumentGuard(
        unsafe { FPDF_LoadMemDocument64(bytes.as_ptr(), bytes.len(), core::ptr::null()) },
        PhantomData,
    );
    if doc.0.is_null() {
        return Err(PdfError::Load {
            // SAFETY: pure query of thread-local error state.
            last_error: unsafe { FPDF_GetLastError() },
        });
    }

    // SAFETY: live document handle.
    let pages = unsafe { FPDF_GetPageCount(doc.handle()) };
    if pages <= 0 {
        return Err(PdfError::Empty);
    }
    Ok((doc, pages))
}

/// Dimensions page `index` renders at, in pixels, AFTER applying
/// [`MAX_DIMENSION`].
///
/// Loading a page is cheap next to painting one; querying first lets the
/// caller enforce `decode_pixels` BEFORE any bitmap exists — which is the
/// entire point of a limit, since a refusal that has already paid for the
/// allocation it refuses bounds nothing.
///
/// # Errors
///
/// [`PdfError::Page`] when the page will not load.
pub fn page_dimensions(doc: &PdfDocumentGuard<'_>, index: u32) -> Result<(u32, u32), PdfError> {
    // SAFETY: live document; the caller checked `index` against the page
    // count open_document reported.
    let page = PageGuard(unsafe { FPDF_LoadPage(doc.handle(), index as i32) });
    if page.0.is_null() {
        return Err(PdfError::Page {
            index: index as i32,
            // SAFETY: pure query of thread-local error state.
            last_error: unsafe { FPDF_GetLastError() },
        });
    }
    Ok(scaled_page_dimensions(page.0))
}

/// Read a page's point dimensions and apply the dimension cap.
///
/// Shared by [`page_dimensions`] (measure only) and [`render_page`]
/// (measure then paint) so the size a caller budgeted against is bit-for-bit
/// the size the bitmap gets.
fn scaled_page_dimensions(page: FPDF_PAGE) -> (u32, u32) {
    sized_page_dimensions(page, None)
}

fn sized_page_dimensions(page: FPDF_PAGE, max_side: Option<u32>) -> (u32, u32) {
    // SAFETY: live page handle; dimensions are floats in PDF points.
    let width_pt = unsafe { FPDF_GetPageWidthF(page) };
    let height_pt = unsafe { FPDF_GetPageHeightF(page) };
    let mut width = (width_pt.ceil().max(1.0)) as u32;
    let mut height = (height_pt.ceil().max(1.0)) as u32;

    if let Some(target) = max_side {
        let scale = target.clamp(64, MAX_DIMENSION) as f64 / width.max(height) as f64;
        width = (width as f64 * scale).round().max(1.0) as u32;
        height = (height as f64 * scale).round().max(1.0) as u32;
    }

    // The dimension cap. Applied before CreateEx so the memory is never
    // asked for: scale BOTH axes by the same factor to keep the aspect.
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        let long_side = width.max(height);
        let scale = f64::from(MAX_DIMENSION) / f64::from(long_side);
        width = ((f64::from(width) * scale).ceil().max(1.0)) as u32;
        height = ((f64::from(height) * scale).ceil().max(1.0)) as u32;
    }
    (width, height)
}

/// Render page `index` of an open document to a BGRA bitmap and copy the
/// pixels out.
///
/// The loop over pages lives in the caller: this renders ONE page per call,
/// which is what keeps a page render's memory bounded at one bitmap however
/// many pages the document has. Pages larger than [`MAX_DIMENSION`] on either
/// axis are scaled down proportionally BEFORE the bitmap is allocated — the
/// cap bounds the allocation, not the render. Callers that enforce a pixel
/// budget should consult [`page_dimensions`] first and refuse before this
/// function runs, because rendering allocates the bitmap it paints into.
///
/// # Errors
///
/// [`PdfError::Page`] when the page will not load, [`PdfError::Bitmap`] when
/// the render surface fails to materialise, [`PdfError::BadOutput`] when the
/// surface comes back with an unusable layout.
pub fn render_page(doc: &PdfDocumentGuard<'_>, index: u32) -> Result<RenderedPage, PdfError> {
    render_page_sized(doc, index, None)
}
pub fn render_page_sized(
    doc: &PdfDocumentGuard<'_>,
    index: u32,
    max_side: Option<u32>,
) -> Result<RenderedPage, PdfError> {
    // SAFETY: live document; the caller checked `index` against the count.
    let page = PageGuard(unsafe { FPDF_LoadPage(doc.handle(), index as i32) });
    if page.0.is_null() {
        return Err(PdfError::Page {
            index: index as i32,
            // SAFETY: pure query of thread-local error state.
            last_error: unsafe { FPDF_GetLastError() },
        });
    }

    let (width, height) = sized_page_dimensions(page.0, max_side);

    // SAFETY: positive dimensions within the cap; a null first_scan with
    // stride 0 asks pdfium to allocate and align the buffer itself.
    let bitmap = BitmapGuard(unsafe {
        FPDFBitmap_CreateEx(
            width as i32,
            height as i32,
            FPDFBITMAP_FORMAT_BGRA,
            core::ptr::null_mut(),
            0,
        )
    });
    if bitmap.0.is_null() {
        return Err(PdfError::Bitmap);
    }

    // PAINT THE PAGE BACKGROUND FIRST. A fresh FPDFBitmap_CreateEx buffer is
    // ZEROED, and in BGRA that is transparent BLACK -- not white, and not
    // nothing. pdfium draws only what the page's content stream contains, and
    // a PDF page does not carry its own white background: the white is a
    // viewer convention, which means the viewer has to supply it.
    //
    // Without this, a normal text page rendered to BLACK GLYPHS ON
    // TRANSPARENT BLACK. PNG hid the damage, because the alpha channel still
    // carried the glyph shapes and every page therefore produced a
    // different-sized file -- so page selection looked like it worked, and
    // the output was a blank image with a clean class-B receipt over it.
    // JPEG has no alpha to hide behind and flattened the same buffer to a
    // uniformly black page, which is what finally made it visible: three
    // pages, three receipts naming three different page indices, one
    // byte-identical output.
    //
    // 0xFFFF_FFFF is opaque white as pdfium's 0xAARRGGBB.
    //
    // SAFETY: live bitmap; the rect is the whole allocation.
    unsafe {
        FPDFBitmap_FillRect(bitmap.0, 0, 0, width as i32, height as i32, 0xFFFF_FFFF);
    }

    // SAFETY: live bitmap and page; the full page renders into the bitmap at
    // its native size starting at (0, 0), no rotation, annotations on.
    unsafe {
        FPDF_RenderPageBitmap(
            bitmap.0,
            page.0,
            0,
            0,
            width as i32,
            height as i32,
            0,
            RENDER_FLAGS_ANNOT,
        );
    }

    // SAFETY: live bitmap; the buffer is owned by the bitmap until Destroy,
    // which the guard runs AFTER the copy below.
    let buffer = unsafe { FPDFBitmap_GetBuffer(bitmap.0) };
    if buffer.is_null() {
        return Err(PdfError::Bitmap);
    }
    // SAFETY: live bitmap; stride is the allocation's row pitch.
    let stride = unsafe { FPDFBitmap_GetStride(bitmap.0) };
    if stride <= 0 {
        return Err(PdfError::Bitmap);
    }
    let stride = stride as usize;

    // De-plane: copy row by row, because the stride may exceed width*4 for
    // alignment and the caller gets a tightly-packed buffer or nothing. A
    // stride SMALLER than a row would overlap rows on every read past the
    // first — rejected rather than copied, the same rule heif.rs applies to
    // libheif planes.
    let row_len = width as usize * 4;
    if stride < row_len {
        return Err(PdfError::BadOutput("plane stride smaller than a pixel row"));
    }
    let mut bgra = vec![0u8; row_len * height as usize];
    for (i, row) in bgra.chunks_exact_mut(row_len).enumerate() {
        // SAFETY: rows 0..height of a plane whose stride bounds it; the copy
        // stays inside the plane the bitmap owns until the guard releases it.
        let src = unsafe { buffer.add(i * stride) };
        row.copy_from_slice(unsafe { core::slice::from_raw_parts(src, row_len) });
    }

    Ok(RenderedPage {
        width,
        height,
        bgra,
    })
}

/// Every character on page `index`, with its box and font size.
///
/// # What comes back, and what does not
///
/// A PDF stores text as text: this reads the characters the file ALREADY
/// CONTAINS, in the order its content stream draws them, along with where each
/// one was drawn. Nothing is recognised and nothing is guessed — which is the
/// whole reason `pdf -> txt` is Class B and the OCR rows beside it are Class D.
///
/// An empty vector means the page has no text objects at all. That is a
/// scanned page — a picture of text — and the caller must say so rather than
/// writing out an empty file, because an empty `.txt` looks like success.
///
/// Coordinates are PDF user space, origin bottom-left, y increasing upward.
/// They are NOT the render-time pixel coordinates `render_page` produces, and
/// nothing here applies [`MAX_DIMENSION`]: no bitmap is allocated, so there is
/// nothing to bound.
///
/// # Errors
///
/// [`PdfError::Page`] when the page or its text will not load.
pub fn page_chars(
    doc: &PdfDocumentGuard<'_>,
    index: u32,
) -> Result<Vec<crate::text::Char>, PdfError> {
    // SAFETY: live document; the caller checked `index` against the page count
    // open_document reported.
    let page = PageGuard(unsafe { FPDF_LoadPage(doc.handle(), index as i32) });
    if page.0.is_null() {
        return Err(PdfError::Page {
            index: index as i32,
            // SAFETY: no arguments, no state.
            last_error: unsafe { FPDF_GetLastError() },
        });
    }
    // Declared AFTER `page` so it is dropped BEFORE it — see `TextPageGuard`.
    // SAFETY: live page, held for the whole of this function.
    let text_page = TextPageGuard(unsafe { FPDFText_LoadPage(page.0) });
    if text_page.0.is_null() {
        return Err(PdfError::Page {
            index: index as i32,
            // SAFETY: no arguments, no state.
            last_error: unsafe { FPDF_GetLastError() },
        });
    }

    // SAFETY: live text page.
    let count = unsafe { FPDFText_CountChars(text_page.0) };
    if count <= 0 {
        return Ok(Vec::new());
    }

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        // SAFETY: live text page; `i` is inside the count it reported.
        let code = unsafe { FPDFText_GetUnicode(text_page.0, i) };
        // A code point pdfium cannot map comes back as 0 or as an unpaired
        // surrogate. Dropped rather than substituted: `char::REPLACEMENT` in
        // the middle of a word is noise that reads as damage in the output,
        // and the glyph carried no text either way.
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        if ch == '\0' {
            continue;
        }

        let (mut left, mut right, mut bottom, mut top) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
        // SAFETY: live text page, `i` in range, and four pointers to live
        // stack locals that outlive the call.
        let ok = unsafe {
            FPDFText_GetCharBox(
                text_page.0,
                i,
                &raw mut left,
                &raw mut right,
                &raw mut bottom,
                &raw mut top,
            )
        };
        if ok == 0 {
            continue;
        }
        // SAFETY: live text page, `i` in range.
        let size = unsafe { FPDFText_GetFontSize(text_page.0, i) };

        out.push(crate::text::Char {
            ch,
            left: left as f32,
            right: right as f32,
            bottom: bottom as f32,
            top: top as f32,
            // Font size can come back as 0 for a glyph pdfium has no font
            // record for; the box height is the fallback, and it is what the
            // line grouping would have used anyway.
            size: if size > 0.0 {
                size as f32
            } else {
                (top - bottom).abs() as f32
            },
        });
    }
    Ok(out)
}
