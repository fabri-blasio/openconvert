//! Turning positioned glyphs back into paragraphs.
//!
//! # What this module is, and what it is not
//!
//! **A PDF has no paragraphs.** It has glyphs at coordinates, drawn in
//! whatever order the producer emitted them. Every structural thing a reader
//! sees — lines, paragraphs, headings, the order two columns are read in — is
//! a conclusion drawn from geometry after the fact. This module draws those
//! conclusions, and it is the whole reason `pdf -> docx` is Class C rather
//! than B: the text is read out faithfully, and then a structure the file does
//! not contain is inferred over it.
//!
//! Keeping the inference here, away from the FFI, is deliberate. Every rule
//! below is a judgement call that can be wrong on some document, and a
//! judgement call that can only be exercised through pdfium is a judgement
//! call with no tests. [`layout`] takes a plain `Vec<Char>` and the tests at
//! the bottom of this file build those by hand.
//!
//! # The limit that is not going away
//!
//! Lines are ordered top-to-bottom and then left-to-right, which is reading
//! order for a single column and is NOT reading order for two. A two-column
//! page interleaves. Detecting columns properly means clustering x-positions
//! and deciding whether a gap is a column boundary or a wide table cell, and
//! getting that wrong silently scrambles a document. The receipt says reading
//! order is inferred; it does not claim the inference is right.

/// One glyph, as pdfium reports it.
///
/// Coordinates are PDF user space: origin bottom-left, y increasing UPWARD.
/// That is the opposite of every screen coordinate system and it is the source
/// of the sign confusion this comment exists to prevent — "the next line down"
/// is a SMALLER `top`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Char {
    /// The character itself.
    pub ch: char,
    /// Left edge of its box.
    pub left: f32,
    /// Right edge.
    pub right: f32,
    /// Bottom edge.
    pub bottom: f32,
    /// Top edge.
    pub top: f32,
    /// Font size in points, as the content stream set it.
    pub size: f32,
}

impl Char {
    /// Vertical middle, which is what line grouping compares.
    fn mid_y(self) -> f32 {
        (self.top + self.bottom) / 2.0
    }

    fn height(self) -> f32 {
        (self.top - self.bottom).abs()
    }
}

/// A run of glyphs sharing a baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The text, with runs of whitespace already collapsed.
    pub text: String,
    /// Left edge of the first glyph.
    pub left: f32,
    /// Right edge of the last.
    pub right: f32,
    /// Vertical middle of the line.
    pub mid_y: f32,
    /// The typical font size on this line.
    pub size: f32,
}

/// What a paragraph is being presented as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Ordinary body text.
    Body,
    /// A heading, at a depth.
    ///
    /// # Two levels from a PDF, six from a document that names them
    ///
    /// [`layout`] still produces only 1 and 2, and the reason has not changed:
    /// the only evidence in a PDF is "this line is bigger than the body text",
    /// which supports a title/section distinction and not six of them. Six
    /// levels inferred from one measurement would be six confident claims.
    ///
    /// Markdown, HTML and a `.docx` each **state** their heading depth, so a
    /// source that says `###` is not guessed at — it is read. The type carries
    /// what the source can support; each producer carries its own limit.
    Heading(u8),
    /// An item in a bulleted list, at a nesting depth starting from 0.
    Bullet {
        /// How deeply nested, from 0.
        depth: u8,
    },
    /// An item in a numbered list.
    Ordered {
        /// How deeply nested, from 0.
        depth: u8,
        /// The number the source gave it.
        number: u64,
    },
    /// A preformatted block: source code, output, anything where the line
    /// breaks and the spacing are the content.
    ///
    /// Held as one paragraph with its newlines intact, because a code block
    /// re-wrapped to the measure is no longer a code block.
    Code,
    /// A block quotation.
    Quote,
}

impl Style {
    /// Whether this style's own line breaks are significant.
    ///
    /// Everything else is prose and may be re-wrapped to whatever measure the
    /// output has. A code block may not: its line breaks ARE the content, and
    /// a typesetter that reflows one has destroyed it.
    #[must_use]
    pub const fn is_preformatted(self) -> bool {
        matches!(self, Self::Code)
    }

    /// The indent level a list marker implies, for producers that render lists
    /// as ordinary paragraphs.
    #[must_use]
    pub const fn depth(self) -> u8 {
        match self {
            Self::Bullet { depth } | Self::Ordered { depth, .. } => depth,
            Self::Quote => 1,
            _ => 0,
        }
    }
}

/// One reconstructed paragraph.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    /// The text, with the lines that made it joined.
    pub text: String,
    /// How it should be presented.
    pub style: Style,
}

/// A line whose right edge falls this far short of the block's right margin is
/// treated as ending a paragraph.
///
/// The classic heuristic, and the reason it works: justified and ragged-right
/// text both run to roughly the same right edge on every line EXCEPT the last
/// one of a paragraph. A fifth of the block width is wide enough not to fire
/// on ordinary ragged-right variation and narrow enough to catch a genuine
/// short final line.
const SHORT_LINE_FRACTION: f32 = 0.20;

/// A vertical gap larger than this multiple of the usual line spacing starts a
/// new paragraph even when the previous line ran full width.
const PARAGRAPH_GAP: f32 = 1.45;

/// A line this much larger than the body size is a heading.
const HEADING_RATIO: f32 = 1.18;

/// The larger of the two heading depths starts here.
const HEADING_1_RATIO: f32 = 1.55;

/// Group glyphs into lines.
///
/// Glyphs arrive in content-stream order, which is emission order and not
/// necessarily reading order, so lines are formed by VERTICAL POSITION rather
/// than by adjacency in the input: a producer that draws a page's italics last
/// would otherwise put every one of them on a line of its own.
fn lines(chars: &[Char]) -> Vec<Line> {
    let mut sorted: Vec<Char> = chars
        .iter()
        .copied()
        .filter(|c| !c.ch.is_control() && c.height() > 0.0)
        .collect();
    if sorted.is_empty() {
        return Vec::new();
    }
    // Descending y (top of the page first), then left to right.
    sorted.sort_by(|a, b| {
        b.mid_y()
            .partial_cmp(&a.mid_y())
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(
                a.left
                    .partial_cmp(&b.left)
                    .unwrap_or(core::cmp::Ordering::Equal),
            )
    });

    // The tolerance is a fraction of glyph height rather than a constant,
    // because a 6 pt footnote and a 30 pt title need different answers to
    // "is this the same line".
    let mut out: Vec<Line> = Vec::new();
    let mut current: Vec<Char> = vec![sorted[0]];
    for &c in &sorted[1..] {
        let reference = current[current.len() - 1];
        let tolerance = reference.height().max(c.height()) * 0.5;
        if (c.mid_y() - reference.mid_y()).abs() <= tolerance {
            current.push(c);
        } else {
            out.push(finish_line(&mut current));
            current.push(c);
        }
    }
    out.push(finish_line(&mut current));
    out.retain(|l| !l.text.is_empty());
    out
}

/// Build one [`Line`] from the glyphs collected for it, and clear the buffer.
fn finish_line(buf: &mut Vec<Char>) -> Line {
    buf.sort_by(|a, b| {
        a.left
            .partial_cmp(&b.left)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    // WORD SPACING IS pdfium's JOB, NOT OURS, AND THIS IS WHY.
    //
    // A producer often positions the next word with a text-matrix offset and
    // emits no space glyph, which is why naive extraction yields
    // "thequickbrown". pdfium's text page already synthesises those spaces,
    // using the FONT's own metrics -- the advance width of the space in that
    // face at that size. All we have is bounding boxes.
    //
    // Inserting our own on top of that was actively worse: a tight glyph box
    // means the gap between the ink of `q` and the ink of `u` routinely
    // exceeds any threshold scaled to box height, and the first run of this
    // produced "The q uick brown fox j u mps ... it is done ." -- spaces
    // inside words, and one before a full stop, because a period's left side
    // bearing is most of its box.
    //
    // What is left here is the case pdfium does NOT cover: a gap far too wide
    // to be word spacing, which is a tab stop or the column of a table. Half
    // the font size is roughly twice the widest space in a normal face, so it
    // fires on those and on nothing a font could produce between two words.
    let mut text = String::new();
    let mut previous_right: Option<f32> = None;
    for c in buf.iter() {
        if let Some(right) = previous_right {
            let gap = c.left - right;
            if gap > c.size.max(c.height()) * 0.5 && !text.ends_with(' ') && !c.ch.is_whitespace() {
                text.push(' ');
            }
        }
        if c.ch.is_whitespace() {
            if !text.ends_with(' ') && !text.is_empty() {
                text.push(' ');
            }
        } else {
            text.push(c.ch);
        }
        previous_right = Some(c.right);
    }

    let line = Line {
        text: text.trim().to_string(),
        left: buf.first().map_or(0.0, |c| c.left),
        right: buf.last().map_or(0.0, |c| c.right),
        mid_y: median(&mut buf.iter().map(|c| c.mid_y()).collect::<Vec<_>>()),
        size: median(
            &mut buf
                .iter()
                .map(|c| c.size.max(c.height()))
                .collect::<Vec<_>>(),
        ),
    };
    buf.clear();
    line
}

/// The middle value, which is what every threshold here is measured against.
///
/// The mean is wrong for all of them: one 48 pt title drags the mean body size
/// up far enough that nothing else registers as a heading, and a single
/// stretched glyph drags the line spacing.
fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    // The LOWER middle on an even count, not the upper and not their mean.
    // Every caller is asking "what is normal here" so that it can notice
    // something larger: normal line spacing, so a paragraph gap stands out;
    // normal font size, so a heading stands out. With two samples the upper
    // middle IS the outlier, and the comparison against it can never fire --
    // which is exactly how the paragraph-gap test failed when it was written.
    values[(values.len() - 1) / 2]
}

/// Reconstruct paragraphs from one page's glyphs.
///
/// Returns an empty vector for a page with no text objects at all — a scanned
/// page, in other words. The caller must not write that out as an empty
/// document: an empty file looks like success, and "this PDF is pictures of
/// text" is something the user needs told.
#[must_use]
pub fn layout(chars: &[Char]) -> Vec<Paragraph> {
    let lines = lines(chars);
    if lines.is_empty() {
        return Vec::new();
    }

    let body_size = median(&mut lines.iter().map(|l| l.size).collect::<Vec<_>>());
    let right_margin = lines
        .iter()
        .map(|l| l.right)
        .fold(f32::MIN, f32::max)
        .max(0.0);
    let block_width = right_margin - lines.iter().map(|l| l.left).fold(f32::MAX, f32::min);

    // The usual distance between consecutive lines, for the paragraph-gap
    // test. Taken from the document rather than assumed, because single and
    // double spacing differ by exactly the factor being tested for.
    let mut spacings: Vec<f32> = lines
        .windows(2)
        .map(|w| (w[0].mid_y - w[1].mid_y).abs())
        .collect();
    let spacing = median(&mut spacings);

    let mut out: Vec<Paragraph> = Vec::new();
    let mut current = String::new();
    let mut current_size = body_size;

    for (i, line) in lines.iter().enumerate() {
        let heading = heading_style(line.size, body_size);

        // A heading is always its own paragraph, on both sides. Merging one
        // into the body text below it is the single most visible way this can
        // go wrong in a Word document.
        let break_before = heading.is_some()
            || heading_style(current_size, body_size).is_some()
            || i > 0 && starts_paragraph(&lines[i - 1], line, spacing, block_width, right_margin);

        if break_before && !current.is_empty() {
            out.push(Paragraph {
                text: core::mem::take(&mut current),
                style: heading_style(current_size, body_size).map_or(Style::Body, Style::Heading),
            });
        }
        if current.is_empty() {
            current_size = line.size;
        } else {
            // Lines inside a paragraph are joined with a space, not a newline:
            // they are a wrapping artefact of the page width, and carrying
            // them into a Word document would freeze this PDF's line breaks
            // into a file that is about to be re-wrapped anyway.
            //
            // A trailing hyphen is the one case where the break was inside a
            // word, so it is removed rather than turned into a space.
            if let Some(stem) = current.strip_suffix('-') {
                current = stem.to_string();
            } else {
                current.push(' ');
            }
        }
        current.push_str(&line.text);
    }
    if !current.is_empty() {
        out.push(Paragraph {
            text: current,
            style: heading_style(current_size, body_size).map_or(Style::Body, Style::Heading),
        });
    }
    out
}

/// Whether `line` begins a new paragraph, given what came before it.
fn starts_paragraph(
    previous: &Line,
    line: &Line,
    spacing: f32,
    block_width: f32,
    right_margin: f32,
) -> bool {
    // Extra leading between the two.
    let gap = (previous.mid_y - line.mid_y).abs();
    if spacing > 0.0 && gap > spacing * PARAGRAPH_GAP {
        return true;
    }
    // The previous line stopped well short of the margin.
    if block_width > 0.0 && right_margin - previous.right > block_width * SHORT_LINE_FRACTION {
        return true;
    }
    // This line is indented relative to the one above it.
    line.left - previous.left > line.size
}

/// The heading depth for a line of this size, if any.
fn heading_style(size: f32, body: f32) -> Option<u8> {
    if body <= 0.0 {
        return None;
    }
    let ratio = size / body;
    if ratio >= HEADING_1_RATIO {
        Some(1)
    } else if ratio >= HEADING_RATIO {
        Some(2)
    } else {
        None
    }
}

/// Every page's paragraphs as plain text.
///
/// Paragraphs are separated by a blank line and pages by nothing else — a
/// page boundary in a PDF is a printing artefact, and reproducing it in a text
/// file with rules or form feeds gives a reader something to strip.
#[must_use]
pub fn to_plain(pages: &[Vec<Paragraph>]) -> String {
    let mut out = String::new();
    for page in pages {
        for para in page {
            out.push_str(para.text.trim());
            out.push_str("\n\n");
        }
    }
    // One trailing newline, the POSIX convention, rather than the two the loop
    // leaves behind.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay out a line of text at a given baseline, one glyph box per character.
    fn line_at(text: &str, top: f32, size: f32, left: f32) -> Vec<Char> {
        let advance = size * 0.5;
        text.chars()
            .enumerate()
            .map(|(i, ch)| Char {
                ch,
                left: left + i as f32 * advance,
                right: left + (i as f32 + 1.0) * advance,
                bottom: top - size,
                top,
                size,
            })
            .collect()
    }

    /// **Wrapped lines are one paragraph.**
    ///
    /// The property everything else rests on. Two full-width lines a normal
    /// leading apart are one paragraph that the page width happened to break;
    /// emitting them as two would put this PDF's line breaks into a Word
    /// document that is about to re-wrap them.
    #[test]
    fn wrapped_lines_become_one_paragraph() {
        let mut chars = line_at("the quick brown fox jumps over", 700.0, 10.0, 72.0);
        chars.extend(line_at(
            "the lazy dog and keeps going ok",
            688.0,
            10.0,
            72.0,
        ));
        let paras = layout(&chars);
        assert_eq!(paras.len(), 1, "got {paras:?}");
        assert!(paras[0].text.contains("fox jumps over the lazy dog"));
    }

    /// A short final line ends the paragraph.
    #[test]
    fn a_short_line_ends_a_paragraph() {
        let mut chars = line_at("the quick brown fox jumps over", 700.0, 10.0, 72.0);
        chars.extend(line_at("the end.", 688.0, 10.0, 72.0));
        chars.extend(line_at("a second paragraph starts here", 676.0, 10.0, 72.0));
        let paras = layout(&chars);
        assert_eq!(paras.len(), 2, "got {paras:?}");
        assert!(paras[0].text.ends_with("the end."));
    }

    /// Extra leading ends the paragraph even when the line ran full width.
    #[test]
    fn a_wide_vertical_gap_ends_a_paragraph() {
        let mut chars = line_at("the quick brown fox jumps over", 700.0, 10.0, 72.0);
        chars.extend(line_at("the quick brown fox jumps agan", 688.0, 10.0, 72.0));
        chars.extend(line_at("far below after a generous gap", 650.0, 10.0, 72.0));
        let paras = layout(&chars);
        assert_eq!(paras.len(), 2, "got {paras:?}");
    }

    /// **A bigger line is a heading, and never merges with the body.**
    #[test]
    fn a_larger_line_becomes_a_heading_of_its_own() {
        let mut chars = line_at("Chapter One", 700.0, 20.0, 72.0);
        chars.extend(line_at("the quick brown fox jumps over", 660.0, 10.0, 72.0));
        chars.extend(line_at(
            "the lazy dog and keeps going ok",
            648.0,
            10.0,
            72.0,
        ));
        let paras = layout(&chars);
        assert_eq!(paras.len(), 2, "got {paras:?}");
        assert_eq!(paras[0].style, Style::Heading(1));
        assert_eq!(paras[0].text, "Chapter One");
        assert_eq!(paras[1].style, Style::Body);
    }

    /// **A tab-width gap becomes a space; an ordinary letter gap does not.**
    ///
    /// Both halves matter. pdfium synthesises word spacing from font metrics
    /// and does it better than bounding boxes can, so the only gap left for
    /// this code is one far too wide to be a space — a tab stop or a table
    /// column. An earlier threshold scaled to glyph height fired between the
    /// letters of a word and produced "The q uick brown fox j u mps".
    #[test]
    fn only_a_gap_too_wide_to_be_word_spacing_becomes_a_space() {
        // Two glyphs a tab apart, with no space glyph between them.
        let mut chars = line_at("a", 700.0, 10.0, 72.0);
        chars.extend(line_at("b", 700.0, 10.0, 72.0 + 5.0 + 12.0));
        assert_eq!(layout(&chars)[0].text, "a b");

        // The same two, a normal inter-letter distance apart.
        let mut tight = line_at("a", 700.0, 10.0, 72.0);
        tight.extend(line_at("b", 700.0, 10.0, 72.0 + 5.0 + 1.0));
        assert_eq!(layout(&tight)[0].text, "ab");
    }

    /// Emission order is not reading order, and lines are formed by position.
    #[test]
    fn glyphs_emitted_out_of_order_still_group_by_position() {
        let mut chars = line_at("second line here", 688.0, 10.0, 72.0);
        chars.extend(line_at("first line here.", 700.0, 10.0, 72.0));
        let paras = layout(&chars);
        assert!(
            paras[0].text.starts_with("first line here."),
            "got {paras:?}"
        );
    }

    /// A hyphen at a line break is joining one word, not separating two.
    #[test]
    fn a_hyphenated_break_rejoins_the_word() {
        let mut chars = line_at("this sentence has an extra-ordi-", 700.0, 10.0, 72.0);
        chars.extend(line_at(
            "nary word broken across the line",
            688.0,
            10.0,
            72.0,
        ));
        let paras = layout(&chars);
        assert!(paras[0].text.contains("extra-ordinary"), "got {paras:?}");
    }

    /// **A page with no text objects yields nothing, not an empty paragraph.**
    ///
    /// The caller distinguishes "a scanned PDF" from "a PDF with an empty
    /// page" on this, and writing out an empty file would look like success.
    #[test]
    fn a_page_with_no_glyphs_yields_no_paragraphs() {
        assert!(layout(&[]).is_empty());
    }

    /// Control characters do not become paragraphs of their own.
    #[test]
    fn control_characters_are_dropped() {
        let mut chars = line_at("hello world", 700.0, 10.0, 72.0);
        chars.push(Char {
            ch: '\u{0}',
            left: 200.0,
            right: 200.0,
            bottom: 300.0,
            top: 310.0,
            size: 10.0,
        });
        let paras = layout(&chars);
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].text, "hello world");
    }

    /// Plain text separates paragraphs with a blank line and ends with one
    /// newline.
    #[test]
    fn plain_text_is_blank_line_separated() {
        let pages = vec![vec![
            Paragraph {
                text: "one".into(),
                style: Style::Body,
            },
            Paragraph {
                text: "two".into(),
                style: Style::Body,
            },
        ]];
        assert_eq!(to_plain(&pages), "one\n\ntwo\n");
    }
}
