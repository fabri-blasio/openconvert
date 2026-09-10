//! Prove every gate fires, by running **the real scanner** against fixtures
//! built to trip it.
//!
//! A gate nobody has watched fail is a gate nobody has tested. That is not a
//! hypothetical worry here: this project already produced a corpus test that
//! passed vacuously when its own input was deleted, and 7.4 million fuzz cases
//! failed to notice (spikes S29, S30). Thirty mutants found it in minutes.
//!
//! **Why this calls `lint::scan_dir` rather than checking the fixture's text.**
//! An earlier version of this file asserted that each fixture *contained* the
//! forbidden string. That tests the fixture, not the gate — if the scanner's
//! comment-stripping had a bug it would pass everything while these checks
//! stayed green. Check the control: point the actual gate at the actual bad
//! input and require it to object.
//!
//! Fixtures live outside the workspace (`exclude` in the root manifest), so an
//! ordinary `cargo build` never compiles code that is meant to be wrong.

use crate::desktop::{self, GATES};
use crate::lint::{self, SCANS};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

struct Fixture {
    /// The gate this file must trip, by `Scan::name`.
    gate: &'static str,
    /// File under `xtask/fixtures/lint/`.
    file: &'static str,
}

/// The same contract for the webview-boundary gates, which read JSON, TOML and
/// Svelte rather than Rust and so cannot share `lint`'s scanner.
///
/// Two of these fixtures trip gates whose real subject does not exist yet
/// (`src-tauri/Cargo.toml`, `apps/desktop/src/`). That is the point of pointing
/// the checker at a fixture: a gate waiting for its file reports "ok" against
/// the tree forever, and only a fixture can tell "correctly silent" from
/// "dead".
struct DesktopFixture {
    /// The gate this file must trip, by `Gate::name`.
    gate: &'static str,
    /// File under `xtask/fixtures/desktop/`.
    file: &'static str,
}

const DESKTOP_FIXTURES: &[DesktopFixture] = &[
    DesktopFixture {
        gate: "no forbidden Tauri plugin (node)",
        file: "plugin_shell.json",
    },
    DesktopFixture {
        gate: "no forbidden Tauri plugin (rust)",
        file: "plugin_shell_cargo.toml",
    },
    DesktopFixture {
        gate: "the capability set is deny-by-default",
        file: "capability_broad.json",
    },
    // A second fixture per gate for the defects a *presence* check waves
    // through: a capability that is scoped, but scoped by glob to everything;
    // and a CSP that is clean except for a host beginning with a permitted
    // origin. Both passed the first version of their checker.
    DesktopFixture {
        gate: "the capability set is deny-by-default",
        file: "capability_wildcard.json",
    },
    DesktopFixture {
        gate: "the webview CSP is strict",
        file: "csp_remote.json",
    },
    DesktopFixture {
        gate: "the webview CSP is strict",
        file: "csp_lookalike.json",
    },
    DesktopFixture {
        gate: "no raw HTML in Svelte components",
        file: "raw_html.svelte",
    },
    // `apply_saved_geometry` as it was written: read two numbers, call
    // `set_position`, and trust the OS to refuse a position that no longer
    // names a pixel. It does not refuse, and the window opened onto a monitor
    // nobody had. The fixture is the real code, so the gate is proven against
    // the defect rather than against a hand-made trigger.
    DesktopFixture {
        gate: "restored window geometry is validated",
        file: "geometry_unchecked.rs",
    },
];

/// One fixture per gate, plus a second for the destructive-API scan because
/// its two sharpest cases fail in different ways: `set_len(0)` truncates
/// through a handle that was never opened for truncation, and `fs::rename`
/// is the one a well-meaning contributor reaches for.
const FIXTURES: &[Fixture] = &[
    Fixture {
        gate: "heavy inference falls back at run time",
        file: "session_for_in_an_adapter.rs",
    },
    Fixture {
        gate: "no destructive filesystem API",
        file: "destructive_set_len.rs",
    },
    Fixture {
        gate: "no destructive filesystem API",
        file: "destructive_rename.rs",
    },
    Fixture {
        gate: "no destructive filesystem API",
        file: "destructive_symlink.rs",
    },
    Fixture {
        gate: "no process creation outside openconvert-os",
        file: "spawn_outside_os.rs",
    },
    Fixture {
        gate: "no untrusted path printed raw",
        file: "raw_path_print.rs",
    },
    Fixture {
        gate: "Deserialize only in wire.rs",
        file: "derive_deserialize.rs",
    },
    Fixture {
        gate: "the purity gate is not bypassed",
        file: "allow_disallowed.rs",
    },
    Fixture {
        gate: "unsafe outside openconvert-os",
        file: "unsafe_outside_os.rs",
    },
    Fixture {
        gate: "exactly one outbound network call site",
        file: "outbound_call.rs",
    },
];

/// The engine-manifest gates, which read TOML rather than source and so share
/// neither of the scanners above.
///
/// One fixture per rule, and the GPL one is **dynamically linked on purpose**:
/// a gate that only refused static GPL would pass it, and the whole point is
/// that no link mode discharges the GPL.
struct EngineFixture {
    /// A substring the violation message must contain.
    expect: &'static str,
    /// File under `xtask/fixtures/engines/`.
    file: &'static str,
}

const ENGINE_FIXTURES: &[EngineFixture] = &[
    EngineFixture {
        expect: "GPL and AGPL are refused",
        file: "gpl_engine.toml",
    },
    EngineFixture {
        expect: "must be dynamic",
        file: "static_lgpl.toml",
    },
    EngineFixture {
        expect: "link_mode",
        file: "typo_link_mode.toml",
    },
];

pub fn run() -> Result<()> {
    let root = workspace_root();
    let dir = root.join("xtask").join("fixtures").join("lint");
    let staging = root.join("target").join("gate-verify");

    let mut problems = Vec::new();
    let mut covered: Vec<&str> = Vec::new();

    for f in FIXTURES {
        let scan = SCANS
            .iter()
            .find(|s| s.name == f.gate)
            .with_context(|| format!("fixture {} names an unknown gate {:?}", f.file, f.gate))?;

        let src = dir.join(f.file);
        if !src.exists() {
            problems.push(format!("  {} — fixture missing (gate: {})", f.file, f.gate));
            continue;
        }

        // Isolate the fixture so the scan sees exactly one file. Otherwise a
        // hit from a neighbouring fixture would let a dead gate look alive.
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        std::fs::copy(&src, staging.join(f.file))?;

        match lint::scan_dir(scan, &staging)?.len() {
            0 => problems.push(format!(
                "  {} — the gate {:?} did NOT fire on it.\n     \
                 Either the fixture stopped being a violation, or the gate is dead.",
                f.file, f.gate
            )),
            n => {
                println!("verify: {:<44} fires on {} ({n} hit(s))", f.gate, f.file);
                covered.push(f.gate);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&staging);

    // The webview gates, same contract, different readers.
    let desktop_dir = root.join("xtask").join("fixtures").join("desktop");
    for f in DESKTOP_FIXTURES {
        let gate = GATES
            .iter()
            .find(|g| g.name == f.gate)
            .with_context(|| format!("fixture {} names an unknown gate {:?}", f.file, f.gate))?;

        let src = desktop_dir.join(f.file);
        if !src.exists() {
            problems.push(format!("  {} — fixture missing (gate: {})", f.file, f.gate));
            continue;
        }

        // Point the checker at the fixture file itself. `check_path` treats a
        // file as a one-element set, so no staging directory is needed and no
        // neighbouring fixture can make a dead gate look alive.
        match desktop::check_path(gate, &src)?.len() {
            0 => problems.push(format!(
                "  {} — the gate {:?} did NOT fire on it.\n     \
                 Either the fixture stopped being a violation, or the gate is dead.",
                f.file, f.gate
            )),
            n => {
                println!("verify: {:<44} fires on {} ({n} hit(s))", f.gate, f.file);
                covered.push(f.gate);
            }
        }
    }

    // The engine manifest gates, same contract, a third reader.
    let engine_dir = root.join("xtask").join("fixtures").join("engines");
    for f in ENGINE_FIXTURES {
        let src = engine_dir.join(f.file);
        if !src.exists() {
            problems.push(format!("  {} - fixture missing", f.file));
            continue;
        }
        // The REAL checker, pointed at the fixture. Asserting the file merely
        // contains "GPL" would test the fixture and not the gate.
        let hits = crate::engines::check_path(&src)?;
        if hits.iter().any(|h| h.contains(f.expect)) {
            println!(
                "verify: {:<44} fires on {} ({} hit(s))",
                f.expect,
                f.file,
                hits.len()
            );
        } else {
            problems.push(format!(
                "  {} - the engine gate did NOT report {:?}.
                      Either the fixture stopped being a violation, or the rule is dead.
                      It reported: {:?}",
                f.file, f.expect, hits
            ));
        }
    }

    // Every gate needs at least one fixture. A gate added without one is a
    // gate whose failure path has never executed.
    for scan in SCANS {
        if !covered.contains(&scan.name) {
            problems.push(format!(
                "  gate {:?} has no negative fixture — its failure path has never run",
                scan.name
            ));
        }
    }
    for gate in GATES {
        if !covered.contains(&gate.name) {
            problems.push(format!(
                "  gate {:?} has no negative fixture — its failure path has never run",
                gate.name
            ));
        }
    }

    if problems.is_empty() {
        println!(
            "\nverify-gates: {} fixtures, {} gates, every failure path exercised",
            FIXTURES.len() + DESKTOP_FIXTURES.len() + ENGINE_FIXTURES.len(),
            SCANS.len() + GATES.len() + ENGINE_FIXTURES.len()
        );
        return Ok(());
    }
    bail!("verify-gates FAILED\n\n{}", problems.join("\n"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}
