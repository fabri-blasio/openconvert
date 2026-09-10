//! Writing Markdown from reconstructed paragraphs.
//!
//! # Why this exists beside `text::to_plain`
//!
//! The document pipeline recovers more than characters. `text::layout` infers
//! headings from glyph size, and a `.docx` names its own in `w:pStyle`. Plain
//! text is where that goes to die: a heading and a sentence come out
//! indistinguishable, and the only honest thing `to_plain` can do is drop the
//! distinction.
//!
//! Markdown is the cheapest format that keeps it. Two characters — `#` and a
//! space — carry a claim the pipeline already made, and the file stays
//! readable in any editor that would have opened the `.txt`.
//!
//! # What it does not claim
//!
//! No tables, no lists, no emphasis, no links. The upstream stages do not
//! recover any of those, so writing syntax for them would be inventing
//! structure rather than preserving it — the receipt says the same thing about
//! what the conversion removed.
//!
//! Headings go no deeper than `##`, because [`Style::Heading`] has exactly two
//! levels and for the reason stated there: the evidence is "this line is
//! bigger than the body text", which supports a title/section distinction and
//! not six of them.

use crate::text::{Paragraph, Style};

/// Every page's paragraphs as Markdown.
///
/// Pages are not marked. A page boundary in a PDF is a printing artefact, and
/// a horizontal rule at each one would give the reader something to strip —
/// the same argument `text::to_plain` makes, and it does not stop being true
/// because the output format has a rule syntax.
#[must_use]
pub fn to_markdown(pages: &[Vec<Paragraph>]) -> String {
    let mut out = String::new();
    for page in pages {
        for para in page {
            let text = para.text.trim();
            if text.is_empty() {
                continue;
            }
            match para.style {
                Style::Heading(level) => {
                    // Clamped, not trusted: `Heading` carries a `u8`, and a
                    // level of 0 would emit no `#` at all while a larger one
                    // would emit a line of them. Six is Markdown's own limit.
                    let depth = usize::from(level.clamp(1, 6));
                    out.push_str(&"#".repeat(depth));
                    out.push(' ');
                    out.push_str(&escape_heading(text));
                }
                Style::Bullet { depth } => {
                    out.push_str(&"  ".repeat(usize::from(depth)));
                    out.push_str("- ");
                    out.push_str(&indent_continuation(text, usize::from(depth) * 2 + 2));
                }
                Style::Ordered { depth, number } => {
                    out.push_str(&"  ".repeat(usize::from(depth)));
                    let marker = format!("{number}. ");
                    let width = usize::from(depth) * 2 + marker.len();
                    out.push_str(&marker);
                    out.push_str(&indent_continuation(text, width));
                }
                // FENCED, NOT INDENTED. An indented code block cannot carry a
                // language and silently swallows a leading blank line; a fence
                // is unambiguous. The fence is long enough to survive content
                // that contains backticks of its own.
                Style::Code => {
                    let fence = "`".repeat(longest_backtick_run(text).max(2) + 1);
                    out.push_str(&fence);
                    out.push('\n');
                    out.push_str(text);
                    out.push('\n');
                    out.push_str(&fence);
                }
                Style::Quote => {
                    for (i, line) in text.lines().enumerate() {
                        if i > 0 {
                            out.push('\n');
                        }
                        out.push_str("> ");
                        out.push_str(line);
                    }
                }
                Style::Body => out.push_str(&escape_body(text)),
            }
            out.push_str("\n\n");
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Indent the second and later lines of a list item so they stay inside it.
///
/// A continuation line that starts at column zero ends the item and begins a
/// new paragraph, which turns one bullet into a bullet followed by loose text.
fn indent_continuation(text: &str, width: usize) -> String {
    let pad = " ".repeat(width);
    let mut out = String::with_capacity(text.len() + width);
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(&pad);
        }
        out.push_str(line);
    }
    out
}

/// The longest run of backticks in the text, so a fence can be made longer.
fn longest_backtick_run(text: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// Stop body text from being read as Markdown it never was.
///
/// A line of a PDF that happens to begin `# ` or `- ` is not a heading or a
/// list item; it is a line that begins with a character Markdown reserves.
/// Escaping only at the START of a line is deliberate — these characters are
/// structural there and ordinary everywhere else, and escaping every `#` in
/// running prose would litter the output to prevent a problem that does not
/// occur.
fn escape_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        out.push_str(indent);
        if starts_a_block(trimmed) {
            out.push('\\');
        }
        out.push_str(trimmed);
    }
    out
}

/// Whether this line would be read as Markdown structure if left alone.
fn starts_a_block(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    match first {
        // A heading, a rule, a quote, a fence.
        '#' | '>' | '`' | '~' | '|' => true,
        // A bullet needs the space to be a bullet: "-3 degrees" is not a list.
        '-' | '*' | '+' => line[1..].starts_with(' '),
        // "1. " is an ordered list; "1975 was" is a year.
        '0'..='9' => {
            let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
            rest.starts_with(". ") || rest.starts_with(") ")
        }
        _ => false,
    }
}

/// A heading's text cannot contain a newline, and its `#` needs no escape.
///
/// The `#` is already consumed by the marker this function's caller wrote, so
/// the only thing to guard is a line break turning one heading into two
/// paragraphs.
fn escape_heading(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str, style: Style) -> Paragraph {
        Paragraph {
            text: text.to_string(),
            style,
        }
    }

    #[test]
    fn headings_become_hashes_and_body_does_not() {
        let out = to_markdown(&[vec![
            para("The Title", Style::Heading(1)),
            para("A section", Style::Heading(2)),
            para("Ordinary prose.", Style::Body),
        ]]);
        assert_eq!(out, "# The Title\n\n## A section\n\nOrdinary prose.\n");
    }

    /// The distinction this module exists for.
    ///
    /// `to_plain` renders all three of the paragraphs above identically. If
    /// this ever matches it, the format has stopped earning its place.
    #[test]
    fn markdown_keeps_what_plain_text_loses() {
        let pages = vec![vec![
            para("The Title", Style::Heading(1)),
            para("Ordinary prose.", Style::Body),
        ]];
        assert_ne!(to_markdown(&pages), crate::text::to_plain(&pages));
    }

    /// A heading level outside what Markdown has is clamped, not trusted.
    ///
    /// Six is Markdown's own limit, and `Style::Heading` carries a `u8`: a
    /// level of 0 would emit no `#` at all and a large one would emit a line of
    /// them. A source that legitimately says `###` gets `###` — the clamp is
    /// for values no source produced.
    #[test]
    fn a_wild_heading_level_cannot_emit_a_line_of_hashes() {
        for level in [0_u8, 9, 40, u8::MAX] {
            let out = to_markdown(&[vec![para("H", Style::Heading(level))]]);
            let hashes = out.chars().take_while(|c| *c == '#').count();
            assert!(
                (1..=6).contains(&hashes),
                "level {level} emitted {hashes} hashes"
            );
        }
        // And the levels a real document names survive exactly.
        for level in 1_u8..=6 {
            let out = to_markdown(&[vec![para("H", Style::Heading(level))]]);
            let hashes = out.chars().take_while(|c| *c == '#').count();
            assert_eq!(hashes, usize::from(level), "level {level} was not kept");
        }
    }

    /// Body text that looks like Markdown is escaped at the line start.
    ///
    /// A PDF line reading "# 3 of 12" is a caption, not a heading, and a
    /// round trip that promoted it would be inventing structure.
    #[test]
    fn body_that_looks_like_markdown_is_escaped() {
        let out = to_markdown(&[vec![para("# 3 of 12", Style::Body)]]);
        assert!(out.starts_with("\\# 3 of 12"), "got {out:?}");

        let listish = to_markdown(&[vec![para("- item", Style::Body)]]);
        assert!(listish.starts_with("\\- item"), "got {listish:?}");

        let ordered = to_markdown(&[vec![para("1. first", Style::Body)]]);
        assert!(ordered.starts_with("\\1. first"), "got {ordered:?}");
    }

    /// And text that merely begins with those characters is left alone.
    #[test]
    fn ordinary_prose_is_not_littered_with_backslashes() {
        for text in [
            "-3 degrees at dawn",
            "1975 was the year",
            "a # in the middle",
        ] {
            let out = to_markdown(&[vec![para(text, Style::Body)]]);
            assert!(
                !out.contains('\\'),
                "{text:?} was escaped and should not have been: {out:?}"
            );
        }
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(to_markdown(&[]), "");
        assert_eq!(to_markdown(&[vec![]]), "");
        assert_eq!(to_markdown(&[vec![para("   ", Style::Body)]]), "");
    }
}
