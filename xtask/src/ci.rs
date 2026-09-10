//! `cargo xtask ci` — run what CI runs, here, in order.
//!
//! # Why this exists
//!
//! The workflow was committed and then went unrun for two phases of work. Not
//! from carelessness: there was no single command that executed it, so
//! "verifying CI" meant reading YAML and hoping. When the steps were finally
//! executed by hand, **`cargo deny check` failed on the first try** — on our
//! own crates, for a reason that would also have blocked `cargo publish`.
//!
//! A pipeline nobody can run locally is a pipeline nobody runs locally. This is
//! the command that makes the excuse unavailable.
//!
//! What it cannot do is the part that genuinely needs GitHub: the three-OS
//! matrix. That is stated at the end of every run rather than left implied.

use anyhow::{bail, Result};
use std::process::Command;

/// One step, named exactly as the workflow names it.
struct Step {
    /// The `name:` from `ci.yml`.
    name: &'static str,
    /// Program and arguments.
    argv: &'static [&'static str],
    /// Needs a tool that is not part of a stock Rust install.
    needs: Option<&'static str>,
}

const STEPS: &[Step] = &[
    Step {
        name: "manifest",
        argv: &["run", "-q", "-p", "xtask", "--", "manifest"],
        needs: None,
    },
    Step {
        name: "fmt",
        argv: &["fmt", "--all", "--check"],
        needs: None,
    },
    Step {
        name: "clippy",
        argv: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        needs: None,
    },
    Step {
        name: "test",
        argv: &["test", "--workspace"],
        needs: None,
    },
    // THE DESKTOP SHELL IS A SEPARATE WORKSPACE, so `--workspace` above does
    // not reach it. That is deliberate -- keeping the webview toolchain out of
    // every library build -- and it meant the shell could stop compiling
    // without a single gate noticing. It did: a new `StepKind` variant landed
    // in the core and the GUI's exhaustive match went stale while eleven steps
    // reported green.
    //
    // `check` rather than `build`: this catches the type errors that a change
    // in the library crates causes, which is the failure mode, without
    // dragging a full webview link into every CI run.
    Step {
        name: "desktop",
        argv: &[
            "check",
            "--manifest-path",
            "apps/desktop/src-tauri/Cargo.toml",
            "--all-targets",
        ],
        needs: None,
    },
    Step {
        name: "depcheck",
        argv: &["run", "-q", "-p", "xtask", "--", "depcheck"],
        needs: None,
    },
    // The NATIVE licence surface. `deny.toml` covers Rust crates and cannot see
    // a .lib; this reads engines.toml, refuses GPL outright, and enforces the
    // dynamic-linking rule that file had described and nobody had implemented.
    Step {
        name: "engines",
        argv: &["run", "-q", "-p", "xtask", "--", "engines"],
        needs: None,
    },
    Step {
        name: "lint",
        argv: &["run", "-q", "-p", "xtask", "--", "lint"],
        needs: None,
    },
    Step {
        name: "verify-gates",
        argv: &["run", "-q", "-p", "xtask", "--", "verify-gates"],
        needs: None,
    },
    // The fuzz targets live in their own workspace (cargo-fuzz needs nightly
    // flags the main tree must not inherit), so `cargo build` never touches
    // them. That is how `route.rs` came to reference a struct field that had
    // been added days earlier: it had not been compiled since. Building them
    // here costs seconds and stops them rotting.
    Step {
        name: "fuzz targets build",
        argv: &["check", "--manifest-path", "fuzz/Cargo.toml"],
        needs: None,
    },
    Step {
        name: "cargo-audit",
        argv: &["audit", "--target-arch", "x86_64", "--target-os", "linux"],
        needs: Some("cargo-audit"),
    },
    Step {
        name: "cargo-deny",
        argv: &["deny", "check", "licenses", "bans", "sources"],
        needs: Some("cargo-deny"),
    },
];

pub fn run() -> Result<()> {
    let mut failed = Vec::new();
    let mut skipped = Vec::new();

    for step in STEPS {
        if let Some(tool) = step.needs {
            if !installed(tool) {
                // Skipped, and **counted**. A run that silently omits a step is
                // exactly the false green this whole command exists to prevent.
                skipped.push(format!(
                    "  {:<14} needs `cargo install {tool} --locked`",
                    step.name
                ));
                continue;
            }
        }

        println!("\n\x1b[1m── {}\x1b[0m", step.name);
        let ok = Command::new(env!("CARGO"))
            .args(step.argv)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            println!("   \x1b[32mok\x1b[0m");
        } else {
            println!("   \x1b[31mFAILED\x1b[0m");
            failed.push(step.name);
        }
    }

    println!("\n{}", "─".repeat(60));
    if !skipped.is_empty() {
        println!("\x1b[33mSKIPPED — these did NOT pass, they did not run:\x1b[0m");
        for s in &skipped {
            println!("{s}");
        }
        println!();
    }

    // Always said out loud. The matrix is the one thing a local run cannot
    // substitute for, and Windows and Linux were measured to be safe in
    // *opposite* places -- so a green run on one platform is not evidence about
    // the other.
    println!(
        "Not covered here: the three-OS matrix (ubuntu, windows, macos).\n\
         This machine is {}. Only a real Actions run covers the other two.",
        std::env::consts::OS
    );

    if failed.is_empty() {
        if skipped.is_empty() {
            println!("\n\x1b[32mAll {} steps passed.\x1b[0m", STEPS.len());
        } else {
            println!(
                "\n\x1b[33m{} of {} steps passed; {} skipped.\x1b[0m",
                STEPS.len() - skipped.len(),
                STEPS.len(),
                skipped.len()
            );
        }
        return Ok(());
    }
    bail!("{} step(s) failed: {}", failed.len(), failed.join(", "))
}

fn installed(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
