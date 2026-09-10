//! Isolation: the sealed profile, and a floor that is three predicates.
//!
//! See `03-ARCHITECTURE.md` §9.

use core::fmt;

/// Filesystem confinement mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsConfinement {
    /// Linux ≥ 5.13. Needs neither privilege nor a user namespace — the
    /// correction that made the `Reduced` tier reachable on a hardened host.
    Landlock,
    /// Windows low-privilege app container.
    AppContainer,
    /// macOS App Sandbox. **Unverified** — no measurement exists.
    AppSandbox,
    /// Linux mount namespace. Requires a user namespace.
    BindMount,
    /// The browser's page origin.
    ///
    /// Confines the *page*, not one library from the rest of the page's
    /// memory — so it scores `Reduced` and never `Full`.
    BrowserOrigin,
    /// Nothing.
    None,
}

/// Network denial mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetConfinement {
    /// Empty network namespace.
    EmptyNetns,
    /// seccomp filter refusing `AF_INET`/`AF_INET6`, permitting `AF_UNIX`.
    ///
    /// Architecture-specific in two ways at once: the `AUDIT_ARCH` constant and
    /// the syscall number both differ between x86_64 and aarch64, and a filter
    /// that fails to validate the arch field of `seccomp_data` is a textbook
    /// bypass. See `TARGETS.md`.
    SeccompBlock,
    /// Absence of the Windows network capability SID.
    ///
    /// Note that socket *creation* remains permitted inside an AppContainer;
    /// restriction is enforced by the filtering platform at connect/send. A
    /// test asserting creation fails proves nothing (spike S9).
    CapabilitySid,
    /// macOS: no network entitlement. **Unverified.**
    NoEntitlement,
    /// Browser content-security policy.
    Csp,
    /// Nothing.
    None,
}

/// Syscall filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallFilter {
    /// seccomp-BPF.
    SeccompBpf,
    /// Nothing.
    None,
}

/// Resource enforcement mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceEnforcement {
    /// Linux cgroup v2.
    CgroupV2,
    /// Windows Job Object.
    ///
    /// Enforces, but **kills the process without a `Failed` message** — the
    /// host needs an IO completion port to learn which limit tripped, or the
    /// failure surfaces as an abort code (spikes S14, S18).
    JobObject,
    /// POSIX `setrlimit`. Produces a *recoverable* allocation error, so a
    /// worker can report the failure itself.
    Rlimit,
    /// Nothing.
    None,
}

/// Privilege reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivDrop {
    /// Unprivileged user namespace.
    UserNs,
    /// Windows restricted token.
    RestrictedToken,
    /// Nothing.
    None,
}

/// How strong a profile is, as one ordinal for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// No meaningful confinement.
    None,
    /// Confinement without privilege separation.
    Minimal,
    /// Filesystem and network confined.
    Reduced,
    /// Everything the platform offers.
    Full,
}

impl fmt::Display for Strength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Reduced => "reduced",
            // "Elevated" in the UI: users read "Full" as "complete", and it is
            // not — CIG does not engage even in this tier.
            Self::Full => "elevated",
        })
    }
}

/// What confinement **actually engaged**.
///
/// Private fields. Only the OS prober constructs one (I11), behind
/// `feature = "attest"` which only `openconvert-os` and the WASM executor enable,
/// asserted by a CI gate over every `Cargo.toml`.
///
/// # Read back, not requested
///
/// I11 guards against a *dishonest* constructor. It does not guard against an
/// **honest one recording a request the OS ignored** — and measurement shows
/// that error runs in both directions (spikes S17b, S20b):
///
/// - **ACG** engages inside an AppContainer whether or not it is requested.
/// - **CIG** does not engage even when it is.
///
/// So these fields are populated from read-back queries *in the child, after it
/// starts* — never from the request. `03` §9.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxProfile {
    filesystem: FsConfinement,
    network: NetConfinement,
    syscalls: SyscallFilter,
    resources: ResourceEnforcement,
    privileges: PrivDrop,
}

impl SandboxProfile {
    /// Construct from read-back. `openconvert-os` and the WASM executor only.
    ///
    /// Gated on `feature = "attest"` so that enabling it is a visible
    /// dependency-manifest diff, blockable by CODEOWNERS and failed by a CI
    /// gate. Rust cannot express "only this crate may call this" across a crate
    /// boundary, so `03` §7.4 classes this **Hard**, not Impossible, and says
    /// so rather than implying otherwise.
    #[cfg(feature = "attest")]
    #[must_use]
    pub const fn attest(
        filesystem: FsConfinement,
        network: NetConfinement,
        syscalls: SyscallFilter,
        resources: ResourceEnforcement,
        privileges: PrivDrop,
    ) -> Self {
        Self {
            filesystem,
            network,
            syscalls,
            resources,
            privileges,
        }
    }

    /// Construct a profile for tests.
    ///
    /// Separate from [`Self::attest`] and available only under `cfg(test)` or
    /// the `test-util` feature, so that a test helper can never become a
    /// production construction path by someone deleting one `#[cfg]`.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub const fn for_test(
        filesystem: FsConfinement,
        network: NetConfinement,
        syscalls: SyscallFilter,
        resources: ResourceEnforcement,
        privileges: PrivDrop,
    ) -> Self {
        Self {
            filesystem,
            network,
            syscalls,
            resources,
            privileges,
        }
    }

    /// The profile that claims **nothing**.
    ///
    /// # Why this one is not sealed
    ///
    /// I11 exists so that nothing can *assert confinement that did not happen*
    /// — a receipt is evidence, and evidence must come from the OS. This
    /// profile asserts the opposite: every mechanism is `None`. It cannot
    /// overstate anything, and a floor check refuses it immediately.
    ///
    /// So it is public and unsealed, deliberately. The alternative would be
    /// forcing every caller that has not probed yet to fabricate a profile
    /// through the attest path — which is exactly the hole the seal exists to
    /// close.
    #[must_use]
    pub const fn unconfined() -> Self {
        Self {
            filesystem: FsConfinement::None,
            network: NetConfinement::None,
            syscalls: SyscallFilter::None,
            resources: ResourceEnforcement::None,
            privileges: PrivDrop::None,
        }
    }

    /// Whether network egress is denied.
    #[must_use]
    pub const fn denies_network(&self) -> bool {
        !matches!(self.network, NetConfinement::None)
    }

    /// Whether the filesystem is confined.
    #[must_use]
    pub const fn confines_filesystem(&self) -> bool {
        !matches!(self.filesystem, FsConfinement::None)
    }

    /// Overall strength.
    ///
    /// `BrowserOrigin` is capped at `Reduced` deliberately: the page origin
    /// confines the page, not one library from the rest of the page's memory.
    /// The payoff of making the profile a value is real; the payoff is not that
    /// a new target gets to describe itself flatteringly.
    #[must_use]
    pub const fn strength(&self) -> Strength {
        if !self.denies_network() || !self.confines_filesystem() {
            return if self.denies_network() || self.confines_filesystem() {
                Strength::Minimal
            } else {
                Strength::None
            };
        }
        let privileged = !matches!(self.privileges, PrivDrop::None);
        let filtered = !matches!(self.syscalls, SyscallFilter::None);
        let bounded = !matches!(self.resources, ResourceEnforcement::None);
        let browser = matches!(self.filesystem, FsConfinement::BrowserOrigin);

        if privileged && filtered && bounded && !browser {
            Strength::Full
        } else {
            Strength::Reduced
        }
    }

    /// The filesystem mechanism, for the receipt and the UI.
    #[must_use]
    pub const fn filesystem(&self) -> FsConfinement {
        self.filesystem
    }
    /// The network mechanism, for the receipt and the UI.
    #[must_use]
    pub const fn network(&self) -> NetConfinement {
        self.network
    }
}

/// What a filesystem confinement requirement can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsRequirement {
    /// The default.
    Confined,
    /// An explicit opt-out, which the user must state.
    Unconfined,
}

/// The isolation floor: **three predicates, not one number**.
///
/// v0.4 had a single `min_strength` and it could not express what SR-1 requires.
/// A machine can offer meaningful filesystem confinement and no way to deny
/// network; one ordinal has to call that either "good enough" or "refuse", and
/// both answers are wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsolationFloor {
    /// Filesystem requirement.
    pub filesystem: FsRequirement,
    /// Minimum overall strength.
    pub strength: Strength,
}

impl Default for IsolationFloor {
    fn default() -> Self {
        Self {
            filesystem: FsRequirement::Confined,
            strength: Strength::Reduced,
        }
    }
}

impl IsolationFloor {
    /// Whether a profile satisfies this floor.
    ///
    /// Delegates to [`Self::refusal`] rather than repeating the checks. An
    /// earlier version implemented the predicate twice -- mutation testing
    /// found six survivors in the copy, because `route()` only ever calls
    /// `refusal` and nothing exercised the other one. Two implementations of
    /// one security predicate is how they drift apart.
    #[must_use]
    pub fn admits(&self, profile: &SandboxProfile) -> bool {
        self.refusal(profile).is_none()
    }

    /// Why a profile was refused, for the blocking warning.
    #[must_use]
    pub fn refusal(&self, profile: &SandboxProfile) -> Option<Refusal> {
        if !profile.denies_network() {
            return Some(Refusal::NoNetworkDenial);
        }
        if matches!(self.filesystem, FsRequirement::Confined) && !profile.confines_filesystem() {
            return Some(Refusal::NoFilesystemConfinement);
        }
        if profile.strength() < self.strength {
            return Some(Refusal::BelowStrength {
                required: self.strength,
                available: profile.strength(),
            });
        }
        None
    }
}

/// Why a machine cannot run a conversion.
///
/// Carried into the user-facing message, which must name the missing
/// *mechanism* and what to do — not "isolation unavailable".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No way to deny network egress. Never lowerable.
    NoNetworkDenial,
    /// No filesystem confinement, and policy requires it.
    NoFilesystemConfinement,
    /// Confinement exists but is weaker than policy requires.
    BelowStrength {
        /// What policy demands.
        required: Strength,
        /// What the machine offers.
        available: Strength,
    },
}

/// Where a step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// In our own process. **Pure-Rust parsers only**, enforced by I10 plus a
    /// property test over the registry.
    InProcess,
    /// A confined subprocess; the profile says how.
    Sandboxed(SandboxProfile),
}
