//! Reading a Jupyter notebook into the paragraph model.
//!
//! # Why a notebook is a document
//!
//! An `.ipynb` is JSON: a list of cells, each markdown or code, each with
//! source text and — for code cells — the outputs of the last run. That maps
//! onto the paragraph model almost exactly. Markdown cells go through
//! `mdread`, code cells become preformatted blocks, and outputs become
//! preformatted blocks marked as such.
//!
//! So `notebook -> pdf` needs no new writer at all. It needs a reader, and
//! everything downstream already exists.
//!
//! # Detection, and why this one is clean
//!
//! A notebook is valid JSON, so it cannot be told from a `.json` by signature.
//! But unlike Markdown against plain text, it *can* be told by content: the
//! format requires `nbformat` and `cells` at the top level. That is a
//! structural check on required members — the same shape as separating a
//! `.docx` from a plain ZIP, which this codebase already does with
//! [`Family`](openconvert_core::format::Family). No guessing is involved.
//!
//! # What is dropped, and why it is said out loud
//!
//! **Images.** A plot is the point of many notebooks and it arrives as
//! base64-encoded PNG in the output data. The paragraph model has nowhere to
//! put an image, so plots are replaced by a line naming what was there rather
//! than silently vanishing — a PDF of a notebook with the figures missing and
//! nothing saying so would be worse than useless.
//!
//! **Execution counts, metadata, attachments, widget state.** None of it is
//! text a reader needs.

use serde_json::Value;

use crate::text::{Paragraph, Style};

/// Whether these bytes are a Jupyter notebook rather than some other JSON.
///
/// Checked on the parsed value, not by scanning for the strings: `"cells"`
/// appearing inside somebody's data file is not a notebook, and the top-level
/// shape is what the format actually requires.
#[must_use]
pub fn looks_like_notebook(value: &Value) -> bool {
    value.get("nbformat").is_some() && value.get("cells").is_some_and(Value::is_array)
}

/// Parse a notebook into paragraphs.
///
/// # Errors
///
/// Invalid JSON, or JSON that is not a notebook.
pub fn parse(bytes: &[u8]) -> Result<Vec<Paragraph>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("this file is not readable JSON: {e}"))?;
    if !looks_like_notebook(&value) {
        return Err(
            "this JSON has no `nbformat` and `cells`, so it is not a Jupyter notebook".into(),
        );
    }

    let cells = value
        .get("cells")
        .and_then(Value::as_array)
        .ok_or("this notebook has no cells array")?;

    let mut out = Vec::new();
    for cell in cells {
        let kind = cell.get("cell_type").and_then(Value::as_str).unwrap_or("");
        let source = join_source(cell.get("source"));

        match kind {
            // A markdown cell IS Markdown, so it goes through the same parser
            // every other text input uses. Headings written in a notebook then
            // behave exactly as headings written in a `.md`.
            "markdown" => out.extend(crate::mdread::parse(&source)),
            "code" => {
                if !source.trim().is_empty() {
                    out.push(Paragraph {
                        text: source.trim_end_matches('\n').to_string(),
                        style: Style::Code,
                    });
                }
                out.extend(outputs_of(cell));
            }
            // `raw` cells are passed through verbatim by nbconvert; the honest
            // equivalent here is preformatted text.
            "raw" if !source.trim().is_empty() => {
                out.push(Paragraph {
                    text: source.trim_end_matches('\n').to_string(),
                    style: Style::Code,
                });
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A cell's `source`, which the format allows to be a string or a list of
/// lines. Both occur in the wild; the list form is what Jupyter writes.
fn join_source(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(lines)) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// The text a code cell produced, and a note where it produced a picture.
fn outputs_of(cell: &Value) -> Vec<Paragraph> {
    let Some(outputs) = cell.get("outputs").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for output in outputs {
        let kind = output
            .get("output_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let text = match kind {
            // Anything written to stdout or stderr.
            "stream" => join_source(output.get("text")),
            // A result or a rich display. `data` holds one entry per MIME
            // type; plain text is the one this pipeline can carry.
            "execute_result" | "display_data" => {
                let data = output.get("data");
                let plain = data
                    .and_then(|d| d.get("text/plain"))
                    .map(|v| join_source(Some(v)))
                    .unwrap_or_default();

                // A FIGURE IS NAMED, NOT DROPPED. It is why many notebooks
                // exist, and a document that silently omitted it would be
                // quietly wrong in the way that matters most.
                let image = data
                    .and_then(Value::as_object)
                    .is_some_and(|d| d.keys().any(|k| k.starts_with("image/")));
                if image {
                    let label = if plain.trim().is_empty() {
                        "[figure omitted: this converter carries text, not images]".to_string()
                    } else {
                        format!(
                            "{}\n[figure omitted: this converter carries text, not images]",
                            plain.trim_end()
                        )
                    };
                    out.push(Paragraph {
                        text: label,
                        style: Style::Code,
                    });
                    continue;
                }
                plain
            }
            "error" => {
                let name = output.get("ename").and_then(Value::as_str).unwrap_or("");
                let value = output.get("evalue").and_then(Value::as_str).unwrap_or("");
                format!("{name}: {value}")
            }
            _ => String::new(),
        };

        let trimmed = text.trim_end_matches('\n');
        if !trimmed.trim().is_empty() {
            out.push(Paragraph {
                text: trimmed.to_string(),
                style: Style::Code,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Built with `json!` rather than raw string literals.
    ///
    /// A notebook's `source` is full of `"#` — every Markdown heading starts
    /// one — and `"#` is exactly what terminates a `r#"..."#` literal. Writing
    /// these as raw strings meant the test fixtures were being cut in half by
    /// their own content.
    fn notebook(cells: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "cells": cells,
        }))
        .expect("fixture")
    }

    #[test]
    fn a_notebook_is_recognised_and_plain_json_is_not() {
        let nb: Value = serde_json::from_slice(&notebook(json!([]))).unwrap();
        assert!(looks_like_notebook(&nb));

        for other in [
            json!({ "cells": [1, 2, 3] }),
            json!({ "nbformat": 4 }),
            json!({ "name": "a package", "version": "1.0" }),
            json!([1, 2, 3]),
        ] {
            assert!(
                !looks_like_notebook(&other),
                "{other} was taken for a notebook"
            );
        }
    }

    #[test]
    fn markdown_cells_become_headings_and_prose() {
        let bytes = notebook(json!([{
            "cell_type": "markdown",
            "source": ["# Title\n", "\n", "Some prose.\n"],
        }]));
        let out = parse(&bytes).expect("parse");
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].style, Style::Heading(1));
        assert_eq!(out[0].text, "Title");
        assert_eq!(out[1].style, Style::Body);
        assert_eq!(out[1].text, "Some prose.");
    }

    #[test]
    fn code_cells_keep_their_line_breaks() {
        let bytes = notebook(json!([{
            "cell_type": "code",
            "source": ["x = 1\n", "print(x)\n"],
            "outputs": [],
        }]));
        let out = parse(&bytes).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].style, Style::Code);
        assert_eq!(out[0].text, "x = 1\nprint(x)");
    }

    /// `source` may be a bare string rather than a list of lines.
    #[test]
    fn a_string_source_reads_the_same_as_a_line_list() {
        let bytes = notebook(json!([{
            "cell_type": "code",
            "source": "a = 2\n",
            "outputs": [],
        }]));
        let out = parse(&bytes).expect("parse");
        assert_eq!(out[0].text, "a = 2");
    }

    #[test]
    fn stream_output_is_kept() {
        let bytes = notebook(json!([{
            "cell_type": "code",
            "source": ["print(1)"],
            "outputs": [{ "output_type": "stream", "name": "stdout", "text": ["1\n"] }],
        }]));
        let out = parse(&bytes).expect("parse");
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[1].text, "1");
    }

    /// The one that matters: a plot must not vanish without a word.
    #[test]
    fn a_figure_is_named_rather_than_dropped() {
        let bytes = notebook(json!([{
            "cell_type": "code",
            "source": ["plot()"],
            "outputs": [{
                "output_type": "display_data",
                "data": {
                    "image/png": "iVBORw0KGgo=",
                    "text/plain": "<Figure size 640x480>",
                },
            }],
        }]));
        let out = parse(&bytes).expect("parse");
        let joined: String = out
            .iter()
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("figure omitted"), "{joined}");
        // And the base64 payload does not leak into the document.
        assert!(!joined.contains("iVBORw0KGgo"), "{joined}");
    }

    #[test]
    fn an_error_output_is_readable() {
        let bytes = notebook(json!([{
            "cell_type": "code",
            "source": ["1/0"],
            "outputs": [{
                "output_type": "error",
                "ename": "ZeroDivisionError",
                "evalue": "division by zero",
                "traceback": [],
            }],
        }]));
        let out = parse(&bytes).expect("parse");
        let joined: String = out
            .iter()
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("ZeroDivisionError: division by zero"),
            "{joined}"
        );
    }

    #[test]
    fn plain_json_is_refused_by_name() {
        let err = parse(br#"{"name":"not a notebook"}"#).unwrap_err();
        assert!(err.contains("not a Jupyter notebook"), "{err}");
    }

    #[test]
    fn broken_json_is_refused_by_name() {
        let err = parse(b"{not json").unwrap_err();
        assert!(err.contains("not readable JSON"), "{err}");
    }
}
