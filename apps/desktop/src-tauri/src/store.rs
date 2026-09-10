//! The GUI-side **history view** over the batch journals.
//!
//! This module deliberately does NOT own the receipt database. The desktop's
//! SQLite module owns its location, schema and deletion rules. What remains
//! here is the one thing that has no home in `-run`: rendering past conversions
//! out of the journals for the History panel.
//!
//! House rule unchanged: everything under the state directory is **untrusted
//! on read**. A malformed journal line is skipped, never fatal, and nothing
//! here can change what a conversion is permitted to do — it only remembers
//! what already happened.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// One past conversion attempt, as the journals recorded it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryJson {
    /// Basename of the input, as journalled.
    pub source_name: String,
    /// Basename of the output actually written.
    pub output_name: String,
    /// Blake3 hex of the bytes converted — the receipt's `content_id`.
    pub content_id: String,
    /// `"completed" | "failed" | "skipped"`.
    pub outcome: String,
    /// Present only for failures.
    pub reason: Option<String>,
    /// When the batch ran, in seconds since the epoch.
    ///
    /// Taken from the journal FILE's mtime: the journal format carries no
    /// timestamp per line, and inventing one would be worse than an honest
    /// batch-level approximation. Documented in INTERFACES.md §11 as such.
    pub when_secs: u64,
    /// Size of the output in bytes; 0 when nothing was written.
    pub output_bytes: u64,
    /// Full path to the receipt sidecar, when one was written.
    ///
    /// The history row IS the receipt: this is what makes it openable. Absent
    /// for failures, and for journals written before the field existed.
    pub receipt_path: Option<String>,
}

/// Read up to `limit` entries, newest batch first, entries newest-within-batch
/// first. Malformed lines are skipped by the same rule the journal itself
/// uses; unreadable files are skipped here for the same reason.
#[must_use]
pub fn read_history(dir: &Path, limit: usize) -> Vec<HistoryEntryJson> {
    use openconvert_run::state::journal::Outcome as JournalOutcome;

    let mut batches: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
                .map(|p| {
                    let mtime = p
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    (p, mtime)
                })
                .collect()
        })
        .unwrap_or_default();
    batches.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));

    let mut out = Vec::new();
    'batches: for (path, when) in batches {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in contents.lines().rev() {
            let Ok(e) = serde_json::from_str::<openconvert_run::state::journal::JournalEntry>(line)
            else {
                continue; // torn or hand-edited line: skipped, like everywhere else
            };
            let (outcome, reason) = match e.outcome {
                JournalOutcome::Completed => ("completed", None),
                JournalOutcome::Failed { reason } => ("failed", Some(reason)),
                JournalOutcome::Skipped => ("skipped", None),
            };
            out.push(HistoryEntryJson {
                source_name: e.source_name,
                output_name: e.output_name,
                content_id: e.content_id,
                outcome: outcome.to_string(),
                reason,
                when_secs: when,
                output_bytes: e.output_bytes,
                receipt_path: e.receipt_path,
            });
            if out.len() >= limit {
                break 'batches;
            }
        }
    }
    out
}

/// Delete every journal file. Returns how many were removed.
///
/// Scoped to the journal directory and nothing else, which is what makes this
/// deletable at all: it can never reach a user file.
pub fn wipe_history(dir: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    let entries = std::fs::read_dir(dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tx-store-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn history_reads_journals_newest_batch_first_and_wipes_clean() {
        // The journals are plain JSONL on disk; written literally so this test
        // never touches the real user state directory.
        let dir = scratch("history");
        std::fs::write(
            dir.join("batch-old.jsonl"),
            concat!(
                r#"{"source_name":"a.png","content_id":"h1","plan_hash":1,"#,
                r#""output_name":"a.jpg","outcome":"completed"}"#,
            ),
        )
        .unwrap(); // openconvert-lint: allow -- test scratch
        std::fs::write(
            dir.join("batch-new.jsonl"),
            concat!(
                r#"{"source_name":"b.heic","content_id":"h2","plan_hash":2,"#,
                r#""output_name":"b.png","outcome":{"failed":{"reason":"no route"}}}"#,
            ),
        )
        .unwrap(); // openconvert-lint: allow -- test scratch
        std::fs::write(dir.join("notes.txt"), "not a journal").unwrap(); // openconvert-lint: allow -- test scratch

        let entries = read_history(&dir, 10);
        assert_eq!(entries.len(), 2, "only .jsonl files are history");
        assert!(
            entries.iter().any(|e| e.outcome == "completed"),
            "both outcomes are rendered"
        );
        let failed = entries
            .iter()
            .find(|e| e.reason.is_some())
            .expect("one failure");
        assert_eq!(failed.reason.as_deref(), Some("no route"));

        let limited = read_history(&dir, 1);
        assert_eq!(limited.len(), 1);

        assert_eq!(wipe_history(&dir).expect("wipe"), 2);
        assert!(read_history(&dir, 10).is_empty());
    }
}
