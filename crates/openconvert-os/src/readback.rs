//! What confinement **actually engaged** — asked of the OS, not of our own
//! intentions.
//!
//! # Why this module exists at all
//!
//! I11 seals `SandboxProfile` so only this crate may assert what confinement is
//! in force. That guards against a *dishonest* constructor. It does not guard
//! against an **honest one recording a request the OS ignored** — and
//! measurement showed that error running in **both** directions.
//!
//! A 2×2 matrix, plain process vs AppContainer, mitigation requested vs not,
//! reading `ProhibitDynamicCode` back from each cell (S17b, S20b):
//!
//! | | not requested | requested |
//! |---|---|---|
//! | **plain process** | 0 | 0 |
//! | **AppContainer** | **0** ← corrected | **1** |
//!
//! # The bottom-left cell has been wrong twice
//!
//! It first read `0` from a *plain process* and was written up as "ACG is
//! requested successfully and silently declines". That was corrected to `1`,
//! on the grounds that ACG is a property of the container rather than of the
//! request — and this module's own comments stated it as settled fact.
//!
//! **It is `0`.** Measured three times against the real
//! `spawn_piped_in_container`, on Windows 11 26200: the child's token reports
//! `TokenIsAppContainer` true and `ProcessDynamicCodePolicy` false. An
//! AppContainer does not bring ACG with it, so the spawn now asks for it
//! explicitly and the child confirms it engaged.
//!
//! Two corrections to one cell is worth stating plainly: a mitigation nobody
//! queries is a mitigation nobody has. Both errors were *optimistic* — each
//! credited the product with confinement it did not have — which is the only
//! direction that matters, and the reason this file asks the OS instead of
//! trusting the write-up.
//!
//! **CIG genuinely does not engage.** Re-tested the same way, a non-Microsoft
//! DLL loaded in all four cells.
//!
//! So the receipt records what the OS says, queried in the child after it
//! starts. A receipt describing what we *asked for* is a receipt about our
//! intentions; only one describing what *happened* is evidence.

#[cfg(feature = "attest")]
use openconvert_core::isolation::{
    FsConfinement, NetConfinement, PrivDrop, ResourceEnforcement, SandboxProfile, SyscallFilter,
};

/// One mitigation, and whether the OS says it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mitigation {
    /// Name as it appears in the receipt and the UI.
    pub name: &'static str,
    /// Whether we asked for it.
    pub requested: bool,
    /// Whether the OS reports it active **now**, in this process.
    pub engaged: bool,
}

impl Mitigation {
    /// Requested and not delivered.
    ///
    /// A first-class event: it lowers the reported tier, appears in the plan
    /// preview, and is recorded in the receipt. Silently accepting the gap is
    /// how a receipt comes to describe a sandbox that is not there.
    #[must_use]
    pub const fn diverged(&self) -> bool {
        self.requested && !self.engaged
    }
}

/// Everything read back from the current process.
#[derive(Debug, Clone, Default)]
pub struct ReadBack {
    /// Per-mitigation results, in a stable order for the receipt.
    pub mitigations: Vec<Mitigation>,
}

impl ReadBack {
    /// Mitigations that were requested and did not engage.
    #[must_use]
    pub fn divergences(&self) -> Vec<&Mitigation> {
        self.mitigations.iter().filter(|m| m.diverged()).collect()
    }

    /// Whether a named mitigation is active.
    #[must_use]
    pub fn engaged(&self, name: &str) -> bool {
        self.mitigations.iter().any(|m| m.name == name && m.engaged)
    }
}

/// Read the confinement state of **this** process.
///
/// Called in the child after it starts, and the answer travels back over the
/// protocol as a `wire::` type.
#[must_use]
pub fn read_back(requested: &[&'static str]) -> ReadBack {
    #[cfg(windows)]
    {
        windows_readback(requested)
    }
    #[cfg(not(windows))]
    {
        // Honest placeholder. Returning an empty read-back means "we know
        // nothing", which lowers the reported tier -- the safe direction.
        // Claiming mitigations we have not queried would be the unsafe one, and
        // it is exactly the error this module exists to prevent.
        let _ = requested;
        ReadBack::default()
    }
}

#[cfg(windows)]
fn windows_readback(requested: &[&'static str]) -> ReadBack {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, ProcessDynamicCodePolicy,
        ProcessSignaturePolicy, PROCESS_MITIGATION_POLICY,
    };

    /// Query one mitigation policy class.
    ///
    /// # Why a bare `u32` rather than the named policy struct
    ///
    /// Both classes read here are a single `DWORD` of flags — the Windows
    /// headers wrap that DWORD in a union with a bitfield over the same bits,
    /// purely for readability. `windows-sys` does not export those wrappers in
    /// every release, and depending on a struct that moves between binding
    /// versions is a build break waiting for a `cargo update`.
    ///
    /// The DWORD is the stable part of the contract, so that is what we ask
    /// for. Bit 0 of each is the flag we want: `ProhibitDynamicCode` for ACG,
    /// `MicrosoftSignedOnly` for CIG.
    fn policy_flag(class: PROCESS_MITIGATION_POLICY) -> bool {
        let mut flags: u32 = 0;
        // SAFETY: `GetProcessMitigationPolicy` writes exactly `cb` bytes into
        // the buffer for the named class. Both classes used here are a single
        // DWORD, `cb` is that DWORD's size, and the pointer is to a live local
        // of exactly that type. `GetCurrentProcess` returns a pseudo-handle
        // that needs no closing and is always valid.
        //
        // A failed call leaves `flags` at zero, which reads as "not engaged" --
        // the conservative answer, and the right one: a query we could not make
        // must never become a mitigation we claim.
        let ok = unsafe {
            GetProcessMitigationPolicy(
                GetCurrentProcess(),
                class,
                core::ptr::from_mut(&mut flags).cast(),
                core::mem::size_of::<u32>(),
            )
        };
        ok != 0 && (flags & 1) != 0
    }

    // ACG (Arbitrary Code Guard) -- ProhibitDynamicCode.
    //
    // THIS COMMENT USED TO SAY ACG "engages because the process is in an
    // AppContainer, whether or not it was asked for", citing spike S17b. On the
    // production spawn that is false, measured three times on Windows 11 26200:
    // the child reports TokenIsAppContainer true and ProcessDynamicCodePolicy
    // false.
    //
    // So `spawn_piped_in_container` now requests it explicitly, via
    // PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, and the child confirms it
    // engaged. Whatever S17b measured, it was not the path the product takes.
    //
    // The correction arrived by exactly the mechanism this module argues for --
    // ask the OS in the child rather than record what the host requested --
    // landing this time on a claim the read-back design had itself produced.
    //
    // CIG (Code Integrity Guard) -- MicrosoftSignedOnly.
    //
    // Measured NOT to engage in any of the four cells tested, including inside
    // an AppContainer that requested it. Queried anyway: the day a Windows
    // build starts honouring it, the receipt should say so without anyone
    // editing this file.
    ReadBack {
        mitigations: vec![
            Mitigation {
                name: "ACG",
                requested: requested.contains(&"ACG"),
                engaged: policy_flag(ProcessDynamicCodePolicy),
            },
            Mitigation {
                name: "CIG",
                requested: requested.contains(&"CIG"),
                engaged: policy_flag(ProcessSignaturePolicy),
            },
            // The one that carries SR-1.
            //
            // ACG and CIG bound what the process may *execute*. Only the
            // container bounds what it may *reach* -- the filesystem it cannot
            // see, and the network it holds no capability for.
            //
            // Asked in the CHILD, of the child's own token, which is the only
            // place the answer means anything (I11, `03` §9.5). The host knows
            // it passed `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`; it does
            // not know the kernel honoured it, and the entire read-back design
            // exists because on this platform that gap has been real in both
            // directions (spikes S17b, S20b).
            Mitigation {
                name: "AppContainer",
                requested: requested.contains(&"AppContainer"),
                engaged: in_app_container(),
            },
        ],
    }
}

/// Whether **this** process runs inside an AppContainer.
///
/// `TokenIsAppContainer` is a `DWORD` on the process token: non-zero means the
/// token is an AppContainer token. A failed query reads as `false`, which is
/// the conservative direction — a question we could not ask must never become
/// a confinement we claim.
#[cfg(windows)]
fn in_app_container() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{TokenIsAppContainer, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = core::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` is a pseudo-handle needing no close. `token`
    // is a live local written on success and left null on failure.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
    if ok == 0 || token.is_null() {
        return false;
    }

    let mut is_container: u32 = 0;
    let mut returned: u32 = 0;
    // SAFETY: `token` is a live handle opened above. `TokenIsAppContainer`
    // writes exactly one DWORD, and the buffer is a live local of that type
    // with its true size passed.
    let ok = unsafe {
        windows_sys::Win32::Security::GetTokenInformation(
            token,
            TokenIsAppContainer,
            core::ptr::from_mut(&mut is_container).cast(),
            core::mem::size_of::<u32>() as u32,
            &raw mut returned,
        )
    };
    // SAFETY: `token` came from OpenProcessToken and is closed exactly once.
    unsafe { CloseHandle(token) };

    ok != 0 && is_container != 0
}

/// Build the sealed profile from what was read back.
///
/// The **only** path to a `SandboxProfile` that claims anything, and it is
/// behind `feature = "attest"` (I11) so that enabling it is a visible
/// dependency-manifest diff.
#[cfg(feature = "attest")]
#[must_use]
pub fn attest_from(
    filesystem: FsConfinement,
    network: NetConfinement,
    syscalls: SyscallFilter,
    resources: ResourceEnforcement,
    privileges: PrivDrop,
) -> SandboxProfile {
    SandboxProfile::attest(filesystem, network, syscalls, resources, privileges)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read-back runs and answers, whatever the platform.
    ///
    /// On Windows it queries the OS; elsewhere it returns "we know nothing",
    /// which is the conservative answer rather than a fabricated one.
    #[test]
    fn read_back_answers_without_panicking() {
        let r = read_back(&["ACG", "CIG", "AppContainer"]);
        if cfg!(windows) {
            assert_eq!(
                r.mitigations.len(),
                3,
                "all three mitigations should be queried"
            );
            assert!(r.mitigations.iter().any(|m| m.name == "ACG"));
            assert!(r.mitigations.iter().any(|m| m.name == "CIG"));
            assert!(r.mitigations.iter().any(|m| m.name == "AppContainer"));
        } else {
            assert!(
                r.mitigations.is_empty(),
                "a platform we have not implemented must claim nothing"
            );
        }
    }

    /// `engaged` distinguishes on, off, and absent.
    ///
    /// Built by hand, because nothing in a `cargo test` process is confined —
    /// so every OS-driven assertion in this file is satisfied by an `engaged`
    /// that returns `false` unconditionally. Mutation testing found exactly
    /// that: `-> false` and `==` flipped to `!=` both survived.
    ///
    /// It matters because `engaged` is what the receipt is built from. A
    /// version stuck on `false` under-claims, which is safe; one stuck on
    /// `true` claims confinement that is not there, which is the failure this
    /// whole module exists to prevent. Neither was covered.
    #[test]
    fn engaged_reports_on_off_and_absent() {
        let r = ReadBack {
            mitigations: vec![
                Mitigation {
                    name: "ACG",
                    requested: true,
                    engaged: true,
                },
                Mitigation {
                    name: "CIG",
                    requested: true,
                    engaged: false,
                },
            ],
        };
        assert!(
            r.engaged("ACG"),
            "an engaged mitigation must read as engaged"
        );
        assert!(
            !r.engaged("CIG"),
            "a mitigation that did not engage must not"
        );
        assert!(
            !r.engaged("AppContainer"),
            "a mitigation never queried must not read as engaged"
        );
    }

    /// A divergence is a request that did not engage — and nothing else.
    #[test]
    fn divergences_are_exactly_the_requests_that_did_not_engage() {
        let r = ReadBack {
            mitigations: vec![
                // Requested, engaged: no divergence.
                Mitigation {
                    name: "ACG",
                    requested: true,
                    engaged: true,
                },
                // Requested, did not engage: THE divergence.
                Mitigation {
                    name: "CIG",
                    requested: true,
                    engaged: false,
                },
                // Engaged without being asked for. Not a divergence to report
                // as a shortfall -- it is the OS giving more than requested,
                // which is the direction S17b once claimed for ACG.
                Mitigation {
                    name: "AppContainer",
                    requested: false,
                    engaged: true,
                },
            ],
        };
        let names: Vec<_> = r.divergences().iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["CIG"]);
    }

    /// The test harness is **not** in an AppContainer, and says so.
    ///
    /// The control for the container assertion in
    /// `oc-images/tests/host_drives_worker.rs`. Without it, a
    /// `TokenIsAppContainer` query hard-wired to `true` would satisfy that test
    /// while proving nothing — and "the confinement check always says yes" is
    /// the worst possible bug in this file.
    #[test]
    #[cfg(windows)]
    fn the_test_harness_is_not_in_an_appcontainer() {
        let r = read_back(&["AppContainer"]);
        assert!(
            !r.engaged("AppContainer"),
            "cargo test does not run in an AppContainer; a `true` here means the \
             query is not really asking the OS"
        );
    }

    /// **The measured expectation, asserted.**
    ///
    /// This test process is *not* in an AppContainer, so ACG must read back as
    /// **not engaged** — which is exactly the cell that produced this project's
    /// published error. Measuring a plain process and generalising to the
    /// container is the mistake; this pins the plain-process value so the
    /// distinction stays visible.
    #[test]
    #[cfg(windows)]
    fn a_plain_process_reports_acg_off() {
        let r = read_back(&[]);
        assert!(
            !r.engaged("ACG"),
            "ACG reads as engaged in an ordinary test process, which contradicts \
             the 2x2 matrix in spikes S17b/S20b"
        );
        assert!(!r.engaged("CIG"), "CIG did not engage in any measured cell");
    }

    /// A requested-but-absent mitigation is a divergence, and divergence is
    /// reportable.
    #[test]
    fn a_requested_mitigation_that_did_not_engage_is_flagged() {
        let r = ReadBack {
            mitigations: vec![
                Mitigation {
                    name: "CIG",
                    requested: true,
                    engaged: false,
                },
                Mitigation {
                    name: "ACG",
                    requested: false,
                    engaged: true,
                },
            ],
        };
        let d = r.divergences();
        assert_eq!(d.len(), 1, "exactly one divergence expected");
        assert_eq!(d[0].name, "CIG");

        // The control: engaging without being requested is NOT a divergence.
        // That is ACG's actual behaviour inside a container, and treating it as
        // a fault would make every real conversion report one.
        assert!(!r.mitigations[1].diverged());
    }

    /// Requesting nothing and getting nothing is not a divergence either.
    #[test]
    fn absence_without_a_request_is_not_a_divergence() {
        let r = ReadBack {
            mitigations: vec![Mitigation {
                name: "CIG",
                requested: false,
                engaged: false,
            }],
        };
        assert!(r.divergences().is_empty());
    }
}
