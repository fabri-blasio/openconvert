//! `openconvert batch` — many files, one journal, resumable.
//!
//! The crash story is the feature: every attempt appends one self-contained
//! line to a JSONL journal keyed by a hash of the directory's file list, so
//! the same command re-run with `--resume` skips exactly what completed
//! before the kill and nothing else. Resume keys on `source_name` with
//! outcome `Completed`; failures are journalled too and are *retried*, not
//! skipped — a transient failure should not become permanent because it
//! happened once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use openconvert_core::target::Target;
use openconvert_run::pool::WorkerPool;
use openconvert_run::state::config::UserConfig;
use openconvert_run::state::journal::{Journal, JournalEntry, Outcome};

use super::convert::{convert_one, preview_one, Job};

pub fn run(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let mut dir: Option<PathBuf> = None;
    let mut flag_target: Option<Target> = None;
    let mut resume = false;
    let mut dry_run = false;
    let mut manifest: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-t" | "--to" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("-t needs a format"))?;
                flag_target = Some(Target::Format(crate::parse_format(v)?));
            }
            "--strip-metadata" => anyhow::bail!(
                "batch records a target format; --strip-metadata needs a \
                 per-file answer, so use `convert` for that"
            ),
            "--resume" => resume = true,
            "--dry-run" => dry_run = true,
            "--manifest" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--manifest needs a path"))?;
                manifest = Some(PathBuf::from(v));
            }
            other if dir.is_none() && !other.starts_with('-') => dir = Some(PathBuf::from(other)),
            other => anyhow::bail!("unexpected argument {other:?}"),
        }
    }

    // Three layers: CLI flag < config.toml < refusal to guess.
    let dir = dir.ok_or_else(|| anyhow::anyhow!("batch needs a directory"))?;
    let target = match flag_target {
        Some(t) => t,
        None => match UserConfig::load().default_format {
            Some(fmt) => Target::Format(crate::parse_format(&fmt)?),
            None => anyhow::bail!(
                "say what you want: -t <format>, or set default_format in config.toml"
            ),
        },
    };

    run_batch(&dir, target, resume, dry_run, manifest.as_deref())
}

fn run_batch(
    dir: &Path,
    target: Target,
    resume: bool,
    dry_run: bool,
    manifest_path: Option<&Path>,
) -> anyhow::Result<std::process::ExitCode> {
    // Scan for inputs. Two kinds of file are NOT inputs:
    //
    // - receipt sidecars from our own earlier runs — converting them back
    //   would only manufacture failures;
    // - files already in the target format — converting a JPEG *to* JPEG has
    //   no route, so they could never convert. Excluding them here rather
    //   than failing per-file below is also what keeps the batch id stable:
    //   the outputs of run one must not change the input set of run two,
    //   or `--resume` would find a different journal and resume nothing.
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            !is_receipt_sidecar(&name)
        })
        .filter(|p| !already_in_target_format(p, target))
        .collect();
    // Sorted, so processing order and batch id are both stable across runs.
    files.sort();

    if files.is_empty() {
        eprintln!("no convertible files found in {}", crate::show(dir));
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    // Batch ID = hash of the sorted basenames (stable across invocations), so
    // "run again with --resume" finds the same journal without the user ever
    // handling an id.
    let mut hasher = blake3::Hasher::new();
    for name in &names {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }
    let batch_id = super::hex_encode(&hasher.finalize().as_bytes()[..8]);

    // Open journal (creates if new).
    let mut journal = Journal::open(&batch_id)?;
    // source name -> journalled output name, last entry wins. Completed
    // means the output EXISTED once; the skip below re-checks that it still
    // does, so a user who deleted an output gets it rebuilt rather than a
    // shrug pointing at history.
    let done: HashMap<String, String> = if resume {
        journal
            .read_all()
            .iter()
            .filter(|e| matches!(e.outcome, Outcome::Completed))
            .map(|e| (e.source_name.clone(), e.output_name.clone()))
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    if resume && !done.is_empty() {
        println!(
            "resuming: {} completed entr{} in the journal",
            done.len(),
            if done.len() == 1 { "y" } else { "ies" }
        );
    }

    // The same job `convert` resolves, and for the same reason: a bad naming
    // template or a configured folder that does not exist must fail before
    // the first file, not part-way through fifty.
    let job = Job::resolve(target, true, false)?;
    let env = crate::probe_real();
    let mut pool = WorkerPool::new(job.policy.worker_reuse());
    let g = crate::ui::Glyphs::detect();
    let started = std::time::Instant::now();
    let total = files.len();

    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let (mut bytes_in, mut bytes_out) = (0u64, 0u64);
    let mut records: Vec<serde_json::Value> = Vec::new();

    for (i, (path, name)) in files.iter().zip(names.iter()).enumerate() {
        let n = i + 1;
        if let Some(out_name) = done.get(name) {
            // Where the output WOULD be under this run's destination, which
            // is not necessarily beside the input: a journal records the
            // output's name, and the configured folder says where that name
            // lives. Checking `dir` alone declared every Desktop-bound batch
            // unfinished and converted it again.
            if job.dest_dir(path).join(out_name).exists() {
                skipped += 1;
                records.push(serde_json::json!({ "source": name, "outcome": "skipped" }));
                if !dry_run {
                    eprintln!(
                        "  {} [{n}/{total}] {}   already done",
                        g.status(crate::ui::Status::Skipped),
                        crate::show_base(path)
                    );
                }
                continue;
            }
            // Completed once, but the output is gone (deleted, or lost to a
            // crashed copy). Convert again rather than trust the history.
        }

        if dry_run {
            match preview_one(path, &job, n, &env, &mut pool) {
                Ok(to) => println!(
                    "would convert {} {} {}",
                    crate::show_base(path),
                    g.arrow(),
                    crate::show_name(&to)
                ),
                Err(e) => println!("would refuse {}: {e}", crate::show_base(path)),
            }
            continue;
        }

        match convert_one(path, &job, n, &env, &mut pool) {
            Ok(c) => {
                let out_name = c.output_name();
                bytes_in += c.input_bytes;
                bytes_out += c.output_bytes;
                let class = c.class().map_or("-", |k| g.class(k));
                let guard = if c.sandboxed() {
                    format!("{}{} sandboxed", g.sep(), g.sandboxed())
                } else {
                    String::new()
                };
                eprintln!(
                    "  {} [{n}/{total}] {} {} {}   {class}{guard}   {}",
                    g.status(crate::ui::Status::Ok),
                    crate::show_base(path),
                    g.arrow(),
                    crate::show_name(&out_name),
                    crate::ui::secs(c.elapsed),
                );
                journal.append(&JournalEntry {
                    source_name: name.clone(),
                    content_id: c.content_id_hex,
                    plan_hash: c.request_hash,
                    output_name: out_name.clone(),
                    outcome: Outcome::Completed,
                    output_bytes: c.output_bytes,
                    receipt_path: Some(c.receipt.to_string_lossy().into_owned()),
                    // The folder IS recorded: a conversion run from the command
                    // line is real evidence of a habit in that folder, and the
                    // app and this binary share one history.
                    source_folder: folder_name(path),
                    // The prediction is NOT: the target came from -t or from the
                    // config default, so there is nothing to have been right or
                    // wrong about. A rank of 0 here would report a perfect
                    // suggestion the engine never made.
                    suggested: None,
                    suggested_score: None,
                    chosen_rank: None,
                })?;
                records.push(serde_json::json!({
                    "source": name,
                    "outcome": "completed",
                    "output": out_name,
                }));
                completed += 1;
            }
            Err(e) => {
                // One file's failure ends that file, not the run.
                eprintln!(
                    "  {} [{n}/{total}] {}   {e}",
                    g.status(crate::ui::Status::Failed),
                    crate::show_base(path)
                );
                journal.append(&JournalEntry {
                    source_name: name.clone(),
                    content_id: String::new(),
                    plan_hash: 0,
                    output_name: String::new(),
                    outcome: Outcome::Failed {
                        reason: e.to_string(),
                    },
                    output_bytes: 0,
                    receipt_path: None,
                    // The folder IS recorded: a conversion run from the command
                    // line is real evidence of a habit in that folder, and the
                    // app and this binary share one history.
                    source_folder: folder_name(path),
                    // The prediction is NOT: the target came from -t or from the
                    // config default, so there is nothing to have been right or
                    // wrong about. A rank of 0 here would report a perfect
                    // suggestion the engine never made.
                    suggested: None,
                    suggested_score: None,
                    chosen_rank: None,
                })?;
                records.push(serde_json::json!({
                    "source": name,
                    "outcome": "failed",
                    "reason": e.to_string(),
                }));
                failed += 1;
            }
        }
    }

    if dry_run {
        println!(
            "dry run: {} file(s) previewed, nothing written, nothing journalled",
            files.len()
        );
        return Ok(std::process::ExitCode::SUCCESS);
    }

    let rule = g.rule();
    println!("{rule}");
    println!(
        "  {completed} converted{}{failed} failed{}{skipped} skipped{}{total} total",
        g.sep(),
        g.sep(),
        g.sep()
    );
    println!(
        "  {} elapsed{}{} in {} {} out",
        crate::ui::secs(started.elapsed()),
        g.sep(),
        crate::ui::bytes(bytes_in),
        g.arrow(),
        crate::ui::bytes(bytes_out),
    );
    println!(
        "  journal: {}{}",
        crate::show(
            &openconvert_run::state::paths::journal_dir().join(format!("{batch_id}.jsonl"))
        ),
        if resume {
            String::new()
        } else {
            "  (--resume skips completed)".to_string()
        }
    );
    println!("{rule}");

    if let Some(mp) = manifest_path {
        write_manifest(mp, &batch_id, completed, failed, skipped, &records)?;
        println!("  manifest: {}", crate::show(mp));
    }

    if failed > 0 {
        Ok(std::process::ExitCode::from(1))
    } else {
        Ok(std::process::ExitCode::SUCCESS)
    }
}

/// True for our own receipt sidecars: `<output>.receipt.json`, plus the
/// ` (2)`-style names conflict resolution gives the copies.
///
/// A user file that merely looks like one is excluded from conversion at
/// worst — never touched, never overwritten.
fn is_receipt_sidecar(name: &str) -> bool {
    let Some(base) = name.strip_suffix(".json") else {
        return false;
    };
    let base = match base.rsplit_once(" (") {
        Some((stem, n))
            if n.ends_with(')') && n[..n.len() - 1].bytes().all(|b| b.is_ascii_digit()) =>
        {
            stem
        }
        _ => base,
    };
    base.ends_with(".receipt")
}

/// True when a file's extension already names the target format.
///
/// Extension, not content: this decides what joins the batch, not what any
/// file *is* — detection still runs per file before anything converts.
fn already_in_target_format(path: &Path, target: Target) -> bool {
    let Target::Format(f) = target else {
        return false;
    };
    let Some(row) = f.row() else {
        return false;
    };
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(row.extension))
}

fn write_manifest(
    path: &Path,
    batch_id: &str,
    completed: usize,
    failed: usize,
    skipped: usize,
    records: &[serde_json::Value],
) -> anyhow::Result<()> {
    let doc = serde_json::json!({
        "batch": batch_id,
        "completed": completed,
        "failed": failed,
        "skipped": skipped,
        "files": records,
    });
    std::fs::write(path, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}

/// The NAME of the folder a file sits in, for the prediction history.
///
/// Never the path: see `JournalEntry::source_folder`. `None` when the file
/// has no parent we can name, which is a file at a filesystem root.
fn folder_name(path: &std::path::Path) -> Option<String> {
    path.parent()
        .and_then(std::path::Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The batch input set must exclude our own sidecars — including the
    /// ` (2)` names conflict resolution produces — or the outputs of run one
    /// change the batch id of run two, and `--resume` finds nothing.
    #[test]
    fn receipt_sidecars_are_recognised_in_all_spellings() {
        assert!(is_receipt_sidecar("img1.jpg.receipt.json"));
        assert!(is_receipt_sidecar("img2.jpg.receipt (2).json"));
        assert!(is_receipt_sidecar("img10.jpg.receipt (12).json"));
        // A user file that only resembles a sidecar is still converted.
        assert!(!is_receipt_sidecar("photo.json"));
        assert!(!is_receipt_sidecar("notes (draft).json"));
        assert!(!is_receipt_sidecar("receipt.json"));
        assert!(!is_receipt_sidecar("img1.png"));
    }

    /// Files already in the target format are not inputs: jpeg→jpeg has no
    /// route, so attempting them can only manufacture failures.
    #[test]
    fn target_format_files_are_not_inputs() {
        let jpeg = crate::parse_format("jpeg").unwrap();
        let dir = std::env::temp_dir().join(format!("tx-batch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let jpg = dir.join("a.JPG");
        let png = dir.join("b.png");
        std::fs::write(&jpg, b"x").unwrap();
        std::fs::write(&png, b"x").unwrap();

        assert!(already_in_target_format(&jpg, Target::Format(jpeg)));
        assert!(!already_in_target_format(&png, Target::Format(jpeg)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
