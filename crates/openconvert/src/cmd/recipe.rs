//! `openconvert recipe` — pin a conversion, replay it later.
//!
//! A recipe stores a **request plus its plan hash, never a plan** (I2): the
//! saved file names an input and target format, and `load` re-routes that
//! request against *this* machine right now. If today's hash differs from
//! the stored one — engines changed, isolation ladder changed — the user is
//! told before anything else happens. Loading shows a preview; it never
//! executes.

use std::path::PathBuf;

use openconvert_core::plan::PlanRequest;
use openconvert_core::policy::Policy;
use openconvert_core::route::route;
use openconvert_core::target::Target;
use openconvert_run::handles::HandleTable;
use openconvert_run::pool::WorkerPool;
use openconvert_run::state::{paths, recipe as state_recipe};

pub fn run(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    match args.first().map(String::as_str) {
        Some("save") => save_cmd(&args[1..]),
        Some("load") => load_cmd(&args[1..]),
        _ => {
            eprintln!(
                "USAGE\n  \
                 openconvert recipe save <name> <file> -t <format>\n  \
                 openconvert recipe load <name>"
            );
            Ok(std::process::ExitCode::from(2))
        }
    }
}

fn save_cmd(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("recipe save needs a name"))?;
    let file = args
        .get(1)
        .filter(|a| !a.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("recipe save needs a file to base it on"))?;

    // Recipes record a target FORMAT (that is what the struct stores); the
    // metadata-stripping operation is not expressible here yet.
    let target_format = match super::parse_target_flag(args)? {
        Some(Target::Format(f)) => f,
        Some(Target::Operation(_)) => {
            anyhow::bail!("recipes record a target format; --strip-metadata is not supported here")
        }
        None => anyhow::bail!("say what you want: -t <format>"),
    };

    // Route once through the real pipeline so the recipe can only be saved
    // for something THIS machine can actually do.
    let path = PathBuf::from(file);
    let mut table = HandleTable::new();
    let facts = openconvert_run::detect::detect(&path, &mut table)?;
    let s = facts.sniff();
    let bytes = openconvert_run::detect::bytes_of(&facts, &mut table)?;

    let policy = Policy::default();
    let env = crate::probe_real();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let props = openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool);

    let request = PlanRequest {
        input: s.detected,
        target: Target::Format(target_format),
        polyglot: s.polyglot,
    };
    let plan = route(request, props, &policy, &env);
    if !plan.is_executable() {
        anyhow::bail!(
            "this conversion would not run here: {}",
            plan.warnings()
                .iter()
                .find(|w| w.is_blocking())
                .map_or_else(|| "no executable route".to_string(), crate::describe)
        );
    }

    let recipe = state_recipe::Recipe {
        name: name.to_string(),
        created: state_recipe::today_utc(),
        plan_hash: super::plan_hash(&request),
        input_format: s.detected.to_string(),
        target_format: target_format.to_string(),
    };
    let written = state_recipe::save(&recipe)?;
    println!(
        "saved {} ({}, {} -> {})",
        crate::show(&written),
        recipe.created,
        recipe.input_format,
        recipe.target_format
    );
    Ok(std::process::ExitCode::SUCCESS)
}

fn load_cmd(args: &[String]) -> anyhow::Result<std::process::ExitCode> {
    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("recipe load needs a name"))?;

    let path = paths::recipes_dir().join(format!("{name}.recipe.toml"));
    let stored = state_recipe::load(&path).map_err(anyhow::Error::msg)?;

    let input = crate::parse_format(&stored.input_format)?;
    let output = crate::parse_format(&stored.target_format)?;
    let request = PlanRequest::new(input, Target::Format(output));
    let fresh_hash = super::plan_hash(&request);

    println!(
        "{}  ({} · {} -> {})",
        crate::show(&path),
        stored.created,
        stored.input_format,
        stored.target_format
    );

    // Re-route against THIS machine. The stored plan is never replayed.
    let policy = Policy::default();
    let plan = route(
        request,
        openconvert_core::facts::Properties::None,
        &policy,
        &crate::probe_real(),
    );
    if fresh_hash != stored.plan_hash {
        println!(
            "  ! saved under routing hash {:#x} but this machine routes it as {:#x}.",
            stored.plan_hash, fresh_hash
        );
        println!("    The preview below is TODAY's machine. Nothing has run.");
    }

    crate::print_plan(&path, input, &plan);
    Ok(if plan.is_executable() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(1)
    })
}
