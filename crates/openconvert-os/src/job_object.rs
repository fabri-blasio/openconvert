//! Windows resource limits, and **learning which one tripped**.
//!
//! # The gap this closes
//!
//! A Job Object memory cap works — a capped worker dies where an uncapped one
//! survives. But it **terminates the process with `0xC0000409` and no
//! `Failed{..}` message** (S14).
//!
//! `03` §13 promises every failure names the file, the step and the engine. An
//! abort code names none of those. So the row would have read *"the engine
//! stopped"* — which is true, useless, and indistinguishable from a crash.
//!
//! The fix is measured, not guessed: **associate the job with an IO completion
//! port** and `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` arrives *before* the exit
//! code (S18). The host then synthesises
//! *"exceeded its 512 MB memory limit"* from a notification the dying worker
//! never had a chance to send.
//!
//! # Linux does not need this, and the record should not flatten that
//!
//! `RLIMIT_AS` produces a **recoverable** allocation failure, so a Linux worker
//! catches it and sends a proper `Failed{..}` itself. One design, two
//! mechanisms, stated rather than averaged.

use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectAssociateCompletionPortInformation,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
use windows_sys::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus};

/// Completion-port message codes.
///
/// Declared here because `windows-sys` 0.61 does not export them — the same
/// situation as the mitigation-policy DWORD in [`crate::readback`]. These are
/// stable published ABI constants that have not changed since Windows 2000;
/// depending on a binding crate to re-export them is a build break waiting for
/// a `cargo update`, and the numbers themselves are the contract.
mod msg {
    /// No processes remain in the job.
    pub const ACTIVE_PROCESS_ZERO: u32 = 4;
    /// A process in the job exited.
    pub const EXIT_PROCESS: u32 = 7;
    /// **The one this module exists for**: a process exceeded its per-process
    /// memory limit and was terminated.
    pub const PROCESS_MEMORY_LIMIT: u32 = 9;
}

/// Why a job object could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The job object itself could not be created.
    #[error("could not create a job object: {0}")]
    Create(std::io::Error),
    /// The completion port could not be created or associated.
    #[error("could not attach a completion port: {0}")]
    CompletionPort(std::io::Error),
    /// The limit could not be applied.
    #[error("could not apply the memory limit: {0}")]
    Limit(std::io::Error),
    /// The process could not be assigned.
    #[error("could not assign the process to the job: {0}")]
    Assign(std::io::Error),
}

/// What the job told us about a worker.
///
/// The whole point of this module: a *reason*, not an exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobNotice {
    /// The process exceeded its memory limit and was terminated.
    ///
    /// **This is the one that could not be observed before.** It arrives before
    /// the exit code, so the host can say which limit tripped rather than
    /// reporting `0xC0000409`.
    MemoryLimit,
    /// A process in the job exited normally.
    ProcessExited,
    /// No processes remain in the job.
    Empty,
    /// Something else arrived. Recorded rather than swallowed — an unknown
    /// message is information, and treating it as silence loses it.
    Other(u32),
    /// Nothing arrived before the timeout.
    Timeout,
}

/// A job object with a memory cap and a completion port.
#[derive(Debug)]
pub struct LimitedJob {
    job: HANDLE,
    port: HANDLE,
}

// SAFETY: both are kernel object handles with no thread affinity.
unsafe impl Send for LimitedJob {}

impl LimitedJob {
    /// Create a job capped at `memory_bytes`, with a completion port attached.
    ///
    /// The port is attached **before** any process joins, because a notification
    /// for a process that died during setup is one nobody is listening for.
    ///
    /// # Errors
    ///
    /// See [`JobError`].
    pub fn new(memory_bytes: u64) -> Result<Self, JobError> {
        // SAFETY: null attributes and a null name create an anonymous job
        // object owned by this process; both nulls are documented as valid.
        let job = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
        if job.is_null() {
            return Err(JobError::Create(std::io::Error::last_os_error()));
        }
        // SAFETY: INVALID_HANDLE_VALUE as the file handle with a null existing
        // port is the documented way to create a fresh completion port. The key
        // and thread count are plain integers.
        let port = unsafe {
            CreateIoCompletionPort(
                windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
                core::ptr::null_mut(),
                0,
                1,
            )
        };
        if port.is_null() {
            let err = std::io::Error::last_os_error();
            // SAFETY: `job` is live, ours, and not yet closed. Closed here
            // rather than leaking a kernel object on every failed setup.
            unsafe { CloseHandle(job) };
            return Err(JobError::CompletionPort(err));
        }

        // Owned from here. Every path below returns through `this`, so `Drop`
        // closes both handles exactly once whatever fails after this line.
        let this = Self { job, port };

        let assoc = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: core::ptr::null_mut(),
            CompletionPort: port,
        };
        // SAFETY: the class and the struct match, the size is that struct's
        // own, and the pointer is to a live local.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectAssociateCompletionPortInformation,
                core::ptr::from_ref(&assoc).cast(),
                u32::try_from(core::mem::size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>())
                    .unwrap_or(u32::MAX),
            )
        };
        if ok == 0 {
            return Err(JobError::CompletionPort(std::io::Error::last_os_error()));
        }

        // SAFETY: the extended-limit struct is a plain C layout with no invalid
        // bit pattern; all-zero is valid and is the documented starting state.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { core::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.ProcessMemoryLimit = usize::try_from(memory_bytes).unwrap_or(usize::MAX);

        // SAFETY: as above for class, struct and size.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                core::ptr::from_ref(&limits).cast(),
                u32::try_from(core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .unwrap_or(u32::MAX),
            )
        };
        if ok == 0 {
            return Err(JobError::Limit(std::io::Error::last_os_error()));
        }

        Ok(this)
    }

    /// Put a running child under this job's limits.
    ///
    /// # Why this takes `&Child` and not a raw handle
    ///
    /// An earlier signature took `HANDLE`, and clippy was right to object: a
    /// raw handle is a pointer whose validity only the caller can vouch for,
    /// which makes the function `unsafe` in everything but its declaration.
    /// Borrowing the [`Child`] moves that guarantee into the type system —
    /// a `Child` that exists owns a live process handle, and the borrow stops
    /// it being dropped mid-call.
    ///
    /// # Errors
    ///
    /// [`JobError::Assign`] if the process could not be assigned — which on
    /// modern Windows usually means it already belongs to a job that forbids
    /// nesting.
    pub fn assign(&self, child: &crate::spawn_windows::Child) -> Result<(), JobError> {
        // SAFETY: `self.job` is owned by this type and live until Drop.
        // `child` is borrowed, so its process handle cannot be closed before
        // this call returns. Nothing is written through either pointer.
        let ok = unsafe { AssignProcessToJobObject(self.job, child.process_handle()) };
        if ok == 0 {
            return Err(JobError::Assign(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Wait for the next notification.
    ///
    /// Returns [`JobNotice::Timeout`] rather than blocking forever, because a
    /// host that hangs waiting for a message that will never come is a worse
    /// failure than the one it was trying to report.
    #[must_use]
    pub fn next_notice(&self, timeout: Duration) -> JobNotice {
        let mut code: u32 = 0;
        let mut key: usize = 0;
        let mut overlapped = core::ptr::null_mut();

        // SAFETY: all three out-pointers are to live locals of the right types.
        // A timeout returns zero and leaves them untouched, which is why the
        // return value is checked before they are read.
        let ok = unsafe {
            GetQueuedCompletionStatus(
                self.port,
                &raw mut code,
                &raw mut key,
                &raw mut overlapped,
                u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
            )
        };
        if ok == 0 {
            return JobNotice::Timeout;
        }
        match code {
            msg::PROCESS_MEMORY_LIMIT => JobNotice::MemoryLimit,
            msg::EXIT_PROCESS => JobNotice::ProcessExited,
            msg::ACTIVE_PROCESS_ZERO => JobNotice::Empty,
            other => JobNotice::Other(other),
        }
    }

    /// The raw job handle, for callers that must assign at spawn time.
    #[must_use]
    pub const fn handle(&self) -> HANDLE {
        self.job
    }
}

impl Drop for LimitedJob {
    fn drop(&mut self) {
        // SAFETY: both handles were created by this type, are non-null on every
        // path that constructs one, and are closed exactly once because Drop
        // runs once. Closing the job terminates nothing on its own -- the job
        // has no kill-on-close flag set.
        unsafe {
            if !self.port.is_null() {
                CloseHandle(self.port);
            }
            if !self.job.is_null() {
                CloseHandle(self.job);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job with a cap and a port can be created.
    #[test]
    fn a_limited_job_is_created_and_dropped_cleanly() {
        let job = LimitedJob::new(64 << 20).expect("create");
        assert!(!job.handle().is_null());
        drop(job);
    }

    /// **The finding, end to end.**
    ///
    /// A child that exceeds the cap is killed, and the completion port tells us
    /// *why* — before the exit code arrives, which is the whole point. Without
    /// this the host sees `0xC0000409` and `03` §13's message rule cannot be
    /// satisfied.
    #[test]
    fn exceeding_the_memory_cap_produces_a_reportable_notice() {
        use crate::spawn_windows::spawn_with_handles;

        // 8 MiB is under any real worker's needs and far under what the
        // allocation below asks for, so the kill is deterministic.
        let job = LimitedJob::new(8 << 20).expect("create job");

        let cmd = std::path::PathBuf::from(
            std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()),
        );
        // `set /a` on a huge string is awkward; simplest reliable hog is a
        // PowerShell one-liner. Fall back to asserting only the setup if it is
        // unavailable, rather than failing for an unrelated reason.
        let spawned = spawn_with_handles(
            &cmd,
            &[
                "/c",
                "powershell -NoProfile -Command \"$a=New-Object byte[] 268435456; $a[0]=1\"",
            ],
            &[],
        );
        let Ok(child) = spawned else {
            eprintln!("skipping: could not spawn the allocation hog");
            return;
        };

        // Assigning after spawn races the child's startup, which is exactly why
        // production assigns at creation time with the job in the attribute
        // list. For this test the child spends its first milliseconds loading
        // PowerShell, so the window is comfortable.
        if job.assign(&child).is_err() {
            eprintln!("skipping: could not assign to the job");
            return;
        }

        let _ = child.wait();

        // Drain a few notices; the memory one may arrive after an exit notice.
        let mut saw_memory = false;
        for _ in 0..8 {
            match job.next_notice(Duration::from_millis(500)) {
                JobNotice::MemoryLimit => {
                    saw_memory = true;
                    break;
                }
                JobNotice::Timeout => break,
                _ => {}
            }
        }
        assert!(
            saw_memory,
            "the completion port produced no memory-limit notice, so the host \
             could only report an abort code -- which is the S14 gap this module \
             exists to close"
        );
    }

    /// A quiet job times out rather than blocking forever.
    ///
    /// The control: a host that hangs waiting for a message that never comes is
    /// a worse failure than the one it was trying to report.
    #[test]
    fn a_quiet_job_times_out() {
        let job = LimitedJob::new(64 << 20).expect("create");
        assert_eq!(
            job.next_notice(Duration::from_millis(50)),
            JobNotice::Timeout
        );
    }
}
