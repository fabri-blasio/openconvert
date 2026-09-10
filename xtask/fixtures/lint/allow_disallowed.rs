// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: the purity gate is not bypassed.
// One #[allow] silently undoes clippy.toml. This is why the bypass is itself
// gated -- and why a wasm32 build cannot substitute: std is fully available
// there, so this file compiles for that target either way (spike S1).
#[allow(clippy::disallowed_types)]
pub fn read_config() -> std::path::PathBuf {
    std::path::PathBuf::from("config.toml")
}
