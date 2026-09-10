//! Build script: copy pdfium beside the worker binary on Windows.
//!
//! Same trap as `oc-images/build.rs` (`03` §5.3): shared libraries are loaded
//! INSIDE the sandbox under the container's identity, and a worker whose
//! engines are not readable fails BEFORE `main` with a loader error naming a
//! missing dependency rather than a permission. Copying `pdfium.dll` next to
//! the worker at build time puts it in the same directory the host already
//! grants execute on — one mechanism, one ACL, no installer step.
//!
//! The pdfium tree is located via `PDFIUM_DIR` or `%TEMP%\pdfium`. A missing
//! library is NOT an error at build time in the same spirit as oc-images —
//! but unlike an image format, this worker has exactly one engine and no
//! fallback, so a missing DLL means every conversion answers `Failed`
//! honestly instead of pretending.

use std::path::PathBuf;

fn main() {
    if !cfg!(windows) {
        return;
    }
    println!("cargo:rerun-if-env-changed=PDFIUM_DIR");

    // WHERE PDFIUM COMES FROM, in preference order:
    //
    //   1. `.native/pdfium`, staged by `cargo xtask deps` -- pinned by sha256,
    //      repo-local, and survives `cargo clean`. This is the supported path.
    //   2. `PDFIUM_DIR`, for a developer pointing at their own build.
    //   3. `%TEMP%\pdfium`, which is where this used to look and only there.
    //
    // (3) is kept so an existing tree keeps working, and is last because a
    // stale copy in %TEMP% silently winning over a pinned one is exactly how
    // this build came to link a pdfium whose version NOTICE did not name.
    let repo_native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|r| r.join(".native").join("pdfium"))
        .filter(|d| d.join("bin").join("pdfium.dll").is_file());
    let root = repo_native.unwrap_or_else(|| {
        std::env::var("PDFIUM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("TEMP")
                    .map(|t| PathBuf::from(t).join("pdfium"))
                    .unwrap_or_else(|_| PathBuf::from("pdfium"))
            })
    });

    let bin = root.join("bin");
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    // OUT_DIR is target/.../build/oc-pdf-<hash>/; the exe lives at
    // target/<profile>/. Walk up until we find the dir containing deps/.
    let mut exe_dir = out_dir.as_path();
    for _ in 0..6 {
        match exe_dir.parent() {
            Some(p) => exe_dir = p,
            None => break,
        }
        if exe_dir.join("deps").is_dir() {
            break;
        }
    }

    let dll = "pdfium.dll";
    let src = bin.join(dll);
    // Track the SOURCE library, not just our own inputs: swapping the DLL
    // under %TEMP% otherwise leaves a stale copy beside the binary, and the
    // failure mode (a worker answering Ready from an old engine) is worse
    // than a stale-looking build.
    println!("cargo:rerun-if-changed={}", src.display());
    // Also beside the TEST binaries. Cargo builds integration tests into
    // `target/<profile>/deps/`, so a test process asking "is this DLL where
    // the loader will find it?" is asking about `deps/`, not about the
    // profile directory -- and the availability check in
    // `openconvert-run::engines` reads exactly that answer. oc-images has
    // copied to both since it landed; not doing so here made the same engine
    // report available to the CLI and unavailable to its own tests.
    let deps_dir = exe_dir.join("deps");
    let deps_dir = deps_dir.is_dir().then_some(deps_dir);
    if src.exists() {
        let _ = std::fs::copy(&src, exe_dir.join(dll));
        if let Some(deps) = &deps_dir {
            let _ = std::fs::copy(&src, deps.join(dll));
        }
    } else {
        println!(
            "cargo:warning=oc-pdf: {dll} not found under {} -- PDF rendering will fail at run time",
            bin.display()
        );
    }

    // Link against the import library; at RUNTIME the loader finds
    // pdfium.dll beside the executable (copied above), which keeps the BSD-
    // styled obligation honest: replaceable shared library, named in NOTICE.
    let lib = root.join("lib");
    if lib.join("pdfium.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=pdfium");
    }
}
