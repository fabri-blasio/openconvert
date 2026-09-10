// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: unsafe outside openconvert-os.
// The realistic route in: someone deletes #![forbid(unsafe_code)] from a crate
// header while chasing a build error, and the keyword becomes legal in a crate
// holding compile-time security invariants. The forbid attribute catches this
// too -- but a scan makes the DELETION visible, and the attribute cannot catch
// its own removal.
pub fn transmute_a_profile(bytes: [u8; 8]) -> u64 {
    unsafe { std::mem::transmute(bytes) }
}
