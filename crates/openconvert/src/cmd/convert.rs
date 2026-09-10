//! One file, converted — the pipeline `convert` and `batch` both run.
//!
//! # Why this is shared rather than written twice
//!
//! `convert` and `batch` had separate copies of detect → probe → route →
//! execute, and they had drifted: `batch` honoured neither the configured
//! output destination nor the naming template nor the receipt toggle, so the
//! same `config.toml` produced different files depending on which verb the
//! user typed. That is the shape this project keeps finding — a setting whose
//! write succeeds and whose read never happens — and two loops is how it got
//! there. There is now one.
//!
//! # What is resolved once, and why
//!
//! [`Job::resolve`] settles the destination, the naming template and the
//! policy **before the first file is opened**. `naming_output` already worked
//! this way and said why: a template that fails validation must fail before
//! anything runs, never mid-batch. A missing Desktop folder is the same
//! problem — discovering it at file 37 of 50 leaves the user with a
//! half-written run and no obvious cause.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use openconvert_core::environment::Environment;
use openconvert_core::plan::{Class, Plan, PlanRequest};
use openconvert_core::policy::Policy;
use openconvert_core::route::route;
use openconvert_core::target::Target;
use openconvert_run::handles::HandleTable;
use openconvert_run::pool::WorkerPool;
use openconvert_run::receipt::Receipt;
use openconvert_run::state::config::UserConfig;

/// Where this run's outputs go.
///
/// Mirrors the desktop shell's enum of the same name, and for the same
/// reason: `"desktop"` and `"downloads"` are **located or refused**, never
/// guessed. Writing to a path the user cannot find is worse than saying no.
#[derive(Clone, Debug)]
pub enum Destination {
    /// Beside each input (the default).
    BesideInput,
    /// One fixed folder, for every file in the run.
    Fixed(PathBuf),
    /// Beside each input, then the original is replaced after verification.
    ReplaceSource,
}

/// Everything one invocation settled before opening a file.
pub struct Job {
    /// What the user asked for.
    pub target: Target,
    /// The policy every file in this run converts under.
    pub policy: Policy,
    /// Where outputs land.
    pub destination: Destination,
    /// The configured naming template, already validated.
    pub naming_template: Option<String>,
    /// Write the receipt sidecar.
    pub write_receipt: bool,
    /// Replace each original after its output verifies.
    pub replace_source: bool,
}

impl Job {
    /// Settle the whole run from the flags and `config.toml`.
    ///
    /// # Errors
    ///
    /// An unusable naming template, a configured folder that does not exist,
    /// or a configured `replace_source` without the confirming flag.
    pub fn resolve(
        target: Target,
        write_receipt: bool,
        replace_source: bool,
    ) -> anyhow::Result<Self> {
        let cfg = UserConfig::load();
        let configured = cfg.output_destination.as_deref().unwrap_or("same_folder");

        // Destructive by definition, so the flag confirms THIS run even when
        // the config already asks for it.
        let replace_requested = replace_source || configured == "replace_source";
        anyhow::ensure!(
            !replace_requested || replace_source,
            "the configured destination is \"replace source\"; pass --replace-source to \
             confirm THIS run"
        );

        let destination = if replace_source {
            Destination::ReplaceSource
        } else {
            match configured {
                "desktop" => Destination::Fixed(known_folder("Desktop")?),
                "downloads" => Destination::Fixed(known_folder("Downloads")?),
                _ => Destination::BesideInput,
            }
        };

        // Fail on a bad template here, before the first file, rather than
        // after some outputs already exist under the old name.
        if let Some(t) = cfg.naming_template.as_deref() {
            openconvert_run::naming::validate_template(t)?;
        }

        Ok(Self {
            target,
            policy: policy_for(target),
            destination,
            naming_template: cfg.naming_template,
            write_receipt,
            replace_source,
        })
    }

    /// Where one input's output lands.
    #[must_use]
    pub fn dest_dir(&self, input: &Path) -> PathBuf {
        match &self.destination {
            Destination::Fixed(d) => d.clone(),
            // `--replace-source` converts into a private temp area first; the
            // replacement happens only after the output is written and
            // hash-verified (see `exec::execute_with`).
            Destination::ReplaceSource => crate::job_tmp_dir(),
            Destination::BesideInput => input.parent().unwrap_or(Path::new(".")).to_path_buf(),
        }
    }
}

/// A user folder, located or refused.
///
/// Never a guessed fallback: Windows lets both Desktop and Downloads be
/// relocated, and a guess writes somewhere the user will not think to look.
fn known_folder(name: &str) -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");

    home.map(PathBuf::from)
        .map(|h| h.join(name))
        .filter(|d| d.is_dir())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "config.toml sends output to {name}, and that folder could not be found; \
                 nothing was written"
            )
        })
}

/// The policy for a target, including what its name arms.
///
/// Naming a model operation on the command line IS the explicit request the
/// auto-class ceiling asks for, so it arms for THIS run and no other. See
/// [`Target::arms`] for which name asks for what.
fn policy_for(target: Target) -> Policy {
    // The rule itself is `Target::arms`, in core, because this was the only
    // copy of it and the tool path — which is what the desktop runs — had no
    // copy at all. Two sites deciding independently whether the user asked is
    // how the desktop came to refuse every model-backed tool while the CLI ran
    // them.
    let policy = Policy::default().armed_for(target);

    // `force_sandbox` IS READ HERE, AND IT WAS NOT.
    //
    // The app and this binary share one `config.toml`. `resolve` above reads
    // it for the output folder and the naming template, and the one setting on
    // that screen whose entire subject is confinement was ignored -- so
    // "Sandbox everything possible", switched on in the app, applied in the app
    // and silently did nothing here. A security setting that holds on one of
    // two front doors is worse than one that is missing, because it is
    // believed.
    // Read once: `load` touches the disk, and the two settings below both
    // come from the same file.
    let config = UserConfig::load();

    // The memory ceiling, which is a setting and not a ratchet -- see
    // `Policy::with_worker_memory`. Read here for the same reason
    // `force_sandbox` is: the app and this binary share one config.toml, and a
    // limit that holds on one of two front doors means the model the user
    // raised it for runs from the app and fails from the command line.
    let policy = match config.worker_memory_mb {
        Some(mb) => policy.with_worker_memory(mb.saturating_mul(1 << 20)),
        None => policy,
    };

    // Read here for the same reason `force_sandbox` is: the app and this binary
    // share one config.toml, and a setting that holds on one of two front doors
    // is worse than one that is missing, because it is believed.
    let policy = if config.use_gpu == Some(false) {
        policy.without_gpu()
    } else {
        policy
    };

    if config.force_sandbox == Some(true) {
        return policy.force_sandbox_everywhere();
    }
    policy
}

/// What one successful conversion produced.
pub struct Converted {
    /// Where the output landed.
    pub output: PathBuf,
    /// Where the receipt landed.
    pub receipt: PathBuf,
    /// The receipt itself, for `--json`.
    pub record: Receipt,
    /// The size of the bytes actually converted.
    pub input_bytes: u64,
    /// The size of what was written.
    pub output_bytes: u64,
    /// Hex content id, for the journal.
    pub content_id_hex: String,
    /// The stable request hash, for the journal.
    pub request_hash: u64,
    /// Wall time for this file alone.
    pub elapsed: Duration,
}

impl Converted {
    /// The worst class across the steps, for the progress line.
    #[must_use]
    pub fn class(&self) -> Option<Class> {
        match self.record.class.as_deref() {
            Some("A") => Some(Class::A),
            Some("B") => Some(Class::B),
            Some("C") => Some(Class::C),
            Some("D") => Some(Class::D),
            _ => None,
        }
    }

    /// True when every step reported some confinement.
    ///
    /// Read from the receipt rather than from the request, because the
    /// receipt records what the child read back about itself.
    #[must_use]
    pub fn sandboxed(&self) -> bool {
        !self.record.steps.is_empty()
            && self
                .record
                .steps
                .iter()
                .all(|s| !s.isolation.starts_with("in-process"))
    }

    /// The output's filename, for a journal line.
    #[must_use]
    pub fn output_name(&self) -> String {
        self.output
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }
}

/// Refusal as an error carrying the first blocking warning, so callers say
/// *why* rather than only that it failed.
#[must_use]
pub fn refusal_reason(plan: &Plan) -> String {
    plan.warnings()
        .iter()
        .find(|w| w.is_blocking())
        .map_or_else(|| "no executable route".to_string(), crate::describe)
}

/// Why one file did not convert.
///
/// A refusal is not an error in the usual sense — it is the routing table
/// answering "no", with reasons — so it keeps the [`Plan`] rather than
/// collapsing to a string. A single-file `convert` renders the whole refusal
/// the way it always has; a batch line renders one sentence of it. Both read
/// the same plan, so a divergence between them is a rendering bug rather than
/// two behaviours.
#[derive(Debug)]
pub enum ConvertError {
    /// No executable route, with what was detected and what the router said.
    Refused {
        /// The plan that came back unexecutable.
        plan: Box<Plan>,
        /// What detection concluded the input was.
        detected: openconvert_core::format::FormatId,
    },
    /// Detection or execution failed.
    Failed(anyhow::Error),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { plan, .. } => f.write_str(&refusal_reason(plan)),
            Self::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<anyhow::Error> for ConvertError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

impl From<std::io::Error> for ConvertError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed(e.into())
    }
}

impl From<openconvert_run::exec::ExecError> for ConvertError {
    fn from(e: openconvert_run::exec::ExecError) -> Self {
        Self::Failed(e.into())
    }
}

/// detect → probe → route → execute, for one file.
///
/// `index` is the file's 1-based position in the run, which is what `{index}`
/// in a naming template renders. It only means anything across several files,
/// which is why it is a parameter rather than a constant 1.
///
/// # Errors
///
/// Detection failure, a refusal (no executable route), or an execution error.
pub fn convert_one(
    path: &Path,
    job: &Job,
    index: usize,
    env: &Environment,
    pool: &mut WorkerPool,
) -> Result<Converted, ConvertError> {
    let started = Instant::now();

    // One open, and everything downstream reads through it.
    let mut table = HandleTable::new();
    let facts = openconvert_run::detect::detect(path, &mut table)?;
    let s = facts.sniff();

    // Phase 2. `route::limits_for` narrows `decode_pixels` from what this
    // returns and never widens, so a small image converts under a small
    // ceiling.
    let bytes = openconvert_run::detect::bytes_of(&facts, &mut table)?;
    let input_bytes = bytes.len() as u64;
    let props = openconvert_run::probe::probe(&facts, &bytes, &job.policy.base_limits(), pool);

    let request = PlanRequest {
        input: s.detected,
        target: job.target,
        polyglot: s.polyglot,
    };
    let request_hash = super::plan_hash(&request);
    let plan = route(request, props, &job.policy, env);
    if !plan.is_executable() {
        return Err(ConvertError::Refused {
            plan: Box::new(plan),
            detected: s.detected,
        });
    }

    // Named from the format detection already in hand, so nothing re-opens
    // the file to decide what the output is called.
    let opts = openconvert_run::exec::ExecOptions {
        dest_dir: job.dest_dir(path),
        output_name: output_name(path, job, &plan, s.detected, index)?,
        write_receipt: job.write_receipt,
        replace_source: job.replace_source,
        source_path: Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf())),
    };

    let o =
        openconvert_run::exec::execute_with(&plan, &facts, &mut table, &opts, &job.policy, pool)?;

    Ok(Converted {
        output: o.output,
        receipt: o.receipt,
        input_bytes,
        output_bytes: o.record.output_bytes,
        content_id_hex: super::hex_encode(facts.content_id()),
        request_hash,
        record: o.record,
        elapsed: started.elapsed(),
    })
}

/// The output filename for one input under this job.
///
/// `detected` comes from the caller's already-open handle: naming must not be
/// the reason a file is read a second time.
fn output_name(
    path: &Path,
    job: &Job,
    plan: &openconvert_core::plan::Plan,
    detected: openconvert_core::format::FormatId,
    index: usize,
) -> anyhow::Result<String> {
    // THE NAME COMES FROM THE PLAN, NOT THE TARGET.
    //
    // `Target::output_format` answers "what did the user ask for"; for an
    // operation that is "the format it came in as". What goes in the FILE is
    // whatever the last step produces, and for noise removal those two differ
    // whenever the plan cannot reach the original encoder. Naming by the
    // target produced a WAV called `.mp3`.
    //
    // The fallback is the target, for a step that names no format of its own —
    // a stream copy, a metadata strip — which is correct, because those do not
    // change the format either.
    let out_format = plan
        .output_format()
        .unwrap_or_else(|| job.target.output_format(detected));
    let stem = path.file_stem().map_or_else(
        || "output".to_string(),
        |x| x.to_string_lossy().into_owned(),
    );
    let ext = out_format
        .row()
        .map_or_else(|| "bin".to_string(), |r| r.extension.to_string());
    crate::naming_output(
        job.naming_template.as_deref(),
        &stem,
        &ext,
        path.to_path_buf(),
        index,
    )
}

/// What a `--dry-run` would do, without doing it.
///
/// No journal line and no output: a preview must not look like progress to a
/// later resume.
///
/// # Errors
///
/// Detection failure or a refusal.
pub fn preview_one(
    path: &Path,
    job: &Job,
    index: usize,
    env: &Environment,
    pool: &mut WorkerPool,
) -> anyhow::Result<String> {
    let mut table = HandleTable::new();
    let facts = openconvert_run::detect::detect(path, &mut table)?;
    let s = facts.sniff();
    let bytes = openconvert_run::detect::bytes_of(&facts, &mut table)?;
    let props = openconvert_run::probe::probe(&facts, &bytes, &job.policy.base_limits(), pool);

    let plan = route(
        PlanRequest {
            input: s.detected,
            target: job.target,
            polyglot: s.polyglot,
        },
        props,
        &job.policy,
        env,
    );
    if !plan.is_executable() {
        anyhow::bail!("{}", refusal_reason(&plan));
    }
    output_name(path, job, &plan, s.detected, index)
}
