//! Build script for the desktop shell.
//!
//! # The shell is useless without the engine set beside it
//!
//! `EngineBin` resolves every worker **beside our own executable, never on
//! PATH** — that is deliberate, and `worker_client` explains why: a PATH lookup
//! is the environment choosing our engine for us. The engine crates' own build
//! scripts already honour it, copying their DLLs into `target/<profile>/` next
//! to the workers cargo puts there.
//!
//! The shell is a SEPARATE workspace, so it builds into its own
//! `apps/desktop/src-tauri/target/<profile>/` — and nothing was putting
//! anything there. The app had no `oc-images.exe`, no `oc-pdf.exe`, no
//! `onnxruntime.dll`, and `tauri.conf.json` declared neither `resources` nor
//! `externalBin`. Every sandboxed conversion would have failed with "not
//! installed", and every model-backed tool would have reported itself
//! unavailable — in a UI that otherwise looked complete.
//!
//! So this copies the engine set across. It does not BUILD it: that is
//! `cargo build --workspace` at the repo root, and a shell build that silently
//! triggered one would hide how the two fit together. What it does instead is
//! say so, loudly, when the set is missing.

use std::path::{Path, PathBuf};

/// Worker binaries the shell spawns. Kept in step with `EngineBin::ALL`.
const WORKERS: &[&str] = &["oc-images", "oc-pdf", "oc-archive", "oc-audio", "oc-ai"];

fn main() {
    // BEFORE `tauri_build::build()`, and the order is load-bearing.
    //
    // `tauri_build` validates `bundle.resources` as it runs, and `engines/*`
    // matches nothing until this has put something there -- so with the calls
    // the other way round, a clean checkout failed with "glob pattern engines/*
    // path not found", which says nothing about the actual cause. Staged first,
    // the glob is satisfied and `stage_engines`'s own warning is what a
    // developer sees when the engine set genuinely has not been built.
    stage_engines();
    tauri_build::build();
}

/// Copy the engine set beside the shell, and into the directory the bundler
/// packages from.
///
/// Runs on every platform. The guard here used to be `cfg!(windows)`, so a
/// Linux or macOS `cargo run` produced a shell with no workers beside it and
/// every conversion needing one refused -- on the two platforms where nobody
/// had run it yet. `EXE_SUFFIX` already made the copy itself portable; only the
/// guard was not.
fn stage_engines() {
    let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(dest) = profile_dir(&out) else {
        return;
    };
    // The repo root is four levels above this crate: apps/desktop/src-tauri.
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) else {
        return;
    };
    // Debug artefacts pair with a debug shell, release with release: mixing
    // them is how a stale worker survives a rebuild of everything else.
    let profile = if dest.ends_with("release") {
        "release"
    } else {
        "debug"
    };
    // `CARGO_TARGET_DIR` when the environment sets one, which every
    // containerised and shared-cache build does. Reading only `root/target`
    // meant the engine set was "not built" in exactly those builds -- the
    // warning fired, nothing was staged, and on a bundle run the resource glob
    // then failed with a message about a missing directory rather than about a
    // missing engine set.
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    // `tauri build --target <triple>` puts the matching root-workspace workers
    // under `target/<triple>/<profile>`. Native builds without an explicit
    // target use `target/<profile>`, so prefer the target-specific directory
    // when it exists and retain the native path as the fallback.
    let target_src = std::env::var_os("TARGET")
        .map(PathBuf::from)
        .map(|target| target_root.join(target).join(profile));
    let src = target_src
        .filter(|candidate| candidate.is_dir())
        .unwrap_or_else(|| target_root.join(profile));
    println!("cargo:rerun-if-changed={}", src.display());

    if !src.is_dir() {
        println!(
            "cargo:warning=the engine set has not been built. Run `cargo build --workspace` \
             at the repo root; without it this app converts nothing that needs a worker."
        );
        return;
    }

    // TWO destinations, and the second is what makes an INSTALL work.
    //
    // `dest` is beside the shell's own binary, which is what `cargo run` and
    // `cargo tauri dev` execute from. That was the only destination, and it is
    // why a development build converted files while every installer shipped a
    // binary with no engines at all: `tauri.conf.json` can only declare
    // resources by a path relative to itself, and `target/<profile>/` is
    // neither stable (it moves with CARGO_TARGET_DIR) nor inside this crate.
    //
    // So the set is also staged into `src-tauri/engines/`, which `bundle.resources`
    // declares as `engines/*`. Bundlers preserve that relative path, so the
    // files arrive at `<resources>/engines/` on every platform and
    // `openconvert_run::engine_dir` knows to look there.
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("engines");
    if let Err(e) = std::fs::create_dir_all(&staged) {
        println!("cargo:warning=could not create {}: {e}", staged.display());
    }

    let mut copied = 0usize;
    let mut missing: Vec<String> = Vec::new();
    for w in WORKERS {
        let name = format!("{w}{}", std::env::consts::EXE_SUFFIX);
        let from = src.join(&name);
        if copy_if_newer(&from, &dest.join(&name)) {
            copied += 1;
            copy_if_newer(&from, &staged.join(&name));
        } else {
            missing.push((*w).to_string());
        }
    }

    // Every native library the engine crates staged, whatever they are. Naming
    // them here would mean two lists to keep in step, and the one that drifts
    // is always the copy. Three extensions because the three platforms spell
    // "shared library" differently and the engines link dynamically on all of
    // them (`docs/spec/TARGETS.md`, engine linkage).
    const LIB_EXTENSIONS: [&str; 3] = ["dll", "so", "dylib"];
    if let Ok(entries) = std::fs::read_dir(&src) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| {
                LIB_EXTENSIONS
                    .iter()
                    .any(|want| x.eq_ignore_ascii_case(want))
            }) {
                // Our OWN cdylib outputs are not engines. `openconvert_wasm.dll`
                // is the browser build's artifact and sat in the same directory,
                // so a sweep by extension shipped it into the installer -- a
                // file the app never loads, in a package whose whole claim is
                // that you can see what it contains.
                let ours = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("openconvert"));
                if let Some(name) = p.file_name().filter(|_| !ours) {
                    copy_if_newer(&p, &dest.join(name));
                    copy_if_newer(&p, &staged.join(name));
                }
            }
        }
    }

    if !missing.is_empty() {
        println!(
            "cargo:warning=missing workers ({}). Run `cargo build --workspace` at the repo \
             root; conversions needing them will refuse by name until you do.",
            missing.join(", ")
        );
    }
    println!(
        "cargo:warning=staged {copied} of {} workers (beside the binary and in {})",
        WORKERS.len(),
        staged.display()
    );
}

/// `target/<profile>/` from a build script's `OUT_DIR`.
///
/// `OUT_DIR` is `.../target/<profile>/build/<pkg>-<hash>/out`; the directory
/// holding `deps/` is the one binaries land in.
fn profile_dir(out: &Path) -> Option<PathBuf> {
    let mut dir = out;
    for _ in 0..6 {
        dir = dir.parent()?;
        if dir.join("deps").is_dir() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Keep staging aligned with the selected profile, even after a newer debug build.
fn copy_if_newer(from: &Path, to: &Path) -> bool {
    if !from.is_file() {
        return false;
    }
    let stale = match (modified(from), modified(to)) {
        (Some(a), Some(b)) => a != b,
        _ => true,
    };
    if stale {
        // Report failed staging instead of counting the previous binary as copied.
        return std::fs::copy(from, to).is_ok();
    }
    true
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}
