//! Where state lives.
//!
//! One per-user directory (`03` §12): every persistent artefact the shell
//! writes — journal, config, recipes — resolves under [`state_dir`], so a
//! sweep or an uninstall has exactly one place to reason about. The sweep
//! rule from `03` §13 applies: nothing outside this directory is ever
//! touched, because a cleanup over a shared temp root is a deletion
//! primitive.

use std::path::PathBuf;

/// The per-user state directory, by platform convention.
///
/// Reads only environment variables, and fails open to a relative fallback
/// rather than panicking: a machine with no `HOME` still converts files,
/// it just cannot resume a batch. `std::env::var` is on the purity list for
/// *core*, not here — this module **is** the imperative shell's memory.
#[must_use]
pub fn state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("OpenConvert"))
            .unwrap_or_else(|_| PathBuf::from("OpenConvert"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|h| {
                PathBuf::from(h)
                    .join("Library")
                    .join("Application Support")
                    .join("OpenConvert")
            })
            .unwrap_or_else(|_| PathBuf::from(".openconvert"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var("XDG_STATE_HOME")
            .map(|p| PathBuf::from(p).join("openconvert"))
            .or_else(|_| {
                std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/state/openconvert"))
            })
            .unwrap_or_else(|_| PathBuf::from(".openconvert"))
    }
}

/// Create the state directory if missing, and return where it is.
///
/// # Errors
///
/// Any I/O failure creating the directory.
pub fn ensure_state_dir() -> std::io::Result<PathBuf> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A private working area under the state dir.
///
/// NOT the system temp root. This module's own header says why: a cleanup over
/// a shared temp root is a deletion of other people's files. Everything here is
/// created by us, named by us, and removed by whoever created it.
#[must_use]
pub fn work_dir() -> PathBuf {
    state_dir().join("work")
}

/// Append-only batch journals, one `<batch-id>.jsonl` each.
#[must_use]
pub fn journal_dir() -> PathBuf {
    state_dir().join("journal")
}

/// Saved recipes, one `<name>.recipe.toml` each.
#[must_use]
pub fn recipes_dir() -> PathBuf {
    state_dir().join("recipes")
}

/// The user configuration file.
///
/// Note what does **not** exist: any key that could lower the isolation
/// floor. Network denial has no config key (09 §8), so a hostile
/// `config.toml` can at worst make conversions refuse to run — never run
/// them less confined.
#[must_use]
pub fn config_path() -> PathBuf {
    state_dir().join("config.toml")
}
