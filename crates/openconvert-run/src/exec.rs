//! `execute()` — the imperative shell.
//!
//! Everything interesting already happened in [`openconvert_core::route::route`].
//! This runs the steps it produced, writes the output through the one permitted
//! creation path, and writes a receipt. If the receipt cannot be written, the
//! output is removed and the step fails.

use crate::engines::{self, EngineError};
use crate::handles::HandleTable;
use crate::pool::WorkerPool;
use crate::receipt::{self, Receipt};
use crate::worker_client::WorkerError;
use crate::write::{create_output, Placed};
use openconvert_core::facts::FileFacts;
use openconvert_core::isolation::Isolation;
use openconvert_core::plan::{Plan, StepKind};
use openconvert_core::policy::Policy;
use openconvert_sandbox::argv::EngineBin;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What happened.
#[derive(Debug)]
pub struct Outcome {
    /// Where the output landed.
    ///
    /// Under [`ExecOptions::replace_source`] this is the ORIGINAL input's
    /// path, which the output now occupies under a new name only if formats
    /// differ — see [`replace_input_with_output`] for the exact rule.
    pub output: PathBuf,
    /// Where the receipt landed. `None` is impossible today through
    /// [`execute`]; `execute_with` can produce it when the caller declined
    /// receipt writing (the GUI's privacy setting), in which case the
    /// in-memory [`Outcome::record`] is still complete.
    pub receipt: PathBuf,
    /// The receipt itself, for `--json`.
    pub record: Receipt,
}

/// Options a caller may set around one execution.
///
/// Every field has a default that reproduces [`execute`] exactly, which is
/// why `execute` is now one line of delegation: the desktop shell passes
/// richer options and the CLI keeps the shape it has always had.
#[derive(Debug, Clone)]
pub struct ExecOptions {
    /// Where the output lands.
    pub dest_dir: PathBuf,
    /// What the output is called there.
    pub output_name: String,
    /// Write the receipt at all. The CLI never sets this false — SR-11 makes
    /// its receipt structural — while the GUI's `write_receipts=false`
    /// setting means "no record anywhere", which is a product decision about
    /// the user's own machine, made explicit here rather than buried in a
    /// global.
    pub write_receipt: bool,
    /// After a verified conversion, replace the original input with the
    /// output. Destructive by definition, so it happens LAST, only after both
    /// files are fully written and hashed, and as ONE rename — never a
    /// truncate-in-place. The creation gates above are untouched during the
    /// conversion itself.
    ///
    /// [`Self::source_path`] must name the original: file facts deliberately
    /// carry no path (I1), so the caller supplies what it opened.
    pub replace_source: bool,
    /// The original file that `replace_source` overwrites.
    pub source_path: Option<PathBuf>,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            dest_dir: PathBuf::from("."),
            output_name: String::new(),
            write_receipt: true,
            replace_source: false,
            source_path: None,
        }
    }
}

/// Why a conversion did not complete.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The plan was refused. There is nothing to run.
    #[error("this plan is not executable; nothing was written")]
    NotExecutable,
    /// A step needs a model the registry could not supply.
    ///
    /// Separate from a worker failure because the fix is different: the user
    /// downloads a model, they do not reinstall an engine.
    #[error("the {0} model is not available: {1}")]
    Model(&'static str, String),
    /// A sandboxed step names no worker.
    ///
    /// Structurally unreachable through `route()`, which refuses a format whose
    /// media kind has no engine before a plan is ever built. An error rather
    /// than a fallback to in-process: falling back is how a decision to
    /// sandbox quietly becomes a decision not to.
    #[error("this step needs a sandboxed engine and no engine handles {0}; nothing was written")]
    NoWorker(&'static str),
    /// A sandboxed step failed.
    #[error(transparent)]
    Worker(#[from] WorkerError),
    /// An engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The output could not be created.
    #[error("could not write the output: {0}")]
    Output(std::io::Error),
    /// The facts named a token this table does not hold.
    ///
    /// Structurally unreachable through `detect()`, and an error rather than a
    /// panic because the alternative is a crash on a forged token.
    #[error("internal: the input handle for this file is not open")]
    UnknownToken,
    /// The output was written and its receipt was not.
    ///
    /// **The output has been removed.** A receipt is not optional; an output
    /// without one breaks the product's second pillar, and leaving it behind
    /// would leave the user a file whose provenance nobody can check.
    #[error(
        "converted, but could not write the receipt ({source}). \
         The output was removed."
    )]
    ReceiptUnwritable {
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
}

/// Run a plan against one input.
///
/// # Errors
///
/// See [`ExecError`]. Every variant leaves the filesystem as it found it,
/// except that a successful conversion adds exactly two files.
pub fn execute(
    plan: &Plan,
    facts: &FileFacts,
    table: &mut HandleTable,
    dest_dir: &Path,
    output_name: &str,
    policy: &Policy,
    pool: &mut WorkerPool,
) -> Result<Outcome, ExecError> {
    execute_with(
        plan,
        facts,
        table,
        &ExecOptions {
            dest_dir: dest_dir.to_path_buf(),
            output_name: output_name.to_string(),
            ..ExecOptions::default()
        },
        policy,
        pool,
    )
}

/// Run a plan with the full option set — the entry point the desktop shell's
/// settings feed.
///
/// # Errors
///
/// As [`execute`]. [`ExecError::ReceiptUnwritable`] fires only when the
/// caller ASKED for a receipt; declining one is honoured, not failed.
#[allow(clippy::too_many_lines)]
pub fn execute_with(
    plan: &Plan,
    facts: &FileFacts,
    table: &mut HandleTable,
    opts: &ExecOptions,
    policy: &Policy,
    pool: &mut WorkerPool,
) -> Result<Outcome, ExecError> {
    if !plan.is_executable() {
        return Err(ExecError::NotExecutable);
    }

    // I13/SR-16, structurally rather than by convention.
    //
    // The bytes converted come from the handle `detect()` opened and parked
    // under this token -- the same handle that produced `content_id`. There is
    // no path here to re-open, which is what makes "the bytes routed on are the
    // bytes converted" a property of the types rather than a rule someone has
    // to remember. An earlier version of this function took `&mut R` and hashed
    // it again, which gave the same answer for a different reason: because the
    // caller happened to pass the right thing.
    let bytes = crate::detect::bytes_of(facts, table).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ExecError::UnknownToken
        } else {
            ExecError::Output(e)
        }
    })?;
    let content_id = *facts.content_id();

    let detected = facts.sniff().detected.to_string();
    let declared_mismatch = facts.sniff().mismatched().then(|| {
        facts
            .sniff()
            .declared
            .map_or_else(String::new, |d| d.to_string())
    });

    let mut current = bytes;
    let mut records = Vec::new();

    // What the NEXT step will be reading. The first step reads the file as
    // detected; each one after it reads what the one before produced.
    let mut current_format = facts.sniff().detected;

    /// The format a step produces, when it names one.
    fn step_output_format(kind: StepKind) -> Option<openconvert_core::format::FormatId> {
        match kind {
            StepKind::Transcode { to, .. }
            | StepKind::Infer { to, .. }
            | StepKind::Extract { to, .. } => Some(to),
            _ => None,
        }
    }

    for step in plan.steps() {
        let out = if let StepKind::Trim { start_ms, end_ms } = step.kind {
            // Both parsers are ours and bounded like the sniffer, so the
            // graph edit runs here — the same reasoning that lets StreamCopy
            // stay in-process. The output is re-serialised Matroska carrying
            // the kept samples' payloads verbatim; only presence and
            // timestamps change.
            let demux_err = |e: crate::matroska::DemuxError| EngineError::Decode {
                engine: "openconvert-container",
                format: facts.sniff().detected,
                detail: e.to_string(),
            };
            let graph =
                crate::matroska::demux(&current, step.limits.memory_bytes).map_err(demux_err)?;
            let trimmed = crate::matroska::trim(&graph, start_ms, end_ms).map_err(demux_err)?;
            let bytes = crate::matroska::graph_to_ebml(&trimmed).map_err(demux_err)?;
            engines::StepOutput {
                bytes,
                removed: vec!["samples outside the trim range".to_string()],
                engine: "openconvert-container",
                confinement: None,
            }
        } else if is_video_transcode(step.kind) {
            // BEFORE the generic sandboxed arm, not after.
            //
            // This step IS sandboxed -- `route()` says so, and the receipt
            // must -- but it is confined by this module rather than by the
            // worker protocol, because FFmpeg is not our binary and does not
            // speak it. Ordered after the generic arm, this was unreachable
            // and the run failed with "no engine handles mp4".
            //
            // THE VIDEO MODULE. It is handed `current` -- the bytes this
            // chain has been carrying -- not a path, so I13/SR-16 holds: what
            // is converted is what was routed and hashed. `video` writes them
            // to its own scratch because MP4 demuxing needs to seek, and
            // removes it afterwards.
            let to = plan.request().target.output_format(facts.sniff().detected);
            let bytes = crate::video::transcode(&current, to, &step.limits).map_err(|e| {
                EngineError::Decode {
                    engine: "tx-video",
                    format: facts.sniff().detected,
                    detail: e.to_string(),
                }
            })?;
            engines::StepOutput {
                bytes,
                removed: vec![
                    "both streams were RE-ENCODED; this is not the original video".to_string(),
                    "subtitle, chapter and data tracks".to_string(),
                    "all tags and metadata".to_string(),
                ],
                engine: "tx-video",
                // REQUESTED, not read back -- and the wording says so.
                //
                // Every other sandboxed step reports what the child asked the
                // OS about itself, because the child is our binary and speaks
                // our protocol. FFmpeg is neither. All that can honestly be
                // claimed is what was asked for at spawn, and a receipt that
                // said "engaged" here would be asserting something nobody
                // measured. It previously said "in-process (pure Rust)",
                // which was false twice over.
                confinement: Some(
                    "sandboxed (requested at spawn, not read back): AppContainer, no filesystem beyond one scratch input"
                        .to_string(),
                ),
            }
        } else if matches!(step.isolation, Isolation::Sandboxed(_)) {
            // `current_format`, NOT `facts.sniff().detected`.
            //
            // Every step was told the format of the ORIGINAL file, which is
            // correct exactly while every plan has one step — and every plan
            // in the table did. The moment one grows a second step, the second
            // one is handed bytes in the first one's output format and told
            // they are something else: `EngineBin::for_conversion` picks a
            // worker by that pair, so it would pick the wrong worker and hand
            // it bytes it cannot parse.
            //
            // Threading it costs nothing for a single-step plan, where
            // `current_format` is still the detected format.
            run_sandboxed(
                step,
                current_format,
                plan.request().target.output_format(facts.sniff().detected),
                current,
                facts.provenance(),
                plan.class(),
                pool,
            )?
        } else if matches!(step.kind, StepKind::StreamCopy) {
            // The flagship. Class A video: coded samples move between
            // containers byte-for-byte in this process, because both parsers
            // are ours and bounded like the sniffer (`02` §3.1). The target
            // comes from the plan's request, since a StreamCopy step names no
            // format of its own.
            let to = plan.request().target.output_format(facts.sniff().detected);
            let from = facts.sniff().detected;
            let (bytes, removed) = crate::remux::stream_copy(&current, from, to, &step.limits)
                .map_err(|e| EngineError::Decode {
                    engine: "openconvert-container",
                    format: from,
                    detail: e.to_string(),
                })?;
            engines::StepOutput {
                bytes,
                removed,
                engine: "openconvert-container",
                confinement: None,
            }
        } else {
            engines::run_in_process(step.kind, &mut std::io::Cursor::new(current), &step.limits)?
        };
        // The receipt records what the WORKER read back, not what `route()`
        // planned -- see `StepOutput::confinement`. An in-process step has no
        // worker and keeps the plan's description, which for it is exact.
        let mut rec = receipt::record(step, out.engine, out.removed);
        if let Some(c) = out.confinement {
            rec.isolation = c;
        }
        records.push(rec);
        current = out.bytes;
        // And the bytes are now in whatever format this step produced, which
        // is what the next one will be reading. A step that names no format of
        // its own — a stream copy, a metadata strip — leaves it unchanged,
        // which is exactly right: it did not change the format either.
        if let Some(to) = step_output_format(step.kind) {
            current_format = to;
        }
    }

    // Create first, write second. `create_output` resolves conflicts before
    // opening anything, so an existing file is never at risk (I12/SR-15).
    let (placed, file) = create_output(&opts.dest_dir, &opts.output_name, policy.on_conflict())
        .map_err(ExecError::Output)?;
    let mut output_path = match &placed {
        Placed::Created(p) | Placed::Skipped(p) => p.clone(),
    };
    let Some(mut file) = file else {
        // Skipped by policy. Nothing was written, so there is nothing to
        // attest to.
        return Err(ExecError::Output(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists; skipped by policy", output_path.display()),
        )));
    };
    file.write_all(&current).map_err(ExecError::Output)?;
    let written = current.len() as u64;
    drop(file);

    // Replacement comes BEFORE the receipt is written, so the receipt lands
    // beside the output's FINAL location under its FINAL name — a receipt
    // naming a file that no longer exists anywhere would be worse than none.
    //
    // The order inside the replacement is the safety argument: hash-verify
    // what landed, then ONE rename. If verification refuses, the original is
    // untouched and the freshly-written output simply stays where it was.
    if opts.replace_source {
        let source = opts.source_path.as_ref().ok_or_else(|| {
            ExecError::Output(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "replace_source was requested but no source path was given",
            ))
        })?;
        replace_input_with_output(source, &output_path, &current).map_err(ExecError::Output)?;
        output_path = source.to_path_buf();
    }

    let mut record = Receipt::new(
        plan,
        &content_id,
        &detected,
        declared_mismatch,
        records,
        &output_path,
        written,
    );
    if let Some(name) = output_path.file_name() {
        record.output = name.to_string_lossy().into_owned();
    }

    // The receipt decision. Three shapes:
    //   beside-output (CLI default)      -> write_beside
    //   store-only (GUI, receipts on)    -> write_beside AND put_in(store)
    //   none (GUI, receipts off)         -> nothing on disk; the in-memory
    //                                       record still returns to the
    //                                       caller, because the conversion
    //                                       itself succeeded.
    let receipt_path: Option<PathBuf>;
    if opts.write_receipt {
        match record.write_beside(&output_path, policy.on_conflict()) {
            Ok(p) => receipt_path = Some(p),
            Err(e) => {
                // The output must not survive its receipt. `03` §13. Under
                // replace_source this removes the REPLACED file — which is
                // exactly why the GUI confirms replacement twice before this
                // path can ever run.
                remove_output(&output_path);
                return Err(ExecError::ReceiptUnwritable { source: e });
            }
        }
    } else {
        receipt_path = None;
    }
    Ok(Outcome {
        receipt: receipt_path.unwrap_or_else(|| output_path.clone()),
        output: output_path,
        record,
    })
}

/// Replace the original input with the freshly-written output.
///
/// The order of operations is the safety argument:
///
/// 1. the output on disk is re-read and hashed against the bytes this call
///    converted — a partial or corrupted write never reaches the rename;
/// 2. ONE rename moves the output over the original. No truncation, no
///    in-place edit, no window where neither version exists.
///
/// This is the single deliberate exception to I12 inside the execution path,
/// and it exists because the user asked for exactly this destruction, after
/// the GUI confirmed it twice (settings + per-run dialog). The lint
/// exemption below is that sentence, spelled out where the auditor will look.
fn replace_input_with_output(
    input: &Path,
    output: &Path,
    expected: &[u8],
) -> Result<(), std::io::Error> {
    use std::io::Read;
    let mut verify = Vec::with_capacity(expected.len());
    std::fs::File::open(output)?.read_to_end(&mut verify)?;
    if blake3::hash(&verify).as_bytes() != blake3::hash(expected).as_bytes() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the written output did not match what was converted; refusing to replace \
             the original",
        ));
    }
    // The one journaled replacement. Explicit, confirmed replace_source runs
    // AFTER both files are fully written and hash-verified, as a single
    // atomic rename over the CALLER'S OWN input.
    std::fs::rename(output, input)?; // openconvert-lint: allow -- verified replacement of the caller's own input (see above)
    Ok(())
}

/// Run one step in a confined subprocess.
///
/// Whether that subprocess is fresh or reused is [`WorkerPool`]'s decision,
/// made from `Policy::worker_reuse` (D8) and this file's provenance (SR-20).
/// The pool outlives `execute`, because reuse only means anything across files
/// — a single conversion cannot reuse anything, and a per-call pool would have
/// made the setting decorative in a different way.
fn run_sandboxed(
    step: &openconvert_core::plan::Step,
    plan_input: openconvert_core::format::FormatId,
    plan_target: openconvert_core::format::FormatId,
    bytes: Vec<u8>,
    provenance: openconvert_core::facts::Provenance,
    plan_class: Option<openconvert_core::plan::Class>,
    pool: &mut WorkerPool,
) -> Result<engines::StepOutput, ExecError> {
    // Which format the worker is being asked to produce.
    //
    // `Transcode` names it; `Extract` does not, because the route table's
    // extraction rows are archive-to-archive repacks whose destination lives in
    // the plan's target rather than the step. Both reach a worker the same way
    // -- one blob in, one blob out -- so they share this path rather than
    // getting a second one that would drift.
    let mut params: Vec<(String, String)> = Vec::new();

    // THE ROUTE'S CLASS TRAVELS WITH THE REQUEST.
    //
    // `oc-audio` refuses to quantise float samples into FLAC, which is right
    // for `Wav -> Flac`: that row is Class A, and quantising under a lossless
    // receipt is the one thing this project must never do. It is wrong for
    // `M4a -> Flac`, which is Class B — the source was already lossy, the row
    // says so, and refusing there left an AAC file with no path to FLAC at
    // all.
    //
    // The worker cannot tell those apart; only the plan knows. So the class
    // is sent, and the encoder quantises exactly when the row it is executing
    // has already declared itself lossy.
    //
    // Sent ONLY for a FLAC destination, because that is the only encoder that
    // consults it. Every engine refuses a parameter it does not recognise —
    // correctly, since silent parameter loss converts something other than
    // what was asked — so broadcasting this to all Class B steps made
    // `svg -> png` fail with "this engine takes no parameters".
    if plan_class == Some(openconvert_core::plan::Class::B)
        && plan_target == openconvert_core::format::FormatId::Flac
    {
        params.push(("lossy_route".to_string(), "1".to_string()));
    }
    let to = match step.kind {
        StepKind::Transcode { to, .. } => to,
        StepKind::RenderPage { page } => {
            // The page index is the one parameter any step carries today. It
            // rides the protocol as a key/value pair rather than a positional,
            // so a second parameter later costs nothing here.
            params.push(("page".to_string(), page.to_string()));
            plan_target
        }
        // `Extract` NAMES ITS DESTINATION now, and this used to read
        // `plan_target`. That was the same value while every archive route had
        // one step, and wrong the moment one grew two: `gzip -> tar -> zip`
        // would have asked for a zip in both steps and handed the first one a
        // gzip.
        StepKind::Extract { to, .. } => to,
        // Trim is handled in the in-process path (see execute()); it never
        // reaches a worker because both parsers are pure Rust.
        StepKind::Trim { .. } => plan_target,
        // Still not implemented. Naming the kind beats a generic refusal: it
        // tells a maintainer which row of `03` §6 is missing rather than that
        // something is.
        // The operation rides the wire the same way an inference task does,
        // and for the same reason: the destination format cannot say which of
        // several pixel operations was meant.
        StepKind::Pixel { op } => {
            params.push(("op".to_string(), op.to_string()));
            plan_target
        }
        StepKind::Infer { task, to, .. } => {
            // The adapter is named on the wire, not inferred from the target:
            // several tasks can share one destination format, and a worker
            // guessing which one was meant is a worker converting something
            // other than what was asked.
            params.push(("task".to_string(), task.to_string()));
            to
        }
        StepKind::StreamCopy => return Err(ExecError::NoWorker("a stream copy")),
        StepKind::StripMetadata => return Err(ExecError::NoWorker("a metadata strip")),
    };

    // The weights, read here and sent down the pipe with the input.
    //
    // Read HOST-side because the worker has no filesystem, and verified before
    // sending because a model is code in every way that matters: the registry
    // re-checks the artifact against its sha256 pin, so a store corrupted
    // after download cannot reach the runtime.
    let models: Vec<Vec<u8>> = match step.kind {
        StepKind::Infer { models, .. } => models
            .iter()
            .map(|id| {
                crate::models::verified_bytes(id).map_err(|e| ExecError::Model(id, e.to_string()))
            })
            .collect::<Result<_, _>>()?,
        _ => Vec::new(),
    };

    // THE WORKER IS CHOSEN BY WHAT IT MUST PARSE, NOT BY WHAT IT WRITES.
    //
    // This read the DESTINATION's media kind, which is right only while every
    // route's two sides share one worker. `Pdf -> Png` does not: the
    // destination is an image, so the step was handed to oc-images, which was
    // then given PDF bytes it has no decoder for and answered "the image
    // format could not be determined" -- a refusal that names the wrong
    // problem entirely. `Pdf -> Jpeg` and every `--page` render had the same
    // fate.
    //
    // The source is also the security-relevant side. The bytes a worker
    // parses are the attacker-controlled ones, so "which worker" and "whose
    // parser is exposed" are the same question, and answering it from the
    // destination was answering a different one.
    //
    // A `Transcode` names its own source; `RenderPage` and `Extract` do not,
    // and take the plan's input for the same reason they take its target.
    //
    // `for_conversion` reads BOTH ends, because audio extraction crosses
    // kinds: `mkv -> mp3` parses a video container and writes an audio file,
    // and only oc-audio can do the second half.
    let from = match step.kind {
        StepKind::Transcode { from, .. } | StepKind::Extract { from, .. } => from,
        _ => plan_input,
    };
    // An inference step names its own worker. Every format-based rule would
    // send `png -> png` to oc-images, which is right for every other step and
    // wrong for this one: what decides here is what the STEP does, not what
    // the file is.
    let bin = match step.kind {
        StepKind::Infer { .. } => EngineBin::Ai,
        _ => EngineBin::for_conversion(from, to).ok_or(ExecError::NoWorker(from.name()))?,
    };

    // The limits sent are the step's, which `route()` already clamped against
    // the ceiling (I14). Nothing here re-derives them, because two places
    // computing the same limit is how they come to disagree.
    let (out, removed, confinement) = pool.run_with_models(
        bin,
        provenance,
        &step.limits,
        bytes,
        &models,
        to.name(),
        &params,
    )?;
    Ok(engines::StepOutput {
        confinement: Some(confinement),
        bytes: out,
        removed,
        engine: bin.file_stem(),
    })
}

/// Whether this step is the video module's.
///
/// A `Transcode` between two VIDEO containers is the only shape that reaches
/// the module: audio destinations go to oc-audio, and `Txt` to oc-ai.
fn is_video_transcode(kind: StepKind) -> bool {
    use openconvert_core::format::MediaKind as K;
    match kind {
        StepKind::Transcode { from, to } => {
            from.kind() == Some(K::Video) && to.kind() == Some(K::Video)
        }
        _ => false,
    }
}

/// Remove an output we just created, because its receipt could not be written.
///
/// The one deletion in this crate, and it only ever targets a path
/// `create_output` returned moments earlier — never a user-named path, never a
/// directory, never a glob.
fn remove_output(path: &Path) {
    let _ = std::fs::remove_file(path); // openconvert-lint: allow -- removes only the output we just created, per 03 §13's receipt rule
}
