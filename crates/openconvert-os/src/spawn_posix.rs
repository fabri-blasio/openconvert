//! Process creation on Unix, with the same shape as the Windows path.
//!
//! # Why this is so much shorter, and why that is not comfort
//!
//! `posix_spawn` closes descriptors above the three standard ones by default,
//! so `Command` does not leak the way `CreateProcessW` does (spike S3 measured
//! 64 leaked handles on Windows and none here). The difference is real.
//!
//! **The confinement is not.** On Windows the child is placed in a Job Object
//! and an AppContainer by the host, both verified on this machine. Here the
//! host applies one thing — an address-space cap via `RLIMIT_AS`, which is the
//! recoverable counterpart of the Job Object's kill ([`03` §10.1]): the worker
//! can still report its own failure instead of dying unexplained. Everything
//! else is self-applied by the worker in its own process (Landlock + seccomp
//! before its first read), which is what [`crate::linux`] implements and what
//! the escape tests verify on a real kernel.
//!
//! The split of responsibilities is deliberate: the host bounds what the
//! child may *consume* (`RLIMIT_AS`, set before `exec`, so no allocation the
//! child makes can escape it), and the child denies itself everything it must
//! not *reach*.

use std::path::Path;
use std::process::{Child, Command, Stdio};

extern "C" {
    fn setrlimit(resource: i32, rlim: *const Rlimit) -> i32;
}

// Namespace flags for the pre-exec escalation. Linux only: `unshare(2)` with
// these values has no macOS counterpart, so the namespaced spawn below is
// compiled out elsewhere rather than pretending.
#[cfg(target_os = "linux")]
const CLONE_NEWNS: i32 = 0x0002_0000;
#[cfg(target_os = "linux")]
const CLONE_NEWUSER: i32 = 0x1000_0000;
#[cfg(target_os = "linux")]
const CLONE_NEWNET: i32 = 0x4000_0000;

#[cfg(target_os = "linux")]
extern "C" {
    fn unshare(flags: i32) -> i32;
}

/// `struct rlimit`. Both fields are `rlim_t`, 64 bits on every target we ship.
#[repr(C)]
struct Rlimit {
    rlim_cur: u64,
    rlim_max: u64,
}

const RLIMIT_AS: i32 = 9;

/// Why a spawn failed.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// The child could not be started.
    #[error("could not start {program}: {source}")]
    Create {
        /// What we tried to run.
        program: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// An argument contained a NUL.
    ///
    /// Refused rather than truncated, matching the Windows path: a NUL is
    /// where the string Rust validated stops being the string the OS executes.
    #[error("the command line contains a NUL byte")]
    NulInCommandLine,
}

/// A running child with pipes to its standard streams.
#[derive(Debug)]
pub struct PipedChild {
    child: Child,
}

/// The piped `Command` both spawn paths start from: NUL-validated argv, and
/// the stdio shape the protocol depends on (piped in/out, inherited stderr).
fn piped_command(program: &Path, args: &[&str]) -> Result<Command, SpawnError> {
    if program.to_string_lossy().contains('\0') || args.iter().any(|a| a.contains('\0')) {
        return Err(SpawnError::NulInCommandLine);
    }
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Inherited rather than piped, for the same reason as on Windows: a
        // piped stderr nobody drains fills and blocks the worker mid
        // conversion.
        .stderr(Stdio::inherit());
    Ok(cmd)
}

fn apply_rlimit_as(cmd: &mut Command, bytes: u64) {
    use std::os::unix::process::CommandExt;
    // SAFETY: runs once in the forked child before exec. `setrlimit` is
    // async-signal-safe; the struct is a stack local that outlives the
    // call; nothing is allocated inside the closure. A failure here is
    // fatal for the child rather than silently ignored — spawning a
    // worker whose memory ceiling could not be set would run an
    // attacker-parsed file without the resource bound the plan promised.
    unsafe {
        cmd.pre_exec(move || {
            let rl = Rlimit {
                rlim_cur: bytes,
                rlim_max: bytes,
            };
            if setrlimit(RLIMIT_AS, &rl) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Spawn `program` with pipes to its stdin and stdout, capped at `memory_cap`
/// bytes of address space.
///
/// The cap runs in the child between `fork` and `exec`, so there is no window
/// in which the engine exists uncapped. `RLIMIT_AS` produces a *recoverable*
/// allocation failure — the worker catches it and answers `Failed{..}` with a
/// reason, which is exactly the property Windows cannot offer and needs an IO
/// completion port to fake ([`03` §10.1]).
///
/// # Errors
///
/// See [`SpawnError`].
pub fn spawn_piped_limited(
    program: &Path,
    args: &[&str],
    memory_cap: Option<u64>,
) -> Result<PipedChild, SpawnError> {
    let mut cmd = piped_command(program, args)?;
    if let Some(bytes) = memory_cap {
        apply_rlimit_as(&mut cmd, bytes);
    }
    let child = cmd.spawn().map_err(|source| SpawnError::Create {
        program: program.display().to_string(),
        source,
    })?;
    Ok(PipedChild { child })
}

/// Spawn `program` with pipes to its stdin and stdout, inside a **fresh user
/// namespace, empty network namespace, and fresh mount namespace**, optionally
/// capped at `memory_cap` bytes of address space.
///
/// This is the host half of the Linux Full tier. The child calls
/// `unshare(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWNS)` between `fork` and
/// `exec`, so there is no window in which the engine exists unnamespaced --
/// the same property the RLIMIT_AS closure gives the memory cap, and the same
/// escalation [`crate::linux::apply_userns_netns`] applies to itself.
/// `CLONE_NEWUSER` first is what makes the other two legal without host
/// privilege; the kernel grants this process capabilities *in the new* user
/// namespace.
///
/// Failure is fatal by construction: a failed `unshare` aborts the spawn and
/// surfaces as [`SpawnError::Create`] in the parent -- the exec never happens,
/// so no child ever exists outside the namespace policy demanded.
///
/// The network namespace alone denies egress (no interfaces, nothing to route
/// through); the worker still applies Landlock + seccomp itself once running,
/// which is what [`crate::linux::confine_worker`] verifies and the receipt
/// records.
///
/// # Errors
///
/// See [`SpawnError`].
#[cfg(target_os = "linux")]
pub fn spawn_piped_namespaced(
    program: &Path,
    args: &[&str],
    memory_cap: Option<u64>,
) -> Result<PipedChild, SpawnError> {
    use std::os::unix::process::CommandExt;
    let mut cmd = piped_command(program, args)?;
    // SAFETY: runs once in the forked child before exec. `unshare` takes one
    // flag argument, affects only the calling process, and on failure leaves
    // everything unchanged; the flags are compile-time constants, so the
    // closure captures nothing and allocates nothing. A failure here must be
    // fatal: spawning a worker that escaped the namespace policy demanded is
    // strictly worse than not spawning it.
    unsafe {
        cmd.pre_exec(|| {
            if unshare(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWNS) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    if let Some(bytes) = memory_cap {
        apply_rlimit_as(&mut cmd, bytes);
    }
    let child = cmd.spawn().map_err(|source| SpawnError::Create {
        program: program.display().to_string(),
        source,
    })?;
    Ok(PipedChild { child })
}

/// Spawn `program` with pipes to its stdin and stdout, no resource cap.
///
/// Kept for callers that have no limits to apply yet. Production goes through
/// [`spawn_piped_limited`].
///
/// # Errors
///
/// See [`SpawnError`].
pub fn spawn_piped(program: &Path, args: &[&str]) -> Result<PipedChild, SpawnError> {
    spawn_piped_limited(program, args, None)
}

impl PipedChild {
    /// The write end of the child's stdin, while the session is open.
    pub fn stdin(&mut self) -> Option<&mut std::process::ChildStdin> {
        self.child.stdin.as_mut()
    }

    /// The read end of the child's stdout.
    pub fn stdout(&mut self) -> Option<&mut std::process::ChildStdout> {
        self.child.stdout.as_mut()
    }

    /// Close the child's stdin, which is how a session is ended cleanly.
    pub fn close_stdin(&mut self) {
        self.child.stdin = None;
    }

    /// Wait for the child to exit and return its code.
    ///
    /// # Errors
    ///
    /// Any underlying OS failure waiting.
    pub fn wait(&mut self) -> std::io::Result<u32> {
        self.close_stdin();
        let status = self.child.wait()?;
        // A signalled child has no exit code. Reporting 0 there would turn
        // "killed by the kernel's memory killer" into "succeeded", which is
        // the one answer the host must never receive.
        Ok(status.code().map_or(u32::MAX, |c| c as u32))
    }
}

impl Drop for PipedChild {
    fn drop(&mut self) {
        self.close_stdin();
        let _ = self.child.wait();
    }
}

/// Escape tests for the namespaced spawn. Linux only, like the mechanism.
///
/// The pattern is the same fork-isolation discipline [`crate::linux`] uses,
/// adapted for a *spawned* child: the parent binds a TCP listener in the
/// host's network namespace, then asks whether a namespaced child can reach
/// it. The listener is the control as well as the target — an unnamespaced
/// child demonstrably connects to it first, so a failure from the namespaced
/// child is attributable to the isolation and not to a dead port.
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn host_netns_identity() -> String {
        std::fs::read_link("/proc/self/ns/net")
            .expect("/proc/self/ns/net must be readable")
            .to_string_lossy()
            .into_owned()
    }

    /// The listener every test in this module aims at: bound on an ephemeral
    /// port in THIS (host) network namespace.
    fn host_listener() -> TcpListener {
        TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0") // openconvert-lint: allow -- linux-test-only loopback control for netns escape attribution; compiled out of every non-test binary
    }

    /// **The namespaced child is in a different network namespace, and cannot
    /// reach the host's listener.**
    ///
    /// Two read-backs, because each alone could mislead:
    ///
    /// - `/proc/self/ns/net` identity differs from the parent's — but only the
    ///   connect attempt below proves the difference means anything.
    /// - `bash /dev/tcp` connect fails — but only the identity read proves it
    ///   failed because of isolation rather than a broken target.
    ///
    /// The control at the end ties them together: an ordinary child reaches
    /// the very same listener, so the namespaced child's failure is the
    /// finding.
    #[test]
    fn namespaced_child_cannot_reach_the_hosts_listener() {
        if !crate::linux::userns_netns_available() {
            // This machine refuses unshare entirely; the fatal-failure
            // contract is what the next test asserts here instead.
            return;
        }
        let listener = host_listener();
        let port = listener.local_addr().unwrap().port();

        // Identity read-back through the child's own stdout.
        let mut ident = spawn_piped_namespaced(
            Path::new("/bin/sh"),
            &["-c", "readlink /proc/self/ns/net"],
            None,
        )
        .expect("namespaced spawn where unshare probes available");
        let mut out = String::new();
        ident.stdout().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(ident.wait().unwrap(), 0, "readlink child failed");
        let child_id = out.trim();
        assert!(
            child_id.starts_with("net:["),
            "not a netns identity string: {child_id:?}"
        );
        assert_ne!(
            host_netns_identity(),
            child_id,
            "child reports the HOST network namespace"
        );

        // The escape attempt: connect back to the parent's listener, which
        // lives outside the child's (empty) network namespace. Both network
        // attempts run under `timeout(1)` so a kernel that neither accepts
        // nor refuses can hang one test, not the suite; timeout passes the
        // child's exit code through and reports a kill as 124 -- nonzero
        // either way.
        let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");
        let attempt = ["10", "/bin/bash", "-c", script.as_str()];
        let mut escapee = spawn_piped_namespaced(Path::new("/usr/bin/timeout"), &attempt, None)
            .expect("namespaced spawn");
        let code = escapee.wait().unwrap();
        assert_ne!(
            code, 0,
            "the namespaced child REACHED a socket outside its namespace"
        );

        // Control: without namespaces, the identical attempt succeeds, and the
        // listener really was accepting connections this whole time.
        let mut control = spawn_piped_limited(Path::new("/usr/bin/timeout"), &attempt, None)
            .expect("plain spawn");
        assert_eq!(control.wait().unwrap(), 0, "control connect failed");
        listener
            .accept()
            .expect("the control connection reached us");
    }

    /// **Failure is fatal: where unshare is blocked, nothing is spawned.**
    ///
    /// On a policy-blocked machine (default Docker), the pre_exec hook fails
    /// with EPERM and that error surfaces here as `SpawnError::Create` — there
    /// is no child process running unnamespaced, which is the whole contract.
    #[test]
    fn namespaced_spawn_fails_fatal_where_unshare_is_blocked() {
        if crate::linux::userns_netns_available() {
            return; // unshare works here; the previous test covers this path's complement
        }
        let err = spawn_piped_namespaced(Path::new("/bin/true"), &[], None).unwrap_err();
        assert!(
            matches!(err, SpawnError::Create { .. }),
            "expected Create, got {err:?}"
        );
    }

    /// The memory cap and the namespace escalation coexist: both pre-exec
    /// hooks run, and the capped namespaced child still runs its script.
    #[test]
    fn memory_cap_survives_the_namespace_transition() {
        if !crate::linux::userns_netns_available() {
            return;
        }
        let mut child =
            spawn_piped_namespaced(Path::new("/bin/sh"), &["-c", "echo ok"], Some(512 << 20))
                .expect("capped namespaced spawn");
        let mut out = String::new();
        child.stdout().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(child.wait().unwrap(), 0);
        assert_eq!(out.trim(), "ok");
    }

    /// NUL anywhere in the command line is refused before any process exists,
    /// on both spawn paths. A NUL is where the string Rust validated stops
    /// being the string the OS executes; truncation is not an option.
    ///
    /// The namespaced variant errors during validation, before its pre_exec
    /// hook ever runs, so this holds even where unshare is blocked.
    #[test]
    fn nul_in_the_command_line_is_refused() {
        assert!(matches!(
            spawn_piped_limited(Path::new("/bin/sh\0"), &[], None),
            Err(SpawnError::NulInCommandLine)
        ));
        assert!(matches!(
            spawn_piped_limited(Path::new("/bin/sh"), &["ok", "\0bad"], None),
            Err(SpawnError::NulInCommandLine)
        ));
        assert!(matches!(
            spawn_piped_namespaced(Path::new("/bin/sh\0"), &[], None),
            Err(SpawnError::NulInCommandLine)
        ));
        assert!(matches!(
            spawn_piped_namespaced(Path::new("/bin/sh"), &["ok", "\0bad"], None),
            Err(SpawnError::NulInCommandLine)
        ));
    }
}
