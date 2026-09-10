//! `cargo xtask routes` — regenerate `docs/ROUTES.md` from the route table.
//!
//! # Why generated and not written
//!
//! A hand-written list of what the program converts is a list that is wrong
//! within two commits. This one is produced from `RouteTable::v1()`, which is
//! the same table `route()` consults, so it cannot describe a conversion that
//! does not exist or omit one that does.
//!
//! `--check` verifies the committed file matches, and the gate runs that. The
//! failure mode it prevents is the ordinary one: a route added, the document
//! forgotten, and a reference that quietly becomes fiction.
//!
//! # What it does not tell you
//!
//! Whether a route will run **on your machine**. Every row's requirements are
//! listed, and an engine or a model that is absent turns a listed route into a
//! refusal — deliberately, and `openconvert routes <from> <to>` is what answers
//! that question locally. This is the capability of the build, not of the
//! installation.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Result};
use openconvert_core::format::{FormatId, MediaKind, TABLE};
use openconvert_core::route::{Requirement, Route, RouteTable};

/// Where the generated reference lives.
const DOC: &str = "docs/ROUTES.md";

/// Regenerate, or check.
///
/// # Errors
///
/// When the file cannot be written, or (with `check`) when it is out of date.
pub fn run(check: bool) -> Result<()> {
    let generated = render();
    let path = Path::new(DOC);

    if check {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        // Compare with newlines normalised: the file is committed on a machine
        // whose git may translate them, and a gate that fails on line endings
        // teaches people to ignore it.
        if current.replace("\r\n", "\n") != generated.replace("\r\n", "\n") {
            bail!(
                "{DOC} is out of date. The route table changed and the reference did not.\n\
                 Run `cargo xtask routes` and commit the result."
            );
        }
        println!("routes: ok — {DOC} matches the route table");
        return Ok(());
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &generated)?;
    println!("routes: wrote {DOC}");
    Ok(())
}

/// The document.
fn render() -> String {
    let table = RouteTable::v1();
    let routes = table.all();

    let mut out = String::with_capacity(64 * 1024);
    out.push_str(concat!(
        "# Routes\n",
        "\n",
        "**Generated — do not edit.** `cargo xtask routes` writes this file from\n",
        "`RouteTable::v1()`, the same table the planner consults, and\n",
        "`cargo xtask routes --check` fails the build if the two disagree.\n",
        "\n",
        "Every conversion this build knows how to perform is below. Whether one\n",
        "runs *on a given machine* is a different question: each route lists what\n",
        "it requires, and a missing engine or model turns it into a refusal that\n",
        "names what is absent. `openconvert routes <from> <to>` answers that\n",
        "locally.\n",
        "\n",
        "## Fidelity classes\n",
        "\n",
        "| Class | Meaning |\n",
        "|---|---|\n",
        "| **A** | Lossless. The bytes that matter survive exactly. |\n",
        "| **B** | Lossy, standard. A re-encode; the usual cost of changing format. |\n",
        "| **C** | Rebuilt. The output is reconstructed rather than translated. |\n",
        "| **D** | Generative. A model invented content that was not in the input. |\n",
        "\n",
    ));

    let _ = writeln!(
        out,
        "{} routes, across {} source formats.\n",
        routes.len(),
        routes
            .iter()
            .map(|r| r.from)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );

    // Grouped by the source's kind, then by source format, so the document
    // reads the way someone looks something up: "I have a photo, what can I do
    // with it".
    let mut by_kind: BTreeMap<&'static str, BTreeMap<&'static str, Vec<&Route>>> = BTreeMap::new();
    for route in routes {
        let kind = route.from.kind().map_or("Other", kind_name);
        by_kind
            .entry(kind)
            .or_default()
            .entry(name_of(route.from))
            .or_default()
            .push(route);
    }

    for (kind, sources) in &by_kind {
        let _ = writeln!(out, "## {kind}\n");
        for (source, list) in sources {
            let _ = writeln!(out, "### {source}\n");
            out.push_str("| To | Class | Steps | Requires |\n|---|---|---|---|\n");
            let mut rows: Vec<&&Route> = list.iter().collect();
            rows.sort_by_key(|r| name_of(r.to));
            for route in rows {
                let _ = writeln!(
                    out,
                    "| {} | {:?} | {} | {} |",
                    name_of(route.to),
                    route.class,
                    steps_of(route),
                    requirements_of(route.requires),
                );
            }
            out.push('\n');
        }
    }

    // The formats that exist but are never a destination, said out loud. A
    // reader who wants HEIC output should find out why here rather than by
    // trying it.
    let targets: std::collections::BTreeSet<FormatId> = routes.iter().map(|r| r.to).collect();
    let orphans: Vec<&'static str> = TABLE
        .iter()
        .filter(|f| f.id != FormatId::Unknown && !targets.contains(&f.id))
        .map(|f| f.name)
        .collect();
    if !orphans.is_empty() {
        out.push_str("## Read but never written\n\n");
        out.push_str(concat!(
            "These formats can be opened and converted *from*, and nothing\n",
            "converts *to* them. That is a deliberate absence rather than a gap:\n",
            "a route with no encoder behind it would be a promise the build\n",
            "cannot keep, and refusing by name beats failing at the last step.\n\n",
        ));
        for name in orphans {
            let _ = writeln!(out, "- `{name}`");
        }
        out.push('\n');
    }

    out
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "Images",
        MediaKind::Audio => "Audio",
        MediaKind::Video => "Video",
        MediaKind::Document => "Documents",
        MediaKind::Archive => "Archives",
        MediaKind::Tabular => "Tabular data",
        MediaKind::Font => "Fonts",
        MediaKind::Spreadsheet => "Spreadsheets",
    }
}

fn name_of(id: FormatId) -> &'static str {
    id.row().map_or("unknown", |r| r.name)
}

/// The steps as a readable chain, which is what makes a two-engine route
/// legible: `pdf -> png -> webp` says more than "Transcode, Transcode".
fn steps_of(route: &Route) -> String {
    use openconvert_core::plan::StepKind;

    // Built as a CHAIN, not a list of steps rendered side by side. A two-step
    // route's `to` and the next step's `from` are the same format, so joining
    // the debug output gives `docx → pdf → pdf → png` — which reads as four
    // conversions and is really two.
    let mut chain: Vec<String> = Vec::new();
    let mut push = |name: String| {
        if chain.last() != Some(&name) {
            chain.push(name);
        }
    };

    for step in route.steps {
        match step {
            StepKind::Transcode { from, to } => {
                push(name_of(*from).to_string());
                push(name_of(*to).to_string());
            }
            StepKind::RenderPage { .. } => push("render page".to_string()),
            StepKind::Infer { task, .. } => push(format!("infer ({task})")),
            StepKind::Pixel { op } => push(format!("pixels ({op})")),
            // Named ends, so the chain reads `gzip → tar → zip` rather than
            // `extract → extract`, which said nothing about where a repack
            // passes through.
            StepKind::Extract { from, to } => {
                push(name_of(*from).to_string());
                push(name_of(*to).to_string());
            }
            StepKind::StreamCopy => push("stream copy".to_string()),
            StepKind::StripMetadata => push("strip metadata".to_string()),
            StepKind::Trim { .. } => push("trim".to_string()),
        }
    }
    if chain.is_empty() {
        return "—".to_string();
    }
    chain.join(" → ")
}

fn requirements_of(requires: &[Requirement]) -> String {
    if requires.is_empty() {
        return "—".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for r in requires {
        parts.push(match r {
            Requirement::Engine(name) => format!("`{name}`"),
            Requirement::Always => "—".to_string(),
            other => format!("{other:?}"),
        });
    }
    parts.sort();
    parts.dedup();
    parts.join(", ")
}
