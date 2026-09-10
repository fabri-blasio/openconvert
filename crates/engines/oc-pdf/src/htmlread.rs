//! Reading HTML into the paragraph model.
//!
//! # Why this exists at all
//!
//! `Html` is a declared source with routes to every other document format, and
//! until this module every one of them produced **an empty file**. HTML arrived
//! at `mdread::parse` along with Markdown and plain text, on the reasoning that
//! CommonMark passes raw HTML through. It does — as `Event::Html`, which
//! `mdread` drops, because a Markdown reader emitting markup as prose is worse
//! than a Markdown reader ignoring it. A whole HTML document is one raw block,
//! so the whole document was dropped, and `html -> txt` reported success over
//! nothing.
//!
//! `run_text`'s comment described that as "its prose without its structure".
//! The prose did not survive either. `declared_routes_run` is the test that
//! found it and is what stops it recurring.
//!
//! # What this is, and firmly is not
//!
//! **A text extractor, not an HTML parser.** It does not build a DOM, does not
//! implement the HTML5 tree construction algorithm, does not resolve implied
//! end tags or the adoption agency, and treats every element as closing at its
//! own end tag or at the end of the document. It walks the byte stream once,
//! keeps the text between tags, and reads block-level tags for the structure
//! the [`Paragraph`] model can hold: headings with their depth, list items with
//! their nesting, preformatted blocks, and quotations.
//!
//! That distinction is the security argument. This worker parses hostile input
//! by design, and a real HTML parser is thousands of lines of state machine
//! whose whole job is to make sense of malformed markup. This is one pass with
//! no recursion, no lookahead beyond the current tag, and no allocation that
//! is not bounded by the input's own length. Every construct it does not
//! understand degrades to "the text inside it is kept".
//!
//! # What is dropped, and the receipt says so
//!
//! Inline markup (`<em>`, `<a>`, `<span>`) contributes its text and nothing
//! else: `Paragraph` carries one style per block and no character runs. Tables
//! flatten to a paragraph per row. `<script>`, `<style>`, `<head>` and
//! `<title>` contribute nothing at all — their contents are not prose, and a
//! stylesheet emitted as body text is how a naive tag-stripper announces
//! itself.

use crate::text::{Paragraph, Style};

/// Whether this text should be read as HTML rather than as CommonMark.
///
/// Deliberately the same shape as the signatures in `format.rs`, plus the
/// looser leading-`<` case: detection only calls a file `Html` when it opens
/// with `<!doctype html` or `<html`, but a fragment starting with `<p>` is
/// detected as `Txt` and is still HTML, and reading it as prose would emit its
/// tags as words. A plain-text file that happens to start with `<` loses
/// nothing by coming through here — with no tags to strip, the text arrives
/// unchanged.
#[must_use]
pub fn looks_like(source: &str) -> bool {
    source.trim_start().starts_with('<')
}

/// Elements whose content is not prose and is skipped entirely.
///
/// `head` covers `title`, `meta` and `link` in a well-formed document;
/// `title` is named separately because a document with no explicit `<head>`
/// still has one in every browser, and this reader does not imply elements.
const OPAQUE: [&str; 7] = [
    "script", "style", "head", "title", "template", "noscript", "svg",
];

/// Elements that end the current paragraph and start a new one.
///
/// Not exhaustive and does not need to be: an unlisted element contributes its
/// text to whatever paragraph is open, which is the right answer for every
/// inline element and a merged paragraph for the rare block one.
const BLOCK: [&str; 24] = [
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "address",
    "form",
    "fieldset",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "hr",
    "dl",
    "dt",
    "dd",
    "br",
];

/// The reader's state while it walks the document.
struct Reader {
    out: Vec<Paragraph>,
    /// The text of the paragraph being built.
    text: String,
    /// Open lists, outermost first. `None` is `<ul>`; `Some(n)` is an `<ol>`
    /// whose next item is number `n`.
    lists: Vec<Option<u64>>,
    /// The style the open `<li>` was given when it opened, so a nested list
    /// inside it cannot change the item's own marker retroactively.
    item: Option<Style>,
    /// Open `<h1>`–`<h6>` depth.
    heading: Option<u8>,
    /// `<pre>` nesting. Non-zero means whitespace is content.
    pre: usize,
    /// `<blockquote>` nesting.
    quote: usize,
}

impl Reader {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            text: String::new(),
            lists: Vec::new(),
            item: None,
            heading: None,
            pre: 0,
            quote: 0,
        }
    }

    /// What the paragraph currently being built is.
    ///
    /// Computed at flush time from the open elements rather than assigned when
    /// a tag opens, so an unclosed `<h2>` cannot leave every later paragraph a
    /// heading — the state is the stack, and the stack is what is asked.
    fn style(&self) -> Style {
        if self.pre > 0 {
            return Style::Code;
        }
        if let Some(depth) = self.heading {
            return Style::Heading(depth);
        }
        if let Some(item) = self.item {
            return item;
        }
        if self.quote > 0 {
            return Style::Quote;
        }
        Style::Body
    }

    /// End the paragraph being built, if it holds anything.
    fn flush(&mut self) {
        let style = self.style();
        // A code block's own whitespace is content; everything else is prose
        // and is re-wrapped by whatever writes it. Same rule as `mdread`.
        let body = if style.is_preformatted() {
            self.text.trim_matches('\n').to_string()
        } else {
            self.text.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        self.text.clear();
        if !body.is_empty() {
            self.out.push(Paragraph { text: body, style });
        }
    }

    /// A start tag, already lowercased and stripped of its attributes.
    fn open(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                // The byte after `h`, which the match above proved is 1-6.
                self.heading = name.as_bytes().get(1).map(|d| d - b'0');
            }
            "ul" | "ol" => {
                self.flush();
                self.lists.push(if name == "ol" { Some(1) } else { None });
            }
            "li" => {
                self.flush();
                // Depth is the number of lists ENCLOSING this one, so a
                // top-level item is 0. `u8` saturates rather than wrapping: a
                // document nested 256 deep is pathological, and reporting the
                // deepest depth we can name beats reporting depth 0.
                let depth = u8::try_from(self.lists.len().saturating_sub(1)).unwrap_or(u8::MAX);
                self.item = Some(match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let number = *next;
                        *next += 1;
                        Style::Ordered { depth, number }
                    }
                    // An `<li>` with no list around it is malformed and is
                    // still a list item; a bullet is the honest rendering.
                    Some(None) | None => Style::Bullet { depth },
                });
            }
            "pre" => {
                self.flush();
                self.pre += 1;
            }
            "blockquote" => {
                self.flush();
                self.quote += 1;
            }
            // A cell boundary is a word boundary, not a paragraph one: a row
            // reads as a line. `tr` is in BLOCK and ends it.
            "td" | "th" => self.text.push(' '),
            _ if BLOCK.contains(&name) => self.flush(),
            _ => {}
        }
    }

    /// An end tag, already lowercased.
    fn close(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.heading = None;
            }
            "ul" | "ol" => {
                self.flush();
                self.lists.pop();
                self.item = None;
            }
            "li" => {
                self.flush();
                self.item = None;
            }
            "pre" => {
                self.flush();
                self.pre = self.pre.saturating_sub(1);
            }
            "blockquote" => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            // `</body>` and `</html>` end whatever is open. They are not in
            // BLOCK because their START tags must not flush -- doing so is
            // harmless, but saying why here is cheaper than a reader
            // wondering.
            "body" | "html" => self.flush(),
            _ if BLOCK.contains(&name) => self.flush(),
            _ => {}
        }
    }
}

/// Parse HTML into paragraphs.
///
/// Never fails and never panics: every malformed construct has a defined
/// degradation, and an unterminated tag consumes the rest of the input rather
/// than being emitted as text.
#[must_use]
pub fn parse(source: &str) -> Vec<Paragraph> {
    let mut r = Reader::new();
    let mut rest = source;

    while let Some(at) = rest.find('<') {
        let (before, from_tag) = rest.split_at(at);
        push_text(&mut r, before);

        // `<!-- -->`, `<!DOCTYPE>` and `<?...>` carry no text. An unterminated
        // one runs to the end of the document, which is what a browser does.
        if let Some(after) = skip_bracketed(from_tag) {
            rest = after;
            continue;
        }

        let Some((name, closing, after)) = tag(from_tag) else {
            // A `<` that opens no tag is a literal `<` in the text.
            push_text(&mut r, "<");
            rest = &from_tag[1..];
            continue;
        };
        rest = after;

        if OPAQUE.contains(&name.as_str()) && !closing {
            r.flush();
            rest = skip_element(rest, &name);
            continue;
        }
        if closing {
            r.close(&name);
        } else {
            r.open(&name);
        }
    }

    push_text(&mut r, rest);
    r.flush();
    r.out
}

/// Text between tags, with entities resolved.
fn push_text(r: &mut Reader, raw: &str) {
    if raw.is_empty() {
        return;
    }
    // Outside `<pre>` the paragraph is re-wrapped at flush, so whitespace
    // here only has to keep words apart. Inside it, every byte is content.
    if r.pre > 0 {
        decode_into(raw, &mut r.text);
    } else if raw.trim().is_empty() {
        // Whitespace-only text still separates the words on either side of an
        // inline tag: `a <em>b</em> c` must not become "ab c".
        if !r.text.ends_with(' ') && !r.text.is_empty() {
            r.text.push(' ');
        }
    } else {
        decode_into(raw, &mut r.text);
    }
}

/// Skip a comment, doctype or processing instruction, if that is what starts
/// here. Returns what follows it.
fn skip_bracketed(from_tag: &str) -> Option<&str> {
    let end = if from_tag.starts_with("<!--") {
        from_tag.find("-->").map(|i| i + 3)
    } else if from_tag.starts_with("<!") || from_tag.starts_with("<?") {
        from_tag.find('>').map(|i| i + 1)
    } else {
        return None;
    };
    // Unterminated: the rest of the document is inside it.
    Some(end.map_or("", |i| &from_tag[i..]))
}

/// Read one tag: its lowercased name, whether it is an end tag, and what
/// follows it.
///
/// Attributes are scanned rather than parsed, because the only question asked
/// of them is where the tag ends — and a quoted attribute value may contain
/// `>`, which is the one thing a naive scan gets wrong.
fn tag(from_tag: &str) -> Option<(String, bool, &str)> {
    let after_bracket = from_tag.strip_prefix('<')?;
    let (closing, body) = after_bracket
        .strip_prefix('/')
        .map_or((false, after_bracket), |b| (true, b));

    let name: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if name.is_empty() {
        return None;
    }

    let mut quote: Option<char> = None;
    for (i, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            // The tag ends here; `/>` needs no separate case, since a
            // self-closing element has no end tag to match and every element
            // this reader acts on closes at its own or at the document's end.
            (None, '>') => return Some((name, closing, &body[i + 1..])),
            (None, _) => {}
        }
    }
    // Unterminated tag: it swallows the rest of the input.
    Some((name, closing, ""))
}

/// Skip to just past `</name>`, or to the end if it never closes.
///
/// Used for `<script>` and friends, whose contents are not markup: a `<` inside
/// a script is an operator, not a tag, and reading it as one is how a
/// tag-stripper ends up emitting JavaScript as prose.
fn skip_element<'a>(rest: &'a str, name: &str) -> &'a str {
    let mut search = rest;
    let mut consumed = 0_usize;
    while let Some(at) = search.find("</") {
        let after = &search[at + 2..];
        let matches = after.len() >= name.len()
            && after[..name.len()].eq_ignore_ascii_case(name)
            && after[name.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
        if matches {
            // `tag` takes the `<`, so the slice starts AT the bracket. Passing
            // it one byte later handed it `/head>`, which parses as no tag at
            // all — and the fallback for no tag is "the rest of the document
            // is inside this element", which silently ate every document with
            // a `<head>`.
            let tail = &rest[consumed + at..];
            return tag(tail).map_or("", |(_, _, after_tag)| after_tag);
        }
        consumed += at + 2;
        search = after;
    }
    ""
}

/// The named entities worth carrying, and why the list stops here.
///
/// HTML5 names over two thousand; a table of them would be larger than this
/// module and would still be a table. These are the ones that appear in prose,
/// plus the five that are mandatory. Anything else is left **exactly as
/// written** — `&foo;` comes out as `&foo;` — which is visible and honest,
/// where a silent drop would quietly delete text.
const NAMED: [(&str, &str); 30] = [
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("ndash", "\u{2013}"),
    ("mdash", "\u{2014}"),
    ("hellip", "\u{2026}"),
    ("lsquo", "\u{2018}"),
    ("rsquo", "\u{2019}"),
    ("ldquo", "\u{201c}"),
    ("rdquo", "\u{201d}"),
    ("bull", "\u{2022}"),
    ("middot", "\u{b7}"),
    ("copy", "\u{a9}"),
    ("reg", "\u{ae}"),
    ("trade", "\u{2122}"),
    ("deg", "\u{b0}"),
    ("plusmn", "\u{b1}"),
    ("times", "\u{d7}"),
    ("divide", "\u{f7}"),
    ("laquo", "\u{ab}"),
    ("raquo", "\u{bb}"),
    ("sect", "\u{a7}"),
    ("para", "\u{b6}"),
    ("dagger", "\u{2020}"),
    ("euro", "\u{20ac}"),
    ("pound", "\u{a3}"),
    ("yen", "\u{a5}"),
];

/// Append `raw` with its character references resolved.
fn decode_into(raw: &str, out: &mut String) {
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // A reference is short. Bounding the search stops a lone `&` in prose
        // from scanning the rest of the document for a `;` that closes some
        // unrelated entity thousands of characters later.
        let window = after.len().min(32);
        match after[..window]
            .find(';')
            .and_then(|end| resolve(&after[..end]).map(|text| (text, &after[end + 1..])))
        {
            Some((text, tail)) => {
                out.push_str(&text);
                rest = tail;
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

/// One entity's body — what sits between `&` and `;` — as text.
fn resolve(body: &str) -> Option<String> {
    if let Some(digits) = body.strip_prefix('#') {
        let code = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            digits.parse::<u32>().ok()?
        };
        return char::from_u32(code).map(String::from);
    }
    NAMED
        .iter()
        .find(|(name, _)| *name == body)
        .map(|(_, text)| (*text).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(html: &str) -> Vec<(Style, String)> {
        parse(html).into_iter().map(|p| (p.style, p.text)).collect()
    }

    /// The defect this module was written for: our own writer's output, read
    /// back. It used to yield nothing at all.
    #[test]
    fn a_whole_document_round_trips() {
        let html = crate::html::write(
            &[
                Paragraph {
                    text: "A Heading".to_string(),
                    style: Style::Heading(1),
                },
                Paragraph {
                    text: "Some prose.".to_string(),
                    style: Style::Body,
                },
            ],
            "Document",
        );
        assert_eq!(
            styles(&html),
            vec![
                (Style::Heading(1), "A Heading".to_string()),
                (Style::Body, "Some prose.".to_string()),
            ]
        );
    }

    #[test]
    fn headings_carry_their_own_depth() {
        assert_eq!(
            styles("<h1>One</h1><h3>Three</h3>"),
            vec![
                (Style::Heading(1), "One".to_string()),
                (Style::Heading(3), "Three".to_string()),
            ]
        );
    }

    #[test]
    fn lists_carry_their_nesting_and_their_numbers() {
        let out = styles("<ol><li>first<ul><li>inner</li></ul></li><li>second</li></ol>");
        assert_eq!(
            out,
            vec![
                (
                    Style::Ordered {
                        depth: 0,
                        number: 1
                    },
                    "first".to_string()
                ),
                (Style::Bullet { depth: 1 }, "inner".to_string()),
                (
                    Style::Ordered {
                        depth: 0,
                        number: 2
                    },
                    "second".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_pre_block_keeps_its_line_breaks() {
        let out = parse("<pre><code>one\n  two\n</code></pre>");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].style, Style::Code);
        assert_eq!(out[0].text, "one\n  two");
    }

    #[test]
    fn inline_markup_contributes_its_text_and_keeps_the_spaces() {
        assert_eq!(
            styles("<p>a <em>b</em> <a href=\"x\">c</a></p>"),
            vec![(Style::Body, "a b c".to_string())]
        );
    }

    /// A stylesheet emitted as body text is how a naive tag-stripper announces
    /// itself, and `html::write` embeds one in every document it writes.
    #[test]
    fn scripts_stylesheets_and_the_head_contribute_nothing() {
        let out = parse(
            "<head><title>T</title><style>body { color: red }</style></head>\
             <body><script>if (a < b) { alert('x') }</script><p>kept</p></body>",
        );
        assert_eq!(styles_only(&out), vec!["kept".to_string()]);
    }

    fn styles_only(out: &[Paragraph]) -> Vec<String> {
        out.iter().map(|p| p.text.clone()).collect()
    }

    #[test]
    fn entities_resolve_and_unknown_ones_survive_verbatim() {
        assert_eq!(
            styles("<p>a &amp; b &#65; &#x42; &nosuch; &</p>"),
            vec![(Style::Body, "a & b A B &nosuch; &".to_string())]
        );
    }

    #[test]
    fn an_attribute_may_contain_a_bracket() {
        assert_eq!(
            styles("<p title=\"a > b\">text</p>"),
            vec![(Style::Body, "text".to_string())]
        );
    }

    #[test]
    fn a_table_reads_a_paragraph_to_the_row() {
        assert_eq!(
            styles("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>"),
            vec![
                (Style::Body, "a b".to_string()),
                (Style::Body, "c".to_string()),
            ]
        );
    }

    /// Malformed input has a defined degradation rather than a panic, and an
    /// unclosed heading must not make the rest of the document a heading.
    #[test]
    fn malformed_input_terminates() {
        assert_eq!(
            parse("<p>unterminated"),
            vec![Paragraph {
                text: "unterminated".to_string(),
                style: Style::Body,
            }]
        );
        assert!(parse("<!-- never closed").is_empty());
        assert!(parse("<script>never closed").is_empty());
        assert_eq!(styles("a < b"), vec![(Style::Body, "a < b".to_string())]);
        // Brackets that open no tag are text, which is also what a browser
        // renders them as.
        assert_eq!(styles("<<<>>>"), vec![(Style::Body, "<<<>>>".to_string())]);
    }

    #[test]
    fn a_blockquote_is_a_quote() {
        assert_eq!(
            styles("<blockquote><p>said</p></blockquote><p>after</p>"),
            vec![
                (Style::Quote, "said".to_string()),
                (Style::Body, "after".to_string()),
            ]
        );
    }
}
