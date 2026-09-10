//! The state-touching half of the CLI, plus the conversion pipeline both
//! `convert` and `batch` drive.
//!
//! Both commands live by the same rules the state layer enforces: a journal
//! line or a recipe file is untrusted on read, a stored **request** may be
//! replayed while a stored **plan** never is, and nothing read from disk can
//! widen what a conversion is permitted to do.

pub mod batch;
pub mod convert;
pub mod recipe;

use openconvert_core::plan::PlanRequest;
use openconvert_core::target::{Operation, Target};

/// The hash persisted beside every stored request.
///
/// A **stable projection** — the request's own representation, hashed — and
/// deliberately *not* a hash of `Plan`: coupling every future field addition
/// to every persisted recipe and journal is exactly what the review ruled
/// out. When a recipe or a resumed batch re-routes, this is the number
/// compared; a difference means the inputs changed, and the caller is told.
#[must_use]
pub(crate) fn plan_hash(request: &PlanRequest) -> u64 {
    let digest = blake3::hash(format!("{request:?}").as_bytes());
    u64::from_be_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("blake3 digests are 32 bytes"),
    )
}

/// Lowercase hex.
///
/// Three lines instead of a dependency; the only hex in the product is
/// content ids and batch ids going into filenames and journals.
#[must_use]
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The target flag, if one was given.
///
/// Shared by `batch` and `recipe save`, whose three-layer rule is the same:
/// CLI flag wins, then `config.toml`, then refusal to guess.
///
/// # Errors
///
/// A malformed flag (`-t` with no value, unknown format).
pub(crate) fn parse_target_flag(args: &[String]) -> anyhow::Result<Option<Target>> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-t" | "--to" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("-t needs a format"))?;
                return Ok(Some(Target::Format(crate::parse_format(v)?)));
            }
            "--strip-metadata" => return Ok(Some(Target::Operation(Operation::StripMetadata))),
            _ => {}
        }
    }
    Ok(None)
}
