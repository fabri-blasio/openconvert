//! Build script: copy the dynamically-linked LGPL engines beside the worker
//! binary on Windows.
//!
//! # Why this exists
//!
//! `03` §5.3 names the trap: shared libraries are loaded INSIDE the sandbox
//! under the container's identity, and a worker whose engines are not
//! readable fails BEFORE `main` with a loader error naming a missing
//! dependency rather than a permission. Copying `heif.dll` /
//! `libde265.dll` next to the worker at build time puts them in the same
//! directory the host already grants execute on — one mechanism, one ACL,
//! no installer step.
//!
//! The vcpkg tree is located via `VCPKG_ROOT` or `%USERPROFILE%\vcpkg`.
//! Missing libraries are NOT an error: the build succeeds without HEIC/RAW
//! support, and the worker reports honestly what it can decode (the same
//! fail-closed rule the registry applies).

use std::path::PathBuf;

fn main() {
    // Registered unconditionally so `#[cfg(has_heif)]` and friends never trip
    // the unexpected_cfgs lint on a machine without vcpkg.
    println!("cargo:rustc-check-cfg=cfg(has_heif)");
    println!("cargo:rustc-check-cfg=cfg(has_raw)");
    println!("cargo:rustc-check-cfg=cfg(has_lcms2)");
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
    // OUT_DIR is target/.../build/oc-images-<hash>/; the exe lives at
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
    // Test binaries live in deps/, and the Windows loader does not search
    // parent directories for DLLs — anything found beside the exe is seeded
    // there as well when the directory exists.
    let deps_dir = exe_dir.join("deps");
    let deps_dir = deps_dir.is_dir().then_some(deps_dir);

    for dll in [
        "heif.dll",
        "libde265.dll",
        "raw_r.dll",
        "lcms2-2.dll",
        "z.dll",
    ] {
        let src = bin.join(dll);
        if src.exists() {
            let _ = std::fs::copy(&src, exe_dir.join(dll));
            if let Some(deps) = &deps_dir {
                let _ = std::fs::copy(&src, deps.join(dll));
            }
            // raw_r.dll needs lcms2-2.dll and z.dll at load time; shipping
            // libraw without them dies before main with a loader error.
            if dll == "heif.dll" {
                println!("cargo:rustc-cfg=has_heif");
            }
        } else {
            println!(
                "cargo:warning=oc-images: {dll} not found under {} -- some decoding disabled",
                bin.display()
            );
        }
    }

    // Link against the import libraries; at RUNTIME the loader finds the DLLs
    // beside the executable (copied above), which keeps each LGPL obligation
    // discharged: replaceable shared library, no static link. These duplicate
    // the modules' own `#[link]` attributes harmlessly.
    let lib = root.join("installed").join("x64-windows").join("lib");
    if lib.join("heif.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=heif");
    }
    if lib.join("raw_r.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=raw_r");
        println!("cargo:rustc-cfg=has_raw");
    } else {
        println!(
            "cargo:warning=oc-images: raw_r.lib not found under {} -- RAW decoding disabled",
            lib.display()
        );
    }
    if lib.join("lcms2.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=lcms2");
        println!("cargo:rustc-cfg=has_lcms2");
    } else {
        println!(
            "cargo:warning=oc-images: lcms2.lib not found under {} -- ICC colour disabled",
            lib.display()
        );
    }
}
