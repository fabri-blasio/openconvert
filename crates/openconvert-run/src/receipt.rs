//! The receipt — **the product's second pillar**.
//!
//! SR-11: every output has one, recording engines, versions, parameters, class,
//! limits, isolation and the profile that engaged, the `content_id` of the
//! bytes actually converted, and what metadata was removed.
//!
//! # A receipt that cannot be written fails the step
//!
//! Not a warning. `03` §13 is explicit: the output is removed and the user is
//! told, because an output without a receipt breaks the thing the product is
//! for. Someone who cannot verify what happened to their file has a file of
//! unknown provenance, which is worse than not having converted it.
//!
//! # What "the profile that engaged" means
//!
//! Not what was requested. Measurement found that error running in **both**
//! directions — ACG engages in an AppContainer whether or not it is asked for,
//! and CIG does not engage even when it is (spikes S17b, S20b). So the field is
//! populated from read-back in the child, and a divergence between requested
//! and engaged is itself recorded. `03` §9.5.

use openconvert_core::isolation::Isolation;
use openconvert_core::plan::{Class, Plan, Step, StepKind};
use serde::Serialize;
use std::path::Path;

/// Schema version. Receipts persist on disk and in customer archives, so the
/// field exists before there is anything to version.
pub const RECEIPT_VERSION: u32 = 1;

/// What happened to one file.
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    /// Schema version.
    pub version: u32,
    /// The tool that produced this.
    pub tool: String,
    /// Input identity — the hash of the bytes **actually converted**, read
    /// through the one open handle, not a fourth read of the path.
    pub content_id: String,
    /// What detection concluded.
    pub detected: String,
    /// What the filename claimed, when it disagreed. `None` when it did not.
    ///
    /// Recorded rather than resolved: SR-4 says the mismatch is surfaced
    /// always, and a receipt that silently normalises it is not evidence.
    pub declared_mismatch: Option<String>,
    /// Every step that ran.
    pub steps: Vec<StepRecord>,
    /// Worst fidelity class across the steps.
    pub class: Option<String>,
    /// The file written.
    pub output: String,
    /// Its size in bytes.
    pub output_bytes: u64,
}

/// One executed step.
#[derive(Debug, Clone, Serialize)]
pub struct StepRecord {
    /// What it did.
    pub kind: String,
    /// Fidelity cost.
    pub class: String,
    /// Which engine, and its version.
    pub engine: String,
    /// Where it ran, and under what confinement.
    pub isolation: String,
    /// The limits it ran under — the values, not a reference to a policy.
    pub limits: LimitRecord,
    /// What was removed, itemised. "Metadata removed" without a list is not
    /// something a user can check.
    pub removed: Vec<String>,
    /// The models this step ran, by registry id and version.
    ///
    /// **A tiered system whose output does not say which tier produced it is a
    /// system that cannot be debugged or trusted.** The ids were already in the
    /// receipt, but only inside `kind` -- a Rust `{:?}` dump of the step, which
    /// is a debugging artefact that happens to contain them, not a field
    /// anything can read. A tool comparing two results, or a user asking why
    /// this cut-out is worse than yesterday's, needs the answer in a place with
    /// a name.
    ///
    /// Empty for every step that ran no model, which is most of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// Limits as executed.
#[derive(Debug, Clone, Serialize)]
pub struct LimitRecord {
    /// Memory ceiling in bytes.
    pub memory_bytes: u64,
    /// Pixel ceiling.
    pub decode_pixels: u64,
    /// Wall-clock ceiling in seconds.
    pub wall_time_secs: u64,
}

impl Receipt {
    /// Build from an executed plan.
    #[must_use]
    pub fn new(
        plan: &Plan,
        content_id: &[u8; 32],
        detected: &str,
        declared_mismatch: Option<String>,
        steps: Vec<StepRecord>,
        output: &Path,
        output_bytes: u64,
    ) -> Self {
        Self {
            version: RECEIPT_VERSION,
            tool: format!("openconvert {}", env!("CARGO_PKG_VERSION")),
            content_id: hex(content_id),
            detected: detected.to_string(),
            declared_mismatch,
            class: plan.class().map(|c| format!("{c:?}")),
            steps,
            // THE NAME, never the path. A receipt travels with the file it
            // describes and often lands in the same shared folder, so an
            // absolute path here publishes the operator's home directory and
            // username to whoever receives the output.
            //
            // This read `output.display()`, which recorded whatever shape the
            // caller happened to pass: `convert bundle.zip` wrote
            // `"bundle.tar"` and `convert C:\Users\<name>\bundle.zip` wrote the
            // absolute path — the same conversion, two receipts, one of them
            // leaking. Found by capturing a receipt for the website, where the
            // difference was visible because the capture script ran from a
            // temp directory. `03` §11.3 already says identity is the name and
            // the content hash; nothing enforced it.
            output: output_name(output),
            output_bytes,
        }
    }

    /// Write beside the output as `<output>.receipt.json`.
    ///
    /// Uses the same `create_new` path as everything else: a receipt must not
    /// overwrite a receipt any more than an output may overwrite an output.
    ///
    /// # Errors
    ///
    /// Any I/O or serialisation failure. The caller **must** treat this as
    /// fatal to the step.
    pub fn write_beside(
        &self,
        output: &Path,
        on_conflict: openconvert_core::policy::OnConflict,
    ) -> std::io::Result<std::path::PathBuf> {
        use std::io::Write;
        let dir = output.parent().unwrap_or(Path::new("."));
        let name = format!(
            "{}.receipt.json",
            output.file_name().unwrap_or_default().to_string_lossy()
        );
        let (placed, file) = crate::write::create_output(dir, &name, on_conflict)?;
        let path = match &placed {
            crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
        };
        if let Some(mut f) = file {
            let body = serde_json::to_vec_pretty(self)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            f.write_all(&body)?;
            f.write_all(b"\n")?;
        }
        Ok(path)
    }
}

/// Describe a step for the record.
#[must_use]
pub fn record(step: &Step, engine: &str, removed: Vec<String>) -> StepRecord {
    StepRecord {
        kind: format!("{:?}", step.kind),
        class: class_name(step.class).to_string(),
        engine: engine.to_string(),
        isolation: match step.isolation {
            Isolation::InProcess => "in-process (pure Rust)".to_string(),
            Isolation::Sandboxed(p) => format!(
                "sandboxed: {} filesystem, {} network, strength {}",
                fs_name(p.filesystem()),
                net_name(p.network()),
                p.strength()
            ),
        },
        limits: LimitRecord {
            memory_bytes: step.limits.memory_bytes,
            decode_pixels: step.limits.decode_pixels,
            wall_time_secs: step.limits.wall_time.as_secs(),
        },
        removed,
        models: model_ids(step),
    }
}

/// The models a step names, with the version the registry has for each.
///
/// `moss-transcribe-diarize @ 0.9B`, not `moss-transcribe-diarize` and not
/// "transcription": the id says which artifact, the version says which build of
/// it, and a receipt that carries one without the other cannot settle an
/// argument about whether two runs used the same weights.
fn model_ids(step: &Step) -> Vec<String> {
    let StepKind::Infer { models, .. } = step.kind else {
        return Vec::new();
    };
    let rows = crate::models::rows();
    models
        .iter()
        .map(|id| {
            rows.iter().find(|r| r.name == *id).map_or_else(
                || (*id).to_string(),
                |r| format!("{} @ {}", r.name, r.version),
            )
        })
        .collect()
}

const fn class_name(c: Class) -> &'static str {
    match c {
        Class::A => "A (lossless)",
        Class::B => "B (lossy, standard)",
        Class::C => "C (structurally transformed)",
        Class::D => "D (generative)",
    }
}

fn fs_name(f: openconvert_core::isolation::FsConfinement) -> &'static str {
    use openconvert_core::isolation::FsConfinement as F;
    match f {
        F::Landlock => "Landlock",
        F::AppContainer => "AppContainer",
        F::AppSandbox => "App Sandbox",
        F::BindMount => "bind mount",
        F::BrowserOrigin => "browser origin",
        F::None => "none",
    }
}

fn net_name(n: openconvert_core::isolation::NetConfinement) -> &'static str {
    use openconvert_core::isolation::NetConfinement as N;
    match n {
        N::EmptyNetns => "empty netns",
        N::SeccompBlock => "seccomp",
        N::CapabilitySid => "no capability SID",
        N::NoEntitlement => "no entitlement",
        N::Csp => "CSP",
        N::None => "none",
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The output's NAME, for the `output` field.
///
/// Split out so the rule is testable without building a `Plan`: the field is a
/// pure function of the path, and it was the path shape — not the plan — that
/// produced the leak.
///
/// **Both separators, on every platform.** `Path::file_name` asks the HOST what
/// a separator is, and on Linux a backslash is an ordinary character -- so
/// `C:\Users\someone\bundle.tar` came back whole, and a receipt written on Linux
/// could carry a Windows path that a Windows reader renders as one. The property
/// this field has to hold is "not a path", and that does not vary by the
/// operating system the conversion happened to run on. The tests below asserted
/// it on Windows and failed on Linux, which is how the asymmetry surfaced.
///
/// A drive-relative spelling (`C:bundle.tar`) carries no separator at all, so
/// the prefix is stripped too. A colon is already refused by `OutputName` -- it
/// is how an NTFS alternate data stream is spelled -- so nothing legal loses
/// characters here.
///
/// The empty string stays unreachable: a root, or a path ending in `..`, is not
/// a writable output.
///
/// The trade, stated: a Linux filename may legally contain a backslash, and one
/// that did would now be trimmed at it. Nothing this app writes can be such a
/// file -- `OutputName` refuses backslashes before anything is created -- so the
/// case needs a hand-made path to reach, and losing a character there is a
/// better failure than emitting a path-shaped field a Windows reader would
/// resolve.
fn output_name(output: &Path) -> String {
    let raw = output.to_string_lossy();
    let after_sep = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut chars = after_sep.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => chars.as_str().to_string(),
        _ => after_sep.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::output_name;
    use std::path::Path;

    /// A receipt travels with the file it describes. An absolute path in it
    /// publishes the operator's home directory to whoever receives the output,
    /// and the same conversion must not produce two different receipts because
    /// the caller spelled the input differently.
    #[test]
    fn output_is_a_name_never_a_path() {
        let cases = [
            "bundle.tar",
            "./bundle.tar",
            "out/bundle.tar",
            r"C:\Users\someone\Documents\bundle.tar",
            "/home/someone/documents/bundle.tar",
            r"\server\share\bundle.tar",
        ];
        for c in cases {
            let got = output_name(Path::new(c));
            assert_eq!(got, "bundle.tar", "input {c:?} produced {got:?}");
        }
    }

    /// Every spelling of one conversion agrees, which is the property the
    /// leak broke.
    #[test]
    fn spelling_of_the_input_does_not_change_the_receipt() {
        let absolute = output_name(Path::new(r"C:\a\b\c\photo.jpg"));
        let relative = output_name(Path::new("photo.jpg"));
        assert_eq!(absolute, relative);
    }

    /// Unicode and spaces survive byte-for-byte; the field is a name, not a
    /// sanitised label.
    #[test]
    fn names_are_not_normalised() {
        assert_eq!(
            output_name(Path::new("/tmp/rapport final.pdf")),
            "rapport final.pdf"
        );
        assert_eq!(
            output_name(Path::new("/tmp/\u{6f22}\u{5b57}.png")),
            "\u{6f22}\u{5b57}.png"
        );
    }
}
