//! macOS confinement — Seatbelt, applied by the worker to itself.
//!
//! # The shape is Linux's, not Windows'
//!
//! Windows confines the child **at spawn time**: the host builds an
//! AppContainer, a restricted token and a handle list, and the worker is born
//! inside them. Linux confines the child **from inside**: the worker is its own
//! warden, calling Landlock and seccomp on itself once it has loaded everything
//! it will ever need. macOS works the second way, so this module is the
//! counterpart of [`crate::linux`] and plugs into the same three points —
//! [`crate::available::probe`], `openconvert_worker::confine_self`, and the
//! read-back that fills the receipt.
//!
//! # ⚠️ Written, typechecked, never executed
//!
//! **No line of this file has run on a Mac.** `docs/spec/TARGETS.md` records
//! that every macOS claim in this project is reasoning rather than measurement,
//! and this module does not change that — it gives the measurement something to
//! be performed against. The private macOS validation runbook's
//! T1 is the runbook that settles whether the confinement below actually
//! engages under a hardened runtime, and it is unrun.
//!
//! What keeps that honest rather than dangerous is the same rule the rest of
//! this crate follows: **the verdict comes from read-back, never from the
//! request.** [`confine_worker`] applies the profile and then *attempts the
//! forbidden operations*. If Seatbelt silently did nothing, both probes still
//! succeed, both mechanisms report `false`, the profile scores `Strength::None`,
//! and the isolation floor refuses the conversion. A macOS build that cannot
//! confine converts nothing — which is the failure worth having.
//!
//! # Why `sandbox_init`, a deprecated API
//!
//! It has carried a deprecation warning since 10.8 and it is still what the
//! platform offers a process that wants to confine itself. Chromium's renderer
//! and Firefox's content processes both reach it. The supported-looking
//! alternative — the App Sandbox — is an *entitlement* applied to a whole
//! signed bundle at launch, which cannot express "this child, from now on, may
//! touch nothing", and would put the host inside the sandbox as well. `03 §9`
//! puts the trust boundary between host and engine, so the host stays out.
//!
//! The named profile `pure-computation` is used rather than a hand-written SBPL
//! string. It denies filesystem access and network access outright, which is
//! exactly the worker's requirement: by the time this runs, the engine's
//! libraries are loaded and the protocol needs nothing but the two pipes it was
//! born holding.

use std::ffi::{c_char, c_int, c_void};

/// Why confinement could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// `sandbox_init` is not present in this process's symbol namespace.
    ///
    /// Not a version check: the symbol was looked for and was not found.
    #[error("the Seatbelt API (sandbox_init) is not available on this machine")]
    SandboxUnavailable,

    /// `sandbox_init` returned non-zero.
    ///
    /// Carries the error string the API allocates, which is the only place the
    /// reason exists — the return value is a bare -1.
    #[error("sandbox_init failed (code {code}): {message}")]
    SandboxInit {
        /// The raw return value.
        code: i32,
        /// The message `sandbox_init` allocated, or a note that it allocated none.
        message: String,
    },
}

/// `SANDBOX_NAMED` — interpret the first argument as a profile name rather than
/// as an SBPL source string. From `sandbox.h`.
const SANDBOX_NAMED: u64 = 0x0001;

/// `kSBXProfilePureComputation` — "no filesystem access, no network access".
///
/// The strictest of the built-in profiles and the one that matches the worker's
/// actual needs. NUL-terminated because it crosses an FFI boundary as a C
/// string, and built as a byte literal so the NUL is visible in the source
/// rather than added by a helper.
const PROFILE_PURE_COMPUTATION: &[u8] = b"pure-computation\0";

/// `RTLD_DEFAULT` on macOS. Searches every image already loaded into the
/// process, which is where libSystem's `sandbox_init` lives.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

unsafe extern "C" {
    /// `int sandbox_init(const char *profile, uint64_t flags, char **errorbuf)`
    ///
    /// Returns 0 on success. On failure it allocates a string into `errorbuf`
    /// which the caller must release with `sandbox_free_error`.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;

    /// Releases the buffer `sandbox_init` allocated. Passing null is safe.
    fn sandbox_free_error(errorbuf: *mut c_char);

    /// Symbol lookup, used to answer "is this API here" without calling it.
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

    /// Used only to ask whether an `AF_INET` socket can be CREATED. Nothing is
    /// bound, connected or sent -- see [`inet_socket_succeeds`].
    fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;

    /// Closes the descriptor the probe above may have obtained.
    fn close(fd: c_int) -> c_int;
}

/// `AF_INET` on Darwin.
const AF_INET: c_int = 2;
/// `SOCK_STREAM` on Darwin.
const SOCK_STREAM: c_int = 1;

/// Whether the Seatbelt API exists in this process.
///
/// **This is a weaker guarantee than its Windows and Linux counterparts, and
/// the difference is deliberate rather than an oversight.**
///
/// `windows_probe` creates an AppContainer and destroys it; `landlock_available`
/// asks the kernel for the ruleset ABI. Both construct the mechanism, because a
/// capability is what the machine grants now rather than what its version
/// implies. There is no equivalent here: the only way to learn whether
/// `sandbox_init` succeeds is to call it, and calling it in the host would
/// confine the host — permanently, and for every job it later runs.
///
/// So this reports whether the *API* is reachable, and the authoritative answer
/// still comes from the child. The consequence of the gap is a planning error
/// in the direction that costs a conversion rather than a receipt: planning may
/// say "sandboxed", the worker may then fail to confine, and the step fails
/// with a named reason instead of running exposed.
///
/// Closing the gap properly needs a probe subprocess, and that is worth
/// building only once MACOS.md T1 says which answer it would be checking.
#[must_use]
pub fn sandbox_available() -> bool {
    // SAFETY: `dlsym` with RTLD_DEFAULT and a NUL-terminated symbol name reads
    // the loaded-image symbol tables and returns a pointer or null. Nothing is
    // dereferenced here — only compared against null.
    let found = unsafe { dlsym(RTLD_DEFAULT, c"sandbox_init".as_ptr()) };
    !found.is_null()
}

/// Confine this process, then prove it by attempting what should now fail.
///
/// Returns one `(mechanism, engaged)` pair per mechanism, in the shape the
/// worker forwards into the receipt. A mechanism that did not engage is
/// reported `false`; nothing here reports a request.
///
/// # Ordering
///
/// Everything the process will ever need must already be loaded. The profile
/// denies filesystem access, so a dynamic library resolved lazily *after* this
/// call fails to load. That is the same constraint Landlock imposes on the
/// Linux path, and the worker satisfies it the same way: engines are linked and
/// initialised before the protocol loop starts.
///
/// No single-threaded check, unlike Linux. Landlock and seccomp bind the
/// calling *thread*, so a second thread there means partial confinement and a
/// receipt that lies by omission. A Seatbelt profile applies to the process, so
/// the hazard does not exist and a check for it would be cargo-culted.
///
/// # Errors
///
/// [`ConfineError::SandboxUnavailable`] if the API is not present;
/// [`ConfineError::SandboxInit`] with the platform's own message otherwise.
pub fn confine_worker() -> Result<Vec<(String, bool)>, ConfineError> {
    if !sandbox_available() {
        return Err(ConfineError::SandboxUnavailable);
    }

    // Baselines FIRST. A denial only means something against a "before" that
    // succeeded: a socket refused by a machine with no network configured is
    // not a refusal this profile produced.
    let fs_before = file_create_succeeds();
    let net_before = inet_socket_succeeds();

    apply_pure_computation()?;

    let fs_after = file_create_succeeds();
    let net_after = inet_socket_succeeds();

    // Went from possible to denied == this profile did it. Anything else --
    // including "was already impossible" -- is reported as not engaged, which
    // lowers the tier rather than claiming a sandbox that cannot be attributed.
    let fs_engaged = fs_before && !fs_after;
    let net_engaged = net_before && !net_after;

    Ok(vec![
        ("Seatbelt (filesystem)".to_string(), fs_engaged),
        ("Seatbelt (network)".to_string(), net_engaged),
    ])
}

/// Apply the named profile. Split out so the FFI and its one `unsafe` block sit
/// alone, with the read-back logic above staying safe Rust.
fn apply_pure_computation() -> Result<(), ConfineError> {
    let mut err: *mut c_char = std::ptr::null_mut();

    // SAFETY: the profile pointer is a NUL-terminated byte literal with static
    // lifetime; `err` is a valid, writable slot for the out-parameter. On
    // failure the API allocates into it and we release it below with the
    // matching free function, exactly once, on every path.
    let code = unsafe {
        sandbox_init(
            PROFILE_PURE_COMPUTATION.as_ptr().cast::<c_char>(),
            SANDBOX_NAMED,
            &raw mut err,
        )
    };

    if code == 0 {
        // Success does not allocate, but freeing null is defined and costs
        // nothing -- and leaving the branch out is how the leak appears the day
        // someone adds an early return above.
        if !err.is_null() {
            // SAFETY: non-null implies it came from sandbox_init.
            unsafe { sandbox_free_error(err) };
        }
        return Ok(());
    }

    let message = if err.is_null() {
        "sandbox_init reported no reason".to_string()
    } else {
        // SAFETY: sandbox_init allocated a NUL-terminated C string here.
        let text = unsafe { std::ffi::CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: released exactly once, and `err` is not read after this.
        unsafe { sandbox_free_error(err) };
        text
    };

    Err(ConfineError::SandboxInit { code, message })
}

/// Can this process create a file? Safe Rust, deliberately.
///
/// The Linux module reaches for `open(2)` directly because it also needs to
/// distinguish errnos. Here the question is binary, so `std` answers it and the
/// probe carries no `unsafe` at all. The file is removed on success; under the
/// profile the create fails and there is nothing to remove.
fn file_create_succeeds() -> bool {
    let path =
        std::env::temp_dir().join(format!("openconvert-confine-probe-{}", std::process::id()));
    match std::fs::File::create(&path) {
        Ok(_) => {
            let _ = std::fs::remove_file(&path);
            true
        }
        Err(_) => false,
    }
}

/// Can this process open an `AF_INET` socket?
///
/// The raw call rather than `std::net`, for the same reason [`crate::linux`]
/// uses it: SR-9 says **exactly one** outbound network call site exists in this
/// codebase -- the weekly signed manifest fetch -- and `xtask lint` enforces
/// that by refusing `std::net` anywhere else. It refused this file, correctly.
/// A probe is not an outbound call, but a gate that has to distinguish "opens a
/// socket to look at it" from "opens a socket to use it" is a gate with an
/// exemption list, and the claim it protects is worth more than the convenience.
///
/// `socket(2)` alone answers the question: it neither binds, connects, nor
/// sends, so nothing reaches the network even when the profile allows it.
fn inet_socket_succeeds() -> bool {
    // SAFETY: plain syscall wrapper. The descriptor is closed on success and
    // nothing else ever observes it.
    let fd = unsafe { socket(AF_INET, SOCK_STREAM, 0) };
    if fd >= 0 {
        // SAFETY: `fd` came from the call above and is closed exactly once.
        unsafe { close(fd) };
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probes must agree with the machine before anything is applied.
    ///
    /// Runs on any Mac, confined or not: an unconfined process can do both, and
    /// a test process that could do neither would make every engagement verdict
    /// below meaningless. This is the assumption the read-back rests on, so it
    /// is asserted rather than assumed.
    #[test]
    fn probes_succeed_before_confinement() {
        assert!(
            file_create_succeeds(),
            "the test process cannot create a file in its own temp dir, so the \
             filesystem read-back could never attribute a denial"
        );
        assert!(
            inet_socket_succeeds(),
            "the test process cannot open a loopback UDP socket, so the network \
             read-back could never attribute a denial"
        );
    }

    /// The API is either there or it is not, and either answer is information.
    ///
    /// Not asserted as `true`: a machine without it should fail at
    /// `confine_worker` with a named error, not at a test that assumed the
    /// platform.
    #[test]
    fn availability_is_answerable() {
        let _ = sandbox_available();
    }
}
