//! Source-scan gates.
//!
//! Each one enforces a claim the design record makes. Where a claim was
//! previously enforced by something weaker, the comment says what and why.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub struct Scan {
    /// What the gate is called in `08` section 4.
    pub name: &'static str,
    /// Crates it applies to.
    pub crates: &'static [&'static str],
    /// Substrings that constitute a violation.
    pub needles: &'static [&'static str],
    /// Files exempted, by path suffix.
    pub allow: &'static [&'static str],
    /// Shown when it fires.
    pub why: &'static str,
}

/// Run one scan over one directory, returning every hit.
///
/// Exposed so `verify-gates` can point **the real scanner** at the negative
/// fixtures. Asserting that a fixture merely *contains* a forbidden string
/// would test the fixture, not the gate: if the comment-stripping below had a
/// bug, the fixture would still look correct while the gate silently passed
/// everything. Check the control.
pub fn scan_dir(scan: &Scan, dir: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    if !dir.exists() {
        return Ok(hits);
    }
    let root = workspace_root();
    for file in rust_files(dir)? {
        // Repo-relative, forward slashes: the output is meant to be pasted into
        // an editor, and an absolute Windows path is neither portable nor clickable.
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if scan.allow.iter().any(|a| rel.ends_with(a)) {
            continue;
        }
        let src = std::fs::read_to_string(&file)?;
        for (i, line) in src.lines().enumerate() {
            // An audited exemption. Deliberately spelled out in full on the
            // offending line, so that every one is greppable in a single
            // command and each carries its own reason:
            //
            //     // openconvert-lint: allow -- why this call site is safe
            //
            // Modelled on cargo-vet's exemptions file: an escape hatch that is
            // *visible* is a debt list; one that is invisible is a hole. CI
            // counts these and the count may only ever go down.
            if line.contains("openconvert-lint: allow") {
                continue;
            }
            // Comments are documentation, not code. This repo quotes these API
            // names constantly -- in doc comments, in the design record, and in
            // the fixtures' own explanations of what they demonstrate.
            let code = line.split("//").next().unwrap_or("");
            for needle in scan.needles {
                if code.contains(needle) {
                    hits.push(format!("  {}:{}  {}", rel, i + 1, line.trim()));
                }
            }
        }
    }
    Ok(hits)
}

pub const SCANS: &[Scan] = &[
    Scan {
        name: "no destructive filesystem API",
        crates: &["openconvert-sandbox", "openconvert-run"],
        // ELEVEN, not one. v0.5 banned `File::create` alone and called the
        // result Impossible. Spikes S4/S25 found five other live routes on
        // Windows and seven on Linux.
        //
        // Two deserve naming because no scan for truncating *opens* finds them:
        //   set_len(0)  truncates through a handle opened with plain write(true)
        //   fs::rename  silently replaced its target on both filesystems tested,
        //               and write-to-temp-then-rename is THE idiomatic atomic
        //               write -- the likeliest way I12 breaks in good faith.
        needles: &[
            "File::create(",
            ".truncate(true)",
            "fs::write(",
            "fs::copy(",
            "fs::rename(",
            "fs::remove_file(",
            "fs::remove_dir_all(",
            ".set_len(",
            // `fs::symlink(` and not `symlink(` -- the loose form also matched
            // `is_symlink()`, which is a READ, in the very code that refuses to
            // follow links. A lint that fires on the protection it enforces is
            // a lint someone disables, and then the real rule goes with it.
            "fs::symlink(",
            "symlink_file(",
            "symlink_dir(",
            "MOVEFILE_REPLACE_EXISTING",
            "FileRenameInfo",
        ],
        allow: &[],
        why: "I12/SR-15: a conversion cannot destroy a file that already exists.\n\
              `File::create_new` is the only permitted creation API -- it is O_EXCL\n\
              and it is safe on both platforms.",
    },
    Scan {
        name: "no process creation outside openconvert-os",
        crates: &["openconvert-core", "openconvert-sandbox", "openconvert-run"],
        needles: &["Command::new("],
        allow: &[],
        why: "SR-3: every process launch names its program with EngineBin and passes\n\
              an argument vector. Spawn lives in -os because passing pre-opened\n\
              descriptors needs unsafe on both platforms (spikes S3, S12c).",
    },
    Scan {
        name: "no untrusted path printed raw",
        crates: &["openconvert"],
        // `.display())` is a Display of a path being *consumed* -- as a macro
        // argument or the last thing in an expression. `.display().to_string()`
        // does not match, which is deliberate: `refusal_json` puts the REAL
        // name into JSON, where escaping is serde's job and a sanitised name
        // would be a lie to a machine that can handle the truth.
        needles: &[".display())"],
        allow: &[],
        why: "A15/09 §3: a filename is not something the user typed -- a glob expands\n\
              from disk. `ESC ] 0 ;` in a name retitles the terminal; U+202E makes it\n\
              read backwards. Print paths through `show()`, which returns a\n\
              DisplayName. JSON output is exempt by construction: `.to_string()`\n\
              does not match this needle.",
    },
    Scan {
        name: "Deserialize only in wire.rs",
        crates: &["openconvert-core", "openconvert-sandbox"],
        needles: &["Deserialize"],
        allow: &["wire.rs"],
        why: "I14: everything crossing the protocol arrives as a wire:: type and is\n\
              clamped on conversion. A derived Deserialize on a validated type\n\
              reconstructs it WITHOUT running its validator -- which is how a\n\
              compromised engine would widen its own limits (spike S6).",
    },
    Scan {
        name: "exactly one outbound network call site",
        crates: &[
            "openconvert-core",
            "openconvert-sandbox",
            "openconvert-os",
            "openconvert-run",
            // The fetch path itself lives in the worker crate's net.rs --
            // native TLS crypto cannot link inside -run (depcheck), so this
            // scan follows it there and keeps counting.
            "openconvert-worker",
        ],
        needles: &[
            "TcpStream::connect",
            "UdpSocket::bind",
            "reqwest::",
            "ureq::",
            "hyper::",
            "TcpListener::bind",
        ],
        allow: &["net.rs"],
        why: "SR-9: zero telemetry, and exactly ONE default-on network call exists --\n\
              the weekly update-and-revocation manifest fetch, which is contentless,\n\
              identifier-free, previewable and disableable. It is the reason the\n\
              engine kill switch works at all (SR-12), and model downloads share\n\
              its single call site in worker/src/net.rs.\n\
              \n\
              Nothing else opens a socket. Not the conversion path, not detection,\n\
              not the GUI. A second call site is not a bug to fix later -- it is the\n\
              claim becoming false.",
    },
    Scan {
        name: "unsafe outside openconvert-os",
        crates: &[
            "openconvert-core",
            "openconvert-sandbox",
            "openconvert-run",
            "openconvert",
        ],
        needles: &["unsafe "],
        allow: &[],
        why: "03 section 5: `unsafe` may not share a crate with a compile-time\n\
              security invariant. Four crates forbid it outright; openconvert-os holds\n\
              the syscalls, and one invariant of its own -- the read-back profile.\n\
              \n\
              Belt and braces over #![forbid(unsafe_code)], which does catch the\n\
              keyword but which a contributor can delete in one line while chasing a\n\
              build error. The attribute cannot catch its own removal; this scan can.",
    },
    Scan {
        name: "heavy inference falls back at run time",
        crates: &["engines/oc-ai"],
        needles: &["session_for("],
        // `infer.rs` DEFINES both functions, so it names `session_for` in its
        // own signature and its own `session_from`. Exempting the file rather
        // than those lines is not laziness about the last one: rustfmt moves a
        // trailing comment off a line already at `max_width`, and the
        // definition's signature is exactly 100 characters -- so the
        // per-line exemption cannot survive a format there. The module that
        // owns sessions is covered by `first_success`'s five tests instead.
        // The PATH, not the file name: `allow` is a suffix match, so a bare
        // "infer.rs" would also exempt any future `something_infer.rs` -- and
        // did exempt this gate's own fixture until it was renamed.
        allow: &["oc-ai/src/infer.rs"],
        why: "`session_for` falls back when a provider cannot be CREATED. The failure\n\
              that reaches users is the other one: DirectML builds a session happily\n\
              and then exhausts VIDEO memory partway through run() -- which is not the\n\
              machine's memory and cannot be raised in Settings. BiRefNet does exactly\n\
              that on a 4 GB card.\n\
              \n\
              The doc comment on `session_for` claimed it covered both. It never\n\
              could: the loop returns before run() is ever called. `run_for` covers\n\
              the second, and every adapter outside infer.rs should use it.\n\
              \n\
              This scan exists because that claim was false for a long time and\n\
              nothing noticed -- reproducing it needs a graphics card, a large model\n\
              and an image big enough to exhaust one. An adapter that still reaches\n\
              for `session_for` has to say on the line why its run cannot exhaust a\n\
              card; that line is greppable, counted by CI, and may only go down.",
    },
    Scan {
        name: "the purity gate is not bypassed",
        crates: &["openconvert-core"],
        needles: &["allow(clippy::disallowed"],
        allow: &[],
        why: "03 section 4: the core's purity is enforced by clippy.toml, which one\n\
              #[allow] would silently undo. A wasm32 build does NOT enforce this --\n\
              std is fully available there and a core full of file handles compiles\n\
              cleanly for that target (spike S1).",
    },
];

pub fn run() -> Result<()> {
    let root = workspace_root();
    let mut failures = Vec::new();

    for scan in SCANS {
        let mut hits = Vec::new();
        for c in scan.crates {
            hits.extend(scan_dir(scan, &root.join("crates").join(c).join("src"))?);
        }
        if hits.is_empty() {
            println!("lint: ok -- {}", scan.name);
        } else {
            failures.push(format!(
                "lint FAILED -- {}\n\n{}\n\n{}",
                scan.name,
                hits.join("\n"),
                scan.why
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("\n\n---\n\n"))
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let p = entry?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
