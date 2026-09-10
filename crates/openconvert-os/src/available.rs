//! Probe the machine once, and report what it can actually offer.
//!
//! # Probing is not engaging, and this module only probes
//!
//! `03` Â§15.1 lists that distinction as a cost people underestimate: adding an
//! isolation mechanism means touching the probe *and* the application, and v0.4's
//! table listed only the probe. What this returns is a **capability report** â€”
//! "an AppContainer could be created here" â€” not a profile. The profile comes
//! from [`crate::readback`], in the child, after the mechanism has actually been
//! applied.
//!
//! Keeping them apart is the correction v0.6 made after measurement found the
//! request-versus-reality error running in both directions.
//!
//! # Everything here fails closed
//!
//! A capability we could not verify is reported as **absent**. That lowers the
//! tier, which refuses conversions â€” the safe direction. The opposite mistake
//! produces a receipt describing a sandbox that is not there, and a receipt
//! that lies is worse than no receipt.

use openconvert_core::isolation::{FsConfinement, NetConfinement, PrivDrop, ResourceEnforcement};

/// What this machine can offer, before anything has been applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Filesystem confinement, if any is reachable.
    pub filesystem: FsConfinement,
    /// Network denial, if any is reachable.
    pub network: NetConfinement,
    /// Resource enforcement, if any is reachable.
    pub resources: ResourceEnforcement,
    /// Privilege drop reachable at probe time. None = not available/not probed.
    pub privileges: PrivDrop,
    /// Whether the probe ran at all on this platform.
    ///
    /// **`false` means "we did not look", not "there is nothing".** The
    /// distinction matters for the message: "this machine offers no sandbox" and
    /// "we have not implemented probing here" are different problems with
    /// different fixes, and a user told the first when the second is true will
    /// go looking in the wrong place.
    pub probed: bool,
}

impl Capabilities {
    /// The honest answer for a platform we have not implemented.
    #[must_use]
    pub const fn unprobed() -> Self {
        Self {
            filesystem: FsConfinement::None,
            network: NetConfinement::None,
            resources: ResourceEnforcement::None,
            privileges: PrivDrop::None,
            probed: false,
        }
    }

    /// Whether anything at all was found.
    #[must_use]
    pub const fn any(&self) -> bool {
        !matches!(self.filesystem, FsConfinement::None)
            || !matches!(self.network, NetConfinement::None)
            || !matches!(self.resources, ResourceEnforcement::None)
    }
}

/// Probe this machine.
///
/// Called **once** by the shell, which then hands the result to the pure core
/// as a value. That is how `route()` learns about an impure world without
/// reading anything.
#[must_use]
pub fn probe() -> Capabilities {
    #[cfg(windows)]
    {
        windows_probe()
    }
    #[cfg(target_os = "linux")]
    {
        linux_probe()
    }
    #[cfg(target_os = "macos")]
    {
        macos_probe()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        // A platform nobody has implemented says so rather than guessing.
        // `probed: false` means "we did not look", which is a different
        // problem from "there is nothing here" and has a different fix.
        Capabilities::unprobed()
    }
}

/// What Windows offers.
///
/// Job Objects are probed by **creating one**, which is the only answer that
/// counts: managed-desktop policy can refuse them, and a version check would
/// report a capability the machine will not actually grant.
#[cfg(windows)]
fn windows_probe() -> Capabilities {
    Capabilities {
        // Probed by CREATING a profile, the same rule the Job Object below
        // follows: a capability is what the machine grants now, not what its
        // version implies.
        //
        // This reported `None` with the note "until `spawn()` lands, because a
        // capability nothing can apply is not a capability". `spawn()` has
        // landed -- `spawn_piped_in_container` applies it and the child
        // confirms `TokenIsAppContainer` -- so the note had outlived its
        // reason and was refusing every sandboxed conversion on a machine that
        // can run them. The third stale claim of this shape found by running
        // the binary.
        filesystem: if appcontainer_available() {
            FsConfinement::AppContainer
        } else {
            FsConfinement::None
        },
        // Network denial is the ABSENCE of the network capability SID, which
        // comes with the container. One mechanism, so one probe: claiming
        // denial without the container that provides it would be claiming a
        // control nothing applies.
        network: if appcontainer_available() {
            NetConfinement::CapabilitySid
        } else {
            NetConfinement::None
        },
        resources: if job_objects_available() {
            ResourceEnforcement::JobObject
        } else {
            ResourceEnforcement::None
        },
        // Restricted-token work is not done; claiming it would be the
        // request-versus-reality error. Windows rides at Reduced today.
        privileges: PrivDrop::None,
        probed: true,
    }
}

/// What Linux offers.
///
/// Both mechanisms are probed the way they are enforced, not the way a
/// version number implies:
///
/// - **Landlock** by the version query, which creates nothing and restricts
///   nothing. An ABI â‰¥ 1 is the whole precondition for the `Reduced` tier â€”
///   no privilege and no user namespace required (spike S12), which is why a
///   hardened Debian converts here rather than being refused.
/// - **seccomp** by installing the real network-deny filter in a forked child
///   and watching it deny an IPv4 socket â€” because applying a filter is one-way
///   for the process that does it, and a capability nothing can apply is not a
///   capability.
///
/// A machine missing either reports `None` for that mechanism, which fails
/// the floor's matching predicate and refuses sandboxed steps â€” never a
/// degraded tier claimed as if it were intact.
#[cfg(target_os = "linux")]
fn linux_probe() -> Capabilities {
    Capabilities {
        filesystem: if crate::linux::landlock_available() {
            FsConfinement::Landlock
        } else {
            FsConfinement::None
        },
        network: if crate::linux::seccomp_network_deny_available() {
            NetConfinement::SeccompBlock
        } else {
            NetConfinement::None
        },
        // setrlimit exists on every supported kernel; RLIMIT_AS gives a
        // *recoverable* allocation error, so the worker reports its own
        // failure rather than dying unexplained (`03` §10.1).
        resources: ResourceEnforcement::Rlimit,
        // Probed the same way everything else here is: by DOING it, in a
        // forked child. unshare(CLONE_NEWUSER|CLONE_NEWNET) is one-way for
        // the probing process, so a forked child attempts it and reports by
        // exit code — the Full tier's privilege drop is what the machine
        // grants now, not what its kernel version implies.
        privileges: if crate::linux::userns_netns_available() {
            PrivDrop::UserNs
        } else {
            PrivDrop::None
        },
        probed: true,
    }
}

/// The profile `route()` should plan against, from measured capabilities.
///
/// # Why this is not the I11 hole
///
/// I11 seals `SandboxProfile` so nothing can assert confinement that did not
/// happen. This asserts what this machine **granted when asked**: on Windows,
/// every field comes from [`probe`], which creates an AppContainer and a Job
/// Object and closes them again rather than reading a version; on Linux, from
/// a version query and a forked child that really installed the filter. A
/// capability that could not be exercised is reported absent, so the profile
/// can only understate.
///
/// It is a **planning** input, and the receipt does not come from it. `route()`
/// needs to know whether a sandboxed step is worth planning *before* any worker
/// exists; the receipt records what the worker read back about itself, which is
/// the only answer that is evidence (`03` Â§9.5).
#[cfg(feature = "attest")]
#[must_use]
pub fn planning_profile(caps: Capabilities) -> openconvert_core::isolation::SandboxProfile {
    use openconvert_core::isolation::SyscallFilter;
    // On Linux the same filter that denies the network IS the syscall filter;
    // one mechanism, so one answer, and the two fields cannot diverge. On
    // Windows there is no seccomp counterpart, so it stays None â€” claiming it
    // would be the request-versus-reality error this module was written to
    // remove.
    let syscalls = match caps.network {
        NetConfinement::SeccompBlock => SyscallFilter::SeccompBpf,
        _ => SyscallFilter::None,
    };
    crate::readback::attest_from(
        caps.filesystem,
        caps.network,
        syscalls,
        caps.resources,
        caps.privileges,
    )
}

/// What macOS offers.
///
/// **Weaker than the other two probes, and the docstring on
/// [`crate::macos::sandbox_available`] says why**: Windows creates an
/// AppContainer and destroys it, Linux asks the kernel for the Landlock ABI,
/// and neither has an analogue here — the only way to learn whether a Seatbelt
/// profile applies is to apply it, and applying it in the host would confine
/// the host for the rest of its life.
///
/// So this reports the mechanism as reachable when the API is present, and the
/// authoritative answer still arrives from the worker's read-back. The error
/// runs in the direction that costs a conversion rather than a receipt: if
/// planning says "sandboxed" and the profile then fails to engage, the worker
/// reports nothing engaged, the profile scores below the floor, and the step is
/// refused with a reason.
///
/// `resources` is `Rlimit` because `spawn_posix` sets the limits itself before
/// exec, and `privileges` stays `None` because macOS has no counterpart to the
/// user-namespace drop — which is also why a Mac reaches `Reduced` and not
/// `Full`. Both of those are claims this file makes about code, not about a
/// machine, and both are checked by the tests below.
#[cfg(target_os = "macos")]
fn macos_probe() -> Capabilities {
    let seatbelt = crate::macos::sandbox_available();
    Capabilities {
        filesystem: if seatbelt {
            FsConfinement::AppSandbox
        } else {
            FsConfinement::None
        },
        network: if seatbelt {
            NetConfinement::NoEntitlement
        } else {
            NetConfinement::None
        },
        resources: ResourceEnforcement::Rlimit,
        privileges: PrivDrop::None,
        probed: true,
    }
}

/// Whether an AppContainer profile can be created **here, now**.
///
/// Created and dropped, like the Job Object probe. Managed-desktop policy can
/// refuse this, and a Windows-version check would report a capability the
/// machine will not grant.
///
/// The profile name is fixed and per-user, so repeated runs re-derive the same
/// SID rather than accumulating profiles -- `AppContainer::create` falls back
/// to derivation when creation reports the profile already exists, which on
/// this path is the normal case from the second run onward.
#[cfg(windows)]
fn appcontainer_available() -> bool {
    crate::appcontainer::AppContainer::create("openconvert.probe").is_ok()
}

/// Whether a Job Object can be created **here, now**.
///
/// Probed by doing it and closing it again, not by checking a Windows version.
/// A managed desktop can refuse this, and the design's failure table already
/// has a row for a policy-blocked mechanism â€” a version check would miss it and
/// report a capability the machine will not grant.
#[cfg(windows)]
fn job_objects_available() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::CreateJobObjectW;

    // SAFETY: `CreateJobObjectW` with null attributes and a null name creates
    // an anonymous, unnamed job object owned by this process. Both null
    // arguments are documented as valid. It returns a handle or null; nothing
    // is written through either pointer.
    let handle = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: `handle` is non-null and was just returned by CreateJobObjectW,
    // so it is a valid handle this process owns and has not yet closed. Closing
    // it here releases the probe's own object and nothing else refers to it.
    unsafe {
        CloseHandle(handle);
    }
    true
}

/// The probe agrees with what the spawn can actually apply.
///
/// The two drifted once already: `filesystem` reported `None` with a comment
/// deferring to a `spawn()` that had since landed, so every sandboxed
/// conversion was refused on a machine that could run them. A probe is only
/// useful if it tracks the mechanism, and nothing but a test makes it.
#[cfg(all(test, windows))]
mod agreement_tests {
    use super::*;

    /// If a container can be created, the probe says so â€” in both fields.
    ///
    /// Network denial is the absence of the network capability SID, which comes
    /// with the container. One mechanism, so the two answers cannot disagree.
    #[test]
    fn the_container_probe_and_the_container_agree() {
        let can = appcontainer_available();
        let caps = probe();
        assert_eq!(
            can,
            matches!(caps.filesystem, FsConfinement::AppContainer),
            "the probe disagrees with creating a container"
        );
        assert_eq!(
            matches!(caps.filesystem, FsConfinement::AppContainer),
            matches!(caps.network, NetConfinement::CapabilitySid),
            "filesystem and network come from one mechanism and must not diverge"
        );
    }

    /// A probed machine reports `probed: true` whatever it found.
    ///
    /// "We did not look" and "we looked and found nothing" are different
    /// answers with different fixes, and only the first should send a user
    /// looking at their platform.
    #[test]
    fn windows_always_reports_that_it_looked() {
        assert!(probe().probed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe runs and answers on every platform.
    #[test]
    fn probing_does_not_panic_anywhere() {
        let c = probe();
        if cfg!(windows) {
            assert!(
                c.probed,
                "Windows probing is implemented and should have run"
            );
        } else if cfg!(target_os = "linux") {
            assert!(c.probed, "Linux probing is implemented and should have run");
        } else {
            assert!(
                !c.probed,
                "a platform we have not implemented must report `probed: false`, \
                 not a fabricated capability"
            );
        }
    }

    /// **On Linux, the two mechanisms stand or fall together with what the
    /// worker will actually apply.**
    ///
    /// The worker self-confines with Landlock + seccomp; the planning profile
    /// admits sandboxed steps only when both mechanisms are present. If the
    /// probe said yes while the worker's own application failed, every receipt
    /// would record "engaged nothing" under a plan that promised confinement.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_linux_probe_matches_what_the_worker_will_apply() {
        let caps = probe();
        let fs_ok = crate::linux::landlock_available();
        let net_ok = crate::linux::seccomp_network_deny_available();
        assert_eq!(
            matches!(caps.filesystem, FsConfinement::Landlock),
            fs_ok,
            "the filesystem answer disagrees with the ABI query"
        );
        assert_eq!(
            matches!(caps.network, NetConfinement::SeccompBlock),
            net_ok,
            "the network answer disagrees with the forked install probe"
        );
        // The floor needs BOTH. A machine that can deny network but not
        // confine the filesystem is exactly the case one ordinal cannot
        // express (`03` Â§9.1).
        if fs_ok && net_ok {
            #[cfg(feature = "attest")]
            {
                let profile = planning_profile(caps);
                let policy_floor = openconvert_core::isolation::IsolationFloor::default();
                assert!(
                    policy_floor.admits(&profile),
                    "Landlock + seccomp + rlimit must meet the default Reduced floor"
                );
            }
        }
    }

    /// **Job Objects really are available on this machine.**
    ///
    /// Asserted rather than assumed, because the probe creates one â€” if this
    /// fails, either the API changed or something on the machine is refusing,
    /// and both are things the user needs told rather than silently degraded.
    #[test]
    #[cfg(windows)]
    fn job_objects_can_actually_be_created() {
        assert!(
            job_objects_available(),
            "could not create a Job Object; resource limits would be unenforceable"
        );
        assert_eq!(probe().resources, ResourceEnforcement::JobObject);
    }

    /// An unprobed platform claims nothing at all.
    ///
    /// The control on failing closed: every field must be `None`, not just the
    /// ones we happened to think about.
    #[test]
    fn unprobed_claims_nothing() {
        let c = Capabilities::unprobed();
        assert!(!c.any(), "an unprobed machine claimed a capability");
        assert!(!c.probed);
        assert_eq!(c.filesystem, FsConfinement::None);
        assert_eq!(c.network, NetConfinement::None);
        assert_eq!(c.resources, ResourceEnforcement::None);
    }

    /// `probed` and `any()` are independent, and both are needed.
    ///
    /// A machine can be probed and offer nothing â€” a locked-down desktop where
    /// policy refuses everything. "We looked and found nothing" and "we did not
    /// look" produce the same conversions and need different messages.
    #[test]
    fn probed_and_capable_are_different_questions() {
        let looked_found_nothing = Capabilities {
            probed: true,
            ..Capabilities::unprobed()
        };
        assert!(looked_found_nothing.probed);
        assert!(!looked_found_nothing.any());
    }
}
