//! Policy: defaults, then the user layer, then flags.
//!
//! Two fields here are **one-way ratchets** -- a later layer may tighten them
//! and may never loosen them. That is not a convention; it is the only reason
//! a managed deployment can rely on a setting an individual user can also see.

use crate::facts::Provenance;
use crate::isolation::IsolationFloor;
use crate::limits::{Budget, Limits};
use crate::plan::Class;

/// Whether one worker process may serve more than one file.
///
/// This is a **user setting**, and the reason is worth stating: the residual it
/// governs is a *cross-file* one, so whether it matters is a property of the
/// user's threat model rather than of ours. A photographer batching 400 of
/// their own RAWs and a lawyer processing discovery from opposing counsel want
/// opposite things, and we have no basis for choosing between them.
///
/// The numbers behind it (spikes S7, S8, S16): a fresh worker costs 40.15 ms on
/// Windows against a 25 ms budget, of which the sandbox is only 4.43 ms; a
/// reused worker costs 2.19 ms. Linux is 1.96 ms and does not need this.
///
/// See `03-ARCHITECTURE.md` section 8.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkerReuse {
    /// Reuse a worker across files that share a **trusted** provenance. A file
    /// of untrusted provenance always gets a worker of its own.
    ///
    /// A leak between two files the user already trusts is a leak from the user
    /// to themselves.
    Balanced,
    /// One worker per file, always. The strongest setting, and roughly 38 ms
    /// more per file on Windows -- which the UI states rather than hides.
    Isolated,
}

impl WorkerReuse {
    /// Whether two files with these provenances may share a worker under this
    /// setting. SR-20.
    #[must_use]
    pub const fn may_share(self, a: Provenance, b: Provenance) -> bool {
        match self {
            Self::Isolated => false,
            Self::Balanced => a.may_share_worker() && b.may_share_worker(),
        }
    }
}

/// The user-adjustable rules a plan is built under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    force_sandbox: bool,
    worker_reuse: WorkerReuse,
    floor: IsolationFloor,
    max_auto_class: Class,
    on_conflict: OnConflict,
    base_limits: Limits,
    limit_ceiling: Limits,
    budget: Budget,
}

/// What to do when an output name already exists.
///
/// Resolved **before anything is opened**. That ordering is the whole control:
/// a conflict discovered mid-write has already destroyed something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// `report.pdf` becomes `report (2).pdf`. The receipt records the name
    /// actually written, so an automated caller is never confused about what
    /// it got.
    Suffix,
    /// Leave the existing file and skip the output.
    Skip,
    /// Abort the whole step.
    ///
    /// Nothing is written -- not even the outputs that would have succeeded --
    /// because a half-extracted archive is worse than a refused one.
    Fail,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            worker_reuse: WorkerReuse::Balanced,
            floor: IsolationFloor::default(),
            // Class D is reachable only from a Target naming the operation.
            // I8, and a property test asserts it over every signal combination.
            // Off by default. The in-process set is pure-Rust code we compile
            // ourselves, and confining it costs a process launch per file for
            // isolation from a parser that is already memory-safe. Turning it
            // on is defence in depth, which is a choice rather than a default.
            force_sandbox: false,
            max_auto_class: Class::B,
            on_conflict: OnConflict::Suffix,
            base_limits: Limits::defaults(),
            limit_ceiling: Limits::defaults(),
            budget: Budget::defaults(),
        }
    }
}

/// The most memory one conversion may be given, whatever the config says.
///
/// 16 GiB. Past this the limit has stopped being a limit: the point of the
/// number is that a hostile file cannot exhaust the machine, and a ceiling
/// larger than most machines have does not do that. A config asking for more
/// gets this, silently, because a clamped ceiling is still a ceiling.
pub const MAX_WORKER_MEMORY: u64 = 16 << 30;

impl Policy {
    /// The current worker-reuse setting.
    #[must_use]
    pub const fn worker_reuse(&self) -> WorkerReuse {
        self.worker_reuse
    }

    /// Whether every step that CAN be confined must be.
    #[must_use]
    pub const fn force_sandbox(&self) -> bool {
        self.force_sandbox
    }

    /// Require confinement wherever a worker exists.
    ///
    /// **A ratchet, like `raise_worker_reuse`**: a layer may turn this on and
    /// none may turn it off, so a managed deployment can pin it and a
    /// user-level config cannot undo it. There is deliberately no setter that
    /// takes `false`.
    ///
    /// It does not, and cannot, confine everything: `MediaKind::has_worker`
    /// says where there is something to confine into, and CSV/JSON and
    /// Matroska stream copy have no worker by design. A flag that claimed
    /// otherwise would be the kind of overstatement this codebase exists to
    /// avoid.
    #[must_use]
    pub const fn force_sandbox_everywhere(mut self) -> Self {
        self.force_sandbox = true;
        self
    }

    /// Apply a later policy layer's worker-reuse preference.
    ///
    /// **This is a ratchet and there is deliberately no setter.** A layer may
    /// raise the setting to `Isolated` and may never lower it, so a managed
    /// deployment can pin the strict value and a user-level config cannot
    /// undo it. Same pattern as the network floor, which has no config key at
    /// all.
    ///
    /// Asserted by `worker_reuse_never_lowers` from week 2.
    #[must_use]
    pub fn raise_worker_reuse(mut self, requested: WorkerReuse) -> Self {
        self.worker_reuse = self.worker_reuse.max(requested);
        self
    }

    /// The isolation floor. Three predicates, and `network: Denied` is not one
    /// of them because it is checked unconditionally and has no setter.
    #[must_use]
    pub const fn floor(&self) -> &IsolationFloor {
        &self.floor
    }

    /// The highest class `route()` will arm without an explicit request.
    #[must_use]
    pub const fn max_auto_class(&self) -> Class {
        self.max_auto_class
    }

    /// Raise the auto-class ceiling for ONE run the user asked for by name.
    ///
    /// # Why this is a builder and not a config key
    ///
    /// `max_auto_class` defaults to `B` so nothing lossier than a re-encode
    /// ever happens because a route was convenient. Class D — a model deciding
    /// which pixels are the subject — is the case that ceiling exists for, and
    /// the refusal it produces says "ask for the operation by name to run it".
    ///
    /// This is that asking. It is deliberately per-invocation and deliberately
    /// not persistable: consent to run a model over one file is not consent
    /// for every file afterwards, and a config key would quietly become the
    /// latter. The interface passes it when the user picks the tool; the CLI
    /// passes it for `--remove-background`.
    ///
    /// It only ever RAISES, matching the ratchet the rest of this type uses.
    #[must_use]
    pub const fn arming(mut self, class: Class) -> Self {
        if (class as u8) > (self.max_auto_class as u8) {
            self.max_auto_class = class;
        }
        self
    }

    /// This policy, armed for whatever the target's name asks for.
    ///
    /// **The whole reason this is one function.** Five places route a
    /// request -- the CLI, the tool path, and the desktop's dropdown, plan
    /// preview and convert -- and each of them must arm identically or they
    /// disagree about what is possible. They already did: the tool path had no
    /// arming at all, so every model-backed tool in the desktop answered "that
    /// did not produce a file" on a machine where the models worked. The rule
    /// itself is [`Target::arms`]; this spares each caller from restating how
    /// to apply it.
    #[must_use]
    pub const fn armed_for(self, target: crate::target::Target) -> Self {
        match target.arms() {
            Some(class) => self.arming(class),
            None => self,
        }
    }

    /// What to do about a name collision.
    #[must_use]
    pub const fn on_conflict(&self) -> OnConflict {
        self.on_conflict
    }

    /// Starting limits, before content narrows them.
    #[must_use]
    pub const fn base_limits(&self) -> Limits {
        self.base_limits
    }

    /// The ceiling engine facts may never exceed. I14.
    #[must_use]
    pub const fn limit_ceiling(&self) -> Limits {
        self.limit_ceiling
    }

    /// The per-job budget.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// Whether conversions may use a GPU.
    ///
    /// # A ratchet, unlike the memory limit beside it
    ///
    /// `with_worker_memory` goes both ways, because a number someone raised for
    /// one model has to be lowerable again. This one only ever turns the GPU
    /// OFF, and that asymmetry is deliberate: it is the shape that lets a
    /// managed deployment forbid GPU use — on a driver stack it does not trust,
    /// or a shared machine whose card belongs to something else — without a
    /// later layer quietly handing it back.
    ///
    /// "On" is therefore not a value this can set; it is the default, and the
    /// only thing any layer can do is take it away. `LimitCeiling::clamp` ANDs
    /// the flag for the same reason.
    #[must_use]
    pub const fn without_gpu(mut self) -> Self {
        self.base_limits.use_gpu = false;
        self.limit_ceiling.use_gpu = false;
        self
    }

    /// How much memory one confined conversion may use.
    ///
    /// # Why this is a setting at all
    ///
    /// The default is 1 GiB, and that number is a defence: it is what stops a
    /// decompression bomb, a hostile PDF or a 500-megapixel PNG header from
    /// taking the machine down. Nothing about ordinary conversion needs more.
    ///
    /// Some models do. BiRefNet-lite runs at a fixed 1024x1024 and peaks at
    /// **6,465 MB** for a single small image -- measured, not estimated -- so at
    /// the default it does not run slowly, it fails outright. That is a real
    /// capability sitting behind a number, and the honest way to offer it is to
    /// let someone raise the number knowingly rather than to raise it for
    /// everyone quietly.
    ///
    /// # Both values move, and they have to
    ///
    /// `base_limits` is what a step starts with and `limit_ceiling` is what it
    /// may never exceed. Raising only the first does nothing: `clamp` takes the
    /// minimum of the two, so the ceiling would put it straight back. Raising
    /// only the second does nothing either, because nothing asks for more.
    ///
    /// # This is NOT a ratchet, and that is deliberate
    ///
    /// `force_sandbox_everywhere` and `raise_worker_reuse` only tighten, because
    /// a managed deployment must be able to pin them. This one goes both ways:
    /// a user who raised it for one model has to be able to put it back, and a
    /// setting that can only be loosened is a setting nobody should touch.
    ///
    /// Bounded at both ends regardless. Below the default is refused because a
    /// smaller ceiling breaks ordinary conversions; above [`MAX_WORKER_MEMORY`]
    /// is refused because past that point the limit has stopped being a limit.
    #[must_use]
    pub const fn with_worker_memory(mut self, bytes: u64) -> Self {
        let floor = Limits::defaults().memory_bytes;
        let bytes = if bytes < floor {
            floor
        } else if bytes > MAX_WORKER_MEMORY {
            MAX_WORKER_MEMORY
        } else {
            bytes
        };
        self.base_limits.memory_bytes = bytes;
        self.limit_ceiling.memory_bytes = bytes;
        self
    }

    /// Set the conflict policy.
    ///
    /// Not a ratchet: `Fail` is stricter than `Suffix` in one sense and simply
    /// different in another, and there is no ordering that makes "raise only"
    /// meaningful. The security property (I12) holds under all three, because
    /// none of them can truncate.
    #[must_use]
    pub const fn with_on_conflict(mut self, c: OnConflict) -> Self {
        self.on_conflict = c;
        self
    }

    /// Raise the isolation floor. Like `worker_reuse`, a **ratchet**.
    #[must_use]
    pub fn raise_floor(mut self, requested: IsolationFloor) -> Self {
        if requested.strength > self.floor.strength {
            self.floor.strength = requested.strength;
        }
        if matches!(
            requested.filesystem,
            crate::isolation::FsRequirement::Confined
        ) {
            self.floor.filesystem = crate::isolation::FsRequirement::Confined;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::LimitCeiling;

    #[test]
    fn a_raised_memory_limit_survives_the_clamp() {
        // The trap this pins: `base_limits` is what a step asks for and
        // `limit_ceiling` is what it may have, and `clamp` takes the MINIMUM of
        // the two. Raising only the base is a setting that appears to work,
        // reports the new number, and clamps back to 1 GiB at the moment it
        // matters -- so the model it was raised for still fails, with the
        // screen saying it should not have.
        let raised = Policy::default().with_worker_memory(8 << 30);
        let ceiling = LimitCeiling::new(raised.limit_ceiling());
        assert_eq!(
            ceiling.clamp(raised.base_limits()).memory_bytes,
            8 << 30,
            "the raised limit did not survive its own ceiling"
        );
    }

    #[test]
    fn the_memory_limit_is_bounded_at_both_ends() {
        let floor = Limits::defaults().memory_bytes;

        // Below the default is refused: a smaller ceiling does not make the
        // machine safer, it makes ordinary conversions fail.
        assert_eq!(
            Policy::default()
                .with_worker_memory(1)
                .base_limits()
                .memory_bytes,
            floor
        );

        // Above the hard maximum is clamped, not honoured. A hand-edited
        // config.toml is a front door like any other.
        assert_eq!(
            Policy::default()
                .with_worker_memory(u64::MAX)
                .base_limits()
                .memory_bytes,
            MAX_WORKER_MEMORY
        );
    }

    #[test]
    fn the_memory_limit_is_not_a_ratchet() {
        // Deliberately unlike `force_sandbox_everywhere` and
        // `raise_worker_reuse`. Someone who raised this for one model has to
        // be able to put it back, and the test says so out loud because the
        // two neighbours in this file behave the other way.
        let there_and_back = Policy::default()
            .with_worker_memory(8 << 30)
            .with_worker_memory(Limits::defaults().memory_bytes);
        assert_eq!(
            there_and_back.base_limits().memory_bytes,
            Limits::defaults().memory_bytes
        );
        assert_eq!(
            there_and_back.limit_ceiling().memory_bytes,
            Limits::defaults().memory_bytes
        );
    }

    #[test]
    fn turning_the_gpu_off_is_a_ratchet() {
        // On by default, or the feature is off for everyone who never opens
        // Settings.
        assert!(
            Policy::default().base_limits().use_gpu,
            "the GPU must be on unless something turns it off"
        );

        let off = Policy::default().without_gpu();
        assert!(!off.base_limits().use_gpu);
        assert!(!off.limit_ceiling().use_gpu, "the ceiling must fall too");

        // BOTH VALUES MOVE, for the same reason the memory limit needs both:
        // `clamp` combines them, so a base that still permitted the GPU under a
        // ceiling that forbade it -- or the reverse -- would be a setting that
        // reports one thing and does another.
        let ceiling = LimitCeiling::new(off.limit_ceiling());
        assert!(
            !ceiling.clamp(off.base_limits()).use_gpu,
            "the refusal did not survive its own clamp"
        );
    }

    #[test]
    fn a_clamp_can_withdraw_the_gpu_and_never_grant_it() {
        let permissive = Limits::defaults();
        let forbidding = Limits {
            use_gpu: false,
            ..Limits::defaults()
        };

        // A ceiling that forbids beats a candidate that wants it.
        assert!(
            !LimitCeiling::new(forbidding).clamp(permissive).use_gpu,
            "a step re-enabled hardware the policy above it had refused"
        );

        // And a candidate that forbids stays forbidden under a permissive
        // ceiling -- narrowing is always allowed.
        assert!(!LimitCeiling::new(permissive).clamp(forbidding).use_gpu);

        // Both permitting is the only combination that permits.
        assert!(LimitCeiling::new(permissive).clamp(permissive).use_gpu);
    }
}
