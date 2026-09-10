//! Build script: locate ONNX Runtime and link it dynamically, exactly as
//! `oc-audio` treats LAME/libopus and `oc-pdf` treats pdfium.
//!
//! The vcpkg tree is located via `VCPKG_ROOT` or `%USERPROFILE%\vcpkg`.
//! Missing libraries are NOT an error: the build succeeds without inference,
//! `has_ort` stays unset, and the worker answers every request with the
//! structured refusal naming the rebuild that would fix it. A worker that
//! fails to LOAD is never acceptable; one that refuses by name is the
//! established honesty rule.

use std::path::PathBuf;

fn main() {
    // Registered unconditionally so `#[cfg(has_ort)]` never trips the
    // unexpected_cfgs lint on a machine without vcpkg.
    println!("cargo:rustc-check-cfg=cfg(has_ort)");
    if !cfg!(windows) {
        return;
    }
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");

    println!("cargo:rerun-if-env-changed=ORT_DIR");

    // Candidate roots in preference order, each as its own (bin, lib) pair so
    // a half-populated tree cannot contribute a DLL from one and an import
    // library from another.
    let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();
    // `.native/onnxruntime` first: staged by `cargo xtask deps`, pinned by
    // sha256, repo-local. Everything after it is a fallback for a developer
    // pointing at their own build.
    if let Some(d) = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|r| r.join(".native").join("onnxruntime"))
    {
        candidates.push((d.join("bin"), d.join("lib")));
    }
    if let Ok(dir) = std::env::var("ORT_DIR") {
        let d = PathBuf::from(dir);
        candidates.push((d.join("bin"), d.join("lib")));
    }
    if let Ok(tmp) = std::env::var("TEMP") {
        let d = PathBuf::from(tmp).join("onnxruntime");
        candidates.push((d.join("bin"), d.join("lib")));
    }
    let vcpkg = std::env::var("VCPKG_ROOT")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(|p| PathBuf::from(p).join("vcpkg")))
        .unwrap_or_else(|_| PathBuf::from("vcpkg"));
    let installed = vcpkg.join("installed").join("x64-windows");
    candidates.push((installed.join("bin"), installed.join("lib")));

    let (bin, lib) = candidates
        .iter()
        .find(|(b, l)| b.join("onnxruntime.dll").exists() && l.join("onnxruntime.lib").exists())
        .cloned()
        .unwrap_or_else(|| candidates[candidates.len() - 1].clone());

    if let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) {
        // OUT_DIR is target/.../build/oc-ai-<hash>/; the exe lives at
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
        let deps_dir = exe_dir.join("deps");
        let deps_dir = deps_dir.is_dir().then_some(deps_dir);
        let dll = bin.join("onnxruntime.dll");
        println!("cargo:rerun-if-changed={}", dll.display());
        if dll.exists() {
            let _ = std::fs::copy(&dll, exe_dir.join("onnxruntime.dll"));
            if let Some(deps) = &deps_dir {
                let _ = std::fs::copy(&dll, deps.join("onnxruntime.dll"));
            }
        } else {
            println!(
                "cargo:warning=oc-ai: onnxruntime.dll not found under {} -- inference disabled",
                bin.display()
            );
        }

        // DirectML.dll travels with it, or the GPU provider cannot load.
        //
        // It is a SEPARATE staged dependency -- `Microsoft.AI.DirectML`, not
        // the ONNX Runtime package -- and it must be the redistributable
        // rather than the copy Windows ships in System32. The inbox one is a
        // different variant from the one ONNX Runtime is built against, and
        // using it fails at graph fusion with `80070715 The specified resource
        // type cannot be found in the image file`, on a machine whose System32
        // copy is NEWER. Being present and being compatible are different
        // questions, and only the second one matters.
        //
        // Its absence is not a build failure: DirectML missing means the GPU
        // provider refuses and every model runs on the CPU, which is the
        // behaviour this crate had before the provider existed.
        let dml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .map(|r| {
                r.join(".native")
                    .join("directml")
                    .join("bin")
                    .join("DirectML.dll")
            })
            .unwrap_or_default();
        println!("cargo:rerun-if-changed={}", dml.display());
        if dml.exists() {
            let _ = std::fs::copy(&dml, exe_dir.join("DirectML.dll"));
            if let Some(deps) = &deps_dir {
                let _ = std::fs::copy(&dml, deps.join("DirectML.dll"));
            }
        } else {
            println!(
                "cargo:warning=oc-ai: DirectML.dll not staged -- GPU inference will fall back to the CPU"
            );
        }
    }

    if lib.join("onnxruntime.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=onnxruntime");
        println!("cargo:rustc-cfg=has_ort");
    } else {
        println!(
            "cargo:warning=oc-ai: onnxruntime.lib not found under {} -- inference disabled",
            lib.display()
        );
    }
}
