//! Linux confinement: Landlock filesystem denial and seccomp network denial.
//!
//! This is the Linux half of the isolation ladder (`03` Â§9.2), and it exists
//! because two spikes measured exactly these mechanisms on a real kernel and
//! both held:
//!
//! - **S12** â€” Landlock confines an unprivileged process with **no user
//!   namespace**. The `Reduced` tier's central claim.
//! - **S10** â€” a seccomp filter denying `AF_INET`/`AF_INET6` at `socket(2)`
//!   while permitting `AF_UNIX` does not break real engine libraries, which
//!   open Unix sockets through glib/fontconfig/NSS.
//!
//! # Why the worker denies ALL filesystem access, not "everything but the job
//! directory"
//!
//! The design specifies a job-directory handle and writes via
//! `openat(fd 5, â€¦)`. This protocol does not need it: bytes arrive on stdin
//! and leave on stdout, so the worker opens no file, creates no file, reads no
//! directory. The strongest policy that matches that reality is a ruleset that
//! **handles every filesystem right and adds zero allow rules** â€” everything
//! is denied with `EACCES`, including paths we never thought to enumerate.
//! An allow-list of one directory is weaker than proving no directory is
//! needed at all.
//!
//! # Read back, not requested (I11, `03` Â§9.5)
//!
//! Applying a mechanism returns success or an error; it does not prove the
//! mechanism *engaged*. So after applying, this module attempts the forbidden
//! operations and reports what actually failed â€” the same discipline the
//! Windows side applies to ACG/CIG, where the request-versus-reality error was
//! measured running in both directions (S17b, S20b).
//!
//! The seccomp check carries a subtlety worth stating: a kernel configured
//! without IPv4 also refuses `socket(AF_INET)`, which would look identical to
//! our filter working. The engagement verdict therefore requires the socket to
//! have **succeeded before** the filter was applied and failed after. A denial
//! we cannot attribute to ourselves is not claimed as confinement â€” reporting
//! it would be the optimistic direction, which is the only direction that has
//! ever been wrong in this project.

#![deny(missing_docs)]

use std::fmt;

extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn unshare(flags: i32) -> i32;
    fn prctl(option: i32, a2: u64, a3: u64, a4: u64, a5: u64) -> i32;
    fn __errno_location() -> *mut i32;
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn close(fd: i32) -> i32;
}

fn errno() -> i32 {
    // SAFETY: `__errno_location` takes no arguments and always succeeds.
    unsafe { *__errno_location() }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why self-confinement could not be applied.
#[derive(Debug)]
pub enum ConfineError {
    /// Landlock is absent: kernel < 5.13, `CONFIG_SECURITY_LANDLOCK=n`, or not
    /// in the `lsm=` list. Without it there is **no filesystem containment**
    /// on Linux â€” measured, not assumed: `openat(fd, "..")` escapes on this
    /// platform (spike S12c), which is the empirical case for refusing rather
    /// than degrading.
    LandlockUnavailable(i32),
    /// The ruleset could not be created or applied.
    LandlockApply {
        /// Which step of the application failed.
        step: &'static str,
        /// The raw errno.
        errno: i32,
    },
    /// The syscall filter could not be installed.
    SeccompApply {
        /// Which step of the installation failed.
        step: &'static str,
        /// The raw errno.
        errno: i32,
    },
    /// The user/network namespace could not be created. Reported by the
    /// escalation path, which treats failure as "Reduced tier" â€” never as a
    /// refusal, because the Reduced floor stands on its own.
    NetNsApply {
        /// The raw errno (EPERM: policy/nesting; ENOSPC: limits).
        errno: i32,
    },
    /// `confine_worker` was called from a multithreaded process, and was
    /// refused rather than degraded.
    ///
    /// Both mechanisms bind the calling thread only â€” Landlock's
    /// `restrict_self` and a seccomp filter without TSYNC are per-thread â so
    /// confining one thread of many would issue a receipt its siblings never
    /// earned. `unshare(CLONE_NEWUSER)` additionally refuses multithreaded
    /// callers outright (EINVAL), which would otherwise look identical to
    /// host policy and silently downgrade the tier. Refusing names the
    /// mistake; degrading hides it.
    WorkerNotSingleThreaded {
        /// The thread count that was refused.
        threads: i32,
    },
}

impl fmt::Display for ConfineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LandlockUnavailable(e) => write!(
                f,
                "Landlock is unavailable here (errno {e}); this kernel offers \
                 no filesystem confinement"
            ),
            Self::LandlockApply { step, errno } => {
                write!(f, "Landlock failed at {step} (errno {errno})")
            }
            Self::SeccompApply { step, errno } => {
                write!(f, "seccomp failed at {step} (errno {errno})")
            }
            Self::NetNsApply { errno } => {
                write!(
                    f,
                    "the user/network namespace could not be created \
                     (errno {errno}); running at the Reduced tier"
                )
            }
            Self::WorkerNotSingleThreaded { threads } => write!(
                f,
                "confinement binds the calling thread only and this process \
                 has {threads} threads; confine a freshly spawned \
                 single-threaded worker"
            ),
        }
    }
}

impl std::error::Error for ConfineError {}

// ---------------------------------------------------------------------------
// Landlock
// ---------------------------------------------------------------------------

const SYS_LANDLOCK_CREATE_RULESET: i64 = 444;
const SYS_LANDLOCK_RESTRICT_SELF: i64 = 446;
const LANDLOCK_CREATE_RULESET_VERSION: u64 = 1 << 0;

#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

// ABI 1 filesystem access bits.
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
const FS_REFER: u64 = 1 << 13; // ABI 2
const FS_TRUNCATE: u64 = 1 << 14; // ABI 3

/// The Landlock ABI this kernel implements, or â‰¤ 0 with the errno.
///
/// A version query: creates nothing, restricts nothing, and is safe to call
/// speculatively. The answer must be negotiated at runtime because passing an
/// access bit the kernel does not know is `EINVAL` â€” the tested WSL2 kernel
/// reported **ABI 3**, not the then-current 4 (spike S10b), which is why
/// "kernel â‰¥ 5.13" is necessary but not sufficient.
pub fn landlock_abi() -> Result<i64, i32> {
    // SAFETY: the documented version query â€” null attr, zero size, VERSION
    // flag. On failure it returns a negative value and sets errno.
    let v = unsafe {
        syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            core::ptr::null::<LandlockRulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if v < 0 {
        Err(errno())
    } else {
        Ok(v)
    }
}

/// Whether a fresh user+network namespace can actually be created here.
///
/// Fork-probed like the seccomp filter: `unshare` is one-way for the
/// process that calls it, so a forked child attempts the REAL escalation and
/// reports by exit code. The parent may be multithreaded, so between fork
/// and `_exit` the child issues syscalls only.
#[must_use]
pub fn userns_netns_available() -> bool {
    // SAFETY: identical discipline to `seccomp_network_deny_available`.
    let pid = unsafe { fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        let code = match apply_userns_netns() {
            Ok(()) => 0,
            Err(_) => 2,
        };
        // SAFETY: `_exit` skips destructors by design in the forked child.
        unsafe { _exit(code) };
    }
    let mut status: i32 = 0;
    // SAFETY: our own child; live local for the kernel to write.
    unsafe {
        waitpid(pid, &mut status, 0);
    }
    libc_wifexited(status) && libc_wexitstatus(status) == 0
}

/// Whether this kernel offers Landlock at all.
#[must_use]
pub fn landlock_available() -> bool {
    landlock_abi().is_ok()
}

/// Handled-access set, masked to what this kernel's ABI understands.
fn handled_for(abi: i64) -> u64 {
    let mut bits = FS_EXECUTE
        | FS_WRITE_FILE
        | FS_READ_FILE
        | FS_READ_DIR
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_CHAR
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_MAKE_SOCK
        | FS_MAKE_FIFO
        | FS_MAKE_BLOCK
        | FS_MAKE_SYM;
    if abi >= 2 {
        bits |= FS_REFER;
    }
    if abi >= 3 {
        bits |= FS_TRUNCATE;
    }
    bits
}

const PR_SET_NO_NEW_PRIVS: i32 = 38;

/// Apply a Landlock ruleset that handles every filesystem right and adds **no
/// allow rules**: from this point the process can neither read nor create nor
/// modify anything on any filesystem.
///
/// Already-open descriptors are untouched â€” stdin and stdout keep working,
/// which is exactly why this protocol can afford total denial. `no_new_privs`
/// is set first, which is the unprivileged path to `restrict_self`.
///
/// Returns the ABI the kernel reported, for the receipt.
///
/// # Errors
///
/// [`ConfineError::LandlockUnavailable`] when the kernel has no Landlock, and
/// [`ConfineError::LandlockApply`] when the ruleset cannot be built or applied.
pub fn apply_landlock_all_fs_denied() -> Result<i64, ConfineError> {
    let abi = landlock_abi().map_err(ConfineError::LandlockUnavailable)?;
    let attr = LandlockRulesetAttr {
        handled_access_fs: handled_for(abi),
    };
    // SAFETY: `attr` is a well-formed struct of the exact size the kernel
    // expects for this flags value (0 means "a ruleset", so the attr pointer
    // and size are both consumed).
    let rs = unsafe {
        syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u64,
        )
    };
    if rs < 0 {
        return Err(ConfineError::LandlockApply {
            step: "create_ruleset",
            errno: errno(),
        });
    }
    // SAFETY: `PR_SET_NO_NEW_PRIVS` takes one value argument; setting it is
    // required before `restrict_self` without CAP_SYS_ADMIN, and it is the
    // documented precondition for the Reduced tier (`03` Â§9.2).
    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(ConfineError::LandlockApply {
            step: "no_new_privs",
            errno: errno(),
        });
    }
    // SAFETY: `rs` is a ruleset fd returned by the call above and still open;
    // the flags argument is 0 as documented.
    let r = unsafe { syscall(SYS_LANDLOCK_RESTRICT_SELF, rs, 0u64) };
    let e = errno();
    // SAFETY: `rs` is an fd this function owns and closes exactly once, on
    // both paths.
    unsafe {
        close(rs as i32);
    }
    if r != 0 {
        return Err(ConfineError::LandlockApply {
            step: "restrict_self",
            errno: e,
        });
    }
    Ok(abi)
}

// ---------------------------------------------------------------------------
// seccomp â€” network denial at the syscall filter
// ---------------------------------------------------------------------------

/// One BPF instruction, verbatim from the spike that proved the filter works
/// against real engines (spike S10).
#[repr(C)]
#[derive(Clone, Copy)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JMP_JEQ_K: u16 = 0x15;
const BPF_RET_K: u16 = 0x06;

const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;

// struct seccomp_data { int nr; __u32 arch; __u64 ip; __u64 args[6]; }
const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;
const OFF_ARG0: u32 = 16;

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
// Deliberately kept even though only one is live per build: the fact that the
// compiler reports the other as dead code IS the finding. Both constants and
// the syscall number below change with the target architecture, which is why
// TARGETS.md treats architecture as load-bearing rather than a packaging
// detail.
#[allow(dead_code)]
const AUDIT_ARCH_AARCH64: u32 = 0xc000_00b7;

// Listed for the record: the filter permits everything it does not name, so
// AF_UNIX survives by construction and never appears in the program. Writing
// it down is what stops a future "tightening" from silently breaking the Unix
// sockets engines open.
#[allow(dead_code)]
const AF_UNIX: u32 = 1;
const AF_INET: u32 = 2;
const AF_INET6: u32 = 10;
const EAFNOSUPPORT: u32 = 97;

const PR_SET_SECCOMP: i32 = 22;
const SECCOMP_MODE_FILTER: u64 = 2;

#[cfg(target_arch = "x86_64")]
const NR_SOCKET: u32 = 41;
#[cfg(target_arch = "aarch64")]
const NR_SOCKET: u32 = 198;

#[cfg(target_arch = "x86_64")]
const MY_ARCH: u32 = AUDIT_ARCH_X86_64;
#[cfg(target_arch = "aarch64")]
const MY_ARCH: u32 = AUDIT_ARCH_AARCH64;

/// Deny `AF_INET` and `AF_INET6` at `socket(2)`; permit everything else.
///
/// **The `arch` check at index 1 is load-bearing.** Without it, a process can
/// invoke another ABI where the same syscall numbers mean different syscalls
/// and walk straight past a filter written for its own architecture â€” the
/// textbook bypass, and the reason CPU architecture appears in `TARGETS.md`
/// rather than being left implicit.
///
/// `AF_UNIX` stays permitted deliberately: engines routinely open Unix
/// sockets through glib, fontconfig and NSS, and a filter that breaks those
/// breaks the engine along with the network (spike S10).
fn build_network_deny_filter() -> Vec<SockFilter> {
    //  0  load arch
    //  1  if arch != MY_ARCH -> kill (index 9)
    //  2  load nr
    //  3  if nr != socket    -> allow (index 7)
    //  4  load args[0] (domain)
    //  5  if domain == AF_INET  -> deny (index 8)
    //  6  if domain == AF_INET6 -> deny (index 8)
    //  7  allow
    //  8  deny with EAFNOSUPPORT
    //  9  kill process
    let stmt = |code: u16, k: u32| SockFilter {
        code,
        jt: 0,
        jf: 0,
        k,
    };
    let jump = |code: u16, k: u32, jt: u8, jf: u8| SockFilter { code, jt, jf, k };
    vec![
        stmt(BPF_LD_W_ABS, OFF_ARCH),
        jump(BPF_JMP_JEQ_K, MY_ARCH, 0, 7),
        stmt(BPF_LD_W_ABS, OFF_NR),
        jump(BPF_JMP_JEQ_K, NR_SOCKET, 0, 3),
        stmt(BPF_LD_W_ABS, OFF_ARG0),
        jump(BPF_JMP_JEQ_K, AF_INET, 2, 0),
        jump(BPF_JMP_JEQ_K, AF_INET6, 1, 0),
        stmt(BPF_RET_K, SECCOMP_RET_ALLOW),
        stmt(BPF_RET_K, SECCOMP_RET_ERRNO | EAFNOSUPPORT),
        stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
    ]
}

/// Install a prebuilt filter into **this** process.
///
/// Split from [`apply_seccomp_network_deny`] so a forked probe child can run
/// the install without allocating: between `fork` and `_exit` only
/// async-signal-safe calls are made, because the parent may be multithreaded
/// and a `malloc` lock held elsewhere would deadlock the child mid-install.
///
/// # Errors
///
/// [`ConfineError::SeccompApply`] naming the failing step.
fn install_prebuilt_filter(prog: &[SockFilter]) -> Result<(), ConfineError> {
    let fprog = SockFprog {
        len: prog.len() as u16,
        filter: prog.as_ptr(),
    };
    // SAFETY: `PR_SET_NO_NEW_PRIVS` takes one value argument and is the
    // precondition for installing a filter without CAP_SYS_ADMIN.
    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(ConfineError::SeccompApply {
            step: "no_new_privs",
            errno: errno(),
        });
    }
    // SAFETY: `SECCOMP_MODE_FILTER` consumes a pointer to a `sock_fprog`
    // whose `filter` array outlives the call â€” `prog` borrows a slice the
    // caller keeps alive across this call, and the kernel copies the program
    // during the syscall itself.
    if unsafe {
        prctl(
            PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &fprog as *const SockFprog as u64,
            0,
            0,
        )
    } != 0
    {
        return Err(ConfineError::SeccompApply {
            step: "set_seccomp",
            errno: errno(),
        });
    }
    Ok(())
}

/// Apply the network-deny filter to this process.
///
/// # Errors
///
/// [`ConfineError::SeccompApply`].
pub fn apply_seccomp_network_deny() -> Result<(), ConfineError> {
    install_prebuilt_filter(&build_network_deny_filter())
}

// ---------------------------------------------------------------------------
// Engagement checks â€” asked of the OS, after applying
// ---------------------------------------------------------------------------

/// Whether `socket(AF_INET)` currently succeeds, and its errno when it fails.
fn inet_socket_outcome() -> (bool, i32) {
    const SOCK_STREAM: i32 = 1;
    // SAFETY: plain syscall wrapper; the descriptor is closed on success and
    // nothing else observes it.
    let fd = unsafe { socket(AF_INET as i32, SOCK_STREAM, 0) };
    if fd >= 0 {
        // SAFETY: `fd` came from the call above and is closed exactly once.
        unsafe {
            close(fd);
        }
        (true, 0)
    } else {
        (false, errno())
    }
}

/// Whether creating a file currently succeeds, and its error kind when not.
fn file_create_outcome() -> (bool, Option<std::io::ErrorKind>) {
    let path = std::env::temp_dir().join(format!(".openconvert-probe-{}", std::process::id()));
    match std::fs::File::create_new(&path) {
        Ok(_) => {
            // We just littered temp. Clean up, and report honestly: creation
            // worked, so filesystem confinement did NOT engage.
            let _ = std::fs::remove_file(&path);
            (true, None)
        }
        Err(e) => (false, Some(e.kind())),
    }
}

/// Whether reading a world-readable file currently succeeds, and its error
/// kind when not.
///
/// The write probe needs a writable temp directory to mean anything, which
/// read-only-root hosts do not offer â€” there the write verdict would report
/// an unattributable `false` even with the ruleset fully engaged. The read
/// probe needs only a file that exists, so between them the two probes
/// attribute a denial on machines where either alone could not.
fn file_read_outcome() -> (bool, Option<std::io::ErrorKind>) {
    match std::fs::File::open("/etc/hostname") {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.kind())),
    }
}

/// The number of threads in this process, from `/proc/self/status`, or `None`
/// where procfs is unavailable.
fn process_thread_count() -> Option<i32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Threads:"))?;
    line["Threads:".len()..].trim().parse().ok()
}

/// The receipt shape, as a pure function of measured facts.
///
/// Split from [`confine_worker`] so every branch is unit-testable without
/// fork or namespaces:
///
/// - **not applied** -- omitted entirely: Reduced reads as exactly two
///   entries; nothing was attempted here.
/// - **applied and verified** -- present as `true`.
/// - **applied but unverifiable** (procfs unreadable) -- present as `false`:
///   the mechanism happened and the proof did not. That is honest fail-closed
///   reporting, deliberately unlike the omission above.
fn confinement_pairs(
    landlock_engaged: bool,
    seccomp_engaged: bool,
    ns_applied: bool,
    netns_verified: bool,
) -> Vec<(String, bool)> {
    let mut pairs = vec![
        ("Landlock".to_string(), landlock_engaged),
        ("Seccomp".to_string(), seccomp_engaged),
    ];
    if ns_applied {
        pairs.push(("NetNs".to_string(), netns_verified));
    }
    pairs
}

/// Apply both mechanisms to **this process**, then verify each by attempting
/// the operation it forbids.
///
/// The returned pairs feed the worker's `Ready` message and, through it, the
/// receipt. Every `true` here was earned by watching the OS refuse something
/// this process just tried; every unverifiable claim reports `false` instead.
///
/// The escalation is reported by **presence**, not by a `false` flag: when
/// `unshare` fails (EPERM under policy, ENOSPC at limits) the `NetNs` pair is
/// omitted entirely, so the Reduced tier reads as exactly two entries and the
/// Full tier as exactly three. A receipt that lists what did not happen is
/// noise; one that lists only what was applied, verified, is evidence.
///
/// # Why the caller treats `Err` as fatal
///
/// A machine where either application fails is a machine whose planning
/// profile should already have refused sandboxed steps â€” the probe reports
/// the same facts non-destructively. Reaching this error anyway means the
/// machine changed under us, and converting an attacker-supplied file in a
/// process that failed to confine itself is strictly worse than refusing.
///
/// A multithreaded caller is refused for the same reason in the other
/// direction: the mechanisms bind the calling thread, so a partial
/// confinement would produce a receipt that lies by omission.
///
/// # Errors
///
/// [`ConfineError`] -- see above.
pub fn confine_worker() -> Result<Vec<(String, bool)>, ConfineError> {
    // Refuse the one caller mistake that would otherwise masquerade as host
    // policy: a second thread means per-thread mechanisms cannot cover the
    // process, and unshare(CLONE_NEWUSER) failing EINVAL here is visually
    // identical to an EPERM from a blocking seccomp profile. If procfs is
    // unavailable the check cannot run and we proceed as before â€” the
    // escalation read-back below still reports what actually engaged.
    if let Some(threads) = process_thread_count() {
        if threads > 1 {
            return Err(ConfineError::WorkerNotSingleThreaded { threads });
        }
    }

    // Baselines FIRST. Each engagement verdict needs the before-state to mean
    // anything: a socket refused by the kernel's own configuration is not a
    // refusal our filter produced.
    let fs_before = file_create_outcome();
    let rd_before = file_read_outcome();
    let net_before = inet_socket_outcome();
    let ns_before = netns_identity();

    // The Full tier FIRST, because `unshare(CLONE_NEWUSER)` resets capability
    // state and must happen before anything that depends on it. Failure is
    // NOT fatal: the Reduced tier below satisfies the default floor on its
    // own (`03` Â§9.2), and an unavailable escalation is reported as absent â€”
    // never as a refusal.
    let ns_applied = apply_userns_netns().is_ok();

    let _abi = apply_landlock_all_fs_denied()?;
    apply_seccomp_network_deny()?;

    // Read back by attempting what should now fail.
    let fs_after = file_create_outcome();
    let rd_after = file_read_outcome();
    let net_after = inet_socket_outcome();
    let ns_after = netns_identity();

    // Either forbidden filesystem operation going from possible to denied
    // attributes the denial to the ruleset; each probe alone can be
    // inconclusive (a read-only root vacates the write probe).
    let landlock_engaged = (!fs_after.0 && fs_before.0) || (!rd_after.0 && rd_before.0);
    let seccomp_engaged = !net_after.0 && net_before.0;
    // A NEW network namespace is proven by its identity differing from the
    // namespace we were born into â€” read back from /proc, in THIS process,
    // after the unshare. The empty-netns claim and the seccomp claim are
    // independent mechanisms; both may be true at once (Full) or seccomp
    // alone (Reduced), and the receipt names what actually happened.
    let netns_engaged = ns_applied && ns_before.is_some() && ns_before != ns_after;

    Ok(confinement_pairs(
        landlock_engaged,
        seccomp_engaged,
        ns_applied,
        netns_engaged,
    ))
}

/// Move this process into a fresh user namespace + empty network namespace:
/// the `Full` tier's privilege drop and total egress denial.
///
/// Order is load-bearing: `CLONE_NEWUSER` first grants this process
/// capabilities IN THE NEW namespace, which is exactly what makes the
/// following `CLONE_NEWNET` legal without privilege on the host. Reversing
/// them fails with EPERM on an unprivileged process.
///
/// Already inside a container? The same call still works when the kernel
/// permits nested namespaces and refuses when it does not â€” either way the
/// caller learns the truth from the return value plus read-back, never from
/// optimism.
///
/// # Errors
///
/// [`ConfineError::NetNsApply`] with the raw errno (EPERM: policy or nesting
/// limit; EINVAL: flag combination; ENOSPC: namespace limit exhausted).
pub fn apply_userns_netns() -> Result<(), ConfineError> {
    const CLONE_NEWNS: i32 = 0x0002_0000;
    const CLONE_NEWUSER: i32 = 0x1000_0000;
    const CLONE_NEWNET: i32 = 0x4000_0000;
    // SAFETY: `unshare` takes one flag argument and affects only THIS
    // process. Failure leaves everything unchanged.
    let r = unsafe { unshare(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWNS) };
    if r != 0 {
        return Err(ConfineError::NetNsApply { errno: errno() });
    }
    Ok(())
}

/// The `/proc/self/ns/net` identity string (e.g. `net:[4026531840]`), or
/// `None` if unreadable.
fn netns_identity() -> Option<String> {
    std::fs::read_link("/proc/self/ns/net")
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Probes â€” non-destructive answers for `available::probe()`
// ---------------------------------------------------------------------------

extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn _exit(code: i32) -> !;
}

/// Whether a network-denying seccomp filter can actually be installed here.
///
/// **Probed by doing it**, in a forked child â€” the same rule the Job Object
/// and AppContainer probes follow on Windows: a capability is what the machine
/// grants now, not what its version implies. Installing a filter is one-way,
/// so it must not happen in the probing process itself; the child applies the
/// real filter, attempts the denied operation, and reports by exit code.
///
/// Between `fork` and `_exit` the child performs **syscalls only** â€” the BPF
/// program is built before the fork, because the parent may be multithreaded
/// and a `malloc` lock held elsewhere at fork time would deadlock a child that
/// allocated.
#[must_use]
pub fn seccomp_network_deny_available() -> bool {
    let prog = build_network_deny_filter();
    // SAFETY: `fork` takes no arguments. The child branch below issues only
    // async-signal-safe calls â€” two prctls, one socket, `_exit` â€” over data
    // built before the fork, then never returns. The parent falls through to
    // `waitpid`.
    let pid = unsafe { fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        // Child. Exit codes: 0 = installed AND denied; 1 = installed but the
        // socket still worked (a denial we cannot claim); 2 = install failed.
        let code = match install_prebuilt_filter(&prog) {
            Err(_) => 2,
            Ok(()) => {
                if inet_socket_outcome().0 {
                    1
                } else {
                    0
                }
            }
        };
        // SAFETY: `_exit` skips atexit handlers and destructors, which is the
        // point â€” nothing of the parent's state is touched on the way out.
        unsafe { _exit(code) };
    }
    let mut status: i32 = 0;
    // SAFETY: `pid` is the child this call just returned; `status` is a live
    // local for the kernel to write.
    unsafe {
        waitpid(pid, &mut status, 0);
    }
    // A signal-killed child reports 0 in the low byte only by coincidence of
    // encoding; require clean exit explicitly.
    libc_wifexited(status) && libc_wexitstatus(status) == 0
}

fn libc_wifexited(status: i32) -> bool {
    status & 0x7f == 0
}

fn libc_wexitstatus(status: i32) -> i32 {
    (status >> 8) & 0xff
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Every test here that applies confinement runs in a **forked child**.
///
/// Both mechanisms are one-way for the process that applies them, and cargo
/// runs the tests of one binary in one process on many threads. Confining the
/// harness would deny filesystem access to every sibling test mid-run. So the
/// pattern is always the same: fork, confine, verify by attempting the
/// forbidden thing, report by exit code, assert from the parent.
#[cfg(test)]
mod tests {
    use super::*;

    /// Run `child` in a forked copy and assert it exited 0.
    ///
    /// The child closure must issue syscalls only after the fork boundary's
    /// allocation work is done â€” see [`seccomp_network_deny_available`] for
    /// why.
    fn child_exit_zero(child: impl FnOnce()) -> bool {
        // SAFETY: as documented on `seccomp_network_deny_available`. The
        // closure performs no allocation after this point.
        let pid = unsafe { fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            child();
            unsafe { _exit(0) };
        }
        let mut status: i32 = 0;
        // SAFETY: `pid` is our child; `status` is a live local.
        unsafe {
            waitpid(pid, &mut status, 0);
        }
        libc_wifexited(status) && libc_wexitstatus(status) == 0
    }

    /// This kernel reports a Landlock ABI â€” the precondition for the whole
    /// Reduced tier. On a pre-5.13 kernel this fails, and that failure is the
    /// finding, not a flake.
    #[test]
    fn landlock_reports_an_abi_on_supported_kernels() {
        match landlock_abi() {
            Ok(v) => assert!(v >= 1, "ABI {v} is not a version"),
            Err(e) => panic!(
                "landlock unavailable (errno {e}) â€” on this kernel there is no \
                 filesystem confinement, and the planning profile must refuse"
            ),
        }
    }

    /// **The escape test.** After self-confinement, both forbidden operations
    /// fail: an outbound IPv4 socket and a file creation.
    #[test]
    fn confinement_denies_the_filesystem_and_the_network() {
        assert!(seccomp_network_deny_available(), "the filter must install");
        assert!(
            child_exit_zero(|| {
                let pairs = confine_worker().expect("confine");
                let get = |n: &str| {
                    pairs
                        .iter()
                        .find(|(name, _)| name == n)
                        .map(|(_, e)| *e)
                        .unwrap_or(false)
                };
                assert!(get("Landlock"), "filesystem write was not denied");
                assert!(get("Seccomp"), "outbound socket was not denied");
            }),
            "the confined child escaped"
        );
    }

    /// The control: WITHOUT confinement, this process can still create files.
    ///
    /// Without it, the test above is satisfied by a machine where writes are
    /// impossible anyway â€” which would make the escape test vacuous.
    #[test]
    fn without_confinement_a_file_can_be_created() {
        let (ok, kind) = file_create_outcome();
        assert!(
            ok,
            "baseline create failed with {kind:?}; the escape test above would prove nothing"
        );
    }

    /// The control for the socket half: an ordinary process may open sockets.
    #[test]
    fn without_confinement_an_inet_socket_can_be_created() {
        let (ok, e) = inet_socket_outcome();
        assert!(ok, "baseline AF_INET socket refused (errno {e}); the seccomp verdict would be unverifiable here");
    }

    /// The control for the read probe: an ordinary process may read files.
    ///
    /// Without it, `file_read_outcome` could not attribute a post-confinement
    /// denial on hosts whose temp directories were never writable.
    #[test]
    fn without_confinement_a_file_can_be_read() {
        let (ok, kind) = file_read_outcome();
        assert!(
            ok,
            "baseline open(/etc/hostname) failed with {kind:?}; the read half of \
             the filesystem verdict would be unverifiable here"
        );
    }

    // -----------------------------------------------------------------------
    // Receipt shape, as a pure function
    // -----------------------------------------------------------------------

    /// Not applied -> omitted entirely. Reduced is exactly two entries; there
    /// is no `("NetNs", _)` entry at all, whatever its value would be.
    #[test]
    fn receipt_omits_netns_when_not_applied() {
        let pairs = confinement_pairs(true, true, false, true);
        assert_eq!(pairs.len(), 2);
        assert!(!pairs.iter().any(|(n, _)| n == "NetNs"));
    }

    /// Applied and verified -> present and true. Full is exactly three.
    #[test]
    fn receipt_reports_verified_netns_as_true() {
        let pairs = confinement_pairs(true, true, true, true);
        assert_eq!(pairs.len(), 3);
        assert!(pairs.iter().all(|(_, e)| *e));
    }

    /// Applied but unverifiable -> present as **false**.
    ///
    /// This pair is deliberately unlike the omission above: the mechanism
    /// happened, the proof did not, and fail-closed means reporting the
    /// mechanism as unproven rather than pretending it away. A procfs-less
    /// host is where this branch lives; it must be visible in a test even if
    /// no CI machine ever reaches it through [`confine_worker`] itself.
    #[test]
    fn receipt_reports_applied_but_unverifiable_netns_as_false() {
        let pairs = confinement_pairs(true, true, true, false);
        assert_eq!(pairs.len(), 3);
        let netns = pairs.iter().find(|(n, _)| n == "NetNs").expect("present");
        assert!(!netns.1, "unverified escalation must read false");
    }

    // -----------------------------------------------------------------------
    // Caller mistakes refused loudly
    // -----------------------------------------------------------------------

    /// **A multithreaded caller is refused, not degraded.**
    ///
    /// The mechanisms bind the calling thread only, so confining one thread
    /// of many would issue a receipt the process never earned; and unshare's
    /// own EINVAL for multithreaded callers would otherwise be
    /// indistinguishable from host policy. The child parks a second thread,
    /// waits until the kernel actually counts it, and expects the dedicated
    /// error -- not Reduced with a straight face.
    #[test]
    fn confine_worker_refuses_a_multithreaded_caller() {
        assert!(
            child_exit_zero(|| {
                let (_tx, rx) = std::sync::mpsc::channel::<()>();
                let _handle = std::thread::spawn(move || {
                    let _ = rx.recv();
                });
                // Determinism: the guard reads the same counter we do, so once
                // THIS read sees two threads, the refusal below is certain --
                // spawn() alone does not guarantee the thread has registered.
                let mut seen_two = false;
                for _ in 0..1_000_000 {
                    if process_thread_count().unwrap_or(0) >= 2 {
                        seen_two = true;
                        break;
                    }
                    std::hint::spin_loop();
                }
                assert!(seen_two, "the second thread never registered");
                match confine_worker() {
                    Err(ConfineError::WorkerNotSingleThreaded { .. }) => {}
                    other => panic!("expected multithreaded refusal, got {other:?}"),
                }
            }),
            "a multithreaded caller was not refused"
        );
    }

    /// The probe agrees with reality: it says yes exactly when the confined
    /// child confirms denial.
    #[test]
    fn the_seccomp_probe_matches_what_confinement_does() {
        let available = seccomp_network_deny_available();
        assert!(available, "probe said no; the escape test expects yes");
    }

    // -----------------------------------------------------------------------
    // Tier shape — what the receipt may and may not contain
    // -----------------------------------------------------------------------

    #[cfg(target_arch = "x86_64")]
    const NR_UNSHARE: u32 = 272;
    #[cfg(target_arch = "aarch64")]
    const NR_UNSHARE: u32 = 266;

    /// The errno the tier tests force on `unshare(2)` — deliberately EPERM,
    /// the value policy-blocked hosts produce naturally.
    const EPERM: u32 = 1;

    /// A filter that denies `unshare(2)` with `EPERM`, permits everything else,
    /// and kills on any other architecture — the same three-outcome shape as
    /// [`build_network_deny_filter`], pointed at a different syscall.
    ///
    /// ```text
    /// //  0  load arch
    /// //  1  if arch != MY_ARCH -> kill (index 6)
    /// //  2  load nr
    /// //  3  if nr == unshare   -> deny (index 4); else allow (index 5)
    /// //  4  deny with EPERM
    /// //  5  allow
    /// //  6  kill process
    /// ```
    ///
    /// Its purpose is determinism in the tier tests: whether the HOST blocks
    /// user namespaces (default Docker) or allows them (seccomp-unconfined),
    /// the child below sees the identical EPERM from the syscall itself, so
    /// the Reduced tier's receipt shape can be asserted on every machine that
    /// runs this suite.
    fn build_unshare_deny_filter() -> Vec<SockFilter> {
        let stmt = |code: u16, k: u32| SockFilter {
            code,
            jt: 0,
            jf: 0,
            k,
        };
        let jump = |code: u16, k: u32, jt: u8, jf: u8| SockFilter { code, jt, jf, k };
        vec![
            stmt(BPF_LD_W_ABS, OFF_ARCH),
            jump(BPF_JMP_JEQ_K, MY_ARCH, 0, 4),
            stmt(BPF_LD_W_ABS, OFF_NR),
            jump(BPF_JMP_JEQ_K, NR_UNSHARE, 0, 1),
            stmt(BPF_RET_K, SECCOMP_RET_ERRNO | EPERM),
            stmt(BPF_RET_K, SECCOMP_RET_ALLOW),
            stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        ]
    }

    /// **The Reduced tier's receipt contains exactly two entries.**
    ///
    /// The child denies itself `unshare(2)` first — forcing, on every host,
    /// the failure that policy-only machines produce naturally — then applies
    /// the worker's confinement and inspects its own pairs. Landlock and
    /// seccomp must both be present and engaged; a failed unshare must leave
    /// NO `NetNs` entry behind, not an entry claiming `false`.
    #[test]
    fn reduced_tier_when_userns_blocked() {
        let prog = build_unshare_deny_filter();
        assert!(
            child_exit_zero(move || {
                install_prebuilt_filter(&prog).expect("unshare-deny filter must install");
                let pairs = confine_worker().expect("confine");
                assert_eq!(
                    pairs.len(),
                    2,
                    "Reduced is exactly two entries, got {pairs:?}"
                );
                assert!(
                    !pairs.iter().any(|(n, _)| n == "NetNs"),
                    "a failed unshare must be omitted entirely, got {pairs:?}"
                );
                let get = |name: &str| {
                    pairs
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, e)| *e)
                        .unwrap_or(false)
                };
                assert!(get("Landlock"), "filesystem write was not denied");
                assert!(get("Seccomp"), "outbound socket was not denied");
            }),
            "the reduced-tier child did not behave"
        );
    }

    /// **The Full tier's receipt contains exactly three entries, all true.**
    ///
    /// Runs only where the machine actually grants the escalation; where it
    /// does not (default Docker), this test passes silently — the Reduced
    /// test above covers what must hold there, and a machine that refuses
    /// unshare cannot be asked to demonstrate a tier it can never reach.
    #[test]
    fn full_tier_when_userns_allowed() {
        if !userns_netns_available() {
            return;
        }
        assert!(
            child_exit_zero(|| {
                let before = netns_identity();
                let pairs = confine_worker().expect("confine");
                let after = netns_identity();
                assert_eq!(pairs.len(), 3, "Full is exactly three, got {pairs:?}");
                let get = |name: &str| {
                    pairs
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, e)| *e)
                        .unwrap_or(false)
                };
                assert!(get("Landlock"), "filesystem write was not denied");
                assert!(get("Seccomp"), "outbound socket was not denied");
                assert!(
                    get("NetNs"),
                    "NetNs applied but read-back disagreed: {pairs:?}"
                );
                assert_ne!(
                    before, after,
                    "namespace identity did not change across the unshare"
                );
            }),
            "the full-tier child did not behave"
        );
    }

    /// **Namespace identity, read back from `/proc/self/ns/net`.**
    ///
    /// The same discipline the socket verdict follows, applied to the network
    /// namespace: after the unshare, THIS process reads back a different
    /// identity than the one it was born into. A successful `unshare` return
    /// value alone proves nothing about which namespace we ended up in.
    #[test]
    fn the_namespaced_child_reads_a_new_netns_identity() {
        if !userns_netns_available() {
            return;
        }
        assert!(
            child_exit_zero(|| {
                let before = netns_identity();
                assert!(
                    before.is_some(),
                    "/proc/self/ns/net unreadable before the unshare"
                );
                apply_userns_netns().expect("unshare");
                let after = netns_identity();
                assert_ne!(before, after);
            }),
            "the namespaced child did not observe a new identity"
        );
    }
}
