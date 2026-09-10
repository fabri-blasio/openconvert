//! Build script: copy the dynamically-linked native audio encoders beside
//! the worker binary on Windows, and emit their import-library linkage.
//!
//! # Why this exists
//!
//! Same shape as `oc-images` (`03` §5.3): shared libraries load INSIDE the
//! sandbox under the container's identity, so `libmp3lame.dll` / `opus.dll`
//! are copied next to the worker at build time — one mechanism, one ACL,
//! no installer step.
//!
//! The vcpkg tree is located via `VCPKG_ROOT` or `%USERPROFILE%\vcpkg`.
//! Missing libraries are NOT an error: the build succeeds without MP3/Opus,
//! and `has_lame` / `has_opus` stay unset so nothing references the missing
//! symbols (the same fail-closed rule the registry applies).

use std::path::PathBuf;

fn main() {
    // Registered unconditionally so `#[cfg(has_lame)]` never trips the
    // unexpected_cfgs lint on a machine without vcpkg.
    println!("cargo:rustc-check-cfg=cfg(has_lame)");
    println!("cargo:rustc-check-cfg=cfg(has_opus)");
    if !cfg!(windows) {
        return;
    }
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");

    let root = std::env::var("VCPKG_ROOT")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(|p| PathBuf::from(p).join("vcpkg")))
        .unwrap_or_else(|_| PathBuf::from("vcpkg"));

    let bin = root.join("installed").join("x64-windows").join("bin");
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    // OUT_DIR is target/.../build/oc-audio-<hash>/; the exe lives at
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

    // Also beside the TEST binaries. Cargo builds integration tests into
    // `target/<profile>/deps/`, so a test process asking "is this DLL where
    // the loader will find it?" is asking about `deps/`, not about the
    // profile directory -- and the availability check in
    // `openconvert-run::engines` reads exactly that answer. oc-images has
    // copied to both since it landed; not doing so here made the same engine
    // report available to the CLI and unavailable to its own tests.
    let deps_dir = exe_dir.join("deps");
    let deps_dir = deps_dir.is_dir().then_some(deps_dir);
    for dll in ["libmp3lame.dll", "opus.dll"] {
        let src = bin.join(dll);
        if src.exists() {
            let _ = std::fs::copy(&src, exe_dir.join(dll));
            if let Some(deps) = &deps_dir {
                let _ = std::fs::copy(&src, deps.join(dll));
            }
        } else {
            println!(
                "cargo:warning=oc-audio: {dll} not found under {} -- MP3/Opus output disabled",
                bin.display()
            );
        }
    }

    // Link against the import libraries; at RUNTIME the loader finds
    // libmp3lame.dll / opus.dll beside the executable (copied above), which
    // is what keeps the LGPL obligation discharged: replaceable shared
    // libraries, no static link.
    let lib = root.join("installed").join("x64-windows").join("lib");
    if lib.join("libmp3lame.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=libmp3lame");
        println!("cargo:rustc-cfg=has_lame");
    } else {
        println!(
            "cargo:warning=oc-audio: libmp3lame.lib not found under {} -- MP3 output disabled",
            lib.display()
        );
    }
    if lib.join("opus.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=opus");
        println!("cargo:rustc-cfg=has_opus");
    } else {
        println!(
            "cargo:warning=oc-audio: opus.lib not found under {} -- Opus output disabled",
            lib.display()
        );
    }
}
