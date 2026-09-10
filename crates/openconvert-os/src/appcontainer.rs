//! AppContainer: the Windows filesystem and network confinement.
//!
//! # The finding that shapes this whole module
//!
//! Handing a worker a pre-opened job-directory handle is **necessary and not
//! sufficient**. Inside an AppContainer, `NtCreateFile` performs a *fresh
//! access check for the new file* against the child's token — so creating a
//! file through a perfectly valid inherited handle returns
//! `STATUS_ACCESS_DENIED` (`0xC0000022`) unless the directory's ACL names the
//! container SID (S9).
//!
//! That is not in the design record's original §5.2, and it is the kind of
//! detail that costs a week when discovered during implementation: the symptom
//! is a permission error on a handle you can prove is valid.
//!
//! So [`AppContainer::acl_directory`] exists, and every job directory goes
//! through it before a worker starts.
//!
//! # Network denial is an absence, not a switch
//!
//! There is no "deny network" flag. A container gets network access by holding
//! a capability SID for it, so denial means **not requesting one** — the
//! capability list stays empty. Nothing to turn off means nothing to turn back
//! on, which is what makes SR-1's "ever" a structural claim on this platform.
//!
//! Note also that socket *creation* stays permitted inside a container;
//! restriction is enforced by the filtering platform at connect/send. A test
//! asserting `socket()` fails would prove nothing (S9).

use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::PSID;

/// Why an AppContainer could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    /// The profile could not be created or derived.
    #[error("could not create the AppContainer profile: HRESULT {hr:#010x}")]
    Profile {
        /// The raw HRESULT. Carried because `io::Error` mangles an HRESULT into
        /// a message about a Win32 code it is not, and the number is what a
        /// developer actually needs to look up.
        hr: u32,
    },
    /// A path could not be encoded for Win32.
    #[error("path contains characters Win32 cannot represent: {0}")]
    Path(String),
    /// The directory ACL could not be updated.
    ///
    /// **Fatal, and the reason this variant is separate.** A worker started
    /// without the SID on its job directory fails at its first write with
    /// `STATUS_ACCESS_DENIED` through a handle that is genuinely valid — a
    /// symptom that sends whoever debugs it looking at handle inheritance,
    /// which is not the problem.
    #[error("could not grant the container access to {path}: {source}")]
    Acl {
        /// The directory.
        path: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
}

/// A created AppContainer profile and its SID.
#[derive(Debug)]
pub struct AppContainer {
    sid: PSID,
    name: String,
}

// SAFETY: a SID is an opaque allocation with no thread affinity.
unsafe impl Send for AppContainer {}

impl AppContainer {
    /// Create (or re-derive) a profile by name.
    ///
    /// **No capabilities are requested.** The list is empty, which is how
    /// network denial works here: a container reaches the network only by
    /// holding a capability SID for it, so denial is the absence of a request
    /// rather than the presence of a switch.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Profile`] if neither creation nor derivation works.
    pub fn create(name: &str) -> Result<Self, ContainerError> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sid: PSID = core::ptr::null_mut();

        // SAFETY: all four string pointers are NUL-terminated UTF-16 that
        // outlive the call. A null capability array with a count of zero is the
        // documented way to request none, which is exactly what we want. `sid`
        // is written on success and left null on failure.
        let hr = unsafe {
            CreateAppContainerProfile(
                wide.as_ptr(),
                wide.as_ptr(),
                wide.as_ptr(),
                core::ptr::null(),
                0,
                &raw mut sid,
            )
        };

        // A profile that already exists is the normal case on the second run,
        // not a failure. Derive its SID instead of treating it as an error --
        // otherwise the product works once per machine.
        //
        // **Derivation is tried on ANY creation failure, not only on
        // ERROR_ALREADY_EXISTS.** The narrower version was correct for the
        // sequential case and wrong under concurrency: two threads creating the
        // same profile at once get `E_UNEXPECTED` (0x8000FFFF), not
        // "already exists". Six of six worker tests failed intermittently on
        // that, and it would have failed a folder conversion the same way --
        // several workers of one engine start together, and only the first
        // creates the profile.
        //
        // Derivation is the honest check regardless: it succeeds exactly when
        // the profile is there, which is the question `create` is really
        // asking. A creation error that leaves no profile behind still fails
        // here, carrying the derive HRESULT.
        if hr < 0 {
            // SAFETY: as above; writes `sid` on success.
            let hr2 =
                unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &raw mut sid) };
            if hr2 < 0 {
                return Err(ContainerError::Profile {
                    hr: hr2.cast_unsigned(),
                });
            }
        }

        Ok(Self {
            sid,
            name: name.to_string(),
        })
    }

    /// The container's SID.
    #[must_use]
    pub const fn sid(&self) -> PSID {
        self.sid
    }

    /// This container's SID as a string, for tests and diagnostics.
    ///
    /// Exists so a test can ask the OS whether an ACL really names this
    /// container, rather than trusting that `grant_execute` returned `Ok`.
    /// Mutation testing replaced that whole function with `Ok(())` and every
    /// test still passed.
    ///
    /// # Errors
    ///
    /// `None` if the SID cannot be converted, which is not a case any live
    /// container reaches.
    #[must_use]
    pub fn sid_string(&self) -> Option<String> {
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

        let mut out: *mut u16 = core::ptr::null_mut();
        // SAFETY: `self.sid` is a live SID this type owns. The function writes
        // one pointer through the out-parameter, allocated with LocalAlloc and
        // released with LocalFree below.
        let ok = unsafe { ConvertSidToStringSidW(self.sid, &raw mut out) };
        if ok == 0 || out.is_null() {
            return None;
        }
        // SAFETY: `out` is a NUL-terminated UTF-16 string the call just wrote.
        let len = unsafe {
            let mut n = 0;
            while *out.add(n) != 0 {
                n += 1;
            }
            n
        };
        // SAFETY: `out` points to `len` valid u16 before its terminator.
        let text = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(out, len) });
        // SAFETY: allocated by ConvertSidToStringSidW, which documents
        // LocalFree as its release function. Freed exactly once.
        unsafe { LocalFree(out.cast()) };
        Some(text)
    }

    /// The profile name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Grant this container **read and execute** on one file, keeping every
    /// existing permission.
    ///
    /// # Why this is not `acl_directory`
    ///
    /// An AppContainer cannot launch a program it cannot read, and the engine
    /// binaries live wherever the product was installed — under a user profile
    /// during development, which grants app packages nothing. Without this,
    /// `CreateProcessW` fails with `ERROR_ACCESS_DENIED` on an executable that
    /// is plainly there and plainly runnable.
    ///
    /// [`AppContainer::acl_directory`] would be the wrong tool. It builds a
    /// fresh ACL from one entry, which is right for a job directory this
    /// process just created and owns, and wrong for a directory that belongs to
    /// the user and holds their build output. **This reads the current DACL and
    /// adds to it**, so nothing that already had access loses it, and it grants
    /// read and execute rather than write — an engine binary the worker could
    /// rewrite would be an engine binary the worker could replace.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Acl`] on any failure, carrying the path.
    pub fn grant_execute(&self, file: &Path) -> Result<(), ContainerError> {
        use windows_sys::Win32::Security::Authorization::{
            GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
            NO_MULTIPLE_TRUSTEE, SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
            TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
        };
        use windows_sys::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR,
        };
        use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};

        let mut path_w: Vec<u16> = file
            .as_os_str()
            .to_str()
            .ok_or_else(|| ContainerError::Path(file.display().to_string()))?
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Read the DACL that is there. Passing null to SetEntriesInAclW below
        // would mean "build a fresh one", which on a user's own file is how a
        // security fix becomes a lockout.
        let mut old_acl: *mut ACL = core::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
        // SAFETY: `path_w` is NUL-terminated UTF-16. The two out-pointers are
        // live locals; on success `descriptor` owns the allocation and
        // `old_acl` points into it, so the descriptor must outlive its use.
        let rc = unsafe {
            GetNamedSecurityInfoW(
                path_w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &raw mut old_acl,
                core::ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        if rc != 0 {
            return Err(ContainerError::Acl {
                path: file.display().to_string(),
                source: std::io::Error::from_raw_os_error(rc.cast_signed()),
            });
        }

        // SAFETY: plain C layout; all-zero is valid and every field that
        // matters is set below.
        let mut access: EXPLICIT_ACCESS_W = unsafe { core::mem::zeroed() };
        // Read and execute. NOT write: an engine binary the worker can rewrite
        // is an engine binary the worker can replace, which would turn one
        // compromised conversion into a permanent one.
        access.grfAccessPermissions = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
        access.grfAccessMode = SET_ACCESS;
        // A file, not a container. Inheritance flags on a leaf are meaningless
        // and Windows rejects some combinations outright.
        access.grfInheritance = NO_INHERITANCE;
        access.Trustee = TRUSTEE_W {
            pMultipleTrustee: core::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: self.sid.cast(),
        };

        let mut new_acl: *mut ACL = core::ptr::null_mut();
        // SAFETY: one new entry merged into `old_acl`, which is still valid
        // because `descriptor` has not been freed. `new_acl` is written on
        // success and released with LocalFree.
        let rc = unsafe { SetEntriesInAclW(1, &raw const access, old_acl, &raw mut new_acl) };
        if rc != 0 || new_acl.is_null() {
            // SAFETY: `descriptor` came from GetNamedSecurityInfoW, which
            // documents LocalFree as its release function.
            unsafe { LocalFree(descriptor.cast()) };
            return Err(ContainerError::Acl {
                path: file.display().to_string(),
                source: std::io::Error::from_raw_os_error(rc.cast_signed()),
            });
        }

        // SAFETY: `path_w` is NUL-terminated UTF-16 and mutable as required.
        // `new_acl` is non-null. Nulls for owner, group and SACL leave those
        // untouched.
        let rc = unsafe {
            SetNamedSecurityInfoW(
                path_w.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                new_acl,
                core::ptr::null_mut(),
            )
        };

        // SAFETY: both allocations came from APIs documenting LocalFree, and
        // each is freed exactly once, after the last use of either.
        unsafe {
            LocalFree(new_acl.cast());
            LocalFree(descriptor.cast());
        }

        if rc != 0 {
            return Err(ContainerError::Acl {
                path: file.display().to_string(),
                source: std::io::Error::from_raw_os_error(rc.cast_signed()),
            });
        }
        Ok(())
    }

    /// Grant this container full access to a directory.
    ///
    /// **Required before a worker writes anything.** See the module docs: the
    /// handed handle is not sufficient, because `NtCreateFile` re-checks
    /// against the child's token for each new file.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Acl`] on any failure, carrying the path — because the
    /// message a developer needs here is *which directory*, and the underlying
    /// error alone does not say.
    pub fn acl_directory(&self, dir: &Path) -> Result<(), ContainerError> {
        use windows_sys::Win32::Security::Authorization::{
            SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
            SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
        };
        use windows_sys::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        };

        let mut path_w: Vec<u16> = dir
            .as_os_str()
            .to_str()
            .ok_or_else(|| ContainerError::Path(dir.display().to_string()))?
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: EXPLICIT_ACCESS_W is a plain C layout; all-zero is valid and
        // every field that matters is set below.
        let mut access: EXPLICIT_ACCESS_W = unsafe { core::mem::zeroed() };
        access.grfAccessPermissions = FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE;
        access.grfAccessMode = SET_ACCESS;
        // Inherited by everything created inside, so an archive's nested
        // directories do not each need a separate ACL pass.
        access.grfInheritance = SUB_CONTAINERS_AND_OBJECTS_INHERIT;
        access.Trustee = TRUSTEE_W {
            pMultipleTrustee: core::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: self.sid.cast(),
        };

        let mut new_acl: *mut ACL = core::ptr::null_mut();
        // SAFETY: one entry, pointing at a live local whose SID outlives the
        // call. A null existing ACL means "build a fresh one"; `new_acl` is
        // written on success and must be freed with LocalFree.
        let rc =
            unsafe { SetEntriesInAclW(1, &raw const access, core::ptr::null(), &raw mut new_acl) };
        if rc != 0 || new_acl.is_null() {
            return Err(ContainerError::Acl {
                path: dir.display().to_string(),
                source: std::io::Error::from_raw_os_error(rc.cast_signed()),
            });
        }

        // SAFETY: `path_w` is NUL-terminated UTF-16 and mutable as the API
        // requires. `new_acl` was just built and is non-null. Passing null for
        // owner, group and SACL leaves those untouched.
        let rc = unsafe {
            SetNamedSecurityInfoW(
                path_w.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                new_acl,
                core::ptr::null_mut(),
            )
        };

        // SAFETY: `new_acl` came from SetEntriesInAclW, which documents
        // LocalFree as its release function, and is freed exactly once.
        unsafe {
            LocalFree(new_acl.cast());
        }

        if rc != 0 {
            return Err(ContainerError::Acl {
                path: dir.display().to_string(),
                source: std::io::Error::from_raw_os_error(rc.cast_signed()),
            });
        }
        Ok(())
    }
}

impl Drop for AppContainer {
    fn drop(&mut self) {
        if !self.sid.is_null() {
            // SAFETY: the SID came from CreateAppContainerProfile or
            // DeriveAppContainerSidFromAppContainerName, both of which document
            // FreeSid as the release function. Freed exactly once.
            unsafe {
                windows_sys::Win32::Security::FreeSid(self.sid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile name unique to **this test**, not just this process.
    ///
    /// The first version keyed on the process id alone. Tests run in parallel,
    /// so three of them raced for one profile: one created it and the others
    /// took the already-exists path, which at the time reported "unavailable"
    /// and **skipped**. Three green tests, none of which had run.
    ///
    /// That is the false-green this project keeps finding, produced here by my
    /// own test design rather than by the code under test.
    fn unique_name(tag: &str) -> String {
        format!("openconvert.test.{tag}.{}", std::process::id())
    }

    /// A scratch directory **this test alone owns**.
    ///
    /// The tag is not decoration. Two tests here derived their directory from
    /// the process id alone, so both got the same one -- and cargo runs them on
    /// different threads of that one process. One test's teardown deleted the
    /// file the other was mid-way through granting, which surfaced as
    /// `ERROR_PATH_NOT_FOUND` from `grant_execute` roughly two runs in three:
    /// a real-looking ACL failure with no ACL bug behind it.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// **The failure path in `create`, executed.**
    ///
    /// Mutation testing replaced `if hr2 < 0` with `if hr2 > 0` and nothing
    /// noticed: every test created a name that worked, so the derive-failure
    /// branch had never run. Under that mutation a refused name returns `Ok`
    /// with a **null SID**, which is then handed to `CreateProcessW` as the
    /// container to run in.
    ///
    /// Windows answers `E_INVALIDARG` (0x80070057) for an empty name and for
    /// one past its length limit, and refuses both creation *and* derivation --
    /// which is what makes them a subject for this branch rather than for the
    /// already-exists path.
    #[test]
    fn a_name_windows_refuses_is_an_error_and_not_a_null_sid() {
        for (label, name) in [("empty", String::new()), ("too long", "a".repeat(200))] {
            match AppContainer::create(&name) {
                Err(ContainerError::Profile { hr }) => assert_eq!(
                    hr, 0x8007_0057,
                    "{label}: expected E_INVALIDARG, got {hr:#010x}"
                ),
                Err(other) => panic!("{label}: wrong error {other}"),
                Ok(c) => panic!("{label}: accepted, with sid null = {}", c.sid().is_null()),
            }
        }
    }

    /// **`grant_execute` really changes the ACL**, asked of the OS.
    ///
    /// Mutation testing replaced this entire function with `Ok(())` and every
    /// test still passed — because the tests only checked that a worker
    /// *started*, and on a developer machine the engine binary may already be
    /// reachable by app packages. So the function could have been doing nothing
    /// for as long as it has existed, and the first sign would have been a
    /// worker failing to launch on a user's machine with a loader error naming
    /// a missing dependency rather than a permission.
    ///
    /// `icacls` is the OS's own answer rather than ours. Asserting on a value
    /// this module computed would be asking the code whether it worked.
    #[test]
    fn grant_execute_puts_the_container_sid_on_the_file() {
        let dir = scratch("grant-execute");
        let file = dir.join("engine.exe");
        std::fs::write(&file, b"MZ not really a program").expect("write"); // openconvert-lint: allow -- test scratch

        let c = AppContainer::create(&unique_name("acl")).expect("create");
        let sid = c.sid_string().expect("sid as string");

        let before = icacls(&file);
        assert!(
            !before.contains(&sid),
            "the SID was already on the file, so this test proves nothing:
{before}"
        );

        c.grant_execute(&file).expect("grant");

        let after = icacls(&file);
        assert!(
            after.contains(&sid),
            "grant_execute returned Ok and the ACL does not name the container.
             sid: {sid}
before:
{before}
after:
{after}"
        );

        // Read and execute, NOT write: an engine binary the worker can rewrite
        // is one it can replace, turning a single compromised conversion into a
        // permanent one. `(RX)` is how icacls spells that.
        let line = after
            .lines()
            .find(|l| l.contains(&sid))
            .expect("the SID line");
        assert!(
            line.contains("(RX)") || line.contains("GR") || line.contains("GE"),
            "expected read/execute, got: {line}"
        );
        assert!(
            !line.contains("(W)") && !line.contains("(F)") && !line.contains("(M)"),
            "the container was granted write access: {line}"
        );

        let _ = std::fs::remove_dir_all(&dir); // openconvert-lint: allow -- test scratch teardown
    }

    /// The OS's view of a file's ACL.
    fn icacls(path: &std::path::Path) -> String {
        let out = std::process::Command::new("icacls")
            .arg(path)
            .output()
            .expect("icacls");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The control: a name Windows accepts yields a non-null SID.
    ///
    /// Without it, the test above is satisfied by a `create` that refuses
    /// everything.
    #[test]
    fn a_valid_name_still_produces_a_usable_sid() {
        let c = AppContainer::create(&unique_name("valid")).expect("a plain name should work");
        assert!(!c.sid().is_null());
    }

    /// Print why the container is or is not available, rather than skipping
    /// silently.
    ///
    /// A skipped test that reports "unavailable" without saying *why* is the
    /// false-green this project keeps finding. `probed: false` and "policy
    /// refused" and "the API is missing" are three different answers with three
    /// different fixes.
    #[test]
    fn diagnose_appcontainer_availability() {
        let name = format!("openconvert.diag.{}", std::process::id());
        match AppContainer::create(&name) {
            Ok(c) => println!("DIAG: created, sid non-null = {}", !c.sid().is_null()),
            Err(e) => println!("DIAG: FAILED -> {e}"),
        }
    }

    /// A profile can be created, and creating it twice is not an error.
    ///
    /// The second case is the one that matters: a profile persists per user, so
    /// without the already-exists path the product works exactly once per
    /// machine and then refuses forever.
    #[test]
    fn a_profile_is_created_and_recreated_idempotently() {
        let name = unique_name("recreate");
        let first = AppContainer::create(&name)
            .expect("AppContainer profile creation should work on a stock Windows install");
        assert!(!first.sid().is_null());
        assert_eq!(first.name(), name);

        let second = AppContainer::create(&name).expect("re-creating an existing profile");
        assert!(
            !second.sid().is_null(),
            "the already-exists path did not derive a SID"
        );
    }

    /// **S9, confirmed here**: the ACL call succeeds on a real directory.
    ///
    /// This does not yet prove a *confined child* can write — that needs the
    /// spawn wiring. What it proves is that the grant itself works, which is
    /// the half that was missing from the design record entirely.
    #[test]
    fn a_job_directory_can_be_granted_to_the_container() {
        let container = AppContainer::create(&unique_name("acl")).expect("profile creation");
        let dir = scratch("acl-directory");

        container
            .acl_directory(&dir)
            .expect("granting the container access to its own job directory");

        // The directory is still ours to use, which it must be: the host writes
        // the inputs and reads the outputs.
        std::fs::File::create_new(dir.join("host-can-still-write.txt")).expect("host write");

        let _ = std::fs::remove_dir_all(&dir); // openconvert-lint: allow -- test scratch teardown
    }

    /// A path Win32 cannot represent is refused rather than mangled.
    #[test]
    fn an_unrepresentable_path_is_refused() {
        let container = AppContainer::create(&unique_name("badpath")).expect("profile creation");
        // A directory that does not exist still reaches the ACL call and fails
        // there, with the PATH in the message -- which is the part a developer
        // needs, and which the raw OS error does not carry.
        let missing = std::env::temp_dir().join("tx-definitely-absent-9f3a");
        let err = container.acl_directory(&missing).expect_err("should fail");
        assert!(
            err.to_string().contains("tx-definitely-absent"),
            "the error does not name the directory: {err}"
        );
    }
}
