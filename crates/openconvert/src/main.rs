//! The CLI.
//!
//! `openconvert plan` is `route()` without `execute()` — the plan preview is not
//! a feature, it is the absence of one call. That is the most distinctive thing
//! in the product and it exists because of the architecture rather than in
//! addition to it.

use openconvert_core::environment::Environment;
use openconvert_core::format::{FormatId, TABLE};
use openconvert_core::isolation::Isolation;
use openconvert_core::plan::{Plan, PlanRequest, Warning};
use openconvert_core::policy::Policy;
use openconvert_core::route::{route, Requirement, RouteTable};
use openconvert_core::target::{Operation, Target};
use openconvert_run::detect::sniff;
use openconvert_sandbox::display::DisplayName;
use std::path::{Path, PathBuf};

mod cmd;
mod ui;

use cmd::{batch::run as cmd_batch, recipe::run as cmd_recipe};

/// A path, safe to print.
///
/// **Every path this program prints goes through here.** A filename is not
/// something the user typed — `openconvert convert *.pdf` expands from the
/// filesystem, and a name carrying `ESC ] 0 ;` retitles the terminal while a
/// name carrying U+202E reads backwards. `09` §3 lists that under A15 and,
/// until `DisplayName` existed, nothing enforced it.
///
/// Lossy on purpose: a path that is not valid UTF-8 still has to be shown, and
/// showing it is more useful than refusing to name the file the user asked
/// about.
pub(crate) fn show(path: &Path) -> DisplayName {
    DisplayName::new(&path.to_string_lossy())
}

/// A bare filename, safe to print.
///
/// Same rule as [`show`] and the same reason. An output name is built from
/// the *input's* stem, so a hostile input filename reaches the terminal
/// through our own output unless it goes through here too.
pub(crate) fn show_name(name: &str) -> DisplayName {
    DisplayName::new(name)
}

/// Just the filename, safe to print.
///
/// Progress lists name files, not paths: fifty absolute paths down the left
/// of a terminal push the part that changes off the right-hand edge, which is
/// the only part the reader is scanning for.
pub(crate) fn show_base(path: &Path) -> DisplayName {
    path.file_name()
        .map_or_else(|| show(path), |n| DisplayName::new(&n.to_string_lossy()))
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("openconvert: {e}");
            std::process::ExitCode::from(2)
        }
    }
}

fn run() -> anyhow::Result<std::process::ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("convert") => cmd_convert(&args[1..]),
        Some("plan") => cmd_plan(&args[1..]),
        Some("inspect") => cmd_inspect(&args[1..]),
        Some("routes") => cmd_routes(&args[1..]),
        Some("formats") => cmd_formats(),
        Some("concat") => cmd_concat(&args[1..]),
        Some("models") => cmd_models(&args[1..]),
        Some("batch") => cmd_batch(&args[1..]),
        Some("recipe") => cmd_recipe(&args[1..]),
        Some("--version" | "-V") => {
            println!("openconvert {}", env!("CARGO_PKG_VERSION"));
            Ok(std::process::ExitCode::SUCCESS)
        }
        _ => {
            usage();
            Ok(std::process::ExitCode::from(2))
        }
    }
}

fn usage() {
    eprintln!(
        "openconvert {}\n\
         \n\
           USAGE\n  \
           openconvert convert <file>... -t <fmt>  do it, and write a receipt\n  \
           openconvert convert *.heic -t jpeg    many files, one summary\n  \
           openconvert convert <file> --trim 0:30-1:15\n  \
           openconvert convert <file> -t png --page 3   render page 3 (zero-based)\n  \
           openconvert convert <file> --remux      rewrap without re-encoding\n  \
           openconvert inspect <file>            what is this file, really?\n  \
           openconvert plan <file> -t <format>   what WOULD happen, without doing it\n  \
           openconvert batch <dir> -t <fmt>      many files, one journal; --resume to skip\n  \
           openconvert concat <out.mkv> <in...>  join Matroska files, losslessly\n  \
           openconvert models [list|get <id>|enable|disable|remove]\n  \
           openconvert convert <f> --remove-background  cut the subject out\n  \
           openconvert recipe save|load          pin a conversion, replay it later\n  \
           openconvert routes <from> <to>        print the route table for a pair\n  \
           openconvert formats                   every format we know\n\
         \n\
         Times take SS, MM:SS or HH:MM:SS, with optional .fraction seconds.\n\
         \n\
         `plan` is `convert` minus one call. The preview is the absence of\n\
         execute(), not a feature built beside it.",
        env!("CARGO_PKG_VERSION")
    );
}

// ---------------------------------------------------------------------------

/// What this machine actually offers, measured.
///
/// `openconvert-os` probes once by **creating** each mechanism and closing it
/// again; the pure core consumes the answer as a value. That is how `route()`
/// learns about an impure world without reading anything.
///
/// This comment used to end: *"the profile handed to `route()` stays
/// `unconfined()`... and no child runs yet."* Children run, confined, and
/// confirm it from their own token. The note outlived its reason and refused
/// every sandboxed conversion while it stood.
///
/// The planning/evidence split that replaced it is in
/// [`openconvert_os::available::planning_profile`].
pub(crate) fn probe_real() -> Environment {
    let caps = openconvert_os::available::probe();
    if !caps.probed {
        eprintln!(
            "note: sandbox probing is not implemented on {}. Nothing is claimed for this machine.",
            std::env::consts::OS
        );
    }
    Environment::new(
        // What this machine granted when asked -- `probe()` creates an
        // AppContainer and a Job Object and closes them again rather than
        // reading a version number, and reports absent anything it could not
        // exercise. So the profile can only understate.
        //
        // This is a PLANNING input. `route()` has to decide whether a sandboxed
        // step is worth planning before any worker exists, and the receipt does
        // not come from here: `exec` overwrites each sandboxed step's isolation
        // with what the worker read back about itself, which is the only answer
        // that is evidence (I11, `03` §9.5).
        //
        // Until this call, the shell passed `unconfined()` with a note saying
        // confinement arrives in phase 3. It had arrived, and the note was
        // refusing every sandboxed conversion on a machine that can run them.
        openconvert_os::available::planning_profile(caps),
        openconvert_run::engines::registry(),
        RouteTable::v1(),
    )
}

/// `openconvert convert <file>...`
///
/// One file or fifty. The single-file output is unchanged — it is the shape
/// the receipt documentation and every example already show — and several
/// files switch to a progress list, because fifty copies of the single-file
/// block is not a report.
fn cmd_convert(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let paths = collect_inputs(args)?;
    anyhow::ensure!(!paths.is_empty(), "convert needs at least one file");
    let target = parse_target(args)?;
    let json = args.iter().any(|a| a == "--json");
    let no_receipt = args.iter().any(|a| a == "--no-receipt");
    let replace_source = args.iter().any(|a| a == "--replace-source");

    // Settled once, before the first file is opened. A bad naming template or
    // a configured folder that does not exist must fail HERE, not at file 37
    // of 50 with the first 36 already written.
    let job = cmd::convert::Job::resolve(target, !no_receipt, replace_source)?;
    let env = probe_real();
    // One pool for the whole run: worker reuse (SR-20) is what makes fifty
    // files cost less than fifty spawns, and a pool per file forfeits it.
    let mut pool = openconvert_run::pool::WorkerPool::new(job.policy.worker_reuse());

    if let [only] = paths.as_slice() {
        return convert_single(only, &job, &env, &mut pool, json);
    }
    convert_many(&paths, &job, &env, &mut pool, json)
}

/// The positional arguments, with globs expanded when the shell did not.
///
/// PowerShell and cmd.exe hand wildcards to a native program **unexpanded**,
/// so `openconvert convert *.heic -t jpeg` arrives here as the literal string
/// `*.heic` on exactly the platform this product ships to first. Expanding is
/// the difference between that command working and it reporting one missing
/// file.
///
/// Only `*` and `?` are honoured, and only within a single directory: `**`
/// recursion is what `batch` is for, and quietly walking a tree because a
/// filename contained two stars is not a thing a converter should do.
fn collect_inputs(args: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        // Flags that consume the next argument; their values are not inputs.
        if matches!(a.as_str(), "-t" | "--to" | "--trim" | "--page") {
            it.next();
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        if a.contains('*') || a.contains('?') {
            let matched = expand_glob(Path::new(a))?;
            anyhow::ensure!(
                !matched.is_empty(),
                "no file matches {}",
                DisplayName::new(a)
            );
            out.extend(matched);
        } else {
            out.push(PathBuf::from(a));
        }
    }
    Ok(out)
}

/// Files in one directory matching a `*`/`?` pattern, sorted.
///
/// Sorted because the order decides `{index}` in a naming template, and an
/// output called `holiday_3.jpg` should mean the same file on every run.
fn expand_glob(pattern: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let dir = match pattern.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let Some(name) = pattern
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Ok(Vec::new());
    };
    // A wildcard in a directory component would mean walking the tree to find
    // out what it matched. Refused rather than half-implemented.
    anyhow::ensure!(
        !dir.to_string_lossy().contains(['*', '?']),
        "wildcards are matched in the filename only, not in directories; \
         use `openconvert batch <dir>` to walk a folder"
    );

    let mut matched: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| anyhow::anyhow!("{}: {e}", show(&dir)))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| glob_match(&name, &n.to_string_lossy()))
        })
        .collect();
    matched.sort();
    Ok(matched)
}

/// `*` (any run, including empty) and `?` (exactly one character).
///
/// Case-insensitive on Windows, where the filesystem is, so `*.JPG` and
/// `*.jpg` find the same photographs.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = if cfg!(windows) {
        (
            pattern.to_lowercase().chars().collect(),
            name.to_lowercase().chars().collect(),
        )
    } else {
        (pattern.chars().collect(), name.chars().collect())
    };

    // Iterative backtracking: one star position remembered, so the match is
    // linear in practice and cannot recurse on a hostile pattern.
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ni = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// One file: the output this command has always produced.
fn convert_single(
    path: &Path,
    job: &cmd::convert::Job,
    env: &Environment,
    pool: &mut openconvert_run::pool::WorkerPool,
    json: bool,
) -> anyhow::Result<std::process::ExitCode> {
    match cmd::convert::convert_one(path, job, 1, env, pool) {
        Ok(o) => {
            if json {
                // The receipt IS the machine-readable output. There is no
                // second representation to drift away from it.
                println!("{}", serde_json::to_string_pretty(&o.record)?);
            } else {
                println!("{}", show(&o.output));
                println!(
                    "  {} bytes - class {}",
                    o.record.output_bytes,
                    o.record.class.as_deref().unwrap_or("-")
                );
                for st in &o.record.steps {
                    println!("  {} via {} [{}]", st.kind, st.engine, st.isolation);
                    for r in &st.removed {
                        println!("    removed: {r}");
                    }
                }
                println!("  receipt {}", show(&o.receipt));
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        // Refusal is the absence of a plan. Show why, and write nothing.
        Err(cmd::convert::ConvertError::Refused { plan, detected }) => {
            if json {
                println!("{}", refusal_json(path, &plan));
            } else {
                print_plan(path, detected, &plan);
            }
            Ok(std::process::ExitCode::from(1))
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": false, "error": e.to_string() })
                );
            } else {
                eprintln!("openconvert: {e}");
            }
            Ok(std::process::ExitCode::from(1))
        }
    }
}

/// Several files: a progress line each, then one summary.
///
/// Progress goes to **stderr** so that `--json` on stdout stays a single
/// parseable document and `openconvert convert *.heic -t jpeg > out.json` is
/// still watchable while it runs.
fn convert_many(
    paths: &[PathBuf],
    job: &cmd::convert::Job,
    env: &Environment,
    pool: &mut openconvert_run::pool::WorkerPool,
    json: bool,
) -> anyhow::Result<std::process::ExitCode> {
    let g = ui::Glyphs::detect();
    let total = paths.len();
    let started = std::time::Instant::now();

    let mut records: Vec<serde_json::Value> = Vec::new();
    let (mut completed, mut failed) = (0usize, 0usize);
    let (mut bytes_in, mut bytes_out) = (0u64, 0u64);

    if !json {
        eprintln!("Converting {total} files\n");
    }

    for (i, path) in paths.iter().enumerate() {
        let n = i + 1;
        // `{index}` in a naming template is this file's position in the run,
        // which is the whole reason the token exists.
        match cmd::convert::convert_one(path, job, n, env, pool) {
            Ok(o) => {
                completed += 1;
                bytes_in += o.input_bytes;
                bytes_out += o.output_bytes;
                if json {
                    records.push(serde_json::json!({
                        "input": show(path).to_string(),
                        "ok": true,
                        "output": o.output_name(),
                        "receipt": o.record,
                    }));
                } else {
                    let class = o.class().map_or("-", |c| g.class(c));
                    let guard = if o.sandboxed() {
                        format!("{}{} sandboxed", g.sep(), g.sandboxed())
                    } else {
                        String::new()
                    };
                    eprintln!(
                        "  {} [{n}/{total}] {} {} {}   {class}{guard}   {}",
                        g.status(ui::Status::Ok),
                        show_base(path),
                        g.arrow(),
                        show_name(&o.output_name()),
                        ui::secs(o.elapsed),
                    );
                }
            }
            Err(e) => {
                // One file's failure ends that file, not the run. A batch that
                // stops on the first bad JPEG converts nothing after it.
                failed += 1;
                if json {
                    records.push(serde_json::json!({
                        "input": show(path).to_string(),
                        "ok": false,
                        "error": e.to_string(),
                    }));
                } else {
                    eprintln!(
                        "  {} [{n}/{total}] {}   {e}",
                        g.status(ui::Status::Failed),
                        show_base(path),
                    );
                }
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&records)?);
    } else {
        let rule = g.rule();
        eprintln!("\n{rule}");
        eprintln!(
            "  {completed} converted{}{failed} failed{}{total} total",
            g.sep(),
            g.sep()
        );
        eprintln!(
            "  {} elapsed{}{} in {} {} out",
            ui::secs(started.elapsed()),
            g.sep(),
            ui::bytes(bytes_in),
            g.arrow(),
            ui::bytes(bytes_out),
        );
        if job.write_receipt && completed > 0 {
            eprintln!("  receipts: {completed} written beside outputs");
        }
        eprintln!("{rule}");
        if failed > 0 {
            eprintln!(
                "\n  {} file{} failed; the rest converted.",
                failed,
                if failed == 1 { "" } else { "s" }
            );
        }
    }

    Ok(if failed > 0 {
        std::process::ExitCode::from(1)
    } else {
        std::process::ExitCode::SUCCESS
    })
}
/// A refusal, in machine-readable form.
///
/// Same information the human output carries -- both render the same `Plan`, so
/// a divergence between them is a rendering bug rather than two behaviours.
fn refusal_json(path: &Path, plan: &Plan) -> String {
    serde_json::json!({
        "ok": false,
        "input": path.display().to_string(),
        "executable": false,
        "steps": plan.steps().len(),
        "warnings": plan.warnings().iter().map(|w| serde_json::json!({
            "blocking": w.is_blocking(),
            "message": describe(w),
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

// ---------------------------------------------------------------------------

fn cmd_inspect(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let path = args
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("inspect needs a file"))?;
    let mut f = std::fs::File::open(&path)?;
    let s = sniff(&mut f, path.file_name())?;

    println!("{}", show(&path));
    println!("  detected   {}", s.detected);
    match s.declared {
        Some(d) => println!("  extension  {d}"),
        None => println!("  extension  (none)"),
    }
    if s.mismatched() {
        println!(
            "  ! the extension says {} and the content says {}. Routing by content.",
            s.declared.map_or("nothing".into(), |d| d.to_string()),
            s.detected
        );
    }
    if s.polyglot {
        println!("  ! more than one format signature matched. This file is a polyglot.");
    }
    Ok(std::process::ExitCode::SUCCESS)
}

fn cmd_plan(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let path = args
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("plan needs a file"))?;
    let target = parse_target(args)?;

    // Through `detect()` rather than a bare open, because `plan` must answer
    // for the same bytes `convert` would run on -- including the phase-2 probe.
    // A preview computed from a different read of the file is a preview of a
    // different conversion (I13/SR-16).
    let mut table = openconvert_run::handles::HandleTable::new();
    let facts = openconvert_run::detect::detect(&path, &mut table)?;
    let s = facts.sniff();

    let policy = Policy::default();
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());
    let bytes = openconvert_run::detect::bytes_of(&facts, &mut table)?;
    let props = openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool);

    let env = probe_real();
    let plan = route(
        PlanRequest {
            input: s.detected,
            target,
            polyglot: s.polyglot,
        },
        props,
        &policy,
        &env,
    );

    print_plan(&path, s.detected, &plan);
    Ok(if plan.is_executable() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(1)
    })
}

/// A pixel budget, in a unit that does not round it away.
///
/// Integer-dividing by 1 Mpx printed **"0 Mpx"** for a 40x30 image the moment
/// phase-2 detection started narrowing the limit -- so the plan preview said a
/// conversion could decode nothing, on exactly the files where the new control
/// was working best. `03` s13 asks for messages a user can act on, and "0" is
/// one they would act on by giving up.
fn pixels(n: u64) -> String {
    const M: u64 = 1 << 20;
    const K: u64 = 1 << 10;
    if n >= M {
        format!("{} Mpx", n / M)
    } else if n >= K {
        format!("{} Kpx", n / K)
    } else {
        format!("{n} px")
    }
}

pub(crate) fn print_plan(path: &Path, detected: FormatId, plan: &Plan) {
    println!("{}  ({detected})", show(path));

    if plan.steps().is_empty() {
        println!("\n  NO PLAN. Nothing would run.");
    } else {
        println!("\n  {} step(s):", plan.steps().len());
        for (i, step) in plan.steps().iter().enumerate() {
            let where_ = match step.isolation {
                Isolation::InProcess => "in-process".to_string(),
                Isolation::Sandboxed(p) => format!("sandboxed ({})", p.strength()),
            };
            println!("    {}. {:?}", i + 1, step.kind);
            println!("       class {:?} · {where_}", step.class);
            println!(
                "       limits: {} MiB memory, {} decode, {}s wall",
                step.limits.memory_bytes / (1 << 20),
                pixels(step.limits.decode_pixels),
                step.limits.wall_time.as_secs()
            );
        }
    }

    if !plan.warnings().is_empty() {
        println!();
        for w in plan.warnings() {
            let mark = if w.is_blocking() { "BLOCKED" } else { "note" };
            println!("  {mark}: {}", describe(w));
        }
    }
}

/// Every message names what is wrong **and what to do about it**. `03` §7.5.
pub(crate) fn describe(w: &Warning) -> String {
    use openconvert_core::isolation::Refusal;
    use openconvert_core::plan::NoRoute;
    match w {
        Warning::TypeMismatch { declared, detected } => {
            format!("the name says {declared} and the content says {detected}; routing by content")
        }
        Warning::Polyglot => {
            "more than one format signature matched; this file is being quarantined".into()
        }
        Warning::Lossy { class } => {
            format!("this conversion is class {class:?} — some fidelity is lost")
        }
        Warning::AboveAutoClass { needed, allowed } => format!(
            "this route is class {needed:?} and the policy arms up to {allowed:?}. \
             Ask for the operation by name to run it."
        ),
        Warning::NoRoute(NoRoute::CrossKind) => {
            "those two formats are different kinds of thing; there is no route between them".into()
        }
        Warning::NoRoute(NoRoute::OperationInputMismatch { operation }) => format!(
            "{operation} applies only to the files it makes sense for — trim keeps a time \
             range in Matroska/WebM/MKA video, and page render turns a PDF page into png/jpeg"
        ),
        Warning::NoRoute(NoRoute::NeedsMoreMemory { needed, allowed }) => format!(
            "this needs a memory limit of at least {} GB per conversion, and it is set to {} GB. Raise it in Settings, or choose a smaller model.",
            needed >> 30,
            allowed >> 30
        ),
        Warning::NoRoute(NoRoute::UnsupportedPair) => "no route exists for that pair yet".into(),
        Warning::NoRoute(NoRoute::UnknownInput) => {
            "the content did not match any format we know, so there is nothing to route".into()
        }
        Warning::NoRoute(NoRoute::RequirementUnmet { requirement }) => {
            // Two requirements, two different problems, and one sentence used
            // to describe both. `Engine` is a missing program, which is a
            // machine problem the user can fix by reinstalling. Everything else
            // is a property of the FILE -- and "needs codecs are compatible
            // with the destination container, which is not available on this
            // machine" is both ungrammatical and wrong about whose fault it is.
            // `03` s13 asks for messages a user can act on; that one sent them
            // to check their installation over a video's codec.
            match requirement {
                Requirement::Engine(name) => format!(
                    "the best route needs the {name} engine, which is not installed. Reinstall to repair the engine set."
                ),
                Requirement::StreamsCompatible => "this file's streams cannot be copied into that container unchanged, so a lossless remux is not possible. Converting it instead would re-encode."
                    .into(),
                Requirement::AudioCompatible => "the audio codec in this file cannot move unchanged into that container, so lossless extraction is not possible.".into(),
                Requirement::Always => "the best route was refused".into(),
            }
        }
        Warning::BelowFloor(Refusal::NoNetworkDenial) => {
            "this machine offers no way to deny an engine network access, and that is not \
             something policy can waive. No conversion will run. (No confinement exists yet: \
             openconvert-os lands in phase 3.)"
                .into()
        }
        Warning::BelowFloor(Refusal::NoFilesystemConfinement) => {
            "this machine offers no filesystem confinement, and policy requires it".into()
        }
        Warning::BelowFloor(Refusal::BelowStrength {
            required,
            available,
        }) => {
            format!("policy requires {required} isolation and this machine offers {available}")
        }
    }
}

fn cmd_routes(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let table = RouteTable::v1();
    let (from, to) = match (args.first(), args.get(1)) {
        (Some(a), Some(b)) => (parse_format(a)?, parse_format(b)?),
        _ => {
            for r in table.all() {
                println!("{:>10} -> {:<10}  class {:?}", r.from, r.to, r.class);
            }
            return Ok(std::process::ExitCode::SUCCESS);
        }
    };

    let mut any = false;
    for (i, r) in table.routes_for(from, to).enumerate() {
        any = true;
        println!("{}. class {:?}", i + 1, r.class);
        println!("   steps    {:?}", r.steps);
        if r.requires.is_empty() {
            println!("   requires nothing");
        } else {
            for req in r.requires {
                println!("   requires {}", req.describe());
            }
        }
    }
    if !any {
        println!("no route from {from} to {to}");
        return Ok(std::process::ExitCode::from(1));
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// Every format this build knows, and what it can do with each.
///
/// THE ROUTE COLUMNS ARE NOT DECORATION. A reader takes this list as what the
/// product converts, and `postscript` sat in it with zero routes in either
/// direction -- detectable, then refused for everything. Printing the counts
/// makes a detect-only format say so on the line where someone would otherwise
/// read it as capability. `every_format_routes_or_is_declared_detect_only`
/// keeps that state deliberate; this makes it visible.
fn cmd_formats() -> anyhow::Result<std::process::ExitCode> {
    let table = RouteTable::v1();
    println!(
        "{:<12} {:<10} {:<8} {:>4} {:>4}  PARSER",
        "FORMAT", "KIND", "EXT", "FROM", "TO"
    );
    for f in TABLE {
        let from = table.all().iter().filter(|r| r.from == f.id).count();
        let to = table.all().iter().filter(|r| r.to == f.id).count();
        // WHAT THE FLAG MEANS, WHICH IS NOT WHAT THIS USED TO SAY. It read
        // "C library (sandboxed)", and zip, tar, gzip and 7z are all pure Rust
        // in this build and all sandboxed -- the flag decides whether a step
        // may run in our own address space, not what language wrote the
        // parser. Fonts made the wording wrong in a new way, which is what
        // prompted reading it again.
        let parser = if f.pure_rust_parser {
            "in-process (pure Rust)"
        } else {
            "a sandboxed worker"
        };
        // A format with no routes at all is detected and converted nowhere,
        // and saying that outright beats leaving two zeroes to be interpreted.
        let note = if from == 0 && to == 0 {
            "detected only; no conversions"
        } else {
            parser
        };
        println!(
            "{:<12} {:<10} {:<8} {from:>4} {to:>4}  {note}",
            f.name,
            f.kind.to_string(),
            f.extension,
        );
    }
    Ok(std::process::ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// models — the local model store
// ---------------------------------------------------------------------------

/// Inspect and manage the model store.
///
/// Downloading is always an explicit act, never a side effect of converting: a
/// model is a large file fetched over the network, and the one thing this
/// product promises is that it does nothing over the network the user did not
/// ask for. Enabling is separate again — having a model on disk is not the
/// same as consenting to run it.
fn cmd_models(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    match sub {
        "list" => {
            println!(
                "{:<22} {:>9}  {:<18} {:<12} PURPOSE",
                "MODEL", "SIZE", "LICENCE", "STATE"
            );
            for m in openconvert_run::models::list() {
                let state = if !m.downloaded {
                    "not on disk"
                } else if m.enabled {
                    "ready"
                } else {
                    "off"
                };
                println!(
                    "{:<22} {:>8.1}M  {:<18} {:<12} {}",
                    m.id,
                    m.size_bytes as f64 / 1_048_576.0,
                    m.licence,
                    state,
                    m.purpose
                );
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        "get" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("models get needs a model id"))?;
            let path = openconvert_run::models::download(id).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("downloaded and verified against its pin");
            // Through `show()` like every other path this binary prints. The
            // store path is a sha256 we chose, so nothing here is
            // attacker-shaped -- but a lint that holds only where the author
            // judged it necessary is a lint nobody can rely on.
            println!("  {}", show(&path));
            println!("enable it with: openconvert models enable {id}");
            Ok(std::process::ExitCode::SUCCESS)
        }
        "enable" | "disable" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("this needs a model id"))?;
            openconvert_run::models::set_enabled(id, sub == "enable")
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let word = if sub == "enable" {
                "enabled"
            } else {
                "disabled"
            };
            println!("{id} is now {word}");
            Ok(std::process::ExitCode::SUCCESS)
        }
        "remove" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("models remove needs a model id"))?;
            let gone = openconvert_run::models::delete(id).map_err(|e| anyhow::anyhow!("{e}"))?;
            let word = if gone {
                "removed from the store"
            } else {
                "was not on disk"
            };
            println!("{id}: {word}");
            Ok(std::process::ExitCode::SUCCESS)
        }
        other => anyhow::bail!("unknown: models {other}. Try list, get, enable, disable, remove"),
    }
}

// ---------------------------------------------------------------------------
// concat — the lossless join
// ---------------------------------------------------------------------------

/// Join Matroska-family files end to end.
///
/// The same container surgery trim uses: sample payloads move verbatim, only
/// presence and timestamps change, so the output is Class A and says so. Every
/// input is sniffed first — a polyglot or a non-Matroska file refuses the whole
/// join before anything runs.
fn cmd_concat(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    use openconvert_core::limits::Limits;
    use openconvert_core::policy::OnConflict;

    let (out_arg, inputs) = args.split_first().ok_or_else(|| {
        anyhow::anyhow!(
            "concat needs an output and at least two inputs: openconvert concat <out> <in...>"
        )
    })?;
    anyhow::ensure!(
        inputs.len() >= 2,
        "concat joins TWO OR MORE files; one file needs no joining"
    );
    let out_path = PathBuf::from(out_arg);
    let dest = out_path.parent().unwrap_or(Path::new("."));
    let name = out_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("the output name must be a plain file name"))?;

    let limits = Limits::defaults();
    let mut joined: Option<openconvert_run::matroska::AvGraph> = None;
    let mut detected_names = Vec::new();

    for path in inputs {
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("could not read {}: {e}", show(Path::new(path))))?;
        let mut cursor = std::io::Cursor::new(&bytes[..]);
        let s = openconvert_run::detect::sniff(&mut cursor, Path::new(path).file_name())?;
        if s.polyglot {
            anyhow::bail!(
                "{} matched more than one format signature; refusing it like any polyglot",
                show(Path::new(path))
            );
        }
        if !matches!(s.detected, FormatId::Mkv | FormatId::Webm | FormatId::Mka) {
            anyhow::bail!(
                "{} is a {detected}; concat joins Matroska/WebM/MKA only",
                show(Path::new(path)),
                detected = s.detected
            );
        }
        detected_names.push(s.detected.to_string());
        let graph = openconvert_run::matroska::demux(&bytes, limits.memory_bytes)
            .map_err(|e| anyhow::anyhow!("{}: {e}", show(Path::new(path))))?;
        joined = Some(match joined {
            None => graph,
            Some(acc) => openconvert_run::matroska::concat(&acc, &graph)
                .map_err(|e| anyhow::anyhow!("cannot join {}: {e}", show(Path::new(path))))?,
        });
    }

    let graph = joined.expect("at least two inputs guaranteed above");
    let out_bytes = openconvert_run::matroska::graph_to_ebml(&graph)
        .map_err(|e| anyhow::anyhow!("could not serialise the join: {e}"))?;

    // The one creation path, so nothing already on disk is ever at risk.
    let (placed, mut file) = openconvert_run::write::create_output(dest, name, OnConflict::Suffix)?;
    let output_path = match &placed {
        openconvert_run::write::Placed::Created(p) | openconvert_run::write::Placed::Skipped(p) => {
            p.clone()
        }
    };
    let Some(f) = file.as_mut() else {
        anyhow::bail!("{} exists; skipped by policy", show(&output_path));
    };
    use std::io::Write as _;
    f.write_all(&out_bytes)?;
    drop(file);

    // A receipt, because an unaccounted-for output is worse than none. Built
    // here rather than through Receipt::new because there is no Plan: concat
    // joins N inputs, which the plan types model as one operation per pair.
    // Everything the receipt owes its reader is still here: identity hash,
    // what ran, what class it was, where the bytes went.
    let mut hasher = blake3::Hasher::new();
    for path in inputs {
        let bytes = std::fs::read(path)?;
        hasher.update(&bytes);
    }
    let receipt = serde_json::json!({
        "version": 1,
        "tool": format!("openconvert {}", env!("CARGO_PKG_VERSION")),
        "content_id": hasher.finalize().to_hex().to_string(),
        "detected": detected_names.join(" + "),
        "class": "A (lossless)",
        "steps": [{
            "kind": "Concat",
            "engine": "openconvert-container",
            "isolation": "in-process (pure Rust)",
            "removed": ["timestamps renumbered across the join"],
        }],
        "output": name,
        "output_bytes": out_bytes.len(),
    });
    let receipt_name = format!("{name}.receipt.json");
    if let Ok((_, Some(mut rf))) =
        openconvert_run::write::create_output(dest, &receipt_name, OnConflict::Suffix)
    {
        rf.write_all(serde_json::to_string_pretty(&receipt)?.as_bytes())?;
        rf.write_all(b"\n")?;
    }

    println!("{}", show(&output_path));
    println!("  {} bytes - class A", out_bytes.len());
    println!("  receipt {}", show(&dest.join(receipt_name)));
    Ok(std::process::ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------

pub(crate) fn parse_format(s: &str) -> anyhow::Result<FormatId> {
    let want = s.trim_start_matches('.').to_ascii_lowercase();
    TABLE
        .iter()
        .find(|f| f.name == want || f.extension == want)
        .map(|f| f.id)
        .ok_or_else(|| anyhow::anyhow!("unknown format {s:?} — try `openconvert formats`"))
}

fn parse_target(args: &[String]) -> anyhow::Result<Target> {
    let mut it = args.iter();
    let mut to: Option<FormatId> = None;
    let mut page: Option<u32> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "-t" | "--to" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("-t needs a format"))?;
                to = Some(parse_format(v)?);
            }
            "--strip-metadata" => return Ok(Target::Operation(Operation::StripMetadata)),
            "--remux" => return Ok(Target::Operation(Operation::Remux)),
            "--denoise" => return Ok(Target::Operation(Operation::Denoise)),
            "--upscale" => {
                return Ok(Target::Operation(Operation::Upscale {
                    // PNG: enlarging then re-compressing as JPEG would spend
                    // the model's invented detail on JPEG artefacts.
                    to: openconvert_core::format::FormatId::Png,
                }));
            }
            // `=quality` and `=best` pick the heavier segmenters. Separate
            // flags would imply separate features; this is one feature with a
            // ladder of models behind it.
            "--remove-background" | "--remove-background=quality" | "--remove-background=best" => {
                return Ok(Target::Operation(Operation::RemoveBackground {
                    quality: match a.as_str() {
                        "--remove-background=best" => openconvert_core::target::Quality::Best,
                        "--remove-background=quality" => openconvert_core::target::Quality::Better,
                        _ => openconvert_core::target::Quality::Standard,
                    },
                    // PNG: the result is a matte, and a destination without an
                    // alpha channel cannot hold one.
                    to: openconvert_core::format::FormatId::Png,
                }));
            }
            "--trim" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--trim needs START-END"))?;
                let (s, e) = v.split_once('-').ok_or_else(|| {
                    anyhow::anyhow!("--trim needs START-END, e.g. --trim 0:30-1:15.5")
                })?;
                let start_ms = parse_time(s)?;
                let end_ms = parse_time(e)?;
                anyhow::ensure!(
                    end_ms > start_ms,
                    "--trim: the end must come after the start"
                );
                return Ok(Target::Operation(Operation::Trim { start_ms, end_ms }));
            }
            "--page" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--page needs a number"))?;
                page = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("--page needs a whole number, got {v:?}"))?,
                );
            }
            _ => {}
        }
    }
    if let Some(p) = page {
        let fmt =
            to.ok_or_else(|| anyhow::anyhow!("--page needs -t png or -t jpeg alongside it"))?;
        anyhow::ensure!(
            matches!(fmt, FormatId::Png | FormatId::Jpeg),
            "--page renders to png or jpeg only, got {}",
            fmt
        );
        return Ok(Target::Operation(Operation::RenderPage {
            page: p,
            to: fmt,
        }));
    }
    if let Some(f) = to {
        return Ok(Target::Format(f));
    }
    anyhow::bail!(
        "say what you want: -t <format>, --strip-metadata, --remux, --trim START-END, \
         --remove-background, --upscale, --denoise, or -t png|jpeg with --page N"
    )
}

/// A user-typed duration in `SS`, `MM:SS` or `HH:MM:SS`, with an optional
/// fractional second on the final component. Returns milliseconds.
///
/// Plain numbers are SECONDS, not milliseconds: `--trim 90` means a minute
/// and a half to everyone who was not told otherwise.
fn parse_time(s: &str) -> anyhow::Result<u64> {
    let parts: Vec<&str> = s.split(':').collect();
    anyhow::ensure!(parts.len() <= 3, "times take SS, MM:SS or HH:MM:SS");
    let mut total_secs = 0f64;
    for part in &parts {
        let (whole, frac) = match part.split_once('.') {
            Some((w, f)) if f.len() <= 3 => (w, Some(f)),
            Some((_, f)) => {
                anyhow::bail!("fractions finer than milliseconds are not accepted ({f})")
            }
            None => (*part, None),
        };
        let n: u64 = whole
            .parse()
            .map_err(|_| anyhow::anyhow!("{whole:?} is not a number of seconds"))?;
        anyhow::ensure!(
            frac.is_none() || std::ptr::eq(*part, *parts.last().unwrap()),
            "a fraction belongs on the last component only"
        );
        total_secs = total_secs * 60.0 + f64::from(u32::try_from(n)?);
        if let Some(f) = frac {
            total_secs += f.parse::<f64>().unwrap_or(0.0) / 10f64.powi(f.len() as i32);
        }
    }
    Ok((total_secs * 1000.0).round() as u64)
}

/// The output name for this run: the configured template when there is one,
/// the classic `{name}.{ext}` shape when there is not. A template that fails
/// validation fails HERE, before anything runs — never mid-batch.
fn naming_output(
    template: Option<&str>,
    stem: &str,
    ext: &str,
    _input: PathBuf,
    index: usize,
) -> anyhow::Result<String> {
    let tpl = template.unwrap_or("{name}.{ext}");
    openconvert_run::naming::validate_template(tpl)?;
    Ok(openconvert_run::naming::render(
        tpl,
        &openconvert_run::naming::RenderContext {
            name: stem,
            ext,
            date: &openconvert_run::naming::today_iso(),
            index,
        },
    )?)
}

/// The temp area `--replace-source` converts into before the verified
/// replacement. Under the state directory, so the sweep rule has exactly one
/// place to reason about.
fn job_tmp_dir() -> PathBuf {
    let dir = openconvert_run::state::paths::state_dir()
        .join("tmp")
        .join(std::process::id().to_string());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PowerShell and cmd.exe hand wildcards to a native program unexpanded,
    /// so this matcher IS `openconvert convert *.heic` on the platform we ship
    /// to first.
    #[test]
    fn wildcards_match_the_way_a_shell_would() {
        assert!(glob_match("*.heic", "photo.heic"));
        assert!(!glob_match("*.heic", "photo.heif"));
        assert!(glob_match("photo_00?.png", "photo_001.png"));
        assert!(!glob_match("photo_00?.png", "photo_0012.png"));
        assert!(glob_match("*", "anything.at.all"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        // A trailing star matches nothing at all, which is what makes `a*`
        // match `a`.
        assert!(glob_match("a*", "a"));
    }

    /// Many stars against a long non-matching name must still terminate: the
    /// matcher backtracks iteratively rather than recursing, so a pattern
    /// that came off the command line cannot exhaust the stack.
    #[test]
    fn a_pathological_pattern_still_terminates() {
        let name = "a".repeat(64);
        assert!(!glob_match("*a*a*a*a*a*a*a*a*b", &name));
    }

    /// Flag VALUES are not input files. `-t jpeg` must not send us looking
    /// for a file called `jpeg`.
    #[test]
    fn flag_values_are_not_inputs() {
        let args: Vec<String> = ["a.png", "-t", "jpeg", "--json", "b.png"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(
            collect_inputs(&args).unwrap(),
            vec![PathBuf::from("a.png"), PathBuf::from("b.png")]
        );
    }

    /// Every value-taking flag consumes its value, not just `-t`.
    #[test]
    fn every_value_taking_flag_consumes_its_value() {
        for (flag, value) in [("--page", "3"), ("--trim", "0:30-1:15"), ("--to", "png")] {
            let args: Vec<String> = ["in.pdf", flag, value]
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            assert_eq!(
                collect_inputs(&args).unwrap(),
                vec![PathBuf::from("in.pdf")],
                "{flag} leaked its value into the input list"
            );
        }
    }
}
