//! The native-engine licence gate — the one `engines.toml` promised.
//!
//! # Why this file exists
//!
//! `engines.toml`'s own header says *"CI fails any engine whose licence is LGPL
//! and whose `link_mode` is not `dynamic`."* Nothing read the file. `manifest`
//! asserted it **existed**, which is a different claim, and the rule it
//! describes was enforced by nobody. The sixth thing in this project found to be
//! written down, agreed, and connected to nothing.
//!
//! # The rule that was missing entirely
//!
//! There are two licence surfaces already: `deny.toml` bars copyleft from the
//! **Rust** dependency tree, and this file was meant to bar static LGPL. Neither
//! covers **a GPL native library**, and that gap is not hypothetical.
//!
//! `vcpkg install libheif` — the obvious command, and the one actually run
//! against this machine — installs **x265 (GPL-2.0)** by default, because
//! libheif's default feature set is `["hevc"]` and `hevc` means *encoding* via
//! x265. Decoding HEIC needs only libde265 (LGPL-3.0). So the flagship
//! `heic → jpg` conversion needs no GPL code at all, and the default install
//! brings it anyway.
//!
//! Linking that into `oc-images` makes `oc-images` GPL-2.0, against a project
//! that is Apache-2.0 throughout. `deny.toml` cannot see it — x265 is not a
//! crate — and the LGPL rule does not look at licence class. It would have
//! passed both gates.
//!
//! So the rule is: **GPL and AGPL are refused outright**, whatever the linkage.
//! No `link_mode` discharges the GPL for a work this project distributes, and
//! offering one as an option invites someone to pick it.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// One declared native engine.
#[derive(Debug, serde::Deserialize)]
pub struct Engine {
    /// Library name, as upstream calls it.
    pub name: String,
    /// Version pinned, for the notice and the SBOM.
    ///
    /// Read by neither rule below and required anyway: NOTICE and the SBOM are
    /// generated from this file, and an engine shipped without a version is an
    /// attribution nobody can check.
    #[allow(dead_code)]
    pub version: String,
    /// SPDX-ish licence identifier.
    pub licence: String,
    /// Where it sits relative to us.
    pub linkage: String,
    /// How it is linked, where that applies.
    pub link_mode: String,
    /// Which worker binary contains it.
    pub worker: String,
    /// Upstream, for the source-offer obligation.
    pub source: String,
    /// Whether this build actually ships it.
    ///
    /// A disabled engine is still checked. An entry parked with a forbidden
    /// licence is a decision someone made once and a future contributor will
    /// find already made for them.
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
struct Manifest {
    #[serde(default)]
    engine: Vec<Engine>,
}

/// The workers an engine may be linked into.
const WORKERS: &[&str] = &["oc-images", "oc-pdf", "oc-archive", "oc-audio", "oc-ai"];

const LINKAGES: &[&str] = &["subprocess", "linked-into-worker", "linked-into-host"];
const LINK_MODES: &[&str] = &["dynamic", "static"];

pub fn run() -> Result<()> {
    let path = workspace_root().join("engines.toml");
    let problems = check_path(&path)?;
    if problems.is_empty() {
        println!("engines: every declared engine passes the licence and linkage rules");
        return Ok(());
    }
    bail!("engines FAILED\n\n{}", problems.join("\n"));
}

/// Check one manifest, returning every violation.
///
/// Exposed so `verify-gates` can point **the real checker** at fixtures built
/// to trip it. Asserting a fixture merely contains the word "GPL" would test the
/// fixture, not the gate.
pub fn check_path(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))?;
    Ok(check(&manifest.engine))
}

/// Every rule, over every declared engine.
#[must_use]
pub fn check(engines: &[Engine]) -> Vec<String> {
    let mut problems = Vec::new();

    for e in engines {
        let licence = e.licence.to_ascii_uppercase();

        // RULE 1. GPL and AGPL are refused outright.
        //
        // Checked before the LGPL rule and with a `!contains("LGPL")` guard,
        // because "LGPL-3.0" contains "GPL" and matching naively would refuse
        // every engine this project actually intends to ship.
        if !licence.contains("LGPL") && (licence.contains("GPL") || licence.contains("AGPL")) {
            problems.push(format!(
                "  {} is {} -- GPL and AGPL are refused whatever the linkage.\n     \
                 No link_mode discharges the GPL for a work we distribute, and this \
                 project is Apache-2.0 throughout.\n     \
                 If this arrived as a dependency of something else, check its feature \
                 flags: `vcpkg install libheif` pulls x265 (GPL-2.0) by default for \
                 HEVC ENCODING, which decoding does not need.",
                e.name, e.licence
            ));
        }

        // RULE 2. The rule engines.toml already described. LGPL discharges its
        // relinking obligation by shipping as a replaceable shared library, and
        // a static link removes the user's ability to replace it.
        if licence.contains("LGPL") && e.link_mode != "dynamic" {
            problems.push(format!(
                "  {} is {} and link_mode is {:?} -- LGPL engines must be dynamic.\n     \
                 The obligation is discharged by shipping a replaceable .dll/.so/.dylib \
                 (D19). A static link attaches the relinking obligation to us.",
                e.name, e.licence, e.link_mode
            ));
        }

        // RULE 3. Closed vocabularies, so a typo is a failure rather than a
        // silently unenforced row. `link_mode: "dynmaic"` would otherwise sail
        // past rule 2.
        if !LINKAGES.contains(&e.linkage.as_str()) {
            problems.push(format!(
                "  {} has linkage {:?}, which is not one of {LINKAGES:?}",
                e.name, e.linkage
            ));
        }
        if !LINK_MODES.contains(&e.link_mode.as_str()) {
            problems.push(format!(
                "  {} has link_mode {:?}, which is not one of {LINK_MODES:?}",
                e.name, e.link_mode
            ));
        }

        // RULE 4. An engine names a worker that exists, so a row cannot be
        // orphaned by a rename.
        if e.linkage == "linked-into-worker" && !WORKERS.contains(&e.worker.as_str()) {
            problems.push(format!(
                "  {} is linked into {:?}, which is not one of {WORKERS:?}",
                e.name, e.worker
            ));
        }

        // RULE 5. The source offer needs somewhere to point.
        if e.enabled && e.source.trim().is_empty() {
            problems.push(format!(
                "  {} is enabled with no source URL -- the LGPL source offer needs one",
                e.name
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

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(name: &str, licence: &str, link_mode: &str) -> Engine {
        Engine {
            name: name.into(),
            version: "1.0".into(),
            licence: licence.into(),
            linkage: "linked-into-worker".into(),
            link_mode: link_mode.into(),
            worker: "oc-images".into(),
            source: "https://example.invalid/".into(),
            enabled: true,
        }
    }

    /// **The rule that was missing.** GPL is refused however it is linked.
    #[test]
    fn a_gpl_engine_is_refused_whatever_the_linkage() {
        for mode in ["dynamic", "static"] {
            let p = check(&[engine("x265", "GPL-2.0", mode)]);
            assert!(
                p.iter().any(|s| s.contains("x265")),
                "GPL passed with link_mode {mode}: {p:?}"
            );
        }
        assert!(!check(&[engine("agpl-thing", "AGPL-3.0", "dynamic")]).is_empty());
    }

    /// **The control, and the reason rule 1 is not a naive substring match.**
    ///
    /// "LGPL-3.0" contains "GPL". A gate that matched naively would refuse
    /// libheif, libde265, libvips, libraw and LAME — every LGPL engine this
    /// project intends to ship — and would look like it was working.
    #[test]
    fn lgpl_is_not_caught_by_the_gpl_rule() {
        for name in ["libheif", "libde265", "libvips", "libraw", "LAME"] {
            let p = check(&[engine(name, "LGPL-3.0", "dynamic")]);
            assert!(p.is_empty(), "{name} was refused as if it were GPL: {p:?}");
        }
    }

    /// The rule `engines.toml` described and nothing enforced.
    #[test]
    fn a_static_lgpl_engine_is_refused_and_a_dynamic_one_is_not() {
        let p = check(&[engine("libheif", "LGPL-3.0", "static")]);
        assert!(p.iter().any(|s| s.contains("must be dynamic")), "{p:?}");
        assert!(check(&[engine("libheif", "LGPL-3.0", "dynamic")]).is_empty());
    }

    /// A permissive licence passes either way.
    #[test]
    fn a_permissive_engine_passes() {
        assert!(check(&[engine("lcms2", "MIT", "static")]).is_empty());
        assert!(check(&[engine("libjpeg-turbo", "BSD-3-Clause", "dynamic")]).is_empty());
    }

    /// A typo in a closed vocabulary fails rather than disabling a rule.
    ///
    /// `link_mode: "dynmaic"` is not `"dynamic"`, so rule 2 would fire on an
    /// LGPL engine anyway — but on a permissive one it would sail through
    /// unnoticed, and the field would silently mean nothing.
    #[test]
    fn a_misspelled_link_mode_is_refused() {
        let mut e = engine("libvips", "MIT", "dynmaic");
        e.linkage = "linked-into-wroker".into();
        let p = check(&[e]);
        assert!(p.iter().any(|s| s.contains("link_mode")), "{p:?}");
        assert!(p.iter().any(|s| s.contains("linkage")), "{p:?}");
    }

    /// A disabled engine is still checked.
    ///
    /// An entry parked with a forbidden licence is a decision someone made
    /// once, and a future contributor flipping `enabled` would find it already
    /// made for them.
    #[test]
    fn a_disabled_engine_is_still_checked() {
        let mut e = engine("x265", "GPL-2.0", "dynamic");
        e.enabled = false;
        assert!(!check(&[e]).is_empty(), "a parked GPL entry passed");
    }
}
