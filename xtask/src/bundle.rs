//! **What is about to be packaged is what was built in release.**
//!
//! # The defect this exists for
//!
//! An installer shipped with **debug** engine binaries. `oc-images.exe` went in
//! at 25.7 MB where the release build is 7.2 MB, and four of the five workers
//! were byte-identical to `target/debug` while the fifth was the release one —
//! a mixed set, which the desktop `build.rs` comment already warns is "how a
//! stale worker survives a rebuild of everything else".
//!
//! The route in has nothing to do with anyone being careless:
//!
//! 1. A release build stages the release workers into `src-tauri/engines/`.
//! 2. `cargo xtask ci` compiles the desktop shell — in **debug** — and that
//!    crate's `build.rs` re-stages the *debug* workers over them.
//! 3. `cargo tauri build` runs. Its `build.rs` declares
//!    `rerun-if-changed=target/release`, nothing in there has changed since
//!    step 1, so cargo does not re-run the build script at all.
//! 4. The bundler copies whatever is sitting in `engines/`.
//!
//! Every step is behaving correctly and the result is wrong. Running the test
//! suite before packaging — the obvious, careful thing to do — is what poisons
//! the bundle.
//!
//! # Why this is not part of `xtask ci`
//!
//! Because during CI the staging directory is *supposed* to hold debug
//! binaries: the desktop step compiles in debug and pairs with them. A gate
//! that failed on that would fail the ordinary case.
//!
//! The invariant is about packaging, not about the tree at rest, so it belongs
//! to the packaging step — and it runs **after** the bundle, not before:
//!
//! ```text
//! cargo build --release --workspace      # builds the workers
//! npm run tauri build --prefix apps/desktop
//! cargo run -p xtask -- bundle-check     # what did it actually ship?
//! ```
//!
//! # Why after, which is not where it was first put
//!
//! `openconvert-desktop` is a **separate cargo workspace** — its manifest
//! carries its own `[workspace]` table, deliberately, so a `cargo build` at the
//! repo root does not drag a webview toolchain into every library build. The
//! consequence here is that the root release build never runs the desktop
//! crate's `build.rs`, and `build.rs` is what stages the workers.
//!
//! So before `tauri build` there is nothing to check: clearing the staging
//! directory and running the root build leaves it empty, and this gate
//! correctly reported that "the installer would ship no workers". The staging
//! happens *during* the bundle, which is the only moment the answer exists.
//!
//! That is worth knowing rather than working around. A check placed one step
//! too early would have passed on the previous build's leftovers — which is
//! exactly the failure it exists to catch.
//!
//! When it fails, clear the staging directory and bundle again; the build
//! script re-stages because the files it writes are gone:
//!
//! ```text
//! rm apps/desktop/src-tauri/engines/*.exe
//! npm run tauri build --prefix apps/desktop
//! ```

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Compare what is staged against what the release build produced.
pub fn run() -> Result<()> {
    let root = workspace_root();
    let staged_dir = root.join("apps/desktop/src-tauri/engines");
    let release = root.join("target/release");
    let debug = root.join("target/debug");

    let Ok(entries) = std::fs::read_dir(&staged_dir) else {
        bail!(
            "bundle-check FAILED -- nothing is staged at {}.\n\
             Run `cargo build --release --workspace` first.",
            staged_dir.display()
        );
    };

    let mut problems = Vec::new();
    let mut matched = 0_usize;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("exe") {
            // Third-party DLLs have no profile of their own.
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let want = release.join(name);
        if !want.is_file() {
            problems.push(format!(
                "  {name} is staged and there is no release build of it at {}",
                want.display()
            ));
            continue;
        }
        // SIZE, NOT A HASH. A debug binary of the same worker differs from its
        // release build by megabytes, every time, because of the symbols; the
        // comparison does not need to be cryptographic to be conclusive, and a
        // hash of a 25 MB file on every package is a cost for no more
        // certainty.
        let staged_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let release_len = std::fs::metadata(&want).map(|m| m.len()).unwrap_or(0);
        if staged_len == release_len {
            matched += 1;
            continue;
        }
        let debug_len = std::fs::metadata(debug.join(name)).map(|m| m.len()).ok();
        let which = if debug_len == Some(staged_len) {
            " -- it is the DEBUG build"
        } else {
            ""
        };
        problems.push(format!(
            "  {name} staged at {staged_len} bytes, release build is {release_len}{which}"
        ));
    }

    if !problems.is_empty() {
        bail!(
            "bundle-check FAILED -- the staged engines are not the release ones\n\n{}\n\n\
             The installer bundles whatever is in `engines/`, and `build.rs` only\n\
             re-stages when `target/release` has changed -- so a debug compile of the\n\
             shell (which `xtask ci` does) can leave debug workers here and the\n\
             bundler will ship them.\n\n\
             Fix: rm apps/desktop/src-tauri/engines/*.exe \\\n\
             \x20    && touch apps/desktop/src-tauri/build.rs \\\n\
             \x20    && cargo build --release --workspace",
            problems.join("\n")
        );
    }
    if matched == 0 {
        bail!(
            "bundle-check FAILED -- no staged engine was checked at all; the staging \
               directory holds no .exe, and the installer would ship no workers"
        );
    }
    println!("bundle-check: ok -- {matched} staged engines are the release builds");
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}
