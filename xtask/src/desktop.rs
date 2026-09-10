//! Gates on the webview boundary.
//!
//! **Why this file exists before any UI does.** `09` §1 already places the
//! webview *outside* the trust boundary, and `03` §7.1 classifies I7 — "shell
//! injection is unrepresentable" — as **Impossible**. That classification is
//! true of Rust code the `Command::new` scan can see. It is not true across the
//! IPC boundary: `@tauri-apps/plugin-shell` spawns a process from TypeScript,
//! through a dependency's `Command::new`, which no scan of `crates/` will ever
//! look at. Until the checks below exist, I7 is Impossible in Rust and merely
//! **Forbidden** across IPC — a gap an independent review rated HIGH and called
//! the largest unmodelled control surface in v1.
//!
//! Tauri v2 is deny-by-default *if you never grant broadly*. The failure mode
//! is not a missing ACL; it is an ACL that accretes one convenient permission
//! at a time, each individually defensible. So the allowlist lives here, in a
//! gated file, and widening it is a visible diff — the same shape as the
//! `feature = "attest"` seal.
//!
//! These gates ship **before** the first component, deliberately. A boundary
//! added after the app works is a boundary negotiated against a deadline.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Gate {
    /// What the gate is called in `08` §4.
    pub name: &'static str,
    /// Repo-relative path it inspects. A directory is walked for `ext`.
    pub path: &'static str,
    /// Extension to walk for when `path` is a directory.
    pub ext: Option<&'static str>,
    /// Contents -> violations. Receives one file at a time.
    pub check: fn(&str) -> Vec<String>,
    /// Shown when it fires.
    pub why: &'static str,
}

/// The only permissions any capability set may grant.
///
/// `core:default` is the baseline a window needs to exist. It is **not** a
/// finished answer: once the app has a real command surface this should narrow
/// to the explicit `core:window:*` / `core:event:*` entries it actually uses,
/// because `core:default` is a bundle and bundles grow upstream. Recorded here
/// rather than in a comment on the JSON, so the narrowing is a diff to a gated
/// file when someone finally does it.
///
/// Nothing from a plugin namespace belongs here. If one ever does, it needs a
/// line in `SECURITY.md`'s decision log saying which `SR-*` it touches.
const ALLOWED_PERMISSIONS: &[&str] = &[
    "core:default",
    // The five window verbs a self-drawn title bar needs.
    //
    // `decorations: false` removes the native frame, so the app owns minimise,
    // maximise, close, move and resize. `core:window:default` is READ-ONLY --
    // scale factor, position, is-maximised and so on -- so none of these arrive
    // with the bundle; each is listed here deliberately.
    //
    // They are window-management verbs, not I/O: none of them reads a file,
    // spawns a process or opens a socket, so none touches the boundary this
    // gate exists to hold. A sixth entry that did would need a line in
    // SECURITY.md's decision log saying which SR-* it touches.
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-close",
    "core:window:allow-start-dragging",
    "core:window:allow-start-resize-dragging",
];

/// Plugin identifiers that must not appear in any dependency list.
///
/// `shell` and `process` are the direct hits on I7. `fs` re-opens the arbitrary
/// path the broker exists to prevent — [I5] hands engines pre-opened
/// descriptors precisely so no path crosses that boundary, and a webview with
/// `plugin-fs` walks around it. `http` and `updater` are the SR-9 hit: exactly
/// one outbound call site exists, it is `run/src/net.rs`, and a plugin that
/// fetches is a second one the Rust-side scan cannot see.
const FORBIDDEN_PLUGINS: &[&str] = &[
    "plugin-shell",
    "plugin-process",
    "plugin-fs",
    "plugin-http",
    "plugin-updater",
    "plugin-opener",
];

pub const GATES: &[Gate] = &[
    Gate {
        name: "no forbidden Tauri plugin (node)",
        path: "apps/desktop/package.json",
        ext: None,
        check: check_no_forbidden_plugin,
        why: "SR-3/I7 and SR-9. The `Command::new` source scan reads crates/ and cannot\n\
              see a spawn inside a Tauri plugin invoked from TypeScript. A shell or\n\
              process plugin makes I7 false at the IPC boundary while every Rust gate\n\
              stays green.",
    },
    Gate {
        name: "no forbidden Tauri plugin (rust)",
        path: "apps/desktop/src-tauri/Cargo.toml",
        ext: None,
        check: check_no_forbidden_plugin,
        // Vacuous until the src-tauri crate exists, and switched on now on
        // purpose: `08` §4 makes the same argument for the two licence tables.
        // A gate that is already waiting costs nothing; a gate someone has to
        // remember to add when the file appears is the defect this project has
        // now recorded in three consecutive revisions.
        why: "The same claim on the Rust side of the same boundary. This gate is\n\
              vacuous until apps/desktop/src-tauri/Cargo.toml exists, which is the\n\
              point: it fires on the commit that introduces the file, not on the\n\
              commit where someone remembers to write a gate.",
    },
    Gate {
        name: "the capability set is deny-by-default",
        path: "apps/desktop/src-tauri/capabilities",
        ext: Some("json"),
        check: check_capability,
        why: "Property 4. Tauri v2 is deny-by-default only while the grants stay\n\
              narrow. Every permission must be on ALLOWED_PERMISSIONS in\n\
              xtask/src/desktop.rs, and every capability must name its windows --\n\
              an unscoped capability applies to every window, including any the app\n\
              opens later.",
    },
    Gate {
        name: "the webview CSP is strict",
        path: "apps/desktop/src-tauri/tauri.conf.json",
        ext: None,
        check: check_csp,
        why: "SR-18: the desktop application makes no network request. The week-4\n\
              egress test ran against a CLI with no HTTP stack in it; a webview is an\n\
              entire browser engine and nothing re-asserted the claim for it. A single\n\
              remote font or a dev-time CDN reference surviving to release breaks the\n\
              headline privacy promise, and no other gate would notice.",
    },
    Gate {
        name: "restored window geometry is validated",
        path: "apps/desktop/src-tauri/src/main.rs",
        ext: None,
        check: check_geometry_validated,
        why: "PERSISTED STATE IS NOT A FACT ABOUT THE WORLD. The window position
              saved on exit describes the displays attached at the time, and
              restoring it unchecked opened the app onto a monitor that had since
              been unplugged: nothing visible on any screen, no error, and no way
              back except editing the config by hand. The code carried a comment
              claiming the OS would refuse such a position -- it does not.
              `set_position` must therefore sit with a check against the monitors
              attached NOW, and this gate is what makes deleting that check a
              failing build rather than a bug nobody can reproduce.",
    },
    Gate {
        name: "no raw HTML in Svelte components",
        path: "apps/desktop/src",
        ext: Some("svelte"),
        check: check_no_raw_html,
        why: "A15: `09` §3 names 'a crafted filename rendered into the UI' as a\n\
              concrete example and gives it no control. Every string this app renders\n\
              is attacker-influenceable -- filenames, archive member names, engine\n\
              error text. {@html} is the one construct that turns any of them into\n\
              markup.",
    },
];

// ---------------------------------------------------------------------------
// checks
// ---------------------------------------------------------------------------

/// A restored window position must be checked against the displays attached now.
///
/// Deliberately shallow: this is a text scan, and it cannot prove the check is
/// CORRECT. What it can prove is that the check is still there — that nobody
/// simplified `apply_saved_geometry` back down to "read two numbers, call
/// `set_position`", which is what it was, and which is a shape that looks
/// entirely reasonable in a diff.
///
/// The pairing it enforces: any file that positions a window from saved
/// coordinates must also enumerate the monitors and test visibility against
/// them. Naming both halves means neither can be removed on its own.
fn check_geometry_validated(contents: &str) -> Vec<String> {
    let mut hits = Vec::new();

    // Only files that actually restore a position are subject to this.
    if !contents.contains("set_position") {
        return hits;
    }
    // A comment mentioning the API is not a call. `lint::scan_dir` strips `//`
    // for the same reason the plugin gate does: this repo quotes its own API
    // names constantly, and a gate that fires on its own documentation is a
    // gate someone switches off.
    let code: String = contents
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    if !code.contains("set_position") {
        return hits;
    }

    if !code.contains("available_monitors") {
        hits.push(
            "  a window position is restored without enumerating the attached monitors --              `available_monitors()` is the only thing that knows which coordinates exist today"
                .to_string(),
        );
    }
    if !code.contains("is_visible_on") {
        hits.push(
            "  the saved rectangle is not tested for visibility -- restore it only when              `is_visible_on` says part of it lands on a real display"
                .to_string(),
        );
    }
    if !code.contains("primary_monitor") {
        hits.push(
            "  there is no fallback display -- a position that fails the check must send the              window to the primary monitor, not leave it where it cannot be seen"
                .to_string(),
        );
    }

    hits
}

/// Two files, two readers, and the reason is the mistake this function made first.
///
/// The line-scan version fired on a `"comment"` field that merely *mentioned*
/// `plugin-shell` in prose. `lint::scan_dir` strips `//` for exactly this
/// reason — "this repo quotes these API names constantly" — and a gate that
/// objects to its own documentation is a gate someone switches off, taking the
/// real rule with it.
///
/// It was also making the fixture dishonest: the prose hit came first, so the
/// fixture would have kept "firing" even if the dependency check had broken.
/// That is the vacuous-pass shape spike S29 found in this project once already.
///
/// So: JSON is parsed and only dependency **keys** are inspected; TOML has its
/// `#` comments stripped and is matched on the key side of the assignment.
fn check_no_forbidden_plugin(contents: &str) -> Vec<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(contents) {
        let mut hits = Vec::new();
        for section in ["dependencies", "devDependencies", "peerDependencies"] {
            let Some(deps) = json.get(section).and_then(|d| d.as_object()) else {
                continue;
            };
            for name in deps.keys() {
                if FORBIDDEN_PLUGINS.iter().any(|p| name.contains(p)) {
                    hits.push(format!("  {section}: {name}"));
                }
            }
        }
        return hits;
    }

    // Not JSON, so treat it as a manifest: strip comments, then match only the
    // left of an `=` so a plugin named in a value or a doc line is not a hit.
    contents
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let code = line.split('#').next().unwrap_or("");
            let key = code.split('=').next().unwrap_or("").trim();
            FORBIDDEN_PLUGINS
                .iter()
                .any(|p| key.contains(p))
                .then(|| format!("  line {}: {}", i + 1, code.trim()))
        })
        .collect()
}

fn check_capability(contents: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let json: serde_json::Value = match serde_json::from_str(contents) {
        Ok(v) => v,
        Err(e) => return vec![format!("  not valid JSON: {e}")],
    };

    // Scoped, and scoped to something specific. Tauri matches `windows` as
    // glob patterns, so `["*"]` is an unscoped capability wearing a scope --
    // and it is what someone writes when a second window stops working.
    match json.get("windows").and_then(|w| w.as_array()) {
        None => {
            hits.push("  no `windows` array: an unscoped capability applies to every window".into())
        }
        Some(w) if w.is_empty() => hits.push("  empty `windows` array".into()),
        Some(w) => {
            for label in w {
                match label.as_str() {
                    Some(l) if l.contains('*') => hits.push(format!(
                        "  `windows` entry `{l}` is a glob: it scopes to every matching window"
                    )),
                    Some(_) => {}
                    None => hits.push(format!("  unreadable `windows` entry: {label}")),
                }
            }
        }
    }

    match json.get("permissions").and_then(|p| p.as_array()) {
        None => hits.push("  no `permissions` array".into()),
        Some(perms) => {
            for p in perms {
                // A permission may be a string or an object with `identifier`.
                let id = p
                    .as_str()
                    .or_else(|| p.get("identifier").and_then(|i| i.as_str()));
                match id {
                    None => hits.push(format!("  unreadable permission entry: {p}")),
                    Some(id) if !ALLOWED_PERMISSIONS.contains(&id) => hits.push(format!(
                        "  `{id}` is not on ALLOWED_PERMISSIONS in xtask/src/desktop.rs"
                    )),
                    Some(_) => {}
                }
            }
        }
    }
    hits
}

/// The two `localhost` forms Tauri's own IPC and asset protocols need.
///
/// Anything else carrying a scheme is a remote origin, whatever it looks like.
const CSP_ALLOWED_ORIGINS: &[&str] = &["http://ipc.localhost", "http://asset.localhost"];

fn check_csp(contents: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let json: serde_json::Value = match serde_json::from_str(contents) {
        Ok(v) => v,
        Err(e) => return vec![format!("  not valid JSON: {e}")],
    };

    let csp = json
        .get("app")
        .and_then(|a| a.get("security"))
        .and_then(|s| s.get("csp"))
        .and_then(|c| c.as_str());

    let Some(csp) = csp else {
        return vec![
            "  app.security.csp is absent or not a string -- Tauri then applies none".into(),
        ];
    };

    for required in ["default-src 'self'", "object-src 'none'"] {
        if !csp.contains(required) {
            hits.push(format!("  CSP does not contain `{required}`"));
        }
    }
    for forbidden in ["unsafe-inline", "unsafe-eval"] {
        // style-src may legitimately want 'unsafe-inline' with some bundlers.
        // It is still a decision, not a default: record it here if it is ever
        // taken, rather than letting it arrive inside a config diff.
        if csp.contains(forbidden) {
            hits.push(format!("  CSP contains `{forbidden}`"));
        }
    }

    // Tokenise, then require every source carrying a scheme to be *exactly* one
    // of the permitted origins.
    //
    // The first version of this stripped the allowed origins with `replace` and
    // then looked for a leftover scheme. That is a prefix match, so
    // `http://ipc.localhost.evil.com` became `.evil.com`, no scheme survived,
    // and the gate passed a remote origin -- in the check whose entire job is to
    // refuse one. Comparing whole tokens has no such hole.
    //
    // Only `://` forms are examined. Schemeless sources (`data:`, `asset:`) are
    // local by construction and this project's own config uses them for images.
    for token in csp
        .split(|c: char| c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        if token.contains("://") && !CSP_ALLOWED_ORIGINS.contains(&token) {
            hits.push(format!(
                "  CSP names a remote origin (`{token}`) -- SR-18 permits none"
            ));
        }
    }
    hits
}

fn check_no_raw_html(contents: &str) -> Vec<String> {
    contents
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("{@html"))
        .map(|(i, l)| format!("  line {}: {}", i + 1, l.trim()))
        .collect()
}

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

/// Run one gate over one path, returning every hit.
///
/// Exposed so `verify-gates` can point **the real checker** at the negative
/// fixtures, for the reason `lint::scan_dir` gives: asserting that a fixture
/// merely contains a bad value tests the fixture, not the gate.
///
/// A path that does not exist yields no hits. Two gates here are deliberately
/// waiting for files that do not exist yet.
pub fn check_path(gate: &Gate, path: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    if !path.exists() {
        return Ok(hits);
    }
    let root = workspace_root();
    for file in files_for(path, gate.ext)? {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let contents = std::fs::read_to_string(&file).with_context(|| format!("reading {rel}"))?;
        for hit in (gate.check)(&contents) {
            hits.push(format!("  {rel}\n  {hit}"));
        }
    }
    Ok(hits)
}

pub fn run() -> Result<()> {
    let root = workspace_root();
    let mut failures = Vec::new();

    for gate in GATES {
        let hits = check_path(gate, &root.join(gate.path))?;
        if hits.is_empty() {
            println!("desktop: ok -- {}", gate.name);
        } else {
            failures.push(format!(
                "desktop FAILED -- {}\n\n{}\n\n{}",
                gate.name,
                hits.join("\n"),
                gate.why
            ));
        }
    }

    // The installer check needs two files at once -- the config and the text it
    // points at -- so it cannot be a contents-only Gate either.
    let hits = check_installer(&root);
    if hits.is_empty() {
        println!("desktop: ok -- the installer asks, discloses, and takes nothing");
    } else {
        failures.push(format!(
            "desktop FAILED -- the installer asks, discloses, and takes nothing

{}

             An installer is the first thing a user of a privacy tool sees, and the              defaults it ships with are the ones almost nobody changes. Three of these              are promises the generated text on the licence page makes out loud: that              the install location is a choice, that no file association is seized, and              that the privacy statement is shown before a byte is copied. A config that              drifts from that text turns the page into a lie.",
            hits.join("
")
        ));
    }

    // The IPC surface has to agree with itself across three files, and no
    // contents-only Gate can see three files at once.
    let hits = check_ipc_surface(&root);
    if hits.is_empty() {
        println!("desktop: ok -- the IPC surface agrees across all three files");
    } else {
        failures.push(format!(
            "desktop FAILED -- the IPC surface agrees across all three files\n\n{}\n\n\
             A `#[tauri::command]` lives in three places: the function, the \
             `generate_handler!` list that exposes it, and `ipc.ts` which calls it. \
             Miss the handler and every call is a runtime `undefined`; miss `ipc.ts` \
             and the command is dead weight nobody notices. The preview harness is the \
             fourth: a command it does not implement returns `undefined` in the browser, \
             so the UI is exercised against a hole and looks fine right up until the \
             product runs it. This project has shipped that exact drift more than once \
             -- fixture model rows that existed nowhere in the registry, tool ids spelled \
             differently in the harness than in `tools.rs` -- and each time the browser \
             was green.",
            hits.join("\n")
        ));
    }

    // The shipped-font check needs the filesystem, not one file's contents:
    // it asserts the woff2 exists AND that a stylesheet references it. A
    // contents-only Gate entry would be vacuous on whichever half went
    // missing, and 07 §6.1 makes shipping the font a rule, not a preference.
    let hits = check_font_shipped(&root);
    if hits.is_empty() {
        println!("desktop: ok -- InterVariable ships and is referenced");
    } else {
        failures.push(format!(
            "desktop FAILED -- InterVariable ships and is referenced\n\n{}\n\n07 §6.1: \
             \"Ship the font; don't use system-ui.\" The @font-face in \
             apps/desktop/src/lib/styles/tokens.css must point at the bundled woff2 \
             (SIL OFL 1.1), or every platform falls back to whatever it has and the \
             three platforms stop agreeing.",
            hits.join("\n")
        ));
    }

    // Staged worker binaries have to correspond to the tree that produced
    // them. Nothing about a file's CONTENTS can answer that, so this compares
    // modification times across two directories.
    let hits = check_engines_fresh(&root);
    if hits.is_empty() {
        println!("desktop: ok -- every staged engine is newer than its source");
    } else {
        failures.push(format!(
            "desktop FAILED -- every staged engine is newer than its source\n\n{}\n\n\
             `apps/desktop/src-tauri/engines/` is what the installer bundles. A \
             binary there that is older than the crate which produces it means the \
             shipped app runs code that is not in this tree, and it fails in the one \
             way nothing catches: by working slightly differently rather than by not \
             starting at all. This shipped once. `oc-ai.exe` was eight days behind \
             its own source, and background removal, upscaling and transcription all \
             reported that no file had been written -- in a build whose tests were \
             green, because the tests exercised the source and the product ran the \
             binary. Rebuild the engine and copy it into place.",
            hits.join("\n")
        ));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("\n\n---\n\n"))
    }
}

/// The installer must ask where to go, show what it does, and grab nothing.
///
/// Four properties, each one a thing the generated licence page states as fact:
///
/// 1. **`licenseFile` is set and exists.** Tauri's NSIS template inserts the
///    licence page *only* when the file is defined, so an unset key silently
///    removes the one screen carrying the privacy statement -- no error, no
///    warning, just a missing page nobody notices until a user asks where the
///    privacy policy went.
/// 2. **`installMode` is `both`.** The directory page is always present, but
///    the per-user / per-machine choice appears only in this mode. `currentUser`
///    and `perMachine` each decide for the user silently.
/// 3. **No file associations.** A converter that claims `.pdf` on install is
///    the kind of program people uninstall in anger, and the licence page
///    promises it does not.
/// 4. **A publisher.** Windows shows it on the UAC prompt, and "Unknown
///    publisher" on a security tool is its own kind of answer.
fn check_installer(root: &Path) -> Vec<String> {
    const CONF: &str = "apps/desktop/src-tauri/tauri.conf.json";
    let mut hits = Vec::new();

    let path = root.join(CONF);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![format!("  {CONF} is missing")];
    };

    // Read as text rather than parsing: xtask has no JSON dependency, and
    // adding one to a gate crate to check four keys is a poor trade. Each
    // pattern below is the key with its value, so a key present but empty
    // fails the same way a missing key does.
    let has = |needle: &str| text.contains(needle);

    if !has("\"licenseFile\"") {
        hits.push(
            "  bundle.licenseFile is not set -- NSIS renders no licence page at all, so the privacy statement is never shown"
                .to_string(),
        );
    } else {
        const TEXT: &str = "apps/desktop/src-tauri/installer/LICENSE-AND-PRIVACY.txt";
        if !root.join(TEXT).exists() {
            hits.push(format!(
                "  {TEXT} is missing -- run `node release/render-installer-text.mjs`"
            ));
        }
    }

    if !has("\"installMode\": \"both\"") {
        hits.push(
            "  bundle.windows.nsis.installMode is not \"both\" -- the installer then decides per-user vs per-machine without asking"
                .to_string(),
        );
    }

    if has("\"fileAssociations\"") {
        hits.push(
            "  bundle.fileAssociations is set -- the licence page promises OpenConvert claims no extension. Change the promise or drop the key."
                .to_string(),
        );
    }

    // The licence page says uninstalling offers to remove user data. Tauri's
    // own checkbox deletes %LOCALAPPDATA%\<bundle-id>, and this app writes to
    // %LOCALAPPDATA%\OpenConvert -- a different directory, so without the hook the
    // box is ticked, nothing is removed, and the page has lied.
    if !has("\"installerHooks\"") {
        hits.push(
            "  bundle.windows.nsis.installerHooks is not set -- the uninstaller's \"delete application data\" checkbox then removes a directory this app never wrote to"
                .to_string(),
        );
    } else if !root
        .join("apps/desktop/src-tauri/installer/hooks.nsh")
        .exists()
    {
        hits.push("  apps/desktop/src-tauri/installer/hooks.nsh is missing".to_string());
    }

    if !has("\"publisher\"") {
        hits.push(
            "  bundle.publisher is not set -- the UAC prompt then reads \"Unknown publisher\" on a security tool"
                .to_string(),
        );
    }

    hits
}

/// Every staged engine binary must be newer than the crate that builds it.
///
/// Which crate builds which binary is answered by NAME, not by a table: the
/// worker crates live at `crates/engines/<name>/` and produce `<name>.exe`. A
/// table would be one more thing to forget when a worker is added.
///
/// Shared libraries staged beside the workers (`onnxruntime.dll`, `pdfium.dll`
/// and the rest) are third-party artifacts with no crate of their own, so they
/// have nothing to be compared against and are skipped.
fn check_engines_fresh(root: &Path) -> Vec<String> {
    const STAGED: &str = "apps/desktop/src-tauri/engines";
    let mut hits = Vec::new();

    let dir = root.join(STAGED);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // Nothing staged yet is the state a fresh clone is in, and this gate
        // has no opinion about it; packaging is what requires them.
        return hits;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("exe") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let src = root.join("crates").join("engines").join(stem).join("src");
        if !src.is_dir() {
            // AN ORPHAN, AND THE INSTALLER WOULD SHIP IT.
            //
            // This used to `continue`, on the reasoning that a binary with no
            // crate of that name is not ours to judge. It is: `bundle.resources`
            // declares `engines/*`, so every `.exe` in this directory goes into
            // the installer whether or not anything builds it.
            //
            // The rename is what proved it. `oc-images` and friends were
            // `tx-images` and friends, and after the crates moved the five
            // stale binaries sat here under names no crate produced any more --
            // skipped by this very branch, while the gate reported ok. An
            // installer built then would have carried five engines from the
            // previous build, under names `EngineBin::file_stem` no longer
            // looks for.
            hits.push(format!(
                "  {STAGED}/{stem}.exe has no crate at crates/engines/{stem}; nothing \
                 builds it, and the installer would ship it anyway"
            ));
            continue;
        }
        let Ok(built) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Some(newest) = newest_file_in(&src) else {
            continue;
        };
        if newest > built {
            hits.push(format!(
                "  {STAGED}/{stem}.exe is older than crates/engines/{stem}/src"
            ));
        }
    }
    hits
}

/// The most recent modification time anywhere under `dir`.
fn newest_file_in(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                if newest.is_none_or(|current| modified > current) {
                    newest = Some(modified);
                }
            }
        }
    }
    newest
}

/// The bundled UI font must exist and be wired into the stylesheets.
fn check_font_shipped(root: &Path) -> Vec<String> {
    const FONT_REL: &str = "apps/desktop/public/fonts/InterVariable.woff2";
    let mut hits = Vec::new();

    let font = root.join(FONT_REL);
    if !font.exists() {
        hits.push(format!("  {FONT_REL} is missing"));
        return hits;
    }
    if font.metadata().map(|m| m.len()).unwrap_or(0) < 100_000 {
        hits.push(format!(
            "  {FONT_REL} is suspiciously small for a variable font -- replace it with \
             the real Inter release, not a placeholder"
        ));
    }

    // Referenced by an @font-face that names exactly this file.
    let src = root.join("apps/desktop/src");
    let mut referenced = false;
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e == "css" || e == "svelte")
            {
                let Ok(contents) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if contents.contains("InterVariable.woff2") {
                    referenced = true;
                }
            }
        }
    }
    if !referenced {
        hits.push(
            "  no stylesheet or component references InterVariable.woff2 — the font is \
             shipped but never loaded"
                .to_string(),
        );
    }
    hits
}

/// Every `#[tauri::command]` is registered, called, and previewable.
///
/// Four lists that must contain the same names:
///   1. the `#[tauri::command]` functions in `main.rs`
///   2. the `generate_handler!` registration
///   3. `invoke("...")` in `lib/ipc.ts`
///   4. `case "...":` in the preview harness
///
/// Parsed by pattern rather than by syntax tree, which is the same trade every
/// other gate here makes: a regex over a file the repo owns, held to a shape,
/// and loud when the shape changes.
fn check_ipc_surface(root: &Path) -> Vec<String> {
    let mut problems = Vec::new();

    let main_rs = root.join("apps/desktop/src-tauri/src/main.rs");
    let ipc_ts = root.join("apps/desktop/src/lib/ipc.ts");
    let harness = root.join("apps/desktop/preview/tauri-core.ts");

    let (Ok(main), Ok(ipc), Ok(hx)) = (
        std::fs::read_to_string(&main_rs),
        std::fs::read_to_string(&ipc_ts),
        std::fs::read_to_string(&harness),
    ) else {
        return vec!["one of main.rs, ipc.ts or tauri-core.ts could not be read".to_string()];
    };

    // 1. The functions carrying the attribute.
    let mut declared: Vec<String> = Vec::new();
    let mut lines = main.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        // The signature is the next line that starts one, skipping any
        // attributes stacked underneath.
        for next in lines.by_ref() {
            let t = next.trim_start();
            if let Some(rest) = t
                .strip_prefix("async fn ")
                .or_else(|| t.strip_prefix("fn "))
                .or_else(|| t.strip_prefix("pub async fn "))
                .or_else(|| t.strip_prefix("pub fn "))
            {
                if let Some(name) = rest.split(['(', '<']).next() {
                    declared.push(name.trim().to_string());
                }
                break;
            }
            if t.starts_with("#[") || t.is_empty() {
                continue;
            }
            break;
        }
    }
    if declared.is_empty() {
        return vec![
            "no #[tauri::command] functions found -- this gate has gone blind".to_string(),
        ];
    }

    // 2. The registration block.
    let registered: Vec<String> = match main.split_once("generate_handler![") {
        Some((_, tail)) => match tail.split_once("])") {
            Some((body, _)) => body
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty() && !t.starts_with("//"))
                .collect(),
            None => Vec::new(),
        },
        None => Vec::new(),
    };

    for name in &declared {
        if !registered.iter().any(|r| r == name) {
            problems.push(format!(
                "  {name} is a #[tauri::command] but is not in generate_handler!; every call to it fails at runtime"
            ));
        }
        if !ipc.contains(&format!("invoke(\"{name}\"")) {
            problems.push(format!(
                "  {name} is registered but ipc.ts never calls it; it is dead weight or a missing wrapper"
            ));
        }
        if !hx.contains(&format!("case \"{name}\"")) {
            problems.push(format!(
                "  {name} has no case in the preview harness; the browser gets undefined and the UI is previewed against a hole"
            ));
        }
    }
    for name in &registered {
        if !declared.iter().any(|d| d == name) {
            problems.push(format!(
                "  {name} is registered in generate_handler! but no #[tauri::command] declares it"
            ));
        }
    }
    problems
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}

/// One file, or every file under a directory with the given extension.
fn files_for(path: &Path, ext: Option<&str>) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let Some(ext) = ext else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let p = entry?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
