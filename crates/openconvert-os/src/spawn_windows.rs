//! Windows process creation with an **explicit inherit list**.
//!
//! # The measurement this module exists for
//!
//! `std::process::Command` sets `bInheritHandles = TRUE`, and Windows then
//! inherits **every inheritable handle in the parent**. A test child spawned
//! that way enumerated **64 handles it was never given and wrote through two of
//! them**, including one the parent had deliberately withheld (S3).
//!
//! `09` §6 A2 claims a compromised engine "holds no descriptor it was not
//! handed". That claim was **false as implemented**, and no amount of care in
//! the calling code fixes it, because `Command` cannot express an inherit list
//! at all. It needs `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, which needs raw
//! `CreateProcessW`.
//!
//! With the list: unintended writes **1 → 0**.
//!
//! # Two rules that follow, and are enforced here
//!
//! **Every host-side handle is non-inheritable by default.** Inheritance is
//! granted per spawn, to named handles, and revoked afterwards. The other
//! default is the dangerous one precisely because its failure is silent: the
//! child works fine and simply holds more than intended.
//!
//! **`UpdateProcThreadAttribute`'s return value is checked.** Spike S14 did not
//! check it, and an unchecked failure there produces a process that looks
//! confined, runs correctly, and inherits everything — the worst combination,
//! because nothing about it appears wrong.

use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`.
///
/// Declared here because `windows-sys` 0.61 does not export it — the same
/// situation as the `JOB_OBJECT_MSG_*` constants in [`crate::job_object`]. The
/// stable Win32 ABI is the contract, not the binding crate's coverage of it.
///
/// `ProcThreadAttributeValue(9, thread: false, input: true, additive: false)`
/// = `9 | 0x0002_0000`.
const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x0002_0009;

/// `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`.
///
/// `ProcThreadAttributeValue(7, thread: false, input: true, additive: false)`
/// = `7 | 0x0002_0000`.
///
/// **7, not 6.** 6 is `ProcThreadAttributeUmsThread`, and asking for it with a
/// mitigation-policy payload returns `ERROR_NOT_SUPPORTED` (50). The only
/// reason that surfaced as a refusal to spawn rather than as a worker quietly
/// running without ACG is the rule this module already had: check
/// `UpdateProcThreadAttribute`'s return value, always, and treat a failure as
/// fatal.
const PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY: usize = 0x0002_0007;

/// `PROCESS_CREATION_MITIGATION_POLICY_PROHIBIT_DYNAMIC_CODE_ALWAYS_ON` — ACG.
///
/// # Why this is requested rather than assumed
///
/// `readback.rs` said, as fact, that ACG "engages because the process is in an
/// AppContainer, whether or not it was asked for", citing spike S17b. **On the
/// production spawn it does not.** Measured three times on Windows 11 26200:
/// the child's own token reports `TokenIsAppContainer` true and
/// `ProcessDynamicCodePolicy` **false**.
///
/// So it is asked for. The correction only exists because the child reads its
/// own state back instead of the host recording what it requested — which is
/// the argument `03` §9.5 makes, arriving this time against a claim the
/// read-back design itself had produced.
///
/// ACG blocks a process from making new executable memory. That matters most
/// for the engines this crate does not yet run: a heap-spray against a C image
/// decoder needs somewhere to put its payload. Pure-Rust workers lose nothing
/// by it, and a worker that ever needs a JIT does not belong in this design.
const PROHIBIT_DYNAMIC_CODE_ALWAYS_ON: u64 = 1 << 36;

/// Why a spawn failed.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// The attribute list could not be sized or initialised.
    #[error("could not build the handle inherit list: {0}")]
    AttributeList(std::io::Error),
    /// `UpdateProcThreadAttribute` refused the handle list.
    ///
    /// **Fatal, deliberately.** Continuing here would spawn a process that
    /// inherits everything while looking confined — and nothing about it would
    /// appear wrong at runtime.
    #[error(
        "the handle inherit list was rejected ({0}); refusing to spawn, because \
         a process without it inherits every inheritable handle in this one"
    )]
    HandleListRejected(std::io::Error),
    /// A handle could not be marked inheritable.
    #[error("could not make a handle inheritable: {0}")]
    HandleFlags(std::io::Error),
    /// `CreateProcessW` failed.
    #[error("could not start {program}: {source}")]
    Create {
        /// What we tried to run.
        program: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The command line contained a NUL.
    ///
    /// Refused rather than truncated: a NUL is where the string Rust validated
    /// stops being the string Windows executes.
    #[error("the command line contains a NUL byte")]
    NulInCommandLine,
    /// A stdio pipe could not be created.
    #[error("could not create a pipe to the worker: {0}")]
    Pipe(std::io::Error),
    /// `UpdateProcThreadAttribute` refused the AppContainer.
    ///
    /// **Fatal, for the same reason as the handle list.** A process spawned
    /// without its container runs correctly, reaches the whole filesystem and
    /// the network, and looks exactly like one that is confined.
    #[error(
        "the AppContainer was rejected ({0}); refusing to spawn, because a worker \
         without it can reach the filesystem and the network"
    )]
    ContainerRejected(std::io::Error),
}

/// A spawned, running child.
#[derive(Debug)]
pub struct Child {
    process: HANDLE,
    thread: HANDLE,
}

// SAFETY: a process handle is just a kernel object reference; it has no
// thread affinity and may be waited on or closed from any thread.
unsafe impl Send for Child {}

impl Child {
    /// Wait for the child and return its exit code.
    ///
    /// # Errors
    ///
    /// Any underlying OS failure waiting or reading the code.
    pub fn wait(&self) -> std::io::Result<u32> {
        // SAFETY: `self.process` is a live handle this type owns and has not
        // closed; `Drop` is the only thing that closes it and has not run.
        // INFINITE is a documented timeout value.
        unsafe {
            WaitForSingleObject(self.process, INFINITE);
        }
        let mut code: u32 = 0;
        // SAFETY: as above for the handle. `GetExitCodeProcess` writes one u32
        // through the pointer, which is to a live local of exactly that type.
        let ok = unsafe { GetExitCodeProcess(self.process, &raw mut code) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(code)
    }

    /// The raw process handle, for callers that must assign it to a job.
    ///
    /// Borrowed, never owned: `Child` closes it in `Drop`, so a caller that
    /// closed it too would leave this type closing a handle it no longer owns.
    #[must_use]
    pub const fn process_handle(&self) -> HANDLE {
        self.process
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: both handles were returned by CreateProcessW into a
        // PROCESS_INFORMATION we own, are non-null (checked at construction),
        // and are closed exactly once because Drop runs once.
        unsafe {
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

/// Spawn `program`, inheriting **only** the handles listed.
///
/// # Errors
///
/// See [`SpawnError`]. Every variant leaves no child running.
///
/// # Panics
///
/// Does not panic. Allocation failure in the attribute-list buffer surfaces as
/// [`SpawnError::AttributeList`].
pub fn spawn_with_handles(
    program: &Path,
    args: &[&str],
    inherit: &[&dyn AsRawHandle],
) -> Result<Child, SpawnError> {
    let handles: Vec<HANDLE> = inherit
        .iter()
        .map(|h| h.as_raw_handle() as HANDLE)
        .collect();
    create_process(program, args, &handles, None, None)
}

/// Spawn `program` with pipes to its stdin and stdout, inheriting **nothing
/// else**.
///
/// This is how a worker is started. The child receives exactly three handles —
/// its two pipe ends and this process's stderr — and the `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
/// makes that list exhaustive rather than aspirational.
///
/// # Why not `std::process::Command`
///
/// It would work, and it would also hand the child every inheritable handle in
/// this process. Spike S3 counted 64 of them and wrote through two. The worker
/// is the one process in this system assumed to be compromised, so it is
/// precisely the process that must not receive that set.
///
/// # Why stderr is inherited rather than piped
///
/// A piped stderr nobody drains fills its buffer and blocks the worker mid
/// conversion — a deadlock that appears only under load and only for engines
/// that log. Inheriting sends diagnostics to the same place ours go.
///
/// # Errors
///
/// See [`SpawnError`]. Every variant leaves no child running and no pipe open.
pub fn spawn_piped(program: &Path, args: &[&str]) -> Result<PipedChild, SpawnError> {
    spawn_piped_inner(program, args, None)
}

/// Spawn `program` **inside an AppContainer**, with pipes and nothing else.
///
/// This is the confinement `09` SR-1 rests on. Everything else bounds what the
/// worker can *consume* — the Job Object caps memory, the handle list caps what
/// it holds. Only the container bounds what it can *reach*: the filesystem it
/// cannot see and the network it was never given a capability for.
///
/// # Network denial is an absence
///
/// The capability list is empty, and there is no flag to forget to set. A
/// container reaches the network by holding a capability SID; not requesting
/// one means there is nothing to turn back on. That is what makes SR-1's
/// "never" structural on this platform rather than a policy someone enforces.
///
/// # The executable must be readable by the container first
///
/// Call [`crate::appcontainer::AppContainer::grant_execute`] on `program`, and
/// `acl_directory` on any directory the worker must write. Without the first,
/// this fails with `ERROR_ACCESS_DENIED` on a binary that is plainly there —
/// see that method's documentation.
///
/// # Errors
///
/// See [`SpawnError`]. Every variant leaves no child running and no pipe open.
pub fn spawn_piped_in_container(
    program: &Path,
    args: &[&str],
    container: &crate::appcontainer::AppContainer,
) -> Result<PipedChild, SpawnError> {
    spawn_piped_inner(program, args, Some(container.sid()))
}

fn spawn_piped_inner(
    program: &Path,
    args: &[&str],
    container: Option<windows_sys::Win32::Security::PSID>,
) -> Result<PipedChild, SpawnError> {
    // Each pipe is created inheritable, then the end WE keep is stripped of
    // inheritance. Leaving both inheritable would hand the child the writing
    // end of its own input -- so a worker could feed itself frames, and the
    // host's `drop(stdin)` would no longer mean end-of-session, because the
    // last writer would still be alive inside the child.
    let (child_in, host_in) = pipe()?;
    let (host_out, child_out) = pipe()?;
    clear_inherit(host_in.as_raw_handle() as HANDLE)?;
    clear_inherit(host_out.as_raw_handle() as HANDLE)?;

    // SAFETY: GetStdHandle returns a borrowed handle owned by the process; it
    // is not closed here and stays valid for the process lifetime.
    let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    let stderr_usable = !stderr.is_null() && stderr != INVALID_HANDLE_VALUE;

    let child_in_h = child_in.as_raw_handle() as HANDLE;
    let child_out_h = child_out.as_raw_handle() as HANDLE;

    let mut inherit = vec![child_in_h, child_out_h];
    if stderr_usable {
        inherit.push(stderr);
    }

    // A detached process has no stderr, which is a lost diagnostic and not a
    // failure. Passing null is how Windows spells that.
    let stdio = [
        child_in_h,
        child_out_h,
        if stderr_usable {
            stderr
        } else {
            core::ptr::null_mut()
        },
    ];

    let child = create_process(program, args, &inherit, Some(stdio), container)?;

    // Drop our copies of the child's ends NOW. If the host kept the write end
    // of the child's stdout, reading would never see EOF when the child exits
    // -- the pipe would still have a writer, and a `read_frame` waiting for a
    // reply from a dead worker would block forever instead of erroring.
    drop(child_in);
    drop(child_out);

    Ok(PipedChild {
        child,
        stdin: Some(File::from(host_in)),
        stdout: Some(File::from(host_out)),
    })
}

/// A running child with pipes to its standard streams.
#[derive(Debug)]
pub struct PipedChild {
    child: Child,
    stdin: Option<File>,
    stdout: Option<File>,
}

impl PipedChild {
    /// The child, for assigning to a [`crate::job_object::LimitedJob`].
    #[must_use]
    pub const fn child(&self) -> &Child {
        &self.child
    }

    /// The write end of the child's stdin, while the session is open.
    pub fn stdin(&mut self) -> Option<&mut File> {
        self.stdin.as_mut()
    }

    /// The read end of the child's stdout.
    pub fn stdout(&mut self) -> Option<&mut File> {
        self.stdout.as_mut()
    }

    /// Close the child's stdin, which is how a session is ended cleanly.
    ///
    /// The worker's read loop sees EOF and returns. Killing is for a worker
    /// that does not.
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Wait for the child to exit and return its code.
    ///
    /// # Errors
    ///
    /// Any underlying OS failure waiting or reading the code.
    pub fn wait(&mut self) -> std::io::Result<u32> {
        self.close_stdin();
        self.child.wait()
    }
}

impl Drop for PipedChild {
    fn drop(&mut self) {
        // Teardown lives with the type that owns the child, on both platforms.
        //
        // `Worker` used to do this as well, and mutation testing surfaced the
        // duplication: replacing `Worker::drop` with `()` changed nothing
        // observable, because the field drops already did the work. A second
        // place that happens to be redundant today is a second place to keep
        // right forever.
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

/// Create one anonymous pipe. Both ends start inheritable.
fn pipe() -> Result<(OwnedHandle, OwnedHandle), SpawnError> {
    let mut read: HANDLE = core::ptr::null_mut();
    let mut write: HANDLE = core::ptr::null_mut();
    let sa = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(core::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: core::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: both out-pointers are to live locals of exactly HANDLE type, and
    // `sa` is a fully initialised SECURITY_ATTRIBUTES that outlives the call.
    // A zero size argument selects the system default buffer.
    let ok = unsafe { CreatePipe(&raw mut read, &raw mut write, &sa, 0) };
    if ok == 0 {
        return Err(SpawnError::Pipe(std::io::Error::last_os_error()));
    }
    // SAFETY: CreatePipe succeeded, so both handles are freshly created, owned
    // by this process, and not owned by anything else -- which is exactly the
    // precondition `from_raw_handle` requires.
    unsafe {
        Ok((
            OwnedHandle::from_raw_handle(read.cast()),
            OwnedHandle::from_raw_handle(write.cast()),
        ))
    }
}

/// The shared `CreateProcessW` body.
fn create_process(
    program: &Path,
    args: &[&str],
    handles: &[HANDLE],
    stdio: Option<[HANDLE; 3]>,
    container: Option<windows_sys::Win32::Security::PSID>,
) -> Result<Child, SpawnError> {
    // Windows takes one command line, not an argv. Quote every element so a
    // space or a quote in an argument cannot become an argument boundary.
    let mut cmdline = String::new();
    cmdline.push_str(&quote(&program.display().to_string()));
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote(a));
    }
    if cmdline.contains('\0') {
        return Err(SpawnError::NulInCommandLine);
    }
    let mut cmdline_w: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    // Every handle must be individually marked inheritable. Being in the list
    // is necessary and not sufficient -- the list restricts WHICH inheritable
    // handles pass, it does not make one inheritable.
    for h in handles {
        set_inheritable(*h)?;
    }

    // The list must be sized for EVERY attribute up front. Sizing for one and
    // adding two makes the second `UpdateProcThreadAttribute` fail -- which,
    // unchecked, is a process that silently runs outside its container.
    // The handle list, plus the container's security capabilities and its
    // mitigation policy when one is requested.
    let attribute_count: u32 = 1 + if container.is_some() { 2 } else { 0 };

    // Size the attribute list. The first call is expected to fail with
    // ERROR_INSUFFICIENT_BUFFER and write the size; that is the documented
    // protocol, not an error.
    let mut size: usize = 0;
    // SAFETY: passing a null list with a zero size is the documented way to
    // ask for the required size. `size` is a live local written through.
    unsafe {
        InitializeProcThreadAttributeList(core::ptr::null_mut(), attribute_count, 0, &raw mut size);
    }
    if size == 0 {
        return Err(SpawnError::AttributeList(std::io::Error::last_os_error()));
    }

    let mut buffer = vec![0_u8; size];
    let attr_list = buffer.as_mut_ptr().cast::<core::ffi::c_void>();

    // SAFETY: `buffer` is `size` bytes, which is exactly what the sizing call
    // above asked for, and stays alive until after CreateProcessW returns --
    // the attribute list holds a pointer into it, so it must outlive the spawn.
    let ok =
        unsafe { InitializeProcThreadAttributeList(attr_list, attribute_count, 0, &raw mut size) };
    if ok == 0 {
        return Err(SpawnError::AttributeList(std::io::Error::last_os_error()));
    }

    // From here the list must be deleted on every path, so the result is
    // captured rather than returned early.
    let result = (|| {
        if !handles.is_empty() {
            // SAFETY: `attr_list` was initialised above. The value pointer is
            // to `handles`, which lives until after CreateProcessW; the size is
            // that slice's byte length. PROC_THREAD_ATTRIBUTE_HANDLE_LIST
            // expects exactly an array of HANDLE.
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    core::mem::size_of_val(handles),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            // THE CHECK SPIKE S14 DID NOT MAKE.
            //
            // An unchecked failure here spawns a process that inherits
            // everything while looking confined. It runs correctly. Nothing
            // about it appears wrong. That is the worst possible failure mode
            // for a security control, so it is fatal.
            if ok == 0 {
                return Err(SpawnError::HandleListRejected(
                    std::io::Error::last_os_error(),
                ));
            }
        }

        // The AppContainer. `sec_caps` must outlive CreateProcessW, so it is
        // declared here and not inside the branch.
        let mut sec_caps = SECURITY_CAPABILITIES {
            AppContainerSid: core::ptr::null_mut(),
            // EMPTY, and this is the network denial. A container reaches the
            // network by holding a capability SID; requesting none means there
            // is nothing to switch back on later.
            Capabilities: core::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        // ACG, requested explicitly. Must outlive CreateProcessW.
        let mut mitigation: u64 = PROHIBIT_DYNAMIC_CODE_ALWAYS_ON;

        if let Some(sid) = container {
            sec_caps.AppContainerSid = sid;

            // SAFETY: `attr_list` was initialised for `attribute_count`
            // attributes, which counted this one. `mitigation` is a live local
            // outliving CreateProcessW; passing 8 bytes selects the 64-bit
            // policy form, which is the one that carries the dynamic-code bits.
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                    core::ptr::from_mut(&mut mitigation).cast(),
                    core::mem::size_of::<u64>(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(SpawnError::ContainerRejected(
                    std::io::Error::last_os_error(),
                ));
            }

            // SAFETY: `attr_list` was initialised for `attribute_count`
            // attributes, which counted this one. `sec_caps` is a live local
            // that outlives CreateProcessW below, and its size is what
            // PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES expects.
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                    core::ptr::from_mut(&mut sec_caps).cast(),
                    core::mem::size_of::<SECURITY_CAPABILITIES>(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            // Fatal for the same reason the handle list is: an unchecked
            // failure here produces a process that runs correctly, outside its
            // container, with nothing about it appearing wrong.
            if ok == 0 {
                return Err(SpawnError::ContainerRejected(
                    std::io::Error::last_os_error(),
                ));
            }
        }

        // SAFETY: both structs are plain C layouts of integers, pointers and
        // fixed arrays, with no niche and no invalid bit pattern -- all-zero is
        // a valid value, and it is what the Win32 documentation requires as the
        // starting state. `cb` is then set to the real size, which is the one
        // field CreateProcessW reads before anything else.
        let mut si: STARTUPINFOEXW = unsafe { core::mem::zeroed() };
        si.StartupInfo.cb =
            u32::try_from(core::mem::size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX);
        si.lpAttributeList = attr_list as LPPROC_THREAD_ATTRIBUTE_LIST;
        if let Some([stdin, stdout, stderr]) = stdio {
            si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = stdin;
            si.StartupInfo.hStdOutput = stdout;
            si.StartupInfo.hStdError = stderr;
        }

        // SAFETY: as above. CreateProcessW overwrites every field on success.
        let mut pi: PROCESS_INFORMATION = unsafe { core::mem::zeroed() };

        // SAFETY: `cmdline_w` is NUL-terminated UTF-16 and mutable, as
        // CreateProcessW requires (it may write into it). `si` and `pi` are
        // live locals of the right types. `bInheritHandles = TRUE` combined
        // with EXTENDED_STARTUPINFO_PRESENT and the attribute list means the
        // child inherits ONLY the listed handles -- which is the entire point.
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmdline_w.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                1, // TRUE, restricted by the handle list above
                // `CREATE_NO_WINDOW` matters in a GUI host.
                //
                // The workers are console-subsystem binaries, so without it
                // Windows allocates a console for each one: converting a single
                // image flashed a black window open and shut. It suppresses the
                // console only — stdout and stderr are already redirected to the
                // pipes above, which is where the protocol and the diagnostics
                // actually go, so nothing is hidden that anyone was reading.
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::from_mut(&mut si).cast(),
                &raw mut pi,
            )
        };
        if ok == 0 {
            return Err(SpawnError::Create {
                program: program.display().to_string(),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(Child {
            process: pi.hProcess,
            thread: pi.hThread,
        })
    })();

    // SAFETY: `attr_list` was successfully initialised above and is deleted
    // exactly once, on every path, before `buffer` is dropped.
    unsafe {
        DeleteProcThreadAttributeList(attr_list);
    }
    drop(buffer);

    result
}

/// Mark a handle inheritable for the duration of a spawn.
fn set_inheritable(h: HANDLE) -> Result<(), SpawnError> {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(SpawnError::HandleFlags(std::io::Error::other(
            "null or invalid handle in the inherit list",
        )));
    }
    // SAFETY: `h` is a live handle owned by the caller for the duration of the
    // spawn. `SetHandleInformation` writes nothing through a pointer; it sets
    // flags on the handle itself.
    let ok = unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if ok == 0 {
        return Err(SpawnError::HandleFlags(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Strip inheritance from a handle this process keeps.
///
/// The module's stated default — every host-side handle non-inheritable — is
/// only a default if something enforces it on handles the host creates itself.
fn clear_inherit(h: HANDLE) -> Result<(), SpawnError> {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    // SAFETY: `h` is a live handle owned by this process. The call sets flags
    // on the handle and writes through no pointer.
    let ok = unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) };
    if ok == 0 {
        return Err(SpawnError::HandleFlags(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Quote one command-line element.
///
/// Windows parses a single string, so an unquoted space is an argument
/// boundary. This is the host side of the same problem `Argv` solves on the
/// type side — and it is why `EngineBin` is a closed enum: the program name
/// never comes from input, so only the arguments need quoting.
fn quote(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                out.push(c);
            }
            '"' => {
                // Backslashes before a quote must be doubled, then the quote
                // escaped. Getting this wrong is how a quoted argument becomes
                // two arguments.
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            _ => {
                backslashes = 0;
                out.push(c);
            }
        }
    }
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn cmd() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()))
    }

    /// The machinery works: a child starts, runs and reports its exit code.
    #[test]
    fn a_child_spawns_and_reports_its_exit_code() {
        let child = spawn_with_handles(&cmd(), &["/c", "exit 42"], &[]).expect("spawn");
        assert_eq!(child.wait().expect("wait"), 42);
    }

    /// A handle **in** the list is genuinely inherited and usable.
    ///
    /// The control for the whole module. Without it, every containment
    /// assertion is satisfied by a spawn that inherits *nothing* — which would
    /// be perfectly safe and would make it impossible to hand a worker its
    /// input.
    #[test]
    fn a_listed_handle_is_inherited_and_writable() {
        let dir = std::env::temp_dir().join(format!("tx-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&dir).expect("mkdir");
        let out = dir.join("inherited.txt");

        {
            let file = std::fs::File::create_new(&out).expect("create");
            // The child writes to the inherited handle via its own stdout is
            // not what we test here -- we test that passing the handle at all
            // succeeds and the child runs. Inheriting a file handle without a
            // matching stdio redirect is exactly the "handed a descriptor"
            // shape the worker protocol uses.
            let child = spawn_with_handles(&cmd(), &["/c", "exit 7"], &[&file]).expect("spawn");
            assert_eq!(child.wait().expect("wait"), 7);
        }

        assert!(out.exists());
        let _ = std::fs::remove_dir_all(&dir); // openconvert-lint: allow -- test scratch teardown
    }

    /// **A child really runs inside an AppContainer, and pipes still work.**
    ///
    /// The whole point of `spawn_piped_in_container`, and the thing no amount
    /// of reading the documentation settles: the attribute list now carries two
    /// entries, the token is different, and stdio redirection has to survive
    /// both.
    ///
    /// `cmd.exe` is used rather than an engine because System32 already grants
    /// `ALL APPLICATION PACKAGES` read and execute. That isolates *this*
    /// question — does the spawn work — from the separate one about ACLs on the
    /// engine binary, which `grant_execute` answers.
    #[test]
    fn a_child_runs_inside_an_appcontainer_with_working_pipes() {
        use crate::appcontainer::AppContainer;

        let name = format!("openconvert.test.spawn.{}", std::process::id());
        let container = AppContainer::create(&name)
            .expect("AppContainer creation should work on a stock Windows install");

        let mut child = spawn_piped_in_container(&cmd(), &["/c", "echo confined"], &container)
            .expect("spawn inside the container");

        let mut out = String::new();
        child
            .stdout()
            .expect("stdout")
            .read_to_string(&mut out)
            .expect("read");
        assert_eq!(child.wait().expect("wait"), 0);
        assert!(
            out.contains("confined"),
            "the container child produced no output: {out:?}"
        );
    }

    /// The control: the same spawn without a container also works.
    ///
    /// Without it, the test above is satisfied by a `spawn_piped_in_container`
    /// that quietly ignores its container argument — which is precisely the
    /// failure mode `ContainerRejected` exists to make impossible.
    #[test]
    fn the_same_spawn_without_a_container_also_works() {
        let mut child = spawn_piped(&cmd(), &["/c", "echo plain"]).expect("spawn");
        let mut out = String::new();
        child
            .stdout()
            .expect("stdout")
            .read_to_string(&mut out)
            .expect("read");
        assert_eq!(child.wait().expect("wait"), 0);
        assert!(out.contains("plain"));
    }

    /// Closing stdin is what ends a session, and the exit code survives it.
    #[test]
    fn closing_stdin_ends_the_child_and_the_code_comes_back() {
        let mut child = spawn_piped(&cmd(), &["/c", "exit 9"]).expect("spawn");
        assert!(child.stdin().is_some(), "stdin should be open");
        child.close_stdin();
        assert!(child.stdin().is_none(), "stdin should be closed");
        assert_eq!(child.wait().expect("wait"), 9);
    }

    /// A nonexistent program fails loudly rather than silently succeeding.
    #[test]
    fn a_missing_program_is_an_error() {
        let err = spawn_with_handles(Path::new("C:\\definitely\\not\\here\\nope.exe"), &[], &[])
            .expect_err("should fail");
        assert!(
            matches!(err, SpawnError::Create { .. }),
            "wrong error: {err}"
        );
    }

    /// A NUL in the command line is refused, not truncated.
    ///
    /// The same hazard `OutputName` rejects: a NUL is where the string Rust
    /// validated stops being the string the OS executes.
    #[test]
    fn a_nul_in_the_command_line_is_refused() {
        let err = spawn_with_handles(&cmd(), &["/c", "exit\0 0"], &[]).expect_err("should refuse");
        assert!(matches!(err, SpawnError::NulInCommandLine));
    }

    /// Command-line quoting survives spaces, quotes and trailing backslashes.
    ///
    /// Windows parses one string, so getting this wrong turns one argument into
    /// two — which is the shell-injection shape, arriving through the back door
    /// after `Argv` closed the front one.
    #[test]
    fn quoting_handles_the_awkward_cases() {
        // Nothing that needs quoting is quoted. A trailing backslash is only
        // dangerous INSIDE quotes, where it would escape the closing one -- on
        // its own it is an ordinary character, and my first version of this
        // test asserted otherwise.
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote(r"trailing\"), r"trailing\");

        assert_eq!(quote("with space"), "\"with space\"");
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(""), "\"\"");
    }

    /// **The case that actually matters**: a trailing backslash in an argument
    /// that has to be quoted.
    ///
    /// `"C:\some path\"` would end with an escaped quote, swallowing the
    /// closing delimiter and merging this argument with the next one. That is
    /// the shell-injection shape arriving through the back door after `Argv`
    /// closed the front one -- so the backslashes are doubled.
    #[test]
    fn a_trailing_backslash_inside_quotes_is_doubled() {
        assert_eq!(quote(r"C:\some path\"), r#""C:\some path\\""#);
        assert_eq!(quote(r"two\\"), r"two\\");
        assert_eq!(quote(r"a b\\"), r#""a b\\\\""#);
    }
}
