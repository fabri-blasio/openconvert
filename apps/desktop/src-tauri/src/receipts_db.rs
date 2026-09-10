//! Desktop conversion history, stored as one transactional SQLite database.
//!
//! This is deliberately desktop-owned. The CLI keeps its beside-output JSON
//! receipts; only the GUI needs indexed listing, deletion and exact counts.

#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 1;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// One row as Settings lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptEntry {
    pub id: String,
    pub date: String,
    pub source_name: Option<String>,
    pub output_name: String,
    pub output_bytes: u64,
}

/// A handle to the database path. Connections are intentionally short-lived,
/// letting SQLite perform cross-thread and cross-process locking itself.
#[derive(Debug, Clone)]
pub struct ReceiptDatabase {
    path: PathBuf,
}

impl ReceiptDatabase {
    /// Filesystem location of this database.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Open the application's database and initialize its schema.
    pub fn open() -> Result<Self, String> {
        Self::open_at(database_path())
    }

    /// Open a database at an explicit path. Tests use this to stay out of the
    /// user's application data.
    pub fn open_at(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Self { path };
        db.connect()?;
        Ok(db)
    }

    fn connect(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.path).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        // WAL, NOT DELETE, AND THE DIFFERENCE IS OBSERVABLE.
        //
        // With `DELETE` journalling every write takes an exclusive lock on the
        // whole file and readers block writers. Connections here are
        // deliberately short-lived, so a batch conversion opens and closes one
        // per receipt — and under load that contention exceeded the five-second
        // busy timeout and returned "database is locked". The visible cost is a
        // receipt silently missing from the history for a conversion that
        // succeeded.
        //
        // WAL lets one writer proceed alongside readers, which is exactly the
        // shape of this workload. `synchronous = FULL` is kept, so durability
        // is unchanged: a receipt that was written survives a crash either way.
        //
        // WAL keeps recent pages in a `-wal` sidecar until a checkpoint, which
        // is why `storage_bytes` counts the sidecars and `delete_all`
        // checkpoints before vacuuming. Both would otherwise under-report, and
        // "Reclaim space" would appear to free less than it did.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )
        .map_err(|e| e.to_string())?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| e.to_string())?;
        match version {
            0 => {
                conn.execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE receipts (
                         id TEXT PRIMARY KEY NOT NULL,
                         converted_at INTEGER NOT NULL,
                         source_name TEXT,
                         output_name TEXT NOT NULL,
                         output_bytes INTEGER NOT NULL,
                         content_id TEXT NOT NULL DEFAULT '',
                         detected TEXT NOT NULL DEFAULT '',
                         fidelity_class TEXT,
                         tool TEXT NOT NULL DEFAULT '',
                         steps_json TEXT NOT NULL DEFAULT '[]'
                     );
                     CREATE INDEX receipts_by_date
                         ON receipts(converted_at DESC);
                     PRAGMA user_version = 1;
                     COMMIT;",
                )
                .map_err(|e| e.to_string())?;
            }
            SCHEMA_VERSION => {}
            other => return Err(format!("receipt database schema {other} is not supported")),
        }
        conn.execute_batch("CREATE TABLE IF NOT EXISTS tool_runs (id INTEGER PRIMARY KEY, created INTEGER NOT NULL, body TEXT NOT NULL)").map_err(|e| e.to_string())?;
        Ok(conn)
    }

    pub fn record_run(
        &self,
        request: serde_json::Value,
        results: &[super::ConversionResult],
    ) -> Result<(), String> {
        let conn = self.connect()?;
        let tool = request["target"].clone();
        let mut request = request;
        // Never retain passwords in a reusable request.
        let sensitive = request
            .get("params")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|p| p.keys().any(|k| k.contains("password")));
        if sensitive {
            request = serde_json::Value::Null;
        }
        for (index, r) in results.iter().enumerate() {
            let mut replay = request.clone();
            if let Some(paths) = request.get("paths").and_then(serde_json::Value::as_array) {
                if paths.len() == results.len()
                    && !request["target"]
                        .as_str()
                        .is_some_and(|id| matches!(id, "pdf-compose" | "pdf-merge"))
                {
                    replay["paths"] = serde_json::json!([paths[index]]);
                }
            }
            let body = serde_json::json!({"sourceName":r.file_name,"outputName":std::path::Path::new(&r.output_path).file_name().unwrap_or_default().to_string_lossy(),
                "outputPath":r.output_path,"outputBytes":r.output_bytes,"outcome":if r.success {"completed"} else {"failed"},
                "reason":r.error_message,"receiptPath":r.receipt_path,"contentId":"","whenSecs":unix_seconds(),
                "tool":tool,"request":replay});
            conn.execute(
                "INSERT INTO tool_runs(created,body) VALUES (?1,?2)",
                params![unix_seconds() as i64, body.to_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn runs(&self, limit: usize) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare("SELECT body FROM tool_runs ORDER BY id DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([limit.min(1000) as i64], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        Ok(rows
            .filter_map(Result::ok)
            .filter_map(|r| serde_json::from_str(&r).ok())
            .collect())
    }
    pub fn clear_runs(&self) -> Result<usize, String> {
        self.connect()?
            .execute("DELETE FROM tool_runs", [])
            .map_err(|e| e.to_string())
    }
    pub fn has_output(&self, path: &std::path::Path) -> bool {
        self.runs(1000).unwrap_or_default().iter().any(|r| {
            r["outputPath"]
                .as_str()
                .is_some_and(|p| std::path::Path::new(p) == path)
        })
    }

    /// Insert one in-memory JSON receipt and return its database id.
    pub fn insert_json(&self, body: &str, input_name: &str) -> Result<String, String> {
        let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let id = next_id();
        let converted_at = i64::try_from(unix_seconds()).unwrap_or(i64::MAX);
        let output_name = string_field(&value, "output");
        let output_bytes = value
            .get("output_bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let class = value
            .get("class")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let steps = value
            .get("steps")
            .map(serde_json::Value::to_string)
            .unwrap_or_else(|| "[]".to_string());
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO receipts (
                 id, converted_at, source_name, output_name, output_bytes,
                 content_id, detected, fidelity_class, tool, steps_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                converted_at,
                input_name,
                output_name,
                i64::try_from(output_bytes).unwrap_or(i64::MAX),
                string_field(&value, "content_id"),
                string_field(&value, "detected"),
                class,
                string_field(&value, "tool"),
                steps,
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// List newest rows first.
    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<ReceiptEntry>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare(
                "SELECT id, converted_at, source_name, output_name, output_bytes
                 FROM receipts
                 ORDER BY converted_at DESC, rowid DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(
                params![
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX)
                ],
                |row| {
                    let seconds: i64 = row.get(1)?;
                    let bytes: i64 = row.get(4)?;
                    Ok(ReceiptEntry {
                        id: row.get(0)?,
                        date: iso_time(seconds.max(0) as u64),
                        source_name: row.get(2)?,
                        output_name: row.get(3)?,
                        output_bytes: bytes.max(0) as u64,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Exact row count, independent of pagination.
    pub fn count(&self) -> Result<u64, String> {
        let conn = self.connect()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as u64)
    }

    /// Remove one row by id.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let conn = self.connect()?;
        let removed = conn
            .execute("DELETE FROM receipts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA incremental_vacuum;")
            .map_err(|e| e.to_string())?;
        Ok(removed > 0)
    }

    /// Remove every row and compact the database. Returns bytes reclaimed.
    pub fn delete_all(&self) -> Result<u64, String> {
        let before = self.storage_bytes();
        let conn = self.connect()?;
        conn.execute("DELETE FROM receipts", [])
            .map_err(|e| e.to_string())?;
        // Fold the write-ahead log back into the database before vacuuming, or
        // the deleted rows are still on disk in the sidecar and the space this
        // reports as reclaimed was not.
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| e.to_string())?;
        conn.execute_batch("VACUUM;").map_err(|e| e.to_string())?;
        Ok(before.saturating_sub(self.storage_bytes()))
    }

    /// Current database-file size.
    /// Current database size, INCLUDING the write-ahead sidecars.
    ///
    /// Under WAL, recently written pages live in `-wal` until a checkpoint, so
    /// the main file alone understates what the history occupies — and this
    /// number is what the Settings screen shows and what "Reclaim space"
    /// subtracts from.
    pub fn storage_bytes(&self) -> u64 {
        let mut total = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        for suffix in ["-wal", "-shm"] {
            let mut side = self.path.clone().into_os_string();
            side.push(suffix);
            total += std::fs::metadata(std::path::PathBuf::from(side)).map_or(0, |m| m.len());
        }
        total
    }

    /// Whether one id exists. Used to verify rollback tests and future repair.
    #[cfg(test)]
    fn contains(&self, id: &str) -> Result<bool, String> {
        let conn = self.connect()?;
        conn.query_row("SELECT 1 FROM receipts WHERE id = ?1", params![id], |_| {
            Ok(true)
        })
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(|e| e.to_string())
    }
}

/// Canonical location used by the desktop application.
pub fn database_path() -> PathBuf {
    openconvert_run::state::paths::state_dir()
        .join("receipts")
        .join("receipts.sqlite3")
}

fn string_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn next_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "{}-{}-{}",
        nanos,
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn iso_time(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rem = seconds % 86_400;
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_redacts_passwords_and_repeats_only_selected_input() {
        let db = database("replay");
        let results = [
            super::super::failure("a.png", "test", 0, 10),
            super::super::failure("b.png", "test", 0, 10),
        ];
        db.record_run(serde_json::json!({"paths":["a.png","b.png"],"target":"image-compress","params":{"quality":"65"}}),&results).unwrap();
        let rows = db.runs(10).unwrap();
        assert_eq!(rows[0]["request"]["paths"], serde_json::json!(["b.png"]));
        assert_eq!(rows[1]["request"]["paths"], serde_json::json!(["a.png"]));
        db.record_run(serde_json::json!({"paths":["secret.pdf"],"target":"pdf-password","params":{"password":"private"}}),&results[..1]).unwrap();
        let row = &db.runs(1).unwrap()[0];
        assert!(row["request"].is_null());
        assert_eq!(row["tool"], "pdf-password");
        assert!(!row.to_string().contains("private"));
        assert_eq!(db.clear_runs().unwrap(), 3);
        assert!(db.runs(10).unwrap().is_empty());
    }

    fn database(name: &str) -> ReceiptDatabase {
        let path = std::env::temp_dir().join(format!(
            "openconvert-receipts-{name}-{}-{}.sqlite3",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        ReceiptDatabase::open_at(path).unwrap()
    }

    #[test]
    fn insert_list_count_and_delete_round_trip() {
        let db = database("roundtrip");
        let id = db
            .insert_json(
                r#"{"output":"photo.jpg","output_bytes":42,"content_id":"abc","steps":[]}"#,
                "photo.png",
            )
            .unwrap();
        assert_eq!(db.count().unwrap(), 1);
        let rows = db.list(10, 0).unwrap();
        assert_eq!(rows[0].source_name.as_deref(), Some("photo.png"));
        assert_eq!(rows[0].output_bytes, 42);
        assert!(db.contains(&id).unwrap());
        assert!(db.delete(&id).unwrap());
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn identical_receipts_receive_distinct_ids() {
        let db = database("ids");
        let body = r#"{"output":"same.png","steps":[]}"#;
        let a = db.insert_json(body, "same.jpg").unwrap();
        let b = db.insert_json(body, "same.jpg").unwrap();
        assert_ne!(a, b);
        let rows = db.list(10, 0).unwrap();
        assert_eq!(rows[0].id, b, "the most recently inserted row is first");
    }

    #[test]
    fn concurrent_writers_preserve_every_row() {
        let db = database("concurrent");
        let mut threads = Vec::new();
        for worker in 0..8 {
            let copy = db.clone();
            threads.push(std::thread::spawn(move || {
                for row in 0..25 {
                    copy.insert_json(
                        &format!(r#"{{"output":"{worker}-{row}.png","steps":[]}}"#),
                        "input.png",
                    )
                    .unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(db.count().unwrap(), 200);
    }

    #[test]
    fn delete_all_compacts_a_valid_empty_database() {
        let db = database("clear");
        for i in 0..100 {
            db.insert_json(&format!(r#"{{"output":"{i}.png","steps":[]}}"#), "in")
                .unwrap();
        }
        let _ = db.delete_all().unwrap();
        assert_eq!(db.count().unwrap(), 0);
        assert!(db.storage_bytes() > 0);
    }
}
