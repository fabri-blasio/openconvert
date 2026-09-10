//! `cargo xtask deps` — fetch the native libraries this build links against.
//!
//! # The problem this solves
//!
//! Two libraries are not on crates.io and are not vendored: pdfium (PDF
//! rendering) and ONNX Runtime (every model adapter). They were staged by hand
//! into `%TEMP%`, which meant the build worked on exactly one machine and on no
//! CI runner — and worse, nothing recorded WHICH build had been staged.
//!
//! That last part was not hypothetical. `engines.toml` declared pdfium 7215 and
//! attributed it in NOTICE at that version, while the tree in `%TEMP%` held
//! build 8009 and the DLL beside the worker matched neither. A shipped binary
//! whose attribution names a different version is an attribution gap regardless
//! of how permissive the licence is.
//!
//! So: one pinned URL per library, one sha256 per archive, one command.
//!
//! # Why the artifacts land in `.native/` and not `target/`
//!
//! `cargo clean` should not cost a 70 MB download. `.native/` is gitignored,
//! repo-local, and survives a clean — and both build scripts look there first,
//! so a developer who has run this command once needs no environment variables.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// One native library, pinned.
struct Dep {
    /// Directory under `.native/`, and what the build scripts look for.
    name: &'static str,
    /// Where the archive comes from.
    url: &'static str,
    /// sha256 of the ARCHIVE, checked before anything is extracted.
    sha256: &'static str,
    /// Members to pull out, as (path inside archive, path under `.native/<name>/`).
    extract: &'static [(&'static str, &'static str)],
}

const DEPS: &[Dep] = &[
    Dep {
        name: "pdfium",
        // Build 7215, which is what `engines.toml` declares and NOTICE
        // attributes. Changing this means changing both, in the same commit.
        url: "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7215/pdfium-win-x64.tgz",
        sha256: "5edb69a089f9f54c92bbfa38322a28e12a238b2e023161af76e5e34dc2e70834",
        // No import library upstream -- one is generated from
        // `crates/engines/oc-pdf/pdfium.def` after extraction.
        extract: &[("bin/pdfium.dll", "bin/pdfium.dll")],
    },
    Dep {
        // THE DIRECTML BUILD, NOT THE PLAIN ONE, AND IT IS A SUPERSET.
        //
        // This was `onnxruntime-win-x64-1.20.1.zip`, the CPU-only release.
        // Same version, same CPU provider, plus DirectML -- so nothing that
        // worked before stops working, and `oc-ai` gains a provider it can try
        // on the models where a GPU pays for itself.
        //
        // Measured on this machine (RTX 4050 + Intel Iris Xe, Windows 11
        // 26200) before the pin was changed:
        //
        //   Real-ESRGAN x4, 256x192 -> 1024x768   107.3 s CPU   14.5 s DML
        //   u2netp remove-background              ~1 s CPU      7 s of DML
        //                                                       session build
        //                                                       alone
        //   BiRefNet-lite best tier               46 s CPU      out of VRAM
        //
        // Which is why `accelerator::Workload` exists: the GPU is a large win
        // on the heavy models and a loss on the small ones, and a build that
        // used it for everything would make background removal seven times
        // slower while calling it an improvement.
        name: "onnxruntime",
        url: "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime.DirectML/1.20.1",
        sha256: "6763468507b7cfc777b1334b3e174c11a540ddacb7bd4354bc2e0ec89e56eec2",
        extract: &[
            (
                "runtimes/win-x64/native/onnxruntime.dll",
                "bin/onnxruntime.dll",
            ),
            (
                "runtimes/win-x64/native/onnxruntime.lib",
                "lib/onnxruntime.lib",
            ),
        ],
    },
    Dep {
        // DirectML itself, and it must be the REDISTRIBUTABLE.
        //
        // Windows ships a `DirectML.dll` in System32 -- 1.15.5 on this machine,
        // newer than this one -- and using it fails at graph fusion with
        // `80070715 The specified resource type cannot be found in the image
        // file`. The inbox copy is a different variant from the one ONNX
        // Runtime is built against, so "the system already has a newer one" is
        // exactly the wrong inference to draw. Measured, not assumed: that
        // error is what sent us here.
        //
        // 1.15.2 is the version ORT 1.20.1 is built against.
        name: "directml",
        url: "https://www.nuget.org/api/v2/package/Microsoft.AI.DirectML/1.15.2",
        sha256: "9f07482559087088a4dba4ae76eeeee1fad3f7077a92ccfbdb439c6bc2964c09",
        extract: &[("bin/x64-win/DirectML.dll", "bin/DirectML.dll")],
    },
];

/// Where staged libraries live.
pub fn root() -> PathBuf {
    PathBuf::from(".native")
}

pub fn run() -> Result<()> {
    if !cfg!(windows) {
        println!("  These libraries are Windows-only in this build; nothing to fetch.");
        return Ok(());
    }
    let root = root();
    std::fs::create_dir_all(&root)?;

    for dep in DEPS {
        let dir = root.join(dep.name);
        println!("\n\x1b[1m── {}\x1b[0m", dep.name);

        // Already staged? Check the OUTPUTS, not a marker file: a marker says
        // a previous run started, and the files say it finished.
        let complete = dep.extract.iter().all(|(_, to)| dir.join(to).is_file());
        if complete {
            println!("   already staged in {}", dir.display());
        } else {
            let archive = fetch(dep, &root)?;
            extract(dep, &archive, &dir)?;
            println!("   staged into {}", dir.display());
        }
    }

    make_pdfium_lib(&root.join("pdfium"))?;
    println!("\n\x1b[32m  Native dependencies ready.\x1b[0m");
    println!("  Nothing else to set up; the build scripts look in .native/ first.");
    Ok(())
}

/// Download and verify one archive.
fn fetch(dep: &Dep, root: &Path) -> Result<PathBuf> {
    let path = root.join(format!("{}.archive", dep.name));
    println!("   fetching {}", dep.url);

    // `curl` rather than a Rust HTTP client on purpose. This is a SETUP tool,
    // not the product: SR-9's "exactly one outbound call site" governs what
    // ships, and adding a second TLS stack to the workspace so a setup command
    // can download a file would widen the dependency graph the lint exists to
    // keep narrow.
    let status = std::process::Command::new("curl")
        .args([
            "-sSL",
            "--fail",
            "--max-time",
            "600",
            "-o",
            &path.to_string_lossy(),
            dep.url,
        ])
        .status()
        .context("could not run curl; it ships with Windows 10 and later")?;
    if !status.success() {
        bail!("downloading {} failed", dep.name);
    }

    let bytes = std::fs::read(&path)?;
    let got = sha256_hex(&bytes);
    if got != dep.sha256 {
        // The bad archive is removed: leaving it means the next run sees a
        // file, and a partially-correct cache is worse than none.
        let _ = std::fs::remove_file(&path);
        bail!(
            "{} did NOT match its pin.\n  expected {}\n  got      {}\n\
             Nothing was extracted. Either upstream changed the artifact under \
             its own URL, or the download was tampered with; both need a person.",
            dep.name,
            dep.sha256,
            got
        );
    }
    println!("   sha256 ok ({} bytes)", bytes.len());
    Ok(path)
}

/// Pull the named members out, by archive type.
fn extract(dep: &Dep, archive: &Path, dir: &Path) -> Result<()> {
    let bytes = std::fs::read(archive)?;
    let wanted: Vec<(String, PathBuf)> = dep
        .extract
        .iter()
        .map(|(from, to)| ((*from).to_string(), dir.join(to)))
        .collect();

    // A `.nupkg` IS a zip, and the NuGet download URL carries no extension at
    // all -- `.../api/v2/package/<id>/<version>`. Keying on the suffix alone
    // sent both DirectML packages down the gzip path, where they failed as
    // "invalid gzip header" rather than as anything that named the cause.
    let is_zip = dep.url.ends_with(".zip")
        || dep.url.ends_with(".nupkg")
        || dep.url.contains("nuget.org/api/v2/package/");
    if is_zip {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes))?;
        for (from, to) in &wanted {
            let mut member = zip
                .by_name(from)
                .with_context(|| format!("{} has no member {from}", dep.name))?;
            write_member(to, &mut member)?;
        }
    } else {
        let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(&bytes));
        let mut tar = tar::Archive::new(gz);
        // A tar is sequential, so every wanted member is matched in one pass
        // rather than reopening the archive per file.
        for entry in tar.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().replace('\\', "/");
            if let Some((_, to)) = wanted.iter().find(|(from, _)| *from == path) {
                write_member(to, &mut entry)?;
            }
        }
    }

    for (from, to) in &wanted {
        if !to.is_file() {
            bail!("{} did not contain {from}", dep.name);
        }
    }
    // The archive is not kept: it is 60 MB of nothing once the two files it
    // held are on disk, and `deps` re-downloads it if they go missing.
    let _ = std::fs::remove_file(archive);
    Ok(())
}

fn write_member(to: &Path, from: &mut impl std::io::Read) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::File::create(to)?;
    std::io::copy(from, &mut out)?;
    Ok(())
}

/// Build pdfium's import library from the repo's own `.def`.
///
/// Upstream ships the DLL and no `.lib`, so the linker has nothing to bind
/// against. `lib.exe` turns a list of exported names into an import library,
/// and the list is `crates/engines/oc-pdf/pdfium.def` — a source file, because
/// it declares which pdfium functions this project calls.
fn make_pdfium_lib(dir: &Path) -> Result<()> {
    let lib = dir.join("lib").join("pdfium.lib");
    let def = Path::new("crates/engines/oc-pdf/pdfium.def");
    if lib.is_file() && newer(&lib, def) {
        println!("   import library up to date");
        return Ok(());
    }
    std::fs::create_dir_all(dir.join("lib"))?;

    println!("   generating the import library from {}", def.display());
    let Some(lib_exe) = find_lib_exe() else {
        println!(
            "   [33mlib.exe not found. Install the MSVC build tools (rustc already 
                needs their linker) and run this again. PDF rendering stays disabled 
                until then.[0m"
        );
        return Ok(());
    };
    let status = std::process::Command::new(&lib_exe)
        .args([
            &format!("/DEF:{}", def.display()),
            "/MACHINE:X64",
            &format!("/OUT:{}", lib.display()),
        ])
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => bail!(
            "lib.exe could not build the import library from {}",
            def.display()
        ),
        // Not fatal to `deps` as a whole: everything else is staged, and a
        // developer without the MSVC tools gets a build where oc-pdf refuses
        // by name rather than a setup command that failed for reasons they
        // cannot act on from here.
        Err(_) => {
            println!(
                "   \x1b[33mlib.exe not on PATH — run this from a Developer Command Prompt,\n   \
                 or install the MSVC build tools. PDF rendering stays disabled until then.\x1b[0m"
            );
            Ok(())
        }
    }
}

fn newer(a: &Path, b: &Path) -> bool {
    let t = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (t(a), t(b)) {
        (Some(x), Some(y)) => x >= y,
        _ => false,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Find MSVC's `lib.exe`.
///
/// Not on `PATH` outside a Developer Command Prompt, which is where this used
/// to give up — telling a developer to open a different shell to finish a setup
/// step is a poor answer when rustc already located the linker from the same
/// toolchain.
///
/// `vswhere.exe` ships at a fixed path with every Visual Studio since 2017 and
/// is the supported way to ask where the tools are. The newest MSVC version
/// under the install wins, matching what the linker does.
fn find_lib_exe() -> Option<PathBuf> {
    // Already on PATH (a Developer Command Prompt): nothing to search for.
    if std::process::Command::new("lib.exe")
        .arg("/?")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Some(PathBuf::from("lib.exe"));
    }

    let vswhere = PathBuf::from(std::env::var("ProgramFiles(x86)").ok()?)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    let out = std::process::Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    let install = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    if install.as_os_str().is_empty() {
        return None;
    }

    let tools = install.join("VC").join("Tools").join("MSVC");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(tools)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|v| v.join("bin").join("HostX64").join("x64").join("lib.exe"))
        .find(|p| p.is_file())
}
