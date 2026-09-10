// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: no process creation outside openconvert-os.
// std::process::Command sets bInheritHandles = TRUE, and the child then
// inherits EVERY inheritable handle in the parent -- 64 observed, 2 writable
// (spike S3). Command cannot express PROC_THREAD_ATTRIBUTE_HANDLE_LIST.
pub fn launch() -> std::io::Result<std::process::Child> {
    std::process::Command::new("oc-images").spawn()
}
