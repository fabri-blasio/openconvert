//! **The browser harness must offer the tools the app offers.**
//!
//! # The defect this exists for
//!
//! `apps/desktop/preview/fixtures.ts` mirrors `list_tools()` by hand so the
//! browser preview can render the workspace with no backend. Its own header
//! says the mirror has drifted three times in this project's life, and that
//! each time "the UI worked in the browser and was dead in the product".
//!
//! It had drifted again. The harness listed **four** PDF tools; the registry
//! advertises **eleven**. So the preview showed a rail with Compress, Merge,
//! Reorder and Sign, and the shipped app shows those plus Split, Extract,
//! Remove, Rotate, Crop, Add password and Remove password — seven tools that
//! could not be looked at, laid out, or noticed to be badly laid out, by
//! anyone working in the browser. Which is where this interface is worked on.
//!
//! Nothing caught it because nothing was looking: every gate over the tool
//! registry is Rust, and the mirror is TypeScript.
//!
//! # What it checks, and what it deliberately does not
//!
//! **The ids, in order, and nothing else.** Titles, parameters and
//! availability are not compared — a fixture is allowed to say a model is
//! downloaded when this machine's is not, which is most of what a fixture is
//! FOR. What it may not do is offer a different set of tools, or the same
//! tools in a different order, because then the thing being designed in the
//! browser is not the thing being shipped.
//!
//! Order is part of it because the harness returns its array as written, not
//! through `list_tools()` — so the `BY_FREQUENCY` sort that orders the real
//! rail never reaches the preview. The first time this was checked, it did not:
//! the browser rail read Remove background, Upscale, Invert, Black and white,
//! Compress, and the app's read Remove background, Compress, Upscale, Read
//! text. Comparing sequences is what stops that recurring.
//!
//! Parsed with a regex rather than a TypeScript parser, deliberately: a
//! dependency that understands TS would be a large addition to a workspace
//! that has none, to read a list of quoted strings.

use std::path::{Path, PathBuf};

/// The harness file, found from this crate rather than from the test's cwd.
fn fixtures() -> Option<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = here
        .parent()?
        .parent()?
        .join("apps")
        .join("desktop")
        .join("preview")
        .join("fixtures.ts");
    path.exists().then_some(path)
}

/// The ids inside `export const TOOLS`, in file order.
///
/// Bounded to that one array: `AI_FEATURES` above it also has `id:` keys, and
/// reading the whole file would compare tool ids against feature ids and fail
/// for a reason that is not about anything.
fn tool_ids(source: &str) -> Vec<String> {
    let Some(start) = source.find("export const TOOLS") else {
        return Vec::new();
    };
    let body = &source[start..];
    let end = body.find("\n];").map_or(body.len(), |i| i + 3);
    let body = &body[..end];

    let mut ids = Vec::new();
    let mut rest = body;
    // `{ id: "..."` — the shape every row starts with. A nested object (a
    // parameter, an option) is written `{ id: "..."` too, so this collects
    // parameter ids as well; the comparison below is a SUBSET check in one
    // direction and a membership check in the other, which is why that is
    // harmless. Tool ids all carry a category prefix, and parameter ids do
    // not, so filtering on the prefix separates them without a parser.
    while let Some(i) = rest.find("id: \"") {
        rest = &rest[i + 5..];
        if let Some(j) = rest.find('"') {
            let id = &rest[..j];
            if id.starts_with("image-") || id.starts_with("pdf-") || id.starts_with("audio-") {
                ids.push(id.to_string());
            }
            rest = &rest[j..];
        }
    }
    ids
}

#[test]
fn the_harness_offers_what_the_app_offers() {
    let Some(path) = fixtures() else {
        eprintln!("SKIPPED: the desktop harness is not in this checkout.");
        return;
    };
    let source = std::fs::read_to_string(&path).expect("read fixtures.ts");
    let listed = tool_ids(&source);
    assert!(
        !listed.is_empty(),
        "no tool ids were found in {}; the TOOLS array changed shape and this \
         test is now checking nothing",
        path.display()
    );

    let offered: Vec<String> = openconvert_run::tools::list_tools()
        .into_iter()
        .map(|t| t.id)
        .collect();

    let missing: Vec<&String> = offered.iter().filter(|id| !listed.contains(id)).collect();
    // A PARAMETER id carrying a category prefix would land here too, and that
    // is deliberate: none do today, and one that did would be a name colliding
    // with a tool's, which is worth failing over rather than quietly allowing.
    let extra: Vec<&String> = listed.iter().filter(|id| !offered.contains(id)).collect();

    assert!(
        missing.is_empty(),
        "the app offers these and the browser harness does not, so they cannot \
         be seen or laid out in the preview: {missing:?}\n  {}",
        path.display()
    );
    assert!(
        extra.is_empty(),
        "the browser harness offers these and the app does not, so the preview \
         shows a rail nobody has: {extra:?}\n  {}",
        path.display()
    );

    // AND IN THE SAME ORDER. Checked after the set, so a missing tool is
    // reported as a missing tool rather than as an ordering difference twenty
    // entries long.
    assert_eq!(
        listed,
        offered,
        "the harness lists the tools in a different order from the app. The \
         preview returns this array as written; the real rail is sorted by \
         `BY_FREQUENCY` in tools.rs, so the two only agree if this file is \
         kept in that order.\n  {}",
        path.display()
    );
}
