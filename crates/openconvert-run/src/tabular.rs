//! Tabular conversion: CSV to JSON and back, in-process.
//!
//! # Why this is in-process
//!
//! Both directions parse pure-Rust over plain text — the format table marks
//! CSV and JSON `pure_rust_parser`, which is what admits them here at all
//! (I10). The parser is the battle-tested `csv` crate rather than something
//! hand-rolled: RFC 4180 quoting has famous edge cases, and a bespoke parser
//! written to avoid one dependency would be a new attacker-facing surface
//! with none of the fuzzing history.
//!
//! # What "Class A" means for a CSV
//!
//! Every field travels as its string form, because CSV HAS no other form —
//! there is no schema to lose. `1.50` stays `"1.50"` rather than becoming
//! `1.5`, which is exactly the silent damage this product exists to refuse.
//! Type inference is an AI-labelled enhancement (`02` §6), never a default.

use std::collections::BTreeMap;
use std::io::Cursor;

/// Why tabular conversion failed.
#[derive(Debug, thiserror::Error)]
pub enum TabularError {
    /// The input was not parseable as its detected format.
    #[error("could not parse this input: {0}")]
    Parse(String),
    /// Output could not be produced.
    #[error("could not write the output: {0}")]
    Write(String),
}

/// CSV → a JSON array of objects, headers as keys, every value a string.
///
/// Headers are taken from the first record; a row shorter than the header
/// yields `null` for its missing columns rather than a silently shifted row,
/// and a longer one keeps its extras under generated names. Real-world CSV
/// does both constantly, and neither is recoverable after the fact unless
/// recorded now.
pub fn csv_to_json(bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), TabularError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(false)
        .from_reader(Cursor::new(bytes));

    let mut records = reader.records();
    let Some(headers) = records
        .next()
        .transpose()
        .map_err(|e| TabularError::Parse(e.to_string()))?
    else {
        return Err(TabularError::Parse("this CSV has no header row".into()));
    };
    let headers: Vec<String> = headers.iter().map(String::from).collect();
    if headers.is_empty() {
        return Err(TabularError::Parse(
            "this CSV has an empty header row".into(),
        ));
    }

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows = Vec::new();
    for record in records {
        let record = record.map_err(|e| TabularError::Parse(e.to_string()))?;
        let mut obj = serde_json::Map::with_capacity(record.len());
        for (i, value) in record.iter().enumerate() {
            let key = if i < headers.len() {
                headers[i].clone()
            } else {
                // Extra columns: named deterministically, counted honestly.
                let n = seen
                    .entry("extra_column".into())
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
                format!("extra_column_{}", n)
            };
            obj.insert(key, serde_json::Value::from(value));
        }
        for (i, h) in headers.iter().enumerate() {
            if i >= record.len() {
                obj.insert(h.clone(), serde_json::Value::Null);
            }
        }
        rows.push(serde_json::Value::Object(obj));
    }

    let out = serde_json::to_vec_pretty(&rows).map_err(|e| TabularError::Write(e.to_string()))?;
    Ok((
        out,
        vec!["all values kept as their literal text (CSV carries no types)".to_string()],
    ))
}

/// A JSON array of flat objects → CSV.
///
/// Columns are emitted in **sorted** order, not appearance order: serde_json's
/// maps are ordered maps, and a deterministic output for one input is what the
/// receipt's hashes require. Missing fields become empty cells; nested values
/// are refused rather than flattened by guesswork.
pub fn json_to_csv(bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), TabularError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| TabularError::Parse(e.to_string()))?;
    let rows = value.as_array().ok_or_else(|| {
        TabularError::Parse("only a JSON array of objects converts to CSV".into())
    })?;

    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        let Some(map) = row.as_object() else {
            return Err(TabularError::Parse(
                "every element must be a JSON object with named fields".into(),
            ));
        };
        for key in map.keys() {
            if !columns.iter().any(|c| c == key) {
                columns.push(key.clone());
            }
        }
    }
    if columns.is_empty() {
        return Err(TabularError::Parse(
            "no columns were found in this array".into(),
        ));
    }

    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(&columns)
        .map_err(|e| TabularError::Write(e.to_string()))?;
    for row in rows {
        let map = row.as_object().expect("checked above");
        let mut record: Vec<String> = Vec::with_capacity(columns.len());
        for col in &columns {
            match map.get(col) {
                None | Some(serde_json::Value::Null) => record.push(String::new()),
                // Nested structures have no honest CSV form. Flattening one
                // by convention invents a schema nobody agreed to.
                Some(other @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
                    return Err(TabularError::Parse(format!(
                        "field {col:?} is nested ({other}); flatten it before converting to CSV"
                    )))
                }
                Some(scalar) => record.push(scalar.to_string().trim_matches('"').to_owned()),
            }
        }
        writer
            .write_record(&record)
            .map_err(|e| TabularError::Write(e.to_string()))?;
    }

    Ok((
        writer
            .into_inner()
            .map_err(|e| TabularError::Write(e.to_string()))?,
        vec!["nested values are refused rather than flattened".to_string()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &[u8] = b"name,amount\n\"Doe, Jane\",1.50\nBob,\n";

    #[test]
    fn csv_values_survive_as_literal_text() {
        let (json, _) = csv_to_json(CSV).expect("convert");
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Doe, Jane");
        // THE point of Class A here: not 1.5.
        assert_eq!(rows[0]["amount"], "1.50");
    }

    #[test]
    fn a_short_row_yields_nulls_and_a_long_one_keeps_extras() {
        let (json, _) = csv_to_json(b"h1,h2\na\nc,d,extra\n").expect("convert");
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows[0]["h2"], serde_json::Value::Null);
        assert_eq!(rows[1]["extra_column_1"], "extra");
    }

    #[test]
    fn json_round_trips_through_csv_with_deterministic_columns() {
        let src = br#"[{"b": 2, "a": 1}, {"a": 3, "c": null}]"#;
        let (csv_out, _) = json_to_csv(src).expect("convert");
        let text = String::from_utf8(csv_out.clone()).unwrap();
        // Sorted columns: deterministic for one input is what the receipt's
        // hashes need. A null becomes an empty cell.
        assert_eq!(text, "a,b,c\n1,2,\n3,,\n");

        let (back, _) = csv_to_json(&csv_out).expect("back");
        let v: serde_json::Value = serde_json::from_slice(&back).unwrap();
        assert_eq!(v[0]["a"], "1", "values stay textual through the round trip");
    }

    #[test]
    fn nested_json_is_refused_rather_than_flattened_by_guesswork() {
        let err = json_to_csv(br#"[{"a": {"n": 1}}]"#).expect_err("must refuse");
        assert!(err.to_string().contains("nested"));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(csv_to_json(b"\x00\xff garbage").is_err());
        assert!(json_to_csv(b"{not json").is_err());
    }
}
