// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: no destructive filesystem API.
// The sharpest of the eleven: this truncates through a handle opened with
// plain write(true). There is no truncating OPEN anywhere in the call path,
// so a scan for File::create or O_TRUNC finds nothing (spike S4).
pub fn clobber(f: &std::fs::File) -> std::io::Result<()> {
    f.set_len(0)
}
