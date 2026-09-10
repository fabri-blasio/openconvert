// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: no destructive filesystem API.
// The POSIX-only route: unlink then symlink. No truncating open appears
// anywhere, and the target file is gone before the link is made -- so a scan
// looking for File::create or O_TRUNC sees nothing at all (spike S25).
#[cfg(unix)]
pub fn redirect(victim: &str, attacker_controlled: &str) -> std::io::Result<()> {
    std::os::unix::fs::symlink(attacker_controlled, victim)
}
