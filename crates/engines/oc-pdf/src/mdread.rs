//! Reading Markdown into the paragraph model.
//!
//! # Plain text is Markdown, and that is the whole design
//!
//! Detection in this product is **content, never extension** — `sniff.rs` says
//! so in its first line — and a `.md` file is plain text with no signature. No
//! amount of scanning separates it from a `.txt` reliably: a README with a
//! dash list looks like Markdown, and a minimal Markdown file looks like
//! prose.
//!
//! So this does not try. Every text input is parsed as CommonMark, and plain
//! text is simply CommonMark that happens to contain no markup — it comes out
//! as a run of body paragraphs, which is exactly right. Structure is an
//! **enhancement when present**, never a requirement, and a misidentified file
//! costs formatting rather than the conversion.
//!
//! The cost is the honest one: a plain-text file with a line beginning `# `
//! becomes a heading. That is a caption promoted to a title — visible, minor,
//! and the alternative was refusing `md -> pdf` altogether.
//!
//! # What is kept
//!
//! Headings, paragraphs, bulleted and numbered lists with their nesting, code
//! blocks, and block quotes. Inline emphasis, links and images are flattened
//! to their text: [`Paragraph`] carries a style per block and no character
//! runs, so bold inside a sentence has nowhere to live. The receipt says so
//! rather than implying a document survived.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::text::{Paragraph, Style};

/// Parse Markdown (or plain text) into paragraphs.
///
/// # Errors
///
/// None: every byte sequence that is valid UTF-8 is valid Markdown. Invalid
/// UTF-8 is the caller's problem and is reported there.
#[must_use]
pub fn parse(source: &str) -> Vec<Paragraph> {
    // Tables and footnotes are deliberately OFF. The paragraph model cannot
    // express either, so enabling them would change how the text is chunked
    // without changing what can be produced from it -- a table's cells would
    // arrive as a run of fragments rather than as the row of text they read
    // as with the extension off.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut out: Vec<Paragraph> = Vec::new();
    let mut text = String::new();
    let mut style = Style::Body;
    // The list stack: one entry per open list, carrying the next number for an
    // ordered one. Nesting depth is its length.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut in_code = false;
    let mut quote_depth: usize = 0;

    let flush = |text: &mut String, style: Style, out: &mut Vec<Paragraph>| {
        // A code block's own whitespace is content; everything else is prose.
        let body = if style.is_preformatted() {
            text.trim_end_matches('\n').to_string()
        } else {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        if !body.is_empty() {
            out.push(Paragraph { text: body, style });
        }
        text.clear();
    };

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut text, style, &mut out);
                style = Style::Heading(heading_level(level));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut text, style, &mut out);
                // The language is discarded: nothing downstream renders one,
                // and carrying it in the text would put `rust` on its own line
                // at the top of the block.
                let _ = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                in_code = true;
                style = Style::Code;
            }
            Event::Start(Tag::List(first)) => {
                flush(&mut text, style, &mut out);
                lists.push(first);
            }
            Event::Start(Tag::Item) => {
                flush(&mut text, style, &mut out);
                let depth = u8::try_from(lists.len().saturating_sub(1)).unwrap_or(u8::MAX);
                style = match lists.last_mut() {
                    Some(Some(n)) => {
                        let number = *n;
                        *n += 1;
                        Style::Ordered { depth, number }
                    }
                    _ => Style::Bullet { depth },
                };
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut text, style, &mut out);
                quote_depth += 1;
                style = Style::Quote;
            }
            Event::Start(Tag::Paragraph) => {
                flush(&mut text, style, &mut out);
                // A paragraph inside a list item continues that item rather
                // than becoming a body paragraph beside it.
                if !matches!(style, Style::Bullet { .. } | Style::Ordered { .. }) {
                    style = if quote_depth > 0 {
                        Style::Quote
                    } else {
                        Style::Body
                    };
                }
            }

            Event::End(TagEnd::Heading(_) | TagEnd::Paragraph) => {
                flush(&mut text, style, &mut out);
                style = if quote_depth > 0 {
                    Style::Quote
                } else {
                    Style::Body
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut text, style, &mut out);
                in_code = false;
                style = Style::Body;
            }
            Event::End(TagEnd::Item) => {
                flush(&mut text, style, &mut out);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut text, style, &mut out);
                lists.pop();
                style = Style::Body;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut text, style, &mut out);
                quote_depth = quote_depth.saturating_sub(1);
                style = Style::Body;
            }

            // Inline content, flattened. `Code` here is inline `like this`,
            // not a block, and it reads correctly as its own characters.
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak => text.push(if in_code { '\n' } else { ' ' }),
            Event::HardBreak => text.push('\n'),
            // A thematic break is a paragraph boundary and nothing else: there
            // is no rule to draw in the paragraph model.
            Event::Rule => flush(&mut text, style, &mut out),
            // Raw HTML in Markdown is not rendered. Emitting the tags as text
            // would put `<div>` in the output; dropping them is what a
            // text-only pipeline can honestly do.
            Event::Html(_) | Event::InlineHtml(_) => {}
            _ => {}
        }
    }
    flush(&mut text, style, &mut out);
    out
}

const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_carry_the_level_the_source_states() {
        let out = parse("# One\n\n### Three\n\n###### Six\n");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].style, Style::Heading(1));
        assert_eq!(out[1].style, Style::Heading(3));
        assert_eq!(out[2].style, Style::Heading(6));
        assert_eq!(out[1].text, "Three");
    }

    /// The design claim: plain text parses as a run of body paragraphs.
    #[test]
    fn plain_text_is_markdown_with_no_markup() {
        let out = parse("Just a sentence.\n\nAnd another one.\n");
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|p| p.style == Style::Body));
        assert_eq!(out[0].text, "Just a sentence.");
        assert_eq!(out[1].text, "And another one.");
    }

    /// A wrapped paragraph is one paragraph, not one per line.
    #[test]
    fn soft_wrapped_lines_join() {
        let out = parse("one line\nand its continuation\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "one line and its continuation");
    }

    #[test]
    fn lists_carry_depth_and_numbering() {
        let out = parse("- top\n  - nested\n- second\n\n1. first\n2. second\n");
        let bullets: Vec<&Paragraph> = out
            .iter()
            .filter(|p| matches!(p.style, Style::Bullet { .. }))
            .collect();
        assert_eq!(bullets.len(), 3, "{out:?}");
        assert_eq!(bullets[0].style, Style::Bullet { depth: 0 });
        assert_eq!(bullets[1].style, Style::Bullet { depth: 1 });
        assert_eq!(bullets[2].style, Style::Bullet { depth: 0 });

        let ordered: Vec<&Paragraph> = out
            .iter()
            .filter(|p| matches!(p.style, Style::Ordered { .. }))
            .collect();
        assert_eq!(ordered.len(), 2);
        assert_eq!(
            ordered[0].style,
            Style::Ordered {
                depth: 0,
                number: 1
            }
        );
        assert_eq!(
            ordered[1].style,
            Style::Ordered {
                depth: 0,
                number: 2
            }
        );
    }

    /// A list that does not start at 1 keeps the numbers the source gave.
    #[test]
    fn an_ordered_list_starting_elsewhere_is_not_renumbered() {
        let out = parse("5. five\n6. six\n");
        let nums: Vec<Style> = out.iter().map(|p| p.style).collect();
        assert_eq!(
            nums[0],
            Style::Ordered {
                depth: 0,
                number: 5
            }
        );
        assert_eq!(
            nums[1],
            Style::Ordered {
                depth: 0,
                number: 6
            }
        );
    }

    /// A code block keeps its line breaks and its indentation.
    #[test]
    fn code_blocks_keep_their_own_whitespace() {
        let out = parse("```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n");
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].style, Style::Code);
        assert_eq!(out[0].text, "fn main() {\n    println!(\"hi\");\n}");
    }

    #[test]
    fn block_quotes_are_marked() {
        let out = parse("> quoted text\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].style, Style::Quote);
        assert_eq!(out[0].text, "quoted text");
    }

    /// Emphasis is flattened to its text rather than dropped or shown as
    /// syntax. The paragraph model has no character runs to put it in.
    #[test]
    fn inline_markup_flattens_to_its_text() {
        let out = parse("a **bold** and *italic* and `code` and [link](http://x)\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "a bold and italic and code and link");
    }

    /// Raw HTML does not leak into the output as tags.
    #[test]
    fn raw_html_is_not_emitted_as_text() {
        let out = parse("before\n\n<div class=\"x\">inside</div>\n\nafter\n");
        let joined: String = out
            .iter()
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join("|");
        assert!(!joined.contains('<'), "{joined}");
        assert!(joined.contains("before") && joined.contains("after"));
    }

    #[test]
    fn empty_input_is_no_paragraphs() {
        assert!(parse("").is_empty());
        assert!(parse("   \n\n  \n").is_empty());
    }
}
