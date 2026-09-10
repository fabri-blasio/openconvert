//! **A route the table declares must actually run.**
//!
//! # The defect this exists for
//!
//! `Pdf -> Html`, `Docx -> Html` and `Odt -> Html` were declared in
//! `RouteTable::v1()`, passed routing, spawned the worker, and were refused
//! there with "oc-pdf does not write html" — because the engine's list of
//! accepted targets did not carry `html`, and nothing checked that the table
//! and that list agreed.
//!
//! A declared-but-unimplemented route is worse than a missing one. A missing
//! route refuses during planning, before anything runs, and says why. This kind
//! survives planning, appears in `openconvert routes` and in `docs/ROUTES.md` as
//! a capability of the build, and fails at the last step with an engine's
//! internal wording.
//!
//! `source_formats_are_reachable_by_detection` in the core's property tests
//! catches the mirror image — a route whose SOURCE nothing can detect — and
//! cannot catch this one: the engine's target list is a constant inside a
//! different binary.
//!
//! # Why it drives the real CLI
//!
//! The pipeline that failed is detect → probe → route → execute, and it lives
//! in this binary. Reimplementing it here would test the reimplementation.
//! `CARGO_BIN_EXE_openconvert` is the shipped path, and the engines resolve
//! beside it exactly as they do in an install.
//!
//! # Scope
//!
//! The document family: every declared pair among `pdf docx odt html txt
//! markdown notebook pptx odp epub`, which is where the routes have been
//! changing and where every instance of this defect has appeared. Raster and audio pairs are held
//! by `every_tool_writes_a_file` and by the core's property tests.
//!
//! It asserts that the conversion runs and writes a non-empty file. It does not
//! assert the content is right — that is each engine's own tests, which exist.
//! This is the seam between them and the product.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The binary under test, built by cargo for this integration test.
const CLI: &str = env!("CARGO_BIN_EXE_openconvert");

/// The document formats this test builds a real fixture for.
///
/// `markdown` is absent on purpose: nothing detects it, so it is a target only.
/// `postscript` is absent because it has no routes in either direction — if it
/// gains any, it belongs here.
const SOURCES: [&str; 9] = [
    "txt", "pdf", "docx", "odt", "html", "notebook", "pptx", "odp", "epub",
];

/// The document formats a route may end at.
const TARGETS: [&str; 6] = ["txt", "markdown", "html", "pdf", "docx", "odt"];

/// Below this, the fixtures or the table have changed enough that a pass no
/// longer means what this test says it means. The count at the time of writing
/// is 49.
const EXPECTED_AT_LEAST: usize = 46;

/// A word that appears in the seed, in no format's boilerplate, and in no
/// dictionary — so finding it in an output means the input's own text reached
/// it, rather than a container being well formed and empty.
///
/// `html -> txt` wrote a zero-byte file and reported success; `html -> pdf`
/// wrote a valid PDF of nothing at all, and only the first was caught by
/// "the file is not empty". This is what separates them.
const CANARY: &str = "Quirinalia";

/// Targets whose bytes contain their text as text, so the canary can be looked
/// for directly. DOCX and ODT deflate it and a PDF may compress its streams;
/// checking those means parsing them, which is the engines' own tests' job.
const READABLE: [&str; 3] = ["txt", "markdown", "html"];

// ---------------------------------------------------------------------------

fn run(args: &[&str]) -> (bool, String) {
    // THREE ATTEMPTS, AND ONLY FOR A FAILURE TO START. See
    // `is_transient_spawn_failure` for what is known and what is inferred.
    for attempt in 0..3 {
        let out = Command::new(CLI).args(args).output().expect("run the CLI");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() || !is_transient_spawn_failure(&text) {
            return (out.status.success(), text);
        }
        // Long enough for a scanner to finish with a freshly linked binary,
        // short enough that a genuine failure still reports in under a second.
        std::thread::sleep(std::time::Duration::from_millis(150 * (attempt + 1)));
    }
    let out = Command::new(CLI).args(args).output().expect("run the CLI");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Format name to canonical extension, read from the binary rather than
/// restated here. `markdown` is `md` and `notebook` is `ipynb`; a second copy
/// of that mapping in a test is a second thing to keep in step.
fn extensions() -> BTreeMap<String, String> {
    let (ok, text) = run(&["formats"]);
    assert!(ok, "openconvert formats failed:\n{text}");
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some((f.next()?.to_string(), f.nth(1)?.to_string()))
        })
        .collect()
}

/// Every pair the route table declares, from `openconvert routes` with no
/// arguments — the table itself, not a copy of it.
fn declared() -> Vec<(String, String)> {
    let (ok, text) = run(&["routes"]);
    assert!(ok, "openconvert routes failed:\n{text}");
    let pairs: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let from = f.next()?;
            if f.next()? != "->" {
                return None;
            }
            Some((from.to_string(), f.next()?.to_string()))
        })
        .collect();
    assert!(
        pairs.len() > 100,
        "parsed only {} routes from `openconvert routes`; its output format \
         changed and this test is reading nothing",
        pairs.len()
    );
    pairs
}

/// A private directory for this run.
fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openconvert-declared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    dir
}

/// A transient failure to START a worker, as opposed to a real refusal.
///
/// # What is known, and what is inferred
///
/// **Known.** Under `cargo test --workspace` these tests occasionally fail with
/// `could not start oc-archive: ... The system cannot find the file specified`,
/// naming a path that exists — a watcher polling that file through a whole run
/// never saw it absent. It has never reproduced in isolation: six consecutive
/// runs of this file, and twenty concurrent conversions driven from a shell,
/// all passed. It only appears in a full-workspace run, shortly after cargo
/// links the binaries.
///
/// **Inferred.** On Windows a freshly written executable can be briefly
/// unopenable while a real-time scanner has it, and `CreateProcess` reports
/// that as ERROR_FILE_NOT_FOUND rather than a sharing violation. Real-time
/// protection is on for this machine. That fits every observation above and is
/// not proven — checking the scanner's exclusions needs administrator rights.
///
/// So this retries a failure to START, briefly, and only when the engine
/// binary is actually on disk. It does NOT retry a refusal: a route the engine
/// declines, a malformed file or a cap crossed all fail on the first attempt,
/// which is the whole point of these tests.
fn is_transient_spawn_failure(text: &str) -> bool {
    text.contains("could not start") && engine_dir().is_some()
}

/// The directory the CLI resolves its engines from, if it exists.
fn engine_dir() -> Option<std::path::PathBuf> {
    let dir = Path::new(CLI).parent()?.to_path_buf();
    dir.is_dir().then_some(dir)
}

/// Convert one file, and say what happened.
///
/// The output lands beside the input, named from the input's stem and the
/// target's canonical extension (`cmd::convert::output_name`).
fn convert(input: &Path, to: &str, ext: &str) -> Result<PathBuf, String> {
    let (ok, text) = run(&["convert", &input.to_string_lossy(), "-t", to]);
    let brief = || text.lines().take(6).collect::<Vec<_>>().join(" | ");
    if !ok {
        return Err(brief());
    }
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let out = input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}.{ext}"));
    if out.is_file() {
        Ok(out)
    } else {
        Err(format!("reported success and wrote no {ext}: {}", brief()))
    }
}

/// A minimal notebook, written by hand so the test needs no Jupyter.
fn notebook_bytes() -> Vec<u8> {
    // Written with escaped quotes rather than a raw string: the markdown cell
    // holds a `##` heading, and every raw delimiter short enough to be
    // readable is closed early by some run of `"` and `#` inside JSON that is
    // itself about markdown.
    let cells = format!(
        "{{\"cell_type\":\"markdown\",\"source\":[\"## Heading\\n\",\"\\n\",\"Prose about \
         {CANARY}.\\n\"]}},\
         {{\"cell_type\":\"code\",\"source\":[\"x = 1\\n\"],\"outputs\":[],\
         \"execution_count\":1,\"metadata\":{{}}}}"
    );
    format!("{{\"nbformat\":4,\"nbformat_minor\":5,\"metadata\":{{}},\"cells\":[{cells}]}}")
        .into_bytes()
}

/// One real file per source format.
///
/// The converter builds its own inputs: a PDF fixture produced by `txt -> pdf`
/// is the PDF this build actually writes, which is the file a user then feeds
/// back in. A committed fixture would test a file no version of this program
/// produced.
fn fixtures(dir: &Path, ext: &BTreeMap<String, String>) -> Vec<(&'static str, PathBuf)> {
    let seed = dir.join("seed.txt");
    std::fs::write(
        &seed,
        format!(
            "A Heading\n\nSome prose about {CANARY} long enough that it has to \
             wrap when it is typeset onto a page of a fixed width.\n\nA second \
             paragraph.\n"
        ),
    )
    .expect("seed");

    let mut built: Vec<(&'static str, PathBuf)> = vec![("txt", seed.clone())];
    for from in SOURCES {
        match from {
            // Built above, or built below by hand: nothing in this build
            // WRITES a deck, an OpenDocument deck or a book, so they cannot be
            // produced by converting the seed.
            "txt" | "notebook" | "pptx" | "odp" | "epub" => continue,
            _ => {}
        }
        let e = &ext[from];
        match convert(&seed, from, e) {
            Ok(path) => {
                // Its own stem, so converting it later cannot collide with a
                // sibling's output.
                let named = dir.join(format!("from-{from}.{e}"));
                std::fs::rename(&path, &named).expect("rename fixture");
                built.push((from, named));
            }
            Err(why) => panic!("could not build the {from} fixture from txt: {why}"),
        }
    }

    let nb = dir.join("from-notebook.ipynb");
    std::fs::write(&nb, notebook_bytes()).expect("notebook");
    built.push(("notebook", nb));

    // THE THREE THIS BUILD READS AND DOES NOT WRITE. A deck, an OpenDocument
    // deck and a book have to be built here, because no route produces one --
    // which is exactly why they need this test: nothing else in the workspace
    // drives them end to end through the real worker.
    for (from, bytes) in [
        ("pptx", pptx_bytes()),
        ("odp", odp_bytes()),
        ("epub", epub_bytes()),
    ] {
        let path = dir.join(format!("from-{from}.{}", ext[from]));
        std::fs::write(&path, bytes).expect("write the fixture");
        built.push((leak(from), path));
    }
    built
}

/// The source names, as `&'static str`, so a fixture can be pushed by name.
fn leak(name: &str) -> &'static str {
    SOURCES
        .iter()
        .copied()
        .find(|s| *s == name)
        .unwrap_or_else(|| panic!("{name} is not in SOURCES"))
}

/// A ZIP of the given members.
///
/// **`mimetype` is written first and STORED**, which is what OpenDocument and
/// EPUB both require and what makes them identifiable without decompressing
/// anything. Written deflated, these fixtures were detected as plain zips —
/// which is the right answer for a file that does not follow its own format's
/// rule, and the wrong fixture for testing one that does.
fn zipped(members: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let deflated: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    let stored = deflated.compression_method(zip::CompressionMethod::Stored);
    for (name, body) in members {
        let opts = if *name == "mimetype" {
            stored
        } else {
            deflated
        };
        w.start_file(*name, opts).expect("start");
        w.write_all(body.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A two-slide PPTX whose presentation states the order.
fn pptx_bytes() -> Vec<u8> {
    let slide = |title: &str, point: &str| {
        format!(
            "<p:sld><p:cSld><p:spTree>\
             <p:sp><p:nvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
             <p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>\
             <p:sp><p:txBody><a:p><a:r><a:t>{point}</a:t></a:r></a:p></p:txBody></p:sp>\
             </p:spTree></p:cSld></p:sld>"
        )
    };
    let s1 = slide("A Heading", &format!("Some prose about {CANARY}"));
    let s2 = slide("A Second Slide", "and a second point");
    zipped(&[
        ("ppt/slides/slide1.xml", &s1),
        ("ppt/slides/slide2.xml", &s2),
        (
            "ppt/presentation.xml",
            "<p:presentation><p:sldIdLst>\
             <p:sldId id=\"256\" r:id=\"rId1\"/><p:sldId id=\"257\" r:id=\"rId2\"/>\
             </p:sldIdLst></p:presentation>",
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            "<Relationships>\
             <Relationship Id=\"rId1\" Target=\"slides/slide1.xml\"/>\
             <Relationship Id=\"rId2\" Target=\"slides/slide2.xml\"/>\
             </Relationships>",
        ),
    ])
}

/// A two-page ODP. The `mimetype` member is what identifies it.
fn odp_bytes() -> Vec<u8> {
    let content = format!(
        "<office:document-content><office:body><office:presentation>\
         <draw:page><draw:frame presentation:class=\"title\"><draw:text-box>\
         <text:p>A Heading</text:p></draw:text-box></draw:frame>\
         <draw:frame presentation:class=\"outline\"><draw:text-box>\
         <text:p>Some prose about {CANARY}</text:p></draw:text-box></draw:frame></draw:page>\
         <draw:page><draw:frame presentation:class=\"title\"><draw:text-box>\
         <text:p>A Second Slide</text:p></draw:text-box></draw:frame></draw:page>\
         </office:presentation></office:body></office:document-content>"
    );
    zipped(&[
        (
            "mimetype",
            "application/vnd.oasis.opendocument.presentation",
        ),
        ("content.xml", &content),
    ])
}

/// A two-chapter EPUB whose spine states the order.
fn epub_bytes() -> Vec<u8> {
    let ch1 =
        format!("<html><body><h1>A Heading</h1><p>Some prose about {CANARY}.</p></body></html>");
    zipped(&[
        ("mimetype", "application/epub+zip"),
        (
            "META-INF/container.xml",
            "<container><rootfiles><rootfile full-path=\"OEBPS/content.opf\"/>\
             </rootfiles></container>",
        ),
        (
            "OEBPS/content.opf",
            "<package><manifest>\
             <item id=\"c1\" href=\"one.xhtml\"/><item id=\"c2\" href=\"two.xhtml\"/>\
             </manifest><spine><itemref idref=\"c1\"/><itemref idref=\"c2\"/></spine>\
             </package>",
        ),
        ("OEBPS/one.xhtml", &ch1),
        (
            "OEBPS/two.xhtml",
            "<html><body><h1>Chapter Two</h1><p>The second.</p></body></html>",
        ),
    ])
}

/// Whether the engines are staged beside the CLI.
///
/// A machine that has not built the workers is a legitimate machine; a silent
/// pass is not. Skipped loudly, in the shape `every_tool_writes_a_file` uses.
fn engines_ready() -> bool {
    let dir = Path::new(CLI).parent().expect("the CLI has a directory");
    let worker = dir.join(format!("oc-pdf{}", std::env::consts::EXE_SUFFIX));
    if !worker.is_file() {
        eprintln!(
            "SKIPPED every_declared_document_route_runs: no oc-pdf beside {}. \
             Build the workspace first: cargo build --workspace",
            dir.display()
        );
        return false;
    }
    true
}

#[test]
fn every_declared_document_route_runs() {
    if !engines_ready() {
        return;
    }
    let ext = extensions();
    let table = declared();
    let dir = workspace();
    let built = fixtures(&dir, &ext);

    let mut checked = 0_usize;
    let mut broken: Vec<String> = Vec::new();

    for (from, fixture) in &built {
        for to in TARGETS {
            if *from == to {
                continue;
            }
            if !table.iter().any(|(a, b)| a == from && b == to) {
                continue;
            }
            checked += 1;

            // A fresh copy per pair: `convert` writes beside its input, and a
            // second output of the same name becomes `name (2).ext` rather
            // than the name this test looks for.
            let source_ext = &ext[*from];
            let work = dir.join(format!("try-{from}-{to}.{source_ext}"));
            std::fs::copy(fixture, &work).expect("copy fixture");

            match convert(&work, to, &ext[to]) {
                Ok(out) => {
                    let bytes = std::fs::read(&out).unwrap_or_default();
                    if bytes.is_empty() {
                        broken.push(format!("{from} -> {to}: wrote an empty file"));
                    } else if READABLE.contains(&to)
                        && !String::from_utf8_lossy(&bytes).contains(CANARY)
                    {
                        broken.push(format!(
                            "{from} -> {to}: wrote {} bytes not containing the \
                             input's own text",
                            bytes.len()
                        ));
                    }
                }
                Err(why) => broken.push(format!("{from} -> {to}: {why}")),
            }
        }
    }

    assert!(
        checked >= EXPECTED_AT_LEAST,
        "only {checked} declared document routes were exercised, expected at \
         least {EXPECTED_AT_LEAST}; the fixtures or the table changed and this \
         test no longer covers what it claims"
    );
    assert!(
        broken.is_empty(),
        "the route table declares these and the build will not run them:\n  {}",
        broken.join("\n  ")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
