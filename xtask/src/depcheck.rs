//! D7's gate: `openconvert-run` links no native library.
//!
//! **This replaces a claim that was false.** Four places in the v0.5 record said
//! `-run` "forbids `unsafe`, so it cannot link libvips or pdfium". Spike S2
//! disproved it: a crate with `#![forbid(unsafe_code)]` and no `unsafe` token
//! compiled, linked and called into C through a safe wrapper. `forbid` is a
//! crate-scoped lint on the keyword in *that crate's own source*; the `unsafe`
//! lives in the `-sys` dependency.
//!
//! The authoritative signal is cargo's `links` manifest key, **not** the `-sys`
//! naming convention -- `windows-sys` and `linux-raw-sys` link nothing at all.
//! Normal dependency edges only; dev-dependencies produced false positives in
//! testing.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::process::Command;

/// Crates that must never pull in a native library.
const MUST_NOT_LINK: &[&str] = &["openconvert-run", "openconvert-core", "openconvert-sandbox"];

/// The edges the D7 invariant DELIBERATELY tolerates. Every entry carries its
/// written argument; adding one without a paragraph of justification is how a
/// security gate becomes a suggestion.
///
/// (`links` value prefix, boundary crate.)
const ALLOWED: &[(&str, &str)] = &[
    // TLS for THE single outbound fetch path (SR-9): the weekly revocation
    // manifest and user-initiated model downloads share it, in
    // `openconvert-worker/src/net.rs`. Every pure-Rust TLS stack bottoms out
    // in `ring` or aws-lc-rs -- there is no production TLS without native
    // crypto today. What bounds the risk: the fetched bytes are never
    // executed, and every model artifact is sha256-pinned in models.toml and
    // verified BEFORE use, so even a transport-level compromise cannot
    // substitute different weights.
    ("openconvert-run", "ring_core"),
];

fn tolerated(boundary: &str, links: &str) -> bool {
    ALLOWED.iter().any(|(crate_name, links_prefix)| {
        *crate_name == boundary && links.starts_with(links_prefix)
    })
}

pub fn run() -> Result<()> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--all-features"])
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let meta: Value = serde_json::from_slice(&out.stdout)?;

    // id -> (name, links)
    let mut links: HashMap<&str, (&str, Option<&str>)> = HashMap::new();
    for p in meta["packages"].as_array().context("packages")? {
        links.insert(
            p["id"].as_str().context("package id")?,
            (
                p["name"].as_str().context("package name")?,
                p["links"].as_str(),
            ),
        );
    }

    // id -> normal-dependency ids
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in meta["resolve"]["nodes"].as_array().context("nodes")? {
        let id = n["id"].as_str().context("node id")?;
        let mut kids = Vec::new();
        for d in n["deps"].as_array().context("deps")? {
            // Follow NORMAL edges only. A dev-dependency that links a native
            // library is not shipped and is not a violation.
            let normal = d["dep_kinds"]
                .as_array()
                .map(|ks| ks.iter().any(|k| k["kind"].is_null()))
                .unwrap_or(false);
            if normal {
                kids.push(d["pkg"].as_str().context("dep pkg")?);
            }
        }
        edges.insert(id, kids);
    }

    let mut violations = Vec::new();
    for root_name in MUST_NOT_LINK {
        let Some(root) = links
            .iter()
            .find(|(_, (n, _))| n == root_name)
            .map(|(id, _)| *id)
        else {
            continue; // crate not in the workspace yet
        };
        let mut seen = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some((name, Some(link))) = links.get(id) {
                if !tolerated(root_name, link) {
                    violations.push(format!("  {root_name} -> {name} [links = {link}]"));
                }
            }
            if let Some(kids) = edges.get(id) {
                stack.extend(kids.iter().copied());
            }
        }
    }

    if violations.is_empty() {
        println!("depcheck: ok -- no boundary crate links a native library");
        return Ok(());
    }
    bail!(
        "depcheck FAILED -- a boundary crate links a native library.

{}

         Engines are subprocesses (D7). An engine linked into the host is a licence
         boundary AND a security boundary crossed at once. Note that
         #![forbid(unsafe_code)] does NOT catch this -- see spike S2.",
        violations.join(
            "
"
        )
    )
}
