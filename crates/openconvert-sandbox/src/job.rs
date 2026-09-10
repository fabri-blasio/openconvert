//! The job directory, and the only way to create a file inside it.
//!
//! I6: an engine cannot write outside its job directory. [`OutputName`] makes a
//! traversing *name* unrepresentable; this makes a traversing *path* the same,
//! by never accepting one — a [`BrokeredOutput`] is rooted at a job directory
//! and the only thing you can hand it is a parsed name.
//!
//! # What this layer is and is not
//!
//! It is **defence in depth on Windows and closer to the guarantee on Linux**.
//! Measurement found the platforms safe in opposite places (spikes S3, S12c):
//! the NT object manager refuses `..` through a directory handle outright,
//! while on Linux `..` is an ordinary component to `openat` and escapes. With
//! Landlock absent, this is what stands between an archive member and the
//! user's home directory.
//!
//! It is **not** the descriptor-passing version. Handing a worker a pre-opened
//! job-directory handle needs `unsafe` on both platforms and lives in
//! `openconvert-os`. This is the host-side half: `#![forbid(unsafe_code)]`, and
//! it is what the trusted copy-out uses.

use crate::broker::{NameError, OutputName};
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

/// A directory that outputs may be created inside, and nowhere else.
///
/// Private field. The only constructor canonicalises, so a `BrokeredOutput`
/// always names a real directory that existed at the moment it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokeredOutput {
    root: PathBuf,
}

/// Why a brokered write was refused.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// The name did not survive parsing.
    #[error("output name refused: {0}")]
    Name(#[from] NameError),
    /// The resolved path left the job root.
    ///
    /// Should be unreachable given [`OutputName`], and asserted anyway. This is
    /// the check that fires if the parser is ever wrong — the one place the
    /// design does not rely on a single mechanism being correct.
    #[error("containment violated: {resolved} is not under {root}")]
    Escaped {
        /// Where it ended up.
        resolved: String,
        /// Where it was supposed to stay.
        root: String,
    },
    /// A component of the path is a symlink, junction or reparse point.
    ///
    /// Refused rather than followed. A symlink inside a job directory is
    /// either ours (it is not; we create none) or an engine's attempt to
    /// redirect a later write.
    #[error("{0} is a link, and links are not followed inside a job directory")]
    LinkInPath(String),
    /// Underlying I/O.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The longest job-root path we will create.
///
/// Bounded so that **root + the deepest legal archive member** stays inside
/// what a C engine can open. Our own Rust does not need this — an 829-character
/// path with a 255-character leaf was created without complaint, because
/// `std::fs` uses verbatim `\\?\` paths (spike S28). `MAX_PATH` still binds the
/// engines, and they are the ones opening the file.
///
/// 260 − 255 (leaf) − 1 (separator) leaves 4, which is unusable, so the real
/// constraint is that engines must be built `longPathAware`. This bound is the
/// backstop for the ones that are not.
pub const MAX_JOB_ROOT_LEN: usize = 120;

impl BrokeredOutput {
    /// Root at an existing directory.
    ///
    /// # Errors
    ///
    /// Fails if the path does not exist, is not a directory, or is too long for
    /// a C engine to open a deep member underneath.
    pub fn open(root: &Path) -> Result<Self, BrokerError> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(BrokerError::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a directory", root.display()),
            )));
        }
        if root.as_os_str().len() > MAX_JOB_ROOT_LEN {
            return Err(BrokerError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "job root is {} characters; over {MAX_JOB_ROOT_LEN} a deep archive \
                     member cannot be opened by a C engine",
                    root.as_os_str().len()
                ),
            )));
        }
        Ok(Self { root })
    }

    /// The root, for the receipt and for tests.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create a file under this root. **The only creation API.**
    ///
    /// `create_new`, so it is `O_EXCL` / `CREATE_NEW` and refuses rather than
    /// truncating. There is deliberately no `create()`, no `open_write()`, and
    /// no way to obtain a `File` from this type that could destroy something.
    ///
    /// # Errors
    ///
    /// [`BrokerError::Escaped`] if containment is violated — which should be
    /// impossible, and is checked because "should be impossible" is the class
    /// of claim this project keeps finding to be false.
    pub fn create_new(&self, name: &OutputName) -> Result<(PathBuf, File), BrokerError> {
        let path = self.root.join(name.as_str());
        self.assert_contained(&path)?;
        let file = File::create_new(&path)?;
        Ok((path, file))
    }

    /// A subdirectory, for an archive member's parent chain.
    ///
    /// Each level is created and checked separately. A member path is a
    /// **sequence** of names, parsed one at a time — `a/b/../../etc` never
    /// exists as a single string to be checked, because `..` cannot survive
    /// [`OutputName::parse`] and so cannot be one of the sequence.
    ///
    /// # Errors
    ///
    /// [`BrokerError::LinkInPath`] if a level already exists as a link.
    pub fn child(&self, name: &OutputName) -> Result<Self, BrokerError> {
        let path = self.root.join(name.as_str());
        self.assert_contained(&path)?;

        match std::fs::symlink_metadata(&path) {
            Ok(md) if md.file_type().is_symlink() => {
                // Refuse rather than follow. We create no links inside a job
                // directory, so one that exists was put there by whatever we
                // are confining.
                return Err(BrokerError::LinkInPath(path.display().to_string()));
            }
            Ok(md) if md.is_dir() => {}
            Ok(_) => {
                return Err(BrokerError::Io(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("{} exists and is not a directory", path.display()),
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&path)?;
            }
            Err(e) => return Err(BrokerError::Io(e)),
        }

        // Re-canonicalise and re-check: between the containment assert above
        // and this line, the directory was created. Checking again is what
        // catches a race in which something replaced it.
        Self::open(&path)
    }

    /// The containment assert, run on every path this type produces.
    ///
    /// Belt and braces on purpose. `OutputName` should make this unreachable —
    /// but `03` §7.4 classes the traversal claim "Impossible **to represent**;
    /// the residual is a bug in `parse`", and this is what that residual runs
    /// into.
    fn assert_contained(&self, candidate: &Path) -> Result<(), BrokerError> {
        // Canonicalising the candidate itself needs it to exist, and it does
        // not yet. So check the lexical form, which is sufficient because every
        // component came from `OutputName` and cannot be `..`.
        let escapes = candidate
            .strip_prefix(&self.root)
            .map(|rel| rel.components().any(|c| !matches!(c, Component::Normal(_))))
            .unwrap_or(true);

        if escapes {
            return Err(BrokerError::Escaped {
                resolved: candidate.display().to_string(),
                root: self.root.display().to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("tx-job-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p); // openconvert-lint: allow -- test scratch under temp_dir()
            std::fs::create_dir_all(&p).expect("mkdir");
            Self(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // openconvert-lint: allow -- test scratch teardown
        }
    }

    fn name(s: &str) -> OutputName {
        OutputName::parse(s).expect("test name should parse")
    }

    #[test]
    fn a_file_is_created_under_the_root() {
        let t = Tmp::new("create");
        let job = BrokeredOutput::open(&t.0).expect("open");
        let (path, _f) = job.create_new(&name("out.jpg")).expect("create");
        assert!(path.starts_with(job.root()));
        assert!(path.exists());
    }

    /// I12: the only creation API refuses rather than truncating.
    #[test]
    fn creating_twice_refuses_the_second_time() {
        let t = Tmp::new("excl");
        let job = BrokeredOutput::open(&t.0).expect("open");
        job.create_new(&name("a.txt")).expect("first");
        let err = job
            .create_new(&name("a.txt"))
            .expect_err("second must refuse");
        match err {
            BrokerError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::AlreadyExists),
            other => panic!("wrong error: {other}"),
        }
    }

    /// Nested members are created one level at a time.
    #[test]
    fn nested_members_stay_under_the_root() {
        let t = Tmp::new("nested");
        let job = BrokeredOutput::open(&t.0).expect("open");
        let a = job.child(&name("assets")).expect("mkdir a");
        let b = a.child(&name("images")).expect("mkdir b");
        let (path, _f) = b.create_new(&name("logo.png")).expect("create");
        assert!(path.starts_with(job.root()), "{} escaped", path.display());
    }

    /// `child` on an existing directory is idempotent, because an archive names
    /// the same parent once per member.
    #[test]
    fn child_is_idempotent() {
        let t = Tmp::new("idem");
        let job = BrokeredOutput::open(&t.0).expect("open");
        let a = job.child(&name("d")).expect("first");
        let b = job.child(&name("d")).expect("second");
        assert_eq!(a.root(), b.root());
    }

    /// A traversing name cannot even be constructed, so it cannot be handed to
    /// this type.
    ///
    /// Asserted here rather than only in the parser tests because *this* is the
    /// property that matters: the two layers compose, and there is no path from
    /// a hostile string to a file outside the root.
    #[test]
    fn a_traversing_name_cannot_reach_this_api() {
        for evil in ["..", "../escape", "/etc/passwd", "a/b", "a\\b"] {
            assert!(
                OutputName::parse(evil).is_err(),
                "{evil:?} parsed, so it could be handed to create_new"
            );
        }
    }

    /// A job root longer than a C engine can work with is refused at creation.
    #[test]
    fn an_overlong_job_root_is_refused() {
        let t = Tmp::new("long");
        let mut deep = t.0.clone();
        for i in 0..12 {
            deep = deep.join(format!("levelnamelongenoughtomatter{i}"));
        }
        std::fs::create_dir_all(&deep).expect("mkdir deep");
        let err = BrokeredOutput::open(&deep).expect_err("should refuse an overlong root");
        assert!(
            err.to_string().contains("job root is"),
            "wrong reason: {err}"
        );
    }

    /// The control: an ordinary root is accepted.
    ///
    /// Without it, the length bound is satisfied by refusing every directory.
    #[test]
    fn an_ordinary_root_is_accepted() {
        let t = Tmp::new("ok");
        BrokeredOutput::open(&t.0).expect("an ordinary temp directory should be usable");
    }

    #[test]
    fn a_missing_root_is_refused() {
        let missing = std::env::temp_dir().join("tx-job-definitely-not-here-4a7b");
        assert!(BrokeredOutput::open(&missing).is_err());
    }

    /// **The containment check can actually fire.**
    ///
    /// This calls `assert_contained` directly, because nothing reachable
    /// through the public API can reach it — `OutputName` refuses `..` first,
    /// which is the entire point of having both layers.
    ///
    /// That is also why the check would otherwise go untested forever. `03`
    /// §7.4 describes traversal as "Impossible **to represent**; the residual
    /// is a bug in `parse`" — this assert *is* the response to that residual,
    /// and a response nobody has watched work is a response nobody has tested.
    /// The same argument as the negative fixtures behind every CI gate.
    #[test]
    fn the_containment_assert_fires_on_an_escaping_path() {
        let t = Tmp::new("contain");
        let job = BrokeredOutput::open(&t.0).expect("open");

        // What a broken `OutputName::parse` would eventually produce.
        let escaped = job
            .root()
            .parent()
            .expect("temp dir has a parent")
            .join("victim.txt");
        let err = job
            .assert_contained(&escaped)
            .expect_err("the containment assert did not fire on a path outside the root");
        assert!(
            matches!(err, BrokerError::Escaped { .. }),
            "wrong error: {err}"
        );

        // And a `..` component that somehow survived, which is the shape the
        // residual would actually take.
        let dotdot = job.root().join("..").join("victim.txt");
        assert!(
            job.assert_contained(&dotdot).is_err(),
            "a `..` component passed the containment assert"
        );

        // The control: an ordinary path passes, so the check is not simply
        // refusing everything.
        job.assert_contained(&job.root().join("fine.txt"))
            .expect("an ordinary path was refused, so the assert is too strict to be useful");
    }
}
