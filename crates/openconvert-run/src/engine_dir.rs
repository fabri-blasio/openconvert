//! Where the engine set lives — the one answer, for every caller.
//!
//! # Why this module exists
//!
//! Three places resolved engine binaries and native libraries independently,
//! and all three said the same thing: *beside our own executable*. That is
//! correct during development, correct for a Windows install, and **wrong for
//! every other package format we ship.**
//!
//! Measured, from `tauri-bundler`'s own source: a `.deb` puts the binary in
//! `/usr/bin` and its resources in `/usr/lib/<ProductName>/`; a macOS bundle
//! splits them into `Contents/MacOS/` and `Contents/Resources/`. Only on
//! Windows does the resource directory happen to *be* the directory holding the
//! executable. So an installed copy on Linux or macOS found no workers, and
//! every conversion needing one refused — while `cargo run`, which executes out
//! of `target/<profile>/` where `build.rs` has just staged them, worked
//! perfectly. The build everyone used was the one where the files were already
//! in the right place.
//!
//! # The rule this does NOT relax
//!
//! **Never `PATH`.** A `PATH` lookup is a program name resolved by the
//! environment, which is the environment choosing our engine for us, and
//! `EngineBin` being a closed enum would then guarantee nothing about what
//! actually runs. That rule is unchanged and is asserted by a test below.
//!
//! What changes is only how many of *our own* directories are searched: two,
//! in a fixed order, both supplied by us. The second is set once at startup by
//! the host that knows its own packaging layout — the desktop app passes
//! Tauri's `resource_dir()`. The CLI sets nothing and keeps the old behaviour
//! exactly.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The host's resource directory, if it told us about one.
///
/// `OnceLock` rather than a mutable global: this is a fact about how the
/// program was packaged, fixed before the first conversion and never varying
/// afterwards. A setter that could run twice would let a second call change
/// where engines come from mid-session, which is a supply-chain question, not a
/// configuration one.
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Tell the engine resolver where this package put its resources.
///
/// Call once, before the first conversion. A second call is ignored and returns
/// `false` rather than panicking: losing a race at startup is not a reason to
/// take the process down, and the first value is the one that was already used.
///
/// The CLI never calls this. It runs from a directory that already holds its
/// engines, and adding a second search path it does not need would only widen
/// what an installed copy can execute.
pub fn set_resource_dir(dir: PathBuf) -> bool {
    RESOURCE_DIR.set(dir).is_ok()
}

/// The directory holding our own executable.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// The subdirectory a bundler preserves when it copies our declared resources.
///
/// `bundle.resources` is declared as `engines/*`, and every bundler keeps that
/// relative path -- so the files arrive at `<resources>/engines/`, not at
/// `<resources>/`. Searching the base alone found nothing in a real install and
/// everything in a development build, which is the same asymmetry this module
/// exists to remove.
const STAGED_SUBDIR: &str = "engines";

/// Every directory an engine may live in, in the order they are tried.
///
/// **Two bases, each with an optional `engines/` beneath it.** The bases are
/// the directory holding our executable and, when the host supplied one, its
/// resource directory. Four candidates sounds loose and is not: every one is
/// derived from `current_exe` -- what the kernel says we are -- or from the
/// host's own packaging, read in-process at startup. Nothing here reads `PATH`,
/// and nothing here reads an environment variable a caller could set.
#[must_use]
pub fn search_dirs() -> Vec<PathBuf> {
    let mut bases = Vec::with_capacity(2);
    if let Some(d) = exe_dir() {
        bases.push(d);
    }
    if let Some(d) = RESOURCE_DIR.get() {
        // A Windows install has both, and they are the same place. Listing it
        // twice would make the "we looked in ..." message read like a bug.
        if !bases.iter().any(|existing| existing == d) {
            bases.push(d.clone());
        }
    }
    if bases.is_empty() {
        bases.push(PathBuf::from("."));
    }

    let mut dirs = Vec::with_capacity(bases.len() * 2);
    for base in bases {
        dirs.push(base.join(STAGED_SUBDIR));
        dirs.push(base);
    }
    dirs
}

/// The full path to `name` if it exists in one of our directories.
///
/// Returns `None` when the file is in none of them — which is a missing engine,
/// not a reason to go looking somewhere else.
#[must_use]
pub fn locate(name: &str) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Where `name` *would* go, for an error message when it is nowhere.
///
/// The executable's own directory: what a development build stages into and
/// what a Windows repair restores. Deliberately not the first search entry,
/// which is now the `engines/` subdirectory and may not exist on an install.
#[must_use]
pub fn expected_path(name: &str) -> PathBuf {
    exe_dir().unwrap_or_else(|| PathBuf::from(".")).join(name)
}

/// Every place we looked, rendered for a message a user can act on.
///
/// `03` §13: name the file, the step and the engine, and say what to do. When
/// two directories were searched, saying only one of them sends someone looking
/// in the wrong place.
#[must_use]
pub fn searched_display() -> String {
    search_dirs()
        .iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(" and ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The rule.** Resolution must never consult `PATH`.
    ///
    /// Written as a real attempt rather than a code inspection, and without
    /// touching the environment: `set_var` is `unsafe` in this edition and this
    /// crate forbids `unsafe`, which is the gate working — a security test is
    /// not a reason to open that door.
    ///
    /// So the probe uses a program the platform already guarantees is on `PATH`
    /// and is not beside our executable. If `locate` ever grows a `PATH`
    /// fallback — the obvious "fix" the first time an engine looks missing —
    /// this finds it, rather than a receipt naming an engine we never shipped.
    #[test]
    fn resolution_never_reads_path() {
        // On PATH on every machine that can run the test, and in neither of our
        // directories. `cmd.exe` sits in System32; `sh` in /bin or /usr/bin.
        let on_path = if cfg!(windows) { "cmd.exe" } else { "sh" };

        // The premise, asserted rather than assumed: if the platform does not
        // actually have this on PATH, the test below proves nothing and should
        // say so instead of passing quietly.
        let path_has_it = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(on_path).is_file()))
            .unwrap_or(false);
        assert!(
            path_has_it,
            "{on_path} was expected on PATH; without it this test proves nothing"
        );

        assert!(
            locate(on_path).is_none(),
            "{on_path} resolved, and it exists only on PATH -- resolution is reading the environment, so EngineBin guarantees nothing about what runs"
        );
    }

    /// The search is our own bases and nothing else.
    ///
    /// Two bases at most, each contributing itself and its `engines/`
    /// subdirectory. The upper bound is the point: a resolver that quietly grew
    /// a third base would be searching somewhere nobody decided on.
    #[test]
    fn the_search_is_bounded_and_ours() {
        let dirs = search_dirs();
        assert!(
            (2..=4).contains(&dirs.len()),
            "expected one or two bases, each with an engines/ dir, got {dirs:?}"
        );
        let exe = exe_dir().expect("a current exe");
        assert!(
            dirs.contains(&exe),
            "the executable's own directory must always be searched: {dirs:?}"
        );
    }

    /// A missing engine reports the directory a repair would restore it to.
    #[test]
    fn the_expected_path_is_in_a_searched_directory() {
        let p = expected_path("oc-images");
        let parent = p.parent().expect("a parent");
        assert!(
            search_dirs().iter().any(|d| d == parent),
            "the path named in the error is not one of the searched directories"
        );
    }
}
