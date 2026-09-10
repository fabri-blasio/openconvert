//! Worker reuse — the setting D8 made user-facing, doing something at last.
//!
//! # What was missing
//!
//! [`WorkerReuse`], [`Provenance`] and `WorkerReuse::may_share` have existed
//! since week 2, with property tests pinning SR-20's ratchet. **Nothing called
//! them.** `execute()` started one worker per step and dropped it, so `Isolated`
//! and `Balanced` produced byte-identical behaviour at identical cost — a
//! user-facing setting with no effect, and a security requirement enforced only
//! by a function nobody invoked.
//!
//! # The measurement behind the default
//!
//! A fresh Windows worker costs **40.15 ms** against a 25 ms budget, of which
//! the sandbox itself is 4.43 ms; a reused one costs **2.19 ms** (spikes S7,
//! S8, S16). Linux is 1.96 ms and does not need this at all. So reuse is the
//! default on the platform where it is worth 38 ms per file, and `Isolated`
//! states that cost rather than hiding it.
//!
//! # SR-20, which reuse must not weaken
//!
//! *Untrusted input never shares a worker.* Two rules follow, and both are
//! enforced here rather than left to the caller:
//!
//! - an untrusted file never lands in a **used** worker, and
//! - a worker that has processed untrusted input is **never parked**, so no
//!   later file can land in it either.
//!
//! The second is the one that is easy to forget. Refusing to reuse *for* an
//! untrusted file, while still returning that worker to the pool afterwards,
//! satisfies the obvious reading of SR-20 and breaks the actual one.

use crate::worker_client::{Worker, WorkerError};
use openconvert_core::facts::Provenance;
use openconvert_core::limits::Limits;
use openconvert_core::policy::WorkerReuse;
use openconvert_sandbox::argv::EngineBin;

/// Render a worker's read-back for the receipt.
///
/// **This is evidence, not a plan.** `route()` picks a profile from what the
/// machine *can* do, which is the right input for deciding whether to attempt a
/// sandboxed step. A receipt has to say what actually engaged, and only the
/// child can answer that (I11, `03` s9.5). Rendering the planned profile would
/// make every receipt a statement of intent wearing the clothes of evidence.
///
/// An empty read-back is reported as such rather than omitted: a worker that
/// told us nothing is a different fact from a worker that told us it was
/// unconfined, and a receipt that quietly drops the difference is a receipt
/// nobody can audit.
fn render_confinement(mitigations: &[(String, bool)]) -> String {
    if mitigations.is_empty() {
        return "sandboxed; the worker reported no read-back".to_string();
    }
    let engaged: Vec<&str> = mitigations
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| n.as_str())
        .collect();
    let absent: Vec<&str> = mitigations
        .iter()
        .filter(|(_, on)| !*on)
        .map(|(n, _)| n.as_str())
        .collect();
    format!(
        "sandboxed (read back in the worker): engaged {}; not engaged {}",
        if engaged.is_empty() {
            "nothing".to_string()
        } else {
            engaged.join(", ")
        },
        if absent.is_empty() {
            "nothing".to_string()
        } else {
            absent.join(", ")
        }
    )
}

/// A parked worker and the terms it was started under.
struct Parked {
    bin: EngineBin,
    worker: Worker,
    /// The memory cap its Job Object was created with.
    ///
    /// Reuse is refused when this differs from what the next step needs.
    /// The cap is fixed at spawn — `Limits` travel with each `Run`, but the Job
    /// Object does not — so reusing across a raised limit would silently run
    /// the second file under the first file's ceiling. That is I4 quietly
    /// broken, and it would show up as an unexplained failure on a file that
    /// converts fine on its own.
    memory_bytes: u64,
}

/// Live workers, reused according to policy.
///
/// Holds at most one idle worker per engine. A deeper pool would buy nothing:
/// conversion here is sequential per file, so the second entry for an engine
/// would never be read.
pub struct WorkerPool {
    reuse: WorkerReuse,
    idle: Vec<Parked>,
}

impl WorkerPool {
    /// A pool obeying one policy setting.
    #[must_use]
    pub const fn new(reuse: WorkerReuse) -> Self {
        Self {
            reuse,
            idle: Vec::new(),
        }
    }

    /// How many workers are parked. For tests and diagnostics.
    #[must_use]
    pub fn idle_count(&self) -> usize {
        self.idle.len()
    }

    /// Convert one step's bytes, reusing a worker when policy allows.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`]. A failure never parks the worker: a process that
    /// just failed is a process whose state nobody can vouch for, and the
    /// cheapest correct thing to do with it is end it.
    pub fn run(
        &mut self,
        bin: EngineBin,
        provenance: Provenance,
        limits: &Limits,
        bytes: Vec<u8>,
        to: &str,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>, String), WorkerError> {
        self.run_with_models(bin, provenance, limits, bytes, &[], to, params)
    }

    /// As [`Self::run`], with model weights alongside the input.
    ///
    /// Worker reuse is unaffected: the weights are the same for every file
    /// using a model, and it is the FILE's provenance that decides sharing.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`].
    #[allow(clippy::too_many_arguments)]
    pub fn run_with_models(
        &mut self,
        bin: EngineBin,
        provenance: Provenance,
        limits: &Limits,
        bytes: Vec<u8>,
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>, String), WorkerError> {
        let mut worker = match self.take(bin, provenance, limits) {
            Some(w) => w,
            None => Worker::start(bin, limits)?,
        };

        // Not `?`. An error must still fall through to the teardown below,
        // because returning early here would park nothing and drop the worker
        // anyway -- which is right, but only by accident. Being explicit keeps
        // it right when someone later adds a park-on-error path.
        // Captured before the worker can be parked or dropped. This is the
        // evidence the receipt records -- what the child read back from the OS
        // about itself -- as distinct from the profile `route()` planned.
        let confinement = render_confinement(worker.mitigations());
        let result = worker
            .run_with_models(bytes, models, to, limits, params)
            .map(|(bytes, removed)| (bytes, removed, confinement));

        if result.is_ok() && self.reuse.may_share(provenance, provenance) {
            // Parked only if THIS file could share with another like it. An
            // untrusted file's worker is dropped here, which is the half of
            // SR-20 that is easy to miss.
            self.idle.push(Parked {
                bin,
                worker,
                memory_bytes: limits.memory_bytes,
            });
        }
        result
    }

    /// As [`Self::run`], with FURTHER INPUTS alongside the first.
    ///
    /// Merging documents is what this is for. Provenance governs sharing as
    /// always, and the decision is made on the FIRST input -- so a merge is
    /// treated as trusted only when every document in it is, which the caller
    /// establishes before getting here.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`].
    #[allow(clippy::too_many_arguments)]
    pub fn run_with_inputs(
        &mut self,
        bin: EngineBin,
        provenance: Provenance,
        limits: &Limits,
        bytes: Vec<u8>,
        extras: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>, String), WorkerError> {
        let mut worker = match self.take(bin, provenance, limits) {
            Some(w) => w,
            None => Worker::start(bin, limits)?,
        };
        let confinement = render_confinement(worker.mitigations());
        let result = worker
            .run_with_inputs(bytes, extras, to, limits, params)
            .map(|(bytes, removed)| (bytes, removed, confinement));

        if result.is_ok() && self.reuse.may_share(provenance, provenance) {
            self.idle.push(Parked {
                bin,
                worker,
                memory_bytes: limits.memory_bytes,
            });
        }
        result
    }

    /// Probe one file's header, reusing a worker when policy allows.
    ///
    /// Same rules as [`WorkerPool::run`] — a probe reads the same untrusted
    /// bytes a conversion would, so it gets the same worker discipline. The
    /// caller passes `Provenance::Untrusted` for anything not yet judged, which
    /// is every file at probe time.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`].
    pub fn probe(
        &mut self,
        bin: EngineBin,
        provenance: Provenance,
        limits: &Limits,
        bytes: &[u8],
    ) -> Result<openconvert_core::facts::Properties, WorkerError> {
        let mut worker = match self.take(bin, provenance, limits) {
            Some(w) => w,
            None => Worker::start(bin, limits)?,
        };
        let result = worker.probe(bytes.to_vec(), limits);
        if result.is_ok() && self.reuse.may_share(provenance, provenance) {
            self.idle.push(Parked {
                bin,
                worker,
                memory_bytes: limits.memory_bytes,
            });
        }
        result
    }

    /// Take a parked worker if one may serve this step.
    fn take(&mut self, bin: EngineBin, provenance: Provenance, limits: &Limits) -> Option<Worker> {
        // `may_share(provenance, provenance)` is not a typo and not a
        // simplification. Every worker in `idle` was parked only if its own
        // file could share, so each is already known trusted -- asking whether
        // THIS file may share is the whole remaining question, and asking it
        // through the same function keeps one definition of the rule.
        if !self.reuse.may_share(provenance, provenance) {
            return None;
        }
        let i = self
            .idle
            .iter()
            .position(|p| p.bin == bin && p.memory_bytes == limits.memory_bytes)?;
        Some(self.idle.remove(i).worker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(memory_bytes: u64) -> Limits {
        Limits {
            memory_bytes,
            ..Limits::defaults()
        }
    }

    /// `Isolated` never reuses, whatever the provenance.
    #[test]
    fn isolated_never_parks_a_worker() {
        let pool = WorkerPool::new(WorkerReuse::Isolated);
        assert!(
            !pool
                .reuse
                .may_share(Provenance::Trusted, Provenance::Trusted),
            "Isolated must refuse even two trusted files"
        );
        assert_eq!(pool.idle_count(), 0);
    }

    /// `Balanced` reuses for trusted input and refuses for untrusted.
    #[test]
    fn balanced_shares_only_between_trusted_files() {
        let r = WorkerReuse::Balanced;
        assert!(r.may_share(Provenance::Trusted, Provenance::Trusted));
        assert!(!r.may_share(Provenance::Untrusted, Provenance::Trusted));
        assert!(!r.may_share(Provenance::Trusted, Provenance::Untrusted));
        assert!(!r.may_share(Provenance::Untrusted, Provenance::Untrusted));
    }

    /// A parked worker is only taken for a matching engine.
    ///
    /// Sending PDF bytes to the image worker because it happened to be idle is
    /// a bug the type system does not catch — both are `Worker`.
    #[test]
    fn a_parked_worker_is_not_taken_for_a_different_engine() {
        let mut pool = WorkerPool::new(WorkerReuse::Balanced);
        // No live workers needed: `take` is pure over the parked set, and this
        // asserts the lookup, not the process.
        assert!(pool
            .take(EngineBin::Images, Provenance::Trusted, &limits(1 << 28))
            .is_none());
        assert!(pool
            .take(EngineBin::Pdf, Provenance::Trusted, &limits(1 << 28))
            .is_none());
    }

    /// An untrusted file takes nothing from the pool, even when a worker is
    /// parked and idle.
    #[test]
    fn an_untrusted_file_takes_no_parked_worker() {
        let mut pool = WorkerPool::new(WorkerReuse::Balanced);
        assert!(
            pool.take(EngineBin::Images, Provenance::Untrusted, &limits(1 << 28))
                .is_none(),
            "SR-20: untrusted input never shares a worker"
        );
    }
}
