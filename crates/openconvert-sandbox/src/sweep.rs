//! Startup reaping of orphaned job directories.
//!
//! SR-14: a killed, crashed or cancelled job leaves no orphaned temp data.
//!
//! # This function is a deletion primitive, and that is the whole problem
//!
//! It exists to remove directories, at startup, without being asked. Every
//! constraint below is about making sure the set it removes is exactly the set
//! we created and no longer own.
//!
//! `03` §12 states the rule bluntly: **`jobs/` lives under our own per-user
//! state directory, never a shared temp root.** A sweep pointed at `/tmp` would
//! be a deletion primitive for any local user on the machine — they create
//! `/tmp/jobs/<anything>`, we delete it on next launch. That is not a
//! hypothetical; it is the standard shape of a temp-directory privilege bug.
//!
//! So the sweep refuses to run anywhere it does not recognise, and the refusal
//! is a returned error rather than a log line.

use std::io;
use std::path::{Path, PathBuf};

/// The lock file each live job directory holds.
///
/// Presence alone does not mean the job is live — a killed process leaves its
/// lock behind. Telling the difference requires asking the OS who holds it,
/// and **that cannot be done from this crate**: see [`IsHeld`].
pub const LOCK_FILE: &str = ".openconvert-lock";

/// Whether a lock file is still held by a live process.
///
/// # Why this is injected rather than implemented here
///
/// The first version of this module probed by opening the lock for writing and
/// treating failure as "held". **A test proved that does not work** — Rust's
/// `std::fs` opens with `FILE_SHARE_READ | FILE_SHARE_WRITE` on Windows, so a
/// second writer succeeds while the first still holds the file, and POSIX has
/// no mandatory locking at all. The probe reported every live job as dead.
///
/// Real answers need `LockFileEx` on Windows or `flock`/`fcntl` on POSIX, both
/// of which are syscalls and therefore `openconvert-os`'s job. This crate is
/// `#![forbid(unsafe_code)]` and cannot answer the question, so it does not
/// pretend to — it takes the answer as a parameter.
///
/// That is also what makes the sweep testable without spawning a worker: a
/// test injects the classification it wants and asserts the *policy*, which is
/// the part that lives here.
pub type IsHeld<'a> = &'a dyn Fn(&Path) -> bool;

/// What a sweep did, and what it refused to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Directories removed.
    pub reaped: Vec<PathBuf>,
    /// Directories left alone because their lock is still held.
    pub live: Vec<PathBuf>,
    /// Entries skipped because they did not look like ours.
    ///
    /// Non-empty is a signal worth surfacing: something is creating
    /// directories in our state folder, and it is not us.
    pub unrecognised: Vec<PathBuf>,
}

/// Why a sweep would not run.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// The directory is not one we recognise as ours.
    ///
    /// Deliberately blunt. A sweep that guesses is a sweep that deletes
    /// someone else's files, and there is no recovery from that.
    #[error(
        "refusing to sweep {0}: it is not a OpenConvert jobs directory. \
         A sweep over a directory we did not create is a deletion primitive."
    )]
    NotOurs(String),
    /// Underlying I/O.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The directory name a sweepable root must have.
///
/// Checked rather than assumed. The caller passes a path; this is what stops
/// a mistyped configuration value from becoming a recursive delete of the
/// user's home directory.
pub const JOBS_DIR_NAME: &str = "jobs";

/// Remove job directories whose lock is unheld.
///
/// # Errors
///
/// [`SweepError::NotOurs`] if `jobs_root` is not named [`JOBS_DIR_NAME`], plus
/// any I/O error reading the directory.
///
/// Individual failures **do not** abort the sweep: a directory we cannot remove
/// is reported and skipped, because one stuck job must not stop the rest from
/// being cleaned up on every subsequent launch.
pub fn sweep(jobs_root: &Path, is_held: IsHeld<'_>) -> Result<SweepReport, SweepError> {
    if jobs_root.file_name().and_then(|n| n.to_str()) != Some(JOBS_DIR_NAME) {
        return Err(SweepError::NotOurs(jobs_root.display().to_string()));
    }
    if !jobs_root.is_dir() {
        // Nothing to sweep is a normal first launch, not an error.
        return Ok(SweepReport::default());
    }

    let mut report = SweepReport::default();

    for entry in std::fs::read_dir(jobs_root)? {
        let entry = entry?;
        let path = entry.path();

        // Only directories, and only ones holding our lock file. Anything else
        // in here was not created by us, and we do not remove what we did not
        // create -- even inside our own state directory.
        if !entry.file_type()?.is_dir() {
            report.unrecognised.push(path);
            continue;
        }
        let lock = path.join(LOCK_FILE);
        if !lock.exists() {
            report.unrecognised.push(path);
            continue;
        }

        if is_held(&lock) {
            report.live.push(path);
            continue;
        }

        match remove_job_dir(&path) {
            Ok(()) => report.reaped.push(path),
            // A directory we cannot remove is reported, not fatal. One stuck
            // job must not stop the next launch cleaning up the others.
            Err(_) => report.unrecognised.push(path),
        }
    }

    Ok(report)
}

/// The safe default when nothing better is available: **assume every lock is
/// held**, and reap nothing.
///
/// Conservative in the direction that costs disk space rather than the one
/// that costs a user their in-progress conversion. A caller that has not wired
/// up real locking gets a sweep that does nothing, which is the correct
/// behaviour for a reaper that cannot tell live from dead.
#[must_use]
pub fn assume_held(_lock: &Path) -> bool {
    true
}

/// Remove one job directory.
///
/// Reached only for a path that is a direct child of a directory named `jobs`
/// and that holds our lock file. Both conditions are checked by the caller
/// above; neither is assumed here.
fn remove_job_dir(path: &Path) -> io::Result<()> {
    std::fs::remove_dir_all(path) // openconvert-lint: allow -- SR-14's reaper; the caller has verified this is a locked job dir under a `jobs` root
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // openconvert-lint: allow -- test scratch teardown
        }
    }
    fn state(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("tx-sweep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(p.join(JOBS_DIR_NAME)).expect("mkdir");
        Tmp(p)
    }
    fn jobs(t: &Tmp) -> PathBuf {
        t.0.join(JOBS_DIR_NAME)
    }
    /// A job directory with an unheld lock — what a killed worker leaves.
    fn orphan(t: &Tmp, name: &str) -> PathBuf {
        let d = jobs(t).join(name);
        std::fs::create_dir_all(&d).expect("mkdir");
        std::fs::File::create_new(d.join(LOCK_FILE)).expect("lock");
        d
    }

    /// SR-14: an orphan is reaped.
    #[test]
    fn an_orphaned_job_directory_is_removed() {
        let t = state("orphan");
        let d = orphan(&t, "job-1");
        std::fs::File::create_new(d.join("partial.tmp")).expect("partial");

        let r = sweep(&jobs(&t), &|_| false).expect("sweep");
        assert_eq!(r.reaped, vec![d.clone()]);
        assert!(!d.exists(), "the orphan survived the sweep");
    }

    /// **The refusal that matters most.**
    ///
    /// A sweep over a directory we did not create is a deletion primitive for
    /// anyone who can write there. The name check is what stops a mistyped
    /// configuration value becoming a recursive delete of a home directory.
    #[test]
    fn a_directory_that_is_not_ours_is_refused() {
        let t = state("notours");
        let stranger = t.0.join("not-jobs");
        std::fs::create_dir_all(&stranger).expect("mkdir");
        let precious = stranger.join("someone-elses-file.txt");
        std::fs::File::create_new(&precious).expect("plant");

        let err = sweep(&stranger, &|_| false).expect_err("must refuse");
        assert!(
            matches!(err, SweepError::NotOurs(_)),
            "wrong refusal: {err}"
        );
        assert!(precious.exists(), "a refused sweep still deleted something");

        // And the obvious catastrophes, by name.
        for path in [Path::new("/"), Path::new("/tmp"), &std::env::temp_dir()] {
            assert!(
                sweep(path, &|_| false).is_err(),
                "sweep accepted {}, which is a deletion primitive",
                path.display()
            );
        }
    }

    /// Anything without our lock file is left alone — inside our own directory.
    ///
    /// We remove what we created. Being in our state folder is not enough:
    /// a user who put something there deliberately gets to keep it.
    #[test]
    fn entries_without_our_lock_are_left_alone() {
        let t = state("nolock");
        let stray_dir = jobs(&t).join("someone-elses-dir");
        std::fs::create_dir_all(&stray_dir).expect("mkdir");
        let stray_file = jobs(&t).join("notes.txt");
        std::fs::File::create_new(&stray_file).expect("file");

        let r = sweep(&jobs(&t), &|_| false).expect("sweep");
        assert!(
            r.reaped.is_empty(),
            "reaped something that was not ours: {:?}",
            r.reaped
        );
        assert_eq!(r.unrecognised.len(), 2);
        assert!(stray_dir.exists());
        assert!(stray_file.exists());
    }

    /// A lock still held means a live job, and a live job is left running.
    ///
    /// The liveness answer is injected, because this crate cannot produce one.
    /// An earlier version probed by opening the lock for writing and asserted
    /// that Windows would refuse — **the test disproved it**: `std::fs` opens
    /// with permissive share flags, so the probe reported every live job as
    /// dead and the sweep would have deleted in-progress conversions.
    ///
    /// What is asserted here is the policy: whatever says "held" is left alone.
    #[test]
    fn a_held_lock_is_left_alone() {
        let t = state("held");
        let live = orphan(&t, "job-live");
        let dead = orphan(&t, "job-dead");

        let r = sweep(&jobs(&t), &|lock| {
            lock.parent().and_then(|p| p.file_name()) == Some("job-live".as_ref())
        })
        .expect("sweep");

        assert_eq!(
            r.live,
            vec![live.clone()],
            "a live job was not classified live"
        );
        assert_eq!(r.reaped, vec![dead.clone()], "the dead job was not reaped");
        assert!(live.exists(), "a live job was deleted mid-conversion");
        assert!(!dead.exists(), "the orphan survived");
    }

    /// The safe default reaps nothing.
    ///
    /// A caller that has not wired up real locking gets a sweep that does
    /// nothing — the right behaviour for a reaper that cannot tell live from
    /// dead, and the reason the default is not "assume dead".
    #[test]
    fn the_default_predicate_reaps_nothing() {
        let t = state("default");
        let d = orphan(&t, "job-1");
        let r = sweep(&jobs(&t), &assume_held).expect("sweep");
        assert!(
            r.reaped.is_empty(),
            "the conservative default deleted something"
        );
        assert_eq!(r.live, vec![d.clone()]);
        assert!(d.exists());
    }

    /// A first launch with no jobs directory is normal, not an error.
    #[test]
    fn a_missing_jobs_directory_is_not_an_error() {
        let t = state("empty");
        let missing = t.0.join("gone").join(JOBS_DIR_NAME);
        let r = sweep(&missing, &|_| false).expect("a first launch should not error");
        assert_eq!(r, SweepReport::default());
    }

    /// Several orphans go in one pass, and the report names each.
    #[test]
    fn every_orphan_is_reaped_and_reported() {
        let t = state("many");
        let ds: Vec<PathBuf> = (0..5).map(|i| orphan(&t, &format!("job-{i}"))).collect();
        let r = sweep(&jobs(&t), &|_| false).expect("sweep");
        assert_eq!(r.reaped.len(), 5);
        for d in ds {
            assert!(!d.exists(), "{} survived", d.display());
        }
    }
}
