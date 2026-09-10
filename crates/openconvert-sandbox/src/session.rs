//! The worker session: one job directory at a time, capabilities granted and
//! revoked per job.
//!
//! # Why this is a setting and not a constant
//!
//! `03` §8.4, and it is the one decision in this design whose right answer
//! genuinely depends on who is asking.
//!
//! | | Windows | Linux |
//! |---|---|---|
//! | Fresh worker per file | **40.15 ms** — 161% of the 25 ms budget | 1.96 ms |
//! | Reused worker | **2.19 ms** | — |
//!
//! The sandbox is **4.43 ms** of that forty; the rest is Windows process
//! creation, which is slow for everyone — a Microsoft-signed system binary
//! costs 15.0 ms against our 15.8 (spikes S7, S8, S16). No amount of signing,
//! packaging or shrinking moves it.
//!
//! Reuse is safe in the way that matters and unsafe in one way that does not
//! always matter. A capability can be granted to a running confined worker and
//! **revoked from it** — verified, including inside an AppContainer (S9 × S16).
//! What cannot be revoked is memory: a worker compromised by file A can retain
//! A's bytes and write them into file B's output within the same batch.
//!
//! Whether that matters is a property of the user's threat model rather than of
//! ours, so [`WorkerReuse`] is theirs to set. A photographer batching 400 of
//! their own RAWs and a lawyer processing discovery from opposing counsel want
//! opposite things, and we have no basis for choosing between them.
//!
//! # What this module is not
//!
//! It holds no syscalls. Spawning, granting and revoking are `openconvert-os`'s
//! job because all three need `unsafe` on both platforms (S3, S12c). This is
//! the *policy* half: which files may share a worker, and when a capability
//! must be handed back.

use crate::job::BrokeredOutput;
use openconvert_core::facts::Provenance;
use openconvert_core::policy::WorkerReuse;

/// A worker's lifetime, in policy terms.
///
/// Not a process handle — that lives in `openconvert-os`. This is the decision
/// about *whether a process may be reused*, kept where it can be tested without
/// spawning anything.
#[derive(Debug)]
pub struct WorkerSession {
    reuse: WorkerReuse,
    /// The provenance of files served so far. `None` before the first job.
    ///
    /// One value, not a set: under `Balanced` every file in a session shares a
    /// provenance by construction, because a file that does not match is given
    /// a session of its own.
    serving: Option<Provenance>,
    /// Files served. For the receipt, which records *which* files shared a
    /// worker — the disclosure that makes the residual honest rather than
    /// hidden.
    served: usize,
    /// The job directory currently granted, if any.
    granted: Option<BrokeredOutput>,
}

/// Why a job could not be added to a session.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionError {
    /// A capability is still outstanding.
    ///
    /// Granting a second job directory without revoking the first would leave
    /// the worker holding two — and the whole argument for reuse rests on the
    /// worker holding exactly what the current job needs and nothing else.
    #[error("a job directory is still granted; revoke it before granting another")]
    StillGranted,
    /// Nothing to revoke.
    #[error("no job directory is granted")]
    NotGranted,
}

impl WorkerSession {
    /// Open a session under a reuse policy.
    #[must_use]
    pub const fn open(reuse: WorkerReuse) -> Self {
        Self {
            reuse,
            serving: None,
            served: 0,
            granted: None,
        }
    }

    /// Whether this session may take a file of the given provenance.
    ///
    /// The three rules, in the order they bind:
    ///
    /// 1. **`Isolated` accepts one file, ever.** The strongest setting, and the
    ///    original D11.
    /// 2. **An untrusted file never shares.** It may open a session and may
    ///    never join one, in either direction — a downloaded file must not sit
    ///    behind *or* in front of your own.
    /// 3. **Trusted files share.** A leak among files the user already trusts
    ///    is a leak from the user to themselves.
    ///
    /// SR-20.
    #[must_use]
    pub fn accepts(&self, provenance: Provenance) -> bool {
        match self.serving {
            // A fresh session takes anything; what it takes decides the rest.
            None => true,
            Some(current) => self.reuse.may_share(current, provenance),
        }
    }

    /// Grant this session a job directory.
    ///
    /// # Errors
    ///
    /// [`SessionError::StillGranted`] if the previous job was never revoked.
    /// That is a programming error rather than a runtime condition, and it is
    /// an error rather than a panic because the alternative is a crash in the
    /// middle of a batch.
    pub fn grant(
        &mut self,
        job: BrokeredOutput,
        provenance: Provenance,
    ) -> Result<(), SessionError> {
        if self.granted.is_some() {
            return Err(SessionError::StillGranted);
        }
        self.granted = Some(job);
        self.serving = Some(provenance);
        self.served += 1;
        Ok(())
    }

    /// Withdraw the current job directory.
    ///
    /// **Revocation removes the handle, not the memory**, and `09` §8 says so
    /// rather than implying otherwise. `DUPLICATE_CLOSE_SOURCE` yanks the
    /// descriptor out from under a running worker and it gets
    /// `STATUS_INVALID_HANDLE` if it tries — verified inside an AppContainer.
    /// What it does not do is clear whatever the worker already read.
    ///
    /// # Errors
    ///
    /// [`SessionError::NotGranted`] if there is nothing to revoke.
    pub fn revoke(&mut self) -> Result<BrokeredOutput, SessionError> {
        self.granted.take().ok_or(SessionError::NotGranted)
    }

    /// Whether a capability is currently outstanding.
    #[must_use]
    pub const fn is_granted(&self) -> bool {
        self.granted.is_some()
    }

    /// How many files this session has served. Recorded in the receipt.
    #[must_use]
    pub const fn served(&self) -> usize {
        self.served
    }

    /// The reuse policy in force.
    #[must_use]
    pub const fn reuse(&self) -> WorkerReuse {
        self.reuse
    }

    /// Whether this session shared a worker across more than one file.
    ///
    /// The receipt says so per conversion. That is not an apology for the
    /// tradeoff — it is the product's second pillar applied to its own
    /// internals.
    #[must_use]
    pub const fn was_shared(&self) -> bool {
        self.served > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SR-20: an untrusted file never shares, in **either** direction.
    ///
    /// Both orderings matter and only one is obvious. A downloaded file must
    /// not join a session serving your own files — and your own files must not
    /// join one serving a downloaded file, which is the case a rule written as
    /// "reject untrusted newcomers" would miss.
    #[test]
    fn an_untrusted_file_never_shares_either_way() {
        // Untrusted arriving at a trusted session.
        let mut s = WorkerSession::open(WorkerReuse::Balanced);
        s.serving = Some(Provenance::Trusted);
        assert!(
            !s.accepts(Provenance::Untrusted),
            "a downloaded file joined a session serving the user's own files"
        );

        // Trusted arriving at an untrusted session.
        let mut s = WorkerSession::open(WorkerReuse::Balanced);
        s.serving = Some(Provenance::Untrusted);
        assert!(
            !s.accepts(Provenance::Trusted),
            "the user's own file joined a session that had served a downloaded one"
        );

        // And untrusted never shares with untrusted either: two hostile files
        // are not made safe by being hostile together.
        let mut s = WorkerSession::open(WorkerReuse::Balanced);
        s.serving = Some(Provenance::Untrusted);
        assert!(!s.accepts(Provenance::Untrusted));
    }

    /// The control: `Balanced` genuinely reuses between trusted files.
    ///
    /// Without this, every assertion above is satisfied by a session that
    /// accepts nothing — which would be perfectly safe and would leave the
    /// Windows latency gate with no fix.
    #[test]
    fn balanced_actually_reuses_between_trusted_files() {
        let mut s = WorkerSession::open(WorkerReuse::Balanced);
        s.serving = Some(Provenance::Trusted);
        assert!(
            s.accepts(Provenance::Trusted),
            "Balanced refused to reuse between two of the user's own files"
        );
    }

    /// `Isolated` takes one file and no more.
    #[test]
    fn isolated_never_shares_with_anything() {
        for first in [Provenance::Trusted, Provenance::Untrusted] {
            for next in [Provenance::Trusted, Provenance::Untrusted] {
                let mut s = WorkerSession::open(WorkerReuse::Isolated);
                s.serving = Some(first);
                assert!(
                    !s.accepts(next),
                    "Isolated shared between {first:?} and {next:?}"
                );
            }
        }
    }

    /// A fresh session takes anything; what it takes decides everything after.
    #[test]
    fn a_fresh_session_accepts_any_provenance() {
        for reuse in [WorkerReuse::Balanced, WorkerReuse::Isolated] {
            for p in [Provenance::Trusted, Provenance::Untrusted] {
                assert!(
                    WorkerSession::open(reuse).accepts(p),
                    "a fresh {reuse:?} session refused a {p:?} file"
                );
            }
        }
    }

    /// Grant and revoke pair up, and a second grant without a revoke is an
    /// error rather than a silently doubled capability.
    #[test]
    fn a_capability_cannot_be_granted_twice() {
        let t = tmp("grant");
        let job = BrokeredOutput::open(&t.0).expect("open");
        let mut s = WorkerSession::open(WorkerReuse::Balanced);

        s.grant(job.clone(), Provenance::Trusted)
            .expect("first grant");
        assert!(s.is_granted());
        assert_eq!(
            s.grant(job.clone(), Provenance::Trusted),
            Err(SessionError::StillGranted),
            "a second job directory was granted while the first was outstanding"
        );

        s.revoke().expect("revoke");
        assert!(!s.is_granted(), "revoke left the capability outstanding");
        assert_eq!(s.revoke(), Err(SessionError::NotGranted));

        // And the session is usable again afterwards.
        s.grant(job, Provenance::Trusted)
            .expect("regrant after revoke");
    }

    /// The receipt can tell whether a worker was shared.
    #[test]
    fn sharing_is_visible_in_the_count() {
        let t = tmp("count");
        let job = BrokeredOutput::open(&t.0).expect("open");
        let mut s = WorkerSession::open(WorkerReuse::Balanced);

        assert!(!s.was_shared(), "an empty session claimed to have shared");
        s.grant(job.clone(), Provenance::Trusted).unwrap();
        s.revoke().unwrap();
        assert_eq!(s.served(), 1);
        assert!(!s.was_shared(), "one file is not sharing");

        s.grant(job, Provenance::Trusted).unwrap();
        assert_eq!(s.served(), 2);
        assert!(
            s.was_shared(),
            "two files in one session is sharing, and the receipt must say so"
        );
    }

    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // openconvert-lint: allow -- test scratch teardown
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("tx-sess-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&p).expect("mkdir");
        Tmp(p)
    }
}
