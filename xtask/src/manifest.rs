//! Every file the build references must exist.
//!
//! **This gate exists because I broke the rule it enforces.** The CI workflow
//! was committed referencing `deny.toml`, `engines.toml` and `models.toml`,
//! none of which existed — so the first push would have failed at
//! `cargo deny check`, on a step whose subject had never been created.
//!
//! That is the same defect class the design record documents three times over:
//! a gate scheduled at a week its subject does not exist. `08` §4 carries a
//! standing note telling the next person to check for it. The note was correct
//! and I did not apply it to my own file, which is the argument for making the
//! check mechanical rather than a reminder.
//!
//! So: parse the workflow, extract every path it names, and require each one.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Files the build and the documented process depend on.
///
/// Kept as an explicit list rather than inferred, so that adding one is a
/// deliberate act with a reason attached.
const REQUIRED: &[(&str, &str)] = &[
    ("Cargo.toml", "workspace root"),
    (
        "rust-toolchain.toml",
        "toolchain components; MSRV lives in Cargo.toml",
    ),
    (
        "clippy.toml",
        "the purity gate for openconvert-core (03 §4)",
    ),
    ("deny.toml", "licence surface 1 of 3: Rust crates (09 §9)"),
    (
        "engines.toml",
        "licence surface 2 of 3: native engines + link_mode (D19)",
    ),
    (
        "models.toml",
        "licence surface 3 of 3: model weights (SR-8)",
    ),
    ("LICENSE", "Apache-2.0, promised in README"),
    ("NOTICE", "attribution, generated in part from engines.toml"),
    ("SECURITY.md", "SR-12's disclosure process"),
    (
        ".well-known/security.txt",
        "machine-readable pointer to SECURITY.md",
    ),
    (
        "docs/spec/TARGETS.md",
        "the week-0 target matrix decision (08 §1)",
    ),
    (
        "CHANGELOG.md",
        "what was built per phase, maintained per commit rather than reconstructed",
    ),
    (".github/workflows/ci.yml", "the gates themselves"),
    // The webview boundary. These three land before any UI code, because a
    // boundary added after the app works is a boundary negotiated against a
    // deadline -- and because `08` §6's rule applies to them: anything not in
    // a table is not designed yet.
    (
        "apps/desktop/src-tauri/capabilities/main.json",
        "deny-by-default IPC ACL; I7 is only Forbidden across IPC without it",
    ),
    (
        "apps/desktop/src-tauri/tauri.conf.json",
        "the strict CSP SR-18 requires of the packaged app",
    ),
    (
        "apps/desktop/package.json",
        "the dependency list the plugin gate reads",
    ),
    // The brand assets `tauri.conf.json` names. A missing one here does not
    // fail a `cargo build`; it fails a `tauri build`, at the end, on a machine
    // that has already spent ten minutes compiling. Cheaper to say so now.
    (
        "apps/desktop/src-tauri/icons/icon.ico",
        "the app icon and the NSIS installerIcon, generated from brand/logo",
    ),
    (
        "apps/desktop/src-tauri/installer/header.bmp",
        "the NSIS header strip; referenced by tauri.conf.json",
    ),
    (
        "apps/desktop/src-tauri/installer/sidebar.bmp",
        "the NSIS welcome panel, which is the app's preview before install",
    ),
];

pub fn run() -> Result<()> {
    let root = workspace_root();
    let mut missing = Vec::new();

    for (path, why) in REQUIRED {
        if !root.join(path).exists() {
            missing.push(format!("  {path:<28} — {why}"));
        }
    }

    // Then the reverse direction: anything the workflow names must be present.
    // The explicit list above is only as good as someone's memory of updating
    // it; this half catches a step added to CI that references a new file.
    let wf = root.join(".github/workflows/ci.yml");
    if wf.exists() {
        let src = std::fs::read_to_string(&wf)?;
        for token in src.split_whitespace() {
            let cand = token.trim_matches(|c: char| !c.is_ascii_graphic() || c == '"' || c == '\'');
            let looks_like_file = cand.ends_with(".toml") || cand.ends_with(".md");
            let is_flag = cand.starts_with('-');
            if looks_like_file && !is_flag && !root.join(cand).exists() {
                missing.push(format!(
                    "  {cand:<28} — referenced by .github/workflows/ci.yml and absent"
                ));
            }
        }
    }

    missing.sort();
    missing.dedup();

    if missing.is_empty() {
        println!("manifest: ok — {} required files present", REQUIRED.len());
        return Ok(());
    }
    bail!(
        "manifest FAILED — the build references files that do not exist.\n\n{}\n\n\
         A step whose subject does not exist is the defect 08 §4 warns about, and it\n\
         fails on the first push rather than the first review.",
        missing.join("\n")
    )
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}
