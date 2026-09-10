//! Writing HTML from the paragraph model.
//!
//! # A whole document, not a fragment
//!
//! What comes out is a standalone file with a doctype, a charset and enough
//! style to be readable — because the thing someone asked for was a file they
//! can open, and a bare run of `<p>` elements in a browser is unstyled,
//! full-width and unreadable at any window size.
//!
//! The CSS is deliberately small and inline. A stylesheet beside the file
//! would be a second artifact this converter has nowhere to put, and a link to
//! a CDN would be a network request from a document produced by a program
//! whose entire premise is that nothing leaves the machine.
//!
//! # Escaping is not optional here
//!
//! Every other writer in this crate escapes for XML. This one escapes for
//! HTML, and the difference matters in one place: the text arriving here came
//! out of a PDF, a `.docx` or a Markdown file, all of which can contain `<`
//! and `&` legitimately. Emitting them raw would not merely look wrong, it
//! would let a crafted input inject markup into the output — the same class of
//! defect as an unescaped SQL string, in a file the user is about to open in a
//! browser.

use crate::text::{Paragraph, Style};

/// Render paragraphs as a complete HTML document.
#[must_use]
pub fn write(paragraphs: &[Paragraph], title: &str) -> String {
    let mut out = String::with_capacity(paragraphs.len() * 96 + 1024);
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape(title)));
    out.push_str(STYLE);
    out.push_str("</head>\n<body>\n");

    // Lists are the one structure that needs opening and closing around a run
    // of paragraphs rather than per paragraph, so the writer tracks what is
    // open. `open` holds one entry per nesting level: true for ordered.
    let mut open: Vec<bool> = Vec::new();

    for para in paragraphs {
        let (is_list, depth, ordered) = match para.style {
            Style::Bullet { depth } => (true, usize::from(depth), false),
            Style::Ordered { depth, .. } => (true, usize::from(depth), true),
            _ => (false, 0, false),
        };

        if is_list {
            // Close deeper lists, then any list of the wrong kind at this
            // level, then open what is missing.
            while open.len() > depth + 1 {
                close_list(&mut out, &mut open);
            }
            if open.len() == depth + 1 && open[depth] != ordered {
                close_list(&mut out, &mut open);
            }
            while open.len() < depth + 1 {
                out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
                open.push(ordered);
            }
            out.push_str(&format!("<li>{}</li>\n", escape(&para.text)));
            continue;
        }

        while !open.is_empty() {
            close_list(&mut out, &mut open);
        }

        match para.style {
            Style::Heading(level) => {
                let h = level.clamp(1, 6);
                out.push_str(&format!("<h{h}>{}</h{h}>\n", escape(&para.text)));
            }
            // `<pre>` and nothing else: the newlines and spaces inside are the
            // content, and any element that collapses whitespace destroys it.
            Style::Code => {
                out.push_str(&format!("<pre><code>{}</code></pre>\n", escape(&para.text)));
            }
            Style::Quote => {
                out.push_str(&format!(
                    "<blockquote><p>{}</p></blockquote>\n",
                    escape(&para.text)
                ));
            }
            _ => out.push_str(&format!("<p>{}</p>\n", escape(&para.text))),
        }
    }
    while !open.is_empty() {
        close_list(&mut out, &mut open);
    }

    out.push_str("</body>\n</html>\n");
    out
}

fn close_list(out: &mut String, open: &mut Vec<bool>) {
    if let Some(ordered) = open.pop() {
        out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
    }
}

/// Enough style to be readable, and no more.
const STYLE: &str = concat!(
    "<style>\n",
    "  :root { color-scheme: light dark; }\n",
    "  body {\n",
    "    max-width: 46rem; margin: 3rem auto; padding: 0 1.25rem;\n",
    "    font: 16px/1.6 system-ui, -apple-system, Segoe UI, Roboto, sans-serif;\n",
    "  }\n",
    "  h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2rem 0 .75rem; }\n",
    "  pre {\n",
    "    overflow-x: auto; padding: .85rem 1rem; border-radius: 6px;\n",
    "    background: rgba(127,127,127,.12);\n",
    "  }\n",
    "  code { font: .9em/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; }\n",
    "  blockquote {\n",
    "    margin: 1rem 0; padding-left: 1rem;\n",
    "    border-left: 3px solid rgba(127,127,127,.4); font-style: italic;\n",
    "  }\n",
    "</style>\n",
);

/// HTML-escape, and drop the control characters no document can hold.
///
/// `<` and `&` are the two that matter for injection; `>` and the quotes are
/// escaped as well because this text also reaches attribute position (the
/// `<title>`), and one escaping function that is safe everywhere beats two
/// that are each safe somewhere.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
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
    fn it_writes_a_whole_document() {
        let out = write(&[para("Hello", Style::Body)], "Doc");
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.contains("<meta charset=\"utf-8\">"));
        assert!(out.contains("<title>Doc</title>"));
        assert!(out.contains("<p>Hello</p>"));
        assert!(out.trim_end().ends_with("</html>"));
    }

    /// The defect this writer must not have.
    #[test]
    fn markup_in_the_text_cannot_become_markup_in_the_output() {
        let out = write(
            &[para("<script>alert(1)</script> & \"quoted\"", Style::Body)],
            "t",
        );
        assert!(!out.contains("<script>"), "{out}");
        assert!(out.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(out.contains("&amp;"));
    }

    /// And in the title, which is attribute-adjacent.
    #[test]
    fn the_title_is_escaped_too() {
        let out = write(&[], "</title><script>x</script>");
        assert!(!out.contains("<script>"), "{out}");
    }

    #[test]
    fn headings_use_the_level_given() {
        let out = write(&[para("A", Style::Heading(3))], "t");
        assert!(out.contains("<h3>A</h3>"), "{out}");
    }

    /// Real lists, not markers in text: HTML has the structure natively, so
    /// unlike the DOCX and ODT writers there is no reason to approximate.
    #[test]
    fn lists_nest_and_close() {
        let out = write(
            &[
                para("top", Style::Bullet { depth: 0 }),
                para("nested", Style::Bullet { depth: 1 }),
                para("back", Style::Bullet { depth: 0 }),
                para("after", Style::Body),
            ],
            "t",
        );
        // One outer list containing an inner one, both closed before the
        // paragraph that follows.
        assert_eq!(out.matches("<ul>").count(), 2, "{out}");
        assert_eq!(out.matches("</ul>").count(), 2, "{out}");
        let ul_end = out.rfind("</ul>").expect("closed");
        let p = out.find("<p>after</p>").expect("paragraph");
        assert!(ul_end < p, "the list was not closed before the paragraph");
    }

    /// A bulleted list followed by a numbered one at the same depth closes the
    /// first rather than putting `<li>` of one kind inside the other.
    #[test]
    fn switching_list_kind_closes_the_previous_one() {
        let out = write(
            &[
                para("a", Style::Bullet { depth: 0 }),
                para(
                    "b",
                    Style::Ordered {
                        depth: 0,
                        number: 1,
                    },
                ),
            ],
            "t",
        );
        assert_eq!(out.matches("<ul>").count(), 1);
        assert_eq!(out.matches("</ul>").count(), 1);
        assert_eq!(out.matches("<ol>").count(), 1);
        assert!(out.find("</ul>") < out.find("<ol>"), "{out}");
    }

    #[test]
    fn code_is_preformatted() {
        let out = write(&[para("fn main() {\n    ok\n}", Style::Code)], "t");
        assert!(
            out.contains("<pre><code>fn main() {\n    ok\n}</code></pre>"),
            "{out}"
        );
    }

    #[test]
    fn quotes_are_blockquotes() {
        let out = write(&[para("said so", Style::Quote)], "t");
        assert!(
            out.contains("<blockquote><p>said so</p></blockquote>"),
            "{out}"
        );
    }
}
