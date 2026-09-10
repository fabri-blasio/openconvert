//! The batch journal.
//!
//! An append-only JSONL file per batch (`03` §12, `INTERFACES.md` §10): one
//! self-contained JSON line per attempted conversion. Append-only is the
//! crash story — a killed process leaves a prefix of complete lines plus at
//! worst one torn final line, and resume reads what survived.
//!
//! **Every line is untrusted on read.** The journal sits in the user's own
//! state directory, which anything running as the user can edit; a malformed
//! line is skipped, never fatal, and a hand-edited entry can only cause a
//! re-conversion (the safe direction), never a skipped one.

use serde::{Deserialize, Serialize};
use std::io::Write;

/// One conversion attempt, as recorded in the journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Basename of the input file.
    pub source_name: String,
    /// Hex-encoded Blake3 of the bytes actually converted.
    pub content_id: String,
    /// Hash of the [`openconvert_core::plan::PlanRequest`] that produced the
    /// plan — never the plan itself, which does not deserialise (I2).
    pub plan_hash: u64,
    /// Basename of the output actually written.
    pub output_name: String,
    /// What happened.
    pub outcome: Outcome,

    // The two fields below are `#[serde(default)]` because this format is on
    // disk in copies of the app older than they are. A journal written last
    // month must still read; it simply reports no size and offers no receipt.
    /// Size of the output in bytes, or 0 when nothing was written.
    #[serde(default)]
    pub output_bytes: u64,
    /// Full path to the `<output>.receipt.json` sidecar, when one was written.
    ///
    /// A FULL path, where every other field here is a basename. History is a
    /// list a person opens things from, and "open the folder this receipt is
    /// in" cannot be answered by a basename. It stays inside the user's own
    /// app-data directory and never leaves the machine.
    #[serde(default)]
    pub receipt_path: Option<String>,

    // ---------------------------------------------------------------------
    // THE COUNTERFACTUAL. What was suggested, against what was chosen.
    //
    // Everything above records what HAPPENED. None of it records what the app
    // had *predicted* would happen, so there was no way to ask the only
    // question that matters about the suggestion engine: is it right? Top-1
    // accuracy could not be computed, and `ARM_THRESHOLD` -- a score of 0.85,
    // above which the app runs a conversion on Enter without asking -- had
    // never been checked against whether 0.85 means right 85% of the time.
    //
    // These three fields turn every conversion into a labelled example, on the
    // user's own machine, which is the precondition for tuning anything and
    // for the logistic model `predict.rs` says it is not ready to fit.
    //
    // `#[serde(default)]` for the same reason as the two fields above: this
    // format is on disk in copies of the app older than these fields are, and
    // a journal written last month must still read.
    //
    // Written by the SHELL, from what the interface actually displayed, not
    // recomputed in the backend. What we need to know is whether the user
    // accepted what they were shown; a backend that re-ranked at journal time
    // could disagree with the screen and would record the disagreement as if
    // it were the user's choice.
    // ---------------------------------------------------------------------
    /// The NAME of the folder the input came from -- never its path.
    ///
    /// Prediction keys history by folder as well as by format pair, because
    /// format choice is workflow-local: `~/Scans` goes to PDF and `~/Web` goes
    /// to WebP, and one global tally averages those two habits into a third
    /// that is neither.
    ///
    /// The final component only, deliberately. The habit is "the folder called
    /// scans", which should survive that folder being moved or its drive
    /// letter changing -- and every other field here is a basename, so a full
    /// path would be the one entry in this record that described the machine
    /// rather than the conversion.
    #[serde(default)]
    pub source_folder: Option<String>,

    /// The target the app ranked first for this file, as a format name.
    ///
    /// `None` for a run with no prediction behind it -- a CLI conversion with
    /// an explicit `-t`, or a tool invocation.
    #[serde(default)]
    pub suggested: Option<String>,

    /// The score that suggestion carried, in `0.0..=1.0`.
    ///
    /// Kept beside `suggested` rather than derived later: the score depends on
    /// history, on the folder and on what was on disk at that moment, none of
    /// which can be reconstructed after the fact.
    #[serde(default)]
    pub suggested_score: Option<f32>,

    /// Where the target the user actually chose sat in the ranking.
    ///
    /// `Some(0)` means they took the suggestion. `Some(n)` means they went
    /// n places down the list. `Some(-1)` means they picked something the
    /// ranking did not contain at all, which is the strongest negative signal
    /// available and the one a plain accuracy count would miss.
    #[serde(default)]
    pub chosen_rank: Option<i32>,
}

/// How one journalled attempt ended.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The output and its receipt were written.
    Completed,
    /// Nothing was written, with a reason a user can act on.
    Failed {
        /// Why the conversion did not complete.
        reason: String,
    },
    /// Not attempted — already completed by an earlier run.
    Skipped,
}

/// An append-only JSONL journal for one batch.
pub struct Journal {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl Journal {
    /// Open (creating if new) the journal for `batch_id`.
    ///
    /// # Errors
    ///
    /// Any I/O failure creating the journal directory or opening the file.
    pub fn open(batch_id: &str) -> std::io::Result<Self> {
        let dir = super::paths::journal_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{batch_id}.jsonl"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        Ok(Self { file, path })
    }

    /// Append one entry. Each line is self-contained JSON, flushed before
    /// returning so a crash loses nothing already reported.
    ///
    /// # Errors
    ///
    /// Serialisation failure or any I/O failure writing the line.
    pub fn append(&mut self, entry: &JournalEntry) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        Ok(())
    }

    /// Read all entries. Malformed lines are SKIPPED, never fatal — including
    /// a torn final line from a killed run.
    #[must_use]
    pub fn read_all(&self) -> Vec<JournalEntry> {
        let contents = std::fs::read_to_string(&self.path).unwrap_or_default();
        contents
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Count entries whose outcome is Completed.
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.read_all()
            .iter()
            .filter(|e| matches!(e.outcome, Outcome::Completed))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A journal in its own temp subdirectory, deleted first so repeated
    /// `cargo test` runs start empty instead of appending forever.
    fn test_journal(name: &str) -> Journal {
        let tmp =
            std::env::temp_dir().join(format!("tx-journal-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("test-batch.jsonl");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .unwrap();
        Journal { file, path }
    }

    fn entry(source: &str, outcome: Outcome) -> JournalEntry {
        JournalEntry {
            source_name: source.into(),
            content_id: "abc".into(),
            plan_hash: 1,
            output_name: "out.bin".into(),
            outcome,
            output_bytes: 0,
            receipt_path: None,
            source_folder: None,
            suggested: None,
            suggested_score: None,
            chosen_rank: None,
        }
    }

    /// Write two, read two. The round trip every other behaviour stands on.
    #[test]
    fn round_trip_three_entries() {
        let mut j = test_journal("round-trip");
        j.append(&entry("a.png", Outcome::Completed)).unwrap();
        j.append(&entry(
            "b.png",
            Outcome::Failed {
                reason: "no route".into(),
            },
        ))
        .unwrap();
        j.append(&entry("c.png", Outcome::Completed)).unwrap();

        assert_eq!(j.read_all().len(), 3);
        assert_eq!(j.completed_count(), 2);

        let all = j.read_all();
        assert_eq!(all[1].source_name, "b.png");
        assert_eq!(
            all[1].outcome,
            Outcome::Failed {
                reason: "no route".into()
            }
        );
    }

    /// **Corrupt state degrades, it does not kill.** SR-19's
    /// `corrupt_degrades` for this file: garbage in the middle and a torn
    /// final line are both skipped, and everything around them survives.
    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        let mut j = test_journal("corrupt");
        j.append(&entry("good1.png", Outcome::Completed)).unwrap();
        // Hand-edited garbage, exactly what an attacker or a crash leaves.
        j.file.write_all(b"GARBAGE\n").unwrap();
        j.append(&entry("good2.png", Outcome::Completed)).unwrap();
        // A torn final line: valid JSON cut off mid-write.
        j.file.write_all(br#"{"source_name":"torn""#).unwrap();

        let all = j.read_all();
        let names: Vec<&str> = all.iter().map(|e| e.source_name.as_str()).collect();
        assert_eq!(names, vec!["good1.png", "good2.png"]);
        assert_eq!(j.completed_count(), 2);
    }

    /// An empty or missing journal reads as zero entries, not an error.
    #[test]
    fn empty_journal_reads_as_zero() {
        let j = test_journal("empty");
        assert!(j.read_all().is_empty());
        assert_eq!(j.completed_count(), 0);
    }

    /// A journal written by an older build still reads.
    ///
    /// Every field added after v1 is `#[serde(default)]` on that promise, and
    /// the promise is only worth what a test says it is: the history that
    /// feeds prediction reads every journal on disk, and one `Err` from a line
    /// written last month would silently drop that whole file's evidence.
    #[test]
    fn a_line_from_an_older_build_still_parses() {
        let old = concat!(
            r#"{"source_name":"a.heic","content_id":"x","plan_hash":0,"#,
            r#""output_name":"a.jpg","outcome":"completed"}"#
        );
        let e: JournalEntry = serde_json::from_str(old).expect("an older line must still parse");
        assert_eq!(e.source_name, "a.heic");
        assert_eq!(e.output_bytes, 0);
        assert_eq!(e.receipt_path, None);
        assert_eq!(e.source_folder, None);
        assert_eq!(e.suggested, None);
        assert_eq!(e.suggested_score, None);
        assert_eq!(e.chosen_rank, None);
    }

    /// And a line with today's fields round-trips through both directions.
    #[test]
    fn the_counterfactual_fields_round_trip() {
        let e = JournalEntry {
            source_folder: Some("photos".into()),
            suggested: Some("jpeg".into()),
            suggested_score: Some(0.91),
            chosen_rank: Some(0),
            ..entry("a.heic", Outcome::Completed)
        };
        let text = serde_json::to_string(&e).expect("serialise");
        let back: JournalEntry = serde_json::from_str(&text).expect("deserialise");
        assert_eq!(back.source_folder.as_deref(), Some("photos"));
        assert_eq!(back.suggested.as_deref(), Some("jpeg"));
        assert_eq!(back.chosen_rank, Some(0));
    }
}
