// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: Deserialize only in wire.rs.
// Deriving it on a validated type reconstructs that type WITHOUT running its
// validator -- so an OutputName carrying "../../etc/passwd" would arrive fully
// formed, having never passed through parse().
#[derive(serde::Deserialize)]
pub struct OutputName(String);
