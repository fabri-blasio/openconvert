// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: no destructive filesystem API.
// The LIKELIEST violation, and it comes from someone acting in good faith:
// write-to-temp-then-rename is the idiomatic atomic write, and 03 section 13's
// "partial output removed" rows push an implementer straight toward it.
// Measured: rename replaced its target silently on both filesystems tested.
pub fn atomic_write(tmp: &str, dst: &str) -> std::io::Result<()> {
    std::fs::rename(tmp, dst)
}
