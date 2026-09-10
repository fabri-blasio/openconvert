//! Per-step limits, with values. See `03-ARCHITECTURE.md` section 10.1.
//!
//! v0.5 specified ten fields and not one number -- the only figure anywhere in
//! the record was a `120 s` inside an example error string. A limit without a
//! threshold is not a control. These come from spike S23, measured against
//! libarchive 3.8.5.

use core::time::Duration;

/// Resource ceiling for one step. A required field of `Step` (I4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock ceiling. Derived per `MediaKind`: a four-hour remux and a
    /// 5 MB JPEG cannot share a constant.
    pub wall_time: Duration,
    /// CPU ceiling. Catches a spin that a wall clock alone would allow.
    pub cpu_time: Duration,
    /// Resident memory ceiling, enforced by the OS.
    pub memory_bytes: u64,
    /// Total bytes an output may reach.
    pub output_bytes: u64,
    /// Total bytes scratch space may reach.
    pub temp_bytes: u64,
    /// Output divided by input.
    ///
    /// **Not a primary control, and the design must stop implying it is.** A
    /// legitimate 200 MB zeroed disk image compresses 1029x; a zip bomb 1028x
    /// (spike S23). No threshold separates them. This is a logging tripwire set
    /// high enough to be meaningless as a defence; `archive_depth` and
    /// `archive_total_bytes` do the actual work.
    pub expansion_ratio: u32,
    /// Pixels a decoder may allocate. Read from the header **before** decode.
    /// This is the control that works where `expansion_ratio` does not.
    pub decode_pixels: u64,
    /// Nesting depth for archives.
    pub archive_depth: u8,
    /// Entry count for archives.
    pub archive_entries: u32,
    /// Total extracted bytes for archives.
    pub archive_total_bytes: u64,
    /// Whether this run may use a GPU.
    ///
    /// # Why a permission lives among the ceilings
    ///
    /// Because it behaves like one. It travels the same path every other limit
    /// travels — Policy, then [`LimitCeiling::clamp`], then the worker — and it
    /// obeys the same rule: a layer may take it away and no layer may hand it
    /// back. `clamp` ANDs it, which is `min` for a boolean.
    ///
    /// That is what makes "off" mean off. A managed deployment, a policy layer
    /// or a user can each refuse the GPU, and nothing downstream can re-enable
    /// it — the same guarantee `memory_bytes` gets from `min`, spelled the way
    /// booleans spell it.
    pub use_gpu: bool,
}

impl Limits {
    /// Measured defaults. Every number here has a reason recorded in
    /// `03` section 10.1; none of them was chosen at a keyboard.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            wall_time: Duration::from_secs(120),
            cpu_time: Duration::from_secs(240),
            memory_bytes: 1 << 30,
            output_bytes: 4 << 30,
            temp_bytes: 8 << 30,
            // Tripwire only -- see the field docs.
            expansion_ratio: 10_000,
            // ~1 GiB at 4 bytes per pixel.
            decode_pixels: 256 << 20,
            // Legitimate nesting topped out at 8 in testing; the bomb was 41.
            archive_depth: 32,
            // The bomb had 40,000 -- and so does 01 section 7's archivist
            // workload. This one cannot be tightened without breaking a
            // documented user.
            archive_entries: 100_000,
            archive_total_bytes: 8 << 30,
            // ON by default: the GPU is used where it measurably pays and the
            // adapter falls back on its own where it does not. Defaulting to
            // off would leave a 7x speedup switched off for everyone who never
            // opens Settings.
            use_gpu: true,
        }
    }
}

/// The ceiling a step's limits may never exceed, whatever an engine reports.
///
/// The trust inversion this closes: `Properties` crosses from a **confined,
/// possibly compromised** engine straight into routing. An engine reporting
/// `width = 4_000_000_000` to inflate a pixel budget is refused here rather
/// than believed (spike S6).
///
/// Asserted by `wire::engine_facts_cannot_widen_limits` from week 2. SR-5, I14.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitCeiling(Limits);

impl LimitCeiling {
    /// Build a ceiling from policy.
    #[must_use]
    pub const fn new(max: Limits) -> Self {
        Self(max)
    }

    /// Clamp a candidate against this ceiling.
    ///
    /// Engine-supplied facts may **narrow** a limit and never widen one, so
    /// every field takes the minimum. There is deliberately no method that
    /// takes a maximum.
    #[must_use]
    pub fn clamp(&self, candidate: Limits) -> Limits {
        let c = &self.0;
        Limits {
            // `Duration::min`, not a hand-written comparison. An earlier version
            // spelled this out with an `if`, and mutation testing showed why that
            // was a bad idea: flipping its `<` to `>` made clamp return the LARGER
            // duration -- an engine widening its own wall-clock limit through the
            // function whose job is to refuse exactly that -- and every test still
            // passed. Delegating to std removes the branch a mutant can flip.
            wall_time: candidate.wall_time.min(c.wall_time),
            cpu_time: candidate.cpu_time.min(c.cpu_time),
            memory_bytes: candidate.memory_bytes.min(c.memory_bytes),
            output_bytes: candidate.output_bytes.min(c.output_bytes),
            temp_bytes: candidate.temp_bytes.min(c.temp_bytes),
            expansion_ratio: candidate.expansion_ratio.min(c.expansion_ratio),
            decode_pixels: candidate.decode_pixels.min(c.decode_pixels),
            archive_depth: candidate.archive_depth.min(c.archive_depth),
            archive_entries: candidate.archive_entries.min(c.archive_entries),
            archive_total_bytes: candidate.archive_total_bytes.min(c.archive_total_bytes),
            // `&&` IS `min` FOR A BOOLEAN, and it is the whole point: the
            // ceiling can withdraw the GPU and cannot grant it. Writing this as
            // `candidate.use_gpu` alone would let a step re-enable hardware the
            // policy above it had refused.
            use_gpu: candidate.use_gpu && c.use_gpu,
        }
    }
}

/// Per-**job** ceiling, so that a legal batch cannot fill a disk.
///
/// `Limits` bounds one step. Nothing bounded a whole job until v0.4, which is
/// how a perfectly legitimate 40,000-file archivist batch could exhaust free
/// space one small legal file at a time — no single step doing anything wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Total bytes all outputs may occupy.
    pub output_bytes: u64,
    /// Total bytes scratch space may occupy at any moment.
    pub temp_bytes: u64,
    /// Free space to leave untouched on the destination volume.
    ///
    /// Checked in **preflight**, before any work: a batch projected to pass
    /// this refuses up front rather than failing at file 312 of 4,000.
    pub reserve_bytes: u64,
    /// Wall-clock ceiling for the whole job.
    pub wall_time: Duration,
}

impl Budget {
    /// Defaults for an interactive conversion.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            output_bytes: 64 << 30,
            temp_bytes: 16 << 30,
            // 2 GiB. Filling a disk to the last byte makes a machine
            // unusable in ways that outlast the conversion.
            reserve_bytes: 2 << 30,
            wall_time: Duration::from_secs(60 * 60 * 4),
        }
    }

    /// Whether a projected total fits, given current free space.
    ///
    /// Deliberately takes the projection rather than measuring as it goes:
    /// SR-5 requires a batch that *cannot* fit to refuse in preflight.
    #[must_use]
    pub const fn admits(&self, projected_output: u64, free_space: u64) -> bool {
        projected_output <= self.output_bytes && free_space >= projected_output + self.reserve_bytes
    }
}
