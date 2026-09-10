//! Reading a workbook into rows.
//!
//! # The three questions a workbook asks that a CSV cannot answer
//!
//! **1. Which sheet?** A workbook holds many and a CSV holds one. This takes
//! the first sheet and says which one it took, in the receipt, by name — and
//! names the others so the reader knows what was left. Taking the first
//! silently is the version of this that produces a confident wrong file.
//!
//! **2. Formulas or values?** `calamine` can give either. A CSV of `=SUM(A1:A9)`
//! is almost never what anybody wants, so this writes the **computed values**
//! the workbook has cached — and the receipt says the formulas are gone, since
//! for a few people they were the point.
//!
//! **3. Dates.** A date in a workbook is a serial number counted from an epoch
//! the workbook chooses, and the two epochs in use differ by four years. This
//! does not convert them: the number in the cell is the number written out, and
//! the receipt says so. A date silently shifted by four years is worse than a
//! number a reader can see is a number.
//!
//! # Why this is a worker and CSV is not
//!
//! `MediaKind::Tabular` — CSV and JSON — is parsed in the host's own address
//! space, because it is text this build parses itself. A `.xlsx` is a ZIP of
//! XML written by somebody else, so it is `MediaKind::Spreadsheet` and it is
//! parsed here, confined, like every other container.

use calamine::{Data, Reader};

/// The most rows this will write.
///
/// A sheet's declared dimension is a claim; this bounds what is produced from
/// it. A million rows is Excel's own limit and far past any conversion anybody
/// is waiting on.
const MAX_ROWS: usize = 1_048_576;

/// One cell, with its type kept.
///
/// # Why this is not a `String`
///
/// It was, and for CSV and JSON that is the whole answer — both are text. It
/// is the wrong answer for a workbook DESTINATION: a number written as text is
/// a cell Excel left-aligns and refuses to sum, which is a real defect in the
/// file rather than a cosmetic one. `ODS -> XLSX` is what made the difference
/// matter, so the type survives to the writer and `text()` flattens it at the
/// two places that want text.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// No value at all.
    Empty,
    /// Text, including a date already formatted as one by its producer.
    Text(String),
    /// A number, which in a workbook includes every date and every duration.
    Number(f64),
    Bool(bool),
    /// The workbook's own error value — `#DIV/0!` and its family.
    Error(String),
}

/// One sheet of a workbook.
pub struct SheetData {
    /// The sheet's name, as the workbook states it.
    pub name: String,
    /// Its rows.
    pub rows: Vec<Vec<Cell>>,
}

/// A whole workbook, every sheet in the order the file lists them.
pub struct Workbook {
    /// The sheets.
    pub sheets: Vec<SheetData>,
}

/// What came out of a workbook, for the one-sheet destinations.
pub struct Sheet {
    /// The rows, already in CSV field order.
    pub rows: Vec<Vec<String>>,
    /// The name of the sheet these rows came from.
    pub taken: String,
    /// The names of the sheets that were not converted.
    pub left: Vec<String>,
}

/// Read the first sheet of a workbook, as text.
///
/// # Errors
///
/// A file that is not a workbook this build reads, or one with no sheets.
pub fn read(bytes: &[u8]) -> Result<Sheet, String> {
    let book = read_all(bytes)?;
    let mut sheets = book.sheets.into_iter();
    let first = sheets
        .next()
        .ok_or("this workbook has no sheets".to_string())?;
    Ok(Sheet {
        rows: first
            .rows
            .iter()
            .map(|r| r.iter().map(text).collect())
            .collect(),
        taken: first.name,
        left: sheets.map(|s| s.name).collect(),
    })
}

/// Read every sheet of a workbook, with the cell types kept.
///
/// # Errors
///
/// A file that is not a workbook this build reads, or one with no sheets.
pub fn read_all(bytes: &[u8]) -> Result<Workbook, String> {
    // DISPATCHED ON THE MEMBERS, not through `open_workbook_auto_from_rs`.
    //
    // Auto-detection tries each reader in turn and, when all four fail, says
    // "Cannot detect file format" -- which is the same sentence for a
    // password-protected workbook, a truncated archive and a spreadsheet in a
    // dialect this build does not read. Choosing the reader here means the
    // caller gets the reason the ACTUAL format's parser gave, which is the
    // difference between a message somebody can act on and one they cannot.
    //
    // The choice is by content, like everywhere else in this worker: this file
    // is already known to be a ZIP, and an OpenDocument spreadsheet is the one
    // that carries `content.xml`.
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let is_ods = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .is_ok_and(|z| z.index_for_name("content.xml").is_some());

    let mut book: calamine::Sheets<std::io::Cursor<Vec<u8>>> = if is_ods {
        calamine::Sheets::Ods(
            calamine::open_workbook_from_rs(cursor)
                .map_err(|e| format!("could not read this OpenDocument spreadsheet: {e}"))?,
        )
    } else {
        calamine::Sheets::Xlsx(
            calamine::open_workbook_from_rs(cursor)
                .map_err(|e| format!("could not read this Excel workbook: {e}"))?,
        )
    };

    let names = book.sheet_names().to_vec();
    if names.is_empty() {
        return Err("this workbook has no sheets".into());
    }

    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let range = book
            .worksheet_range(&name)
            .map_err(|e| format!("could not read the sheet `{name}`: {e}"))?;
        let rows: Vec<Vec<Cell>> = range
            .rows()
            .take(MAX_ROWS)
            .map(|row| row.iter().map(cell).collect())
            .collect();
        sheets.push(SheetData { name, rows });
    }
    Ok(Workbook { sheets })
}

/// One of calamine's cells as one of ours.
///
/// Every variant is spelled out rather than leaning on `Display`: what a cell
/// becomes is a decision, and a decision that lives inside somebody else's
/// `Display` implementation is one this code cannot be held to.
fn cell(d: &Data) -> Cell {
    match d {
        Data::Empty => Cell::Empty,
        Data::String(s) => Cell::Text(s.clone()),
        // A date the producer already wrote as text. Kept as text: it is
        // already the string somebody chose.
        Data::DateTimeIso(s) | Data::DurationIso(s) => Cell::Text(s.clone()),
        Data::Float(f) => Cell::Number(*f),
        #[allow(clippy::cast_precision_loss)]
        Data::Int(i) => Cell::Number(*i as f64),
        Data::Bool(b) => Cell::Bool(*b),
        Data::Error(e) => Cell::Error(format!("#{e:?}")),
        // The serial-number case, and it stays a NUMBER -- which is what it
        // is. See this module's header for why it is not converted to a date.
        Data::DateTime(dt) => Cell::Number(dt.as_f64()),
    }
}

/// One cell as text, for the destinations that hold only text.
fn text(c: &Cell) -> String {
    match c {
        Cell::Empty => String::new(),
        Cell::Text(s) | Cell::Error(s) => s.clone(),
        Cell::Bool(b) => b.to_string(),
        // A whole number written whole. `2` rather than `2.0` is what every
        // spreadsheet exporter does, and a CSV full of `.0` is one nobody
        // wants to diff.
        #[allow(clippy::cast_possible_truncation)]
        Cell::Number(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
    }
}

/// The rows as CSV.
///
/// Quoting is RFC 4180: a field containing a comma, a quote or a newline is
/// wrapped in quotes and its own quotes doubled. Written here rather than
/// pulled in, because it is nine lines and a dependency that writes CSV would
/// be a dependency in a worker for nine lines.
#[must_use]
pub fn to_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        for (i, field) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if field.contains([',', '"', '\n', '\r']) {
                out.push('"');
                out.push_str(&field.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push('\n');
    }
    out
}

/// The rows as JSON: an array of arrays of strings.
///
/// Not an array of OBJECTS keyed by the first row. That reading is usually
/// right and is a guess — a sheet whose first row is data, or whose headers
/// repeat, silently loses cells to it. Arrays of arrays is what the sheet
/// actually is, and `csv -> json` in this build makes the same choice.
#[must_use]
pub fn to_json(rows: &[Vec<String>]) -> String {
    let value: Vec<Vec<&str>> = rows
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "[]".to_string())
}

/// Every sheet, written as an Excel workbook.
///
/// # What survives, and it is more than the text destinations get
///
/// **Every sheet**, under its own name, because XLSX holds many and there is
/// no reason to choose one. **Every cell's type**: a number stays a number, so
/// the result is a workbook somebody can sum rather than a grid of left-aligned
/// text. That is the whole reason `Cell` exists.
///
/// # What does not
///
/// Formulas — the cached values are written, and a sheet whose point was its
/// formulas comes out as the answers without the working. Formatting, column
/// widths, merged cells, charts, and the date formats, which is the same
/// serial-number decision the rest of this module makes and for the same
/// reason.
///
/// # Errors
///
/// The writer's own, which for an in-memory workbook means a limit crossed:
/// more than 1,048,576 rows or 16,384 columns, or a sheet name Excel will not
/// accept.
pub fn to_xlsx(book: &Workbook) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Workbook as Out, XlsxError};

    let mut out = Out::new();
    for sheet in &book.sheets {
        let ws = out.add_worksheet();
        // THE NAME IS THE SOURCE'S, AND EXCEL HAS RULES ABOUT NAMES. A sheet
        // called `a/b` or one 40 characters long is legal in OpenDocument and
        // is not legal here, so a refused name falls back to the default
        // (`Sheet1`, `Sheet2`, …) rather than failing the conversion over a
        // label.
        if let Err(
            XlsxError::SheetnameLengthExceeded(_)
            | XlsxError::SheetnameContainsInvalidCharacter(_)
            | XlsxError::SheetnameCannotBeBlank(_)
            | XlsxError::SheetnameStartsOrEndsWithApostrophe(_)
            | XlsxError::SheetnameReused(_),
        ) = ws.set_name(&sheet.name)
        {
            // Left with the writer's default name. Disclosed by
            // `removed_workbook`, so it is in the receipt rather than a
            // surprise when the file opens.
        }

        for (r, row) in sheet.rows.iter().enumerate() {
            let Ok(r) = u32::try_from(r) else { break };
            for (c, value) in row.iter().enumerate() {
                let Ok(c) = u16::try_from(c) else { break };
                let wrote = match value {
                    Cell::Empty => Ok(&mut *ws),
                    Cell::Text(t) => ws.write_string(r, c, t),
                    Cell::Number(n) => ws.write_number(r, c, *n),
                    Cell::Bool(b) => ws.write_boolean(r, c, *b),
                    // An error value is written as the text it reads as: this
                    // build does not construct Excel error codes, and a cell
                    // saying `#DIV/0!` is more honest than an empty one.
                    Cell::Error(e) => ws.write_string(r, c, e),
                };
                wrote.map_err(|e| format!("could not write cell {r},{c}: {e}"))?;
            }
        }
    }
    out.save_to_buffer()
        .map_err(|e| format!("could not assemble the workbook: {e}"))
}

/// The receipt lines for a workbook that stayed a workbook.
#[must_use]
pub fn removed_workbook(book: &Workbook) -> Vec<String> {
    let mut out = vec![
        "formulas: the values the workbook had already calculated are written, not the \
         expressions that produced them"
            .to_string(),
        "cell formatting, column widths, merged cells and charts".to_string(),
        "the date formats, so a date arrives as the serial number the cell holds — the \
         epoch it counts from is a property of the source workbook, and applying the \
         wrong one shifts every date by four years"
            .to_string(),
    ];
    // Names Excel will not accept are silently replaced by the writer, so they
    // are named here rather than left to be discovered.
    let renamed: Vec<&str> = book
        .sheets
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| {
            n.is_empty()
                || n.chars().count() > 31
                || n.contains(['/', '\\', '?', '*', '[', ']', ':'])
        })
        .collect();
    if !renamed.is_empty() {
        out.push(format!(
            "the names of {} sheet(s) Excel will not accept, which now carry its default \
             names instead: {}",
            renamed.len(),
            renamed.join(", ")
        ));
    }
    out
}

/// The receipt lines for what a workbook lost on the way to one sheet of text.
#[must_use]
pub fn removed(sheet: &Sheet) -> Vec<String> {
    let mut out = vec![
        format!(
            "every sheet but `{}`: a workbook holds many and this format holds one",
            sheet.taken
        ),
        "formulas: the values the workbook had already calculated are written, not the \
         expressions that produced them"
            .to_string(),
        "cell formatting, and the date formats with it: a date is written as the serial \
         number the cell holds, because the epoch a workbook counts from is a property of \
         the workbook and getting it wrong shifts every date by four years"
            .to_string(),
    ];
    if !sheet.left.is_empty() {
        out.push(format!("the sheets left behind: {}", sheet.left.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_exactly_what_rfc_4180_requires() {
        let rows = vec![vec![
            "plain".to_string(),
            "has,comma".to_string(),
            "has\"quote".to_string(),
            "has\nnewline".to_string(),
            String::new(),
        ]];
        assert_eq!(
            to_csv(&rows),
            "plain,\"has,comma\",\"has\"\"quote\",\"has\nnewline\",\n"
        );
    }

    #[test]
    fn json_is_rows_of_strings_not_objects_keyed_by_a_guessed_header() {
        let rows = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["1".to_string(), "2".to_string()],
        ];
        let json = to_json(&rows);
        assert!(json.starts_with('['), "{json}");
        assert!(json.contains("\"a\""), "{json}");
        assert!(!json.contains("\"a\":"), "not keyed: {json}");
    }

    /// A cell keeps its TYPE on the way in, and loses it only where the
    /// destination has no types — which is what `text` is for.
    #[test]
    fn a_cell_keeps_its_type_and_flattens_to_text_only_where_asked() {
        assert_eq!(cell(&Data::Float(2.0)), Cell::Number(2.0));
        assert_eq!(cell(&Data::Int(-7)), Cell::Number(-7.0));
        assert_eq!(cell(&Data::Empty), Cell::Empty);
        assert_eq!(cell(&Data::Bool(true)), Cell::Bool(true));

        // A whole number written whole. `2` rather than `2.0` is what every
        // spreadsheet exporter does.
        assert_eq!(text(&Cell::Number(2.0)), "2");
        assert_eq!(text(&Cell::Number(2.5)), "2.5");
        assert_eq!(text(&Cell::Number(-7.0)), "-7");
        assert_eq!(text(&Cell::Empty), "");
        assert_eq!(text(&Cell::Bool(true)), "true");
    }
}
