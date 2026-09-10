//! `xlsx -> csv` and `ods -> json`, over the wire, through the real worker.
//!
//! The document routes are held end to end by `declared_routes_run` in the
//! `openconvert` crate, which drives the shipped CLI through every declared
//! document pair. A workbook's targets are not documents — they are CSV and
//! JSON — so that test does not reach these four rows, and this is what does.
//!
//! The fixtures are built here, in the two shapes the readers actually meet:
//! an OOXML workbook whose part names identify it, and an OpenDocument one
//! whose first member does. Both are written the way real producers write them,
//! because the way they are written is what detection reads.

#![cfg(windows)]

use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};
use std::io::Write;
use std::process::{Command, Stdio};

fn worker_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current exe");
    p.pop(); // deps/
    p.pop(); // target/<profile>/
    p.join(format!("oc-pdf{}", std::env::consts::EXE_SUFFIX))
}

fn limits() -> RunLimits {
    RunLimits {
        decode_pixels: u64::MAX,
        memory_bytes: 1 << 26,
        archive_depth: 32,
        archive_entries: 100_000,
        archive_total_bytes: 1 << 26,
        use_gpu: false,
    }
}

/// One request, one answer, through the real binary.
fn run(input: &[u8], to: &str) -> Result<Vec<u8>, String> {
    let exe = worker_path();
    assert!(exe.exists(), "oc-pdf is not built at {}", exe.display());
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn oc-pdf");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        let body = serde_json::to_vec(&Request::Run {
            input_len: input.len() as u64,
            to: to.to_string(),
            limits: limits(),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        })
        .unwrap();
        write_frame(&mut stdin, &body).unwrap();
        write_content(&mut stdin, input).unwrap();
        stdin.flush().unwrap();
    }
    let out = child.wait_with_output().expect("wait");
    let mut cursor = std::io::Cursor::new(out.stdout);
    let frame = read_frame(&mut cursor).expect("a response");
    match serde_json::from_slice::<Response>(&frame).expect("parse") {
        Response::Done { output_len, .. } => {
            Ok(read_content(&mut cursor, output_len, u64::MAX).expect("content"))
        }
        Response::Failed { message, .. } => Err(message),
        other => panic!("expected Done or Failed, got {other:?}"),
    }
}

/// A ZIP, with `mimetype` stored first when there is one — which is what
/// OpenDocument requires and what identifies the file.
fn zipped(members: &[(&str, &str)]) -> Vec<u8> {
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

/// A two-sheet OOXML workbook, hand-built.
///
/// Minimal, and exactly as minimal as a reader will accept: a workbook part
/// naming the sheets, a relationships part pointing at them, and the sheets
/// themselves with inline strings so there is no shared-strings table to write.
fn xlsx() -> Vec<u8> {
    let sheet = |cells: &[(&str, &str)]| {
        let row: String = cells
            .iter()
            .map(|(r, v)| format!("<c r=\"{r}\" t=\"inlineStr\"><is><t>{v}</t></is></c>"))
            .collect();
        format!(
            "<?xml version=\"1.0\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData><row r=\"1\">{row}</row></sheetData></worksheet>"
        )
    };
    zipped(&[
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             </Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>",
        ),
        (
            "xl/workbook.xml",
            "<?xml version=\"1.0\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
             xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"Sales\" sheetId=\"1\" r:id=\"rId1\"/>\
             <sheet name=\"Notes\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/>\
             </Relationships>",
        ),
        (
            "xl/worksheets/sheet1.xml",
            &sheet(&[("A1", "Region"), ("B1", "Units, sold")]),
        ),
        (
            "xl/worksheets/sheet2.xml",
            &sheet(&[("A1", "left behind")]),
        ),
    ])
}

/// A two-table OpenDocument spreadsheet.
///
/// **No whitespace between the table elements.** Real producers write none, and
/// a strict reader refuses text where it expects a cell — which is exactly what
/// happened to the first version of this fixture.
fn ods() -> Vec<u8> {
    let content = "<?xml version=\"1.0\"?>\
        <office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
        xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
        xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
        <office:body><office:spreadsheet>\
        <table:table table:name=\"Sales\"><table:table-row>\
        <table:table-cell office:value-type=\"string\"><text:p>Region</text:p></table:table-cell>\
        <table:table-cell office:value-type=\"float\" office:value=\"42\"><text:p>42</text:p></table:table-cell>\
        </table:table-row></table:table>\
        <table:table table:name=\"Notes\"><table:table-row>\
        <table:table-cell office:value-type=\"string\"><text:p>left behind</text:p></table:table-cell>\
        </table:table-row></table:table>\
        </office:spreadsheet></office:body></office:document-content>";
    zipped(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content),
        ("META-INF/manifest.xml", "<manifest:manifest/>"),
    ])
}

#[test]
fn a_workbook_reaches_csv_with_its_first_sheet_and_rfc_4180_quoting() {
    let out = run(&xlsx(), "csv").expect("xlsx -> csv");
    let text = String::from_utf8(out).expect("CSV is text");
    assert_eq!(
        text.trim_end(),
        "Region,\"Units, sold\"",
        "the field holding a comma must be quoted, and only that one"
    );
    assert!(
        !text.contains("left behind"),
        "the second sheet must not be in a one-sheet format: {text}"
    );
}

#[test]
fn an_opendocument_workbook_reaches_json_as_rows_of_strings() {
    let out = run(&ods(), "json").expect("ods -> json");
    let text = String::from_utf8(out).expect("JSON is text");
    assert!(text.starts_with('['), "{text}");
    assert!(text.contains("\"Region\""), "{text}");
    // A whole number is written whole, not as `42.0`.
    assert!(text.contains("\"42\""), "{text}");
    // And keyed by nothing: the first row is data, not a header.
    assert!(!text.contains("\"Region\":"), "{text}");
    assert!(!text.contains("left behind"), "{text}");
}

/// The worker refuses a target it does not write, by name — and a workbook has
/// no route to a document, so `xlsx -> pdf` must not quietly produce one.
#[test]
fn a_workbook_is_not_offered_as_a_document() {
    let err = run(&xlsx(), "pdf").expect_err("a workbook is not a document");
    assert!(
        err.contains("Word") || err.contains("OpenDocument") || err.contains("pdf"),
        "the refusal must say what went wrong: {err}"
    );
}
