//! The **only** file-creation path in this crate.
//!
//! I12 / SR-15: a conversion cannot destroy a file that already exists.
//!
//! # Why this is one module and not a helper people reach for
//!
//! v0.5 enforced this by banning `File::create` in a source scan and called the
//! result *Impossible*. Measurement found **five other live routes on Windows
//! and seven on Linux** past that ban (S4, S25).
//! Two deserve naming, because no scan for a truncating *open* would ever find
//! them:
//!
//! - **`set_len(0)`** truncates through a handle opened with plain
//!   `write(true)`. There is no truncating open anywhere in the call path.
//! - **`fs::rename`** silently replaced its target on both filesystems tested,
//!   and write-to-temp-then-rename is *the* idiomatic atomic write. `03` §13's
//!   "partial output removed" rows push an implementer straight toward it.
//!   This is the likeliest way I12 breaks, and it breaks in good faith.
//!
//! So the gate is now a lint over eleven APIs, and this module is the one place
//! permitted to create anything. Everything here goes through
//! `File::create_new`, which is `O_EXCL` / `CREATE_NEW` and is safe on both
//! platforms — verified, not assumed.

use openconvert_core::policy::OnConflict;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// What happened when we tried to place an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// Created at this path.
    Created(PathBuf),
    /// A file was already there and policy said to leave it.
    Skipped(PathBuf),
}

/// Create an output, resolving conflicts **before** anything is opened.
///
/// That ordering is the control. A conflict discovered mid-write has already
/// destroyed something; a conflict resolved first cannot.
///
/// # Errors
///
/// Returns [`io::ErrorKind::AlreadyExists`] under [`OnConflict::Fail`], and any
/// underlying I/O error otherwise.
pub fn create_output(
    dir: &Path,
    name: &str,
    on_conflict: OnConflict,
) -> io::Result<(Placed, Option<File>)> {
    let first = dir.join(name);

    // The happy path is also the safe path: create_new refuses rather than
    // truncates, so the check and the create are one atomic operation and
    // there is no window between them.
    match File::create_new(&first) {
        Ok(f) => return Ok((Placed::Created(first), Some(f))),
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        Err(_) => {}
    }

    match on_conflict {
        OnConflict::Fail => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", first.display()),
        )),
        OnConflict::Skip => Ok((Placed::Skipped(first), None)),
        OnConflict::Suffix => suffix(dir, name),
    }
}

/// `report.pdf` → `report (2).pdf`, then `(3)`, and so on.
///
/// Each attempt is itself a `create_new`, so two processes racing for the same
/// suffix both succeed at different numbers rather than one silently winning.
fn suffix(dir: &Path, name: &str) -> io::Result<(Placed, Option<File>)> {
    let (stem, ext) = split_extension(name);
    // 2..1000 rather than an unbounded loop: a thousand collisions on one name
    // is a bug or an attack, and either way the answer is to stop.
    for n in 2..1000 {
        let candidate = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let path = dir.join(&candidate);
        match File::create_new(&path) {
            Ok(f) => return Ok((Placed::Created(path), Some(f))),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{name}: 998 suffixed variants already exist"),
    ))
}

/// Split on the **last** dot, so `archive.tar.gz` keeps `tar` in the stem.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        // A leading dot is a dotfile, not an extension: ".bashrc" must not
        // become " (2).bashrc".
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A scratch directory that cleans up after itself.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("openconvert-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p); // openconvert-lint: allow -- test scratch dir under temp_dir(), never a user path
            std::fs::create_dir_all(&p).expect("mkdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn plant(&self, name: &str, body: &[u8]) {
            let mut f = File::create_new(self.0.join(name)).expect("plant");
            f.write_all(body).expect("write");
        }
        fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.0.join(name)).expect("read")
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // openconvert-lint: allow -- test scratch dir teardown
        }
    }

    /// SR-15's headline: an existing file survives.
    ///
    /// This is the case an earlier revision of the design lost — an archive
    /// whose members are named after files already in the destination. Every
    /// traversal check passes them, correctly, because they are not traversal.
    #[test]
    fn an_existing_file_is_never_truncated() {
        let t = Tmp::new("notrunc");
        t.plant("report.pdf", b"IRREPLACEABLE");

        let (placed, file) =
            create_output(t.path(), "report.pdf", OnConflict::Suffix).expect("place");
        drop(file);

        assert_eq!(
            t.read("report.pdf"),
            b"IRREPLACEABLE",
            "the original was destroyed"
        );
        assert_eq!(placed, Placed::Created(t.path().join("report (2).pdf")));
    }

    #[test]
    fn skip_leaves_the_original_and_creates_nothing() {
        let t = Tmp::new("skip");
        t.plant("a.txt", b"KEEP");
        let (placed, file) = create_output(t.path(), "a.txt", OnConflict::Skip).expect("place");
        assert!(file.is_none(), "Skip handed back a writable file");
        assert_eq!(placed, Placed::Skipped(t.path().join("a.txt")));
        assert_eq!(t.read("a.txt"), b"KEEP");
    }

    /// `Fail` aborts rather than leaving a half-done job.
    ///
    /// A half-extracted archive is worse than a refused one: the user cannot
    /// tell which members are missing.
    #[test]
    fn fail_refuses_and_writes_nothing() {
        let t = Tmp::new("fail");
        t.plant("a.txt", b"KEEP");
        let err = create_output(t.path(), "a.txt", OnConflict::Fail).expect_err("should refuse");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(t.read("a.txt"), b"KEEP");
    }

    /// The control: with no collision, the file is created at its own name.
    ///
    /// Without this, every test above passes on an implementation that always
    /// refuses — which would be perfectly safe and completely useless.
    #[test]
    fn no_collision_creates_the_plain_name() {
        let t = Tmp::new("plain");
        let (placed, file) = create_output(t.path(), "new.txt", OnConflict::Suffix).expect("place");
        assert!(file.is_some(), "no file handle was returned");
        assert_eq!(placed, Placed::Created(t.path().join("new.txt")));
    }

    #[test]
    fn suffixes_climb_past_multiple_collisions() {
        let t = Tmp::new("climb");
        t.plant("x.txt", b"1");
        t.plant("x (2).txt", b"2");
        t.plant("x (3).txt", b"3");
        let (placed, _) = create_output(t.path(), "x.txt", OnConflict::Suffix).expect("place");
        assert_eq!(placed, Placed::Created(t.path().join("x (4).txt")));
        assert_eq!(t.read("x.txt"), b"1");
        assert_eq!(t.read("x (2).txt"), b"2");
    }

    /// `archive.tar.gz` keeps its compound extension; `.bashrc` is a dotfile.
    #[test]
    fn extensions_split_sensibly() {
        assert_eq!(
            split_extension("archive.tar.gz"),
            ("archive.tar", Some("gz"))
        );
        assert_eq!(split_extension("noext"), ("noext", None));
        assert_eq!(split_extension(".bashrc"), (".bashrc", None));
    }
}
