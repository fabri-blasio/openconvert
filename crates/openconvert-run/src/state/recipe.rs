//! Saved recipes.
//!
//! A recipe is *a conversion request worth keeping*: name, target formats,
//! and the hash of the [`openconvert_core::plan::PlanRequest`] it was routed
//! from. It stores **a request plus a hash, never a `Plan`** — plans do not
//! deserialise by design (I2), because a plan built against one machine's
//! isolation ladder must never be replayed onto another as if nothing
//! changed. Loading re-routes against the current machine; a differing hash
//! is surfaced, not hidden.

use openconvert_core::policy::OnConflict;
use serde::{Deserialize, Serialize};
use std::io::Write as _;

/// A pinned conversion request, stored as `<name>.recipe.toml`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Recipe {
    /// The name the user chose; also the file stem.
    pub name: String,
    /// Creation date, UTC, `YYYY-MM-DD`.
    pub created: String,
    /// Hash of the request this was routed from at save time. On load, the
    /// same hash is recomputed over the fresh route and any difference is
    /// reported to the user rather than papered over.
    pub plan_hash: u64,
    /// The detected input format's name (`"png"`).
    pub input_format: String,
    /// The target format's name (`"jpeg"`).
    pub target_format: String,
}

/// Where a named recipe lives.
#[must_use]
pub fn path_for(name: &str) -> std::path::PathBuf {
    super::paths::recipes_dir().join(format!("{name}.recipe.toml"))
}

/// Write a recipe into the state directory.
///
/// # Errors
///
/// Serialisation failure, any I/O failure writing the file, and
/// [`std::io::ErrorKind::AlreadyExists`] when a recipe of that name is
/// already on disk — a re-save refuses rather than replacing.
pub fn save(recipe: &Recipe) -> std::io::Result<std::path::PathBuf> {
    save_at(&super::paths::recipes_dir(), recipe)
}

/// Save into an explicit directory. Same rules as [`save`]; separate so
/// tests can point at a temp directory instead of the user's real state.
fn save_at(dir: &std::path::Path, recipe: &Recipe) -> std::io::Result<std::path::PathBuf> {
    // The state directory is created lazily by whatever writes first, and until
    // now nothing on the recipe path did: on a machine that had never run a
    // batch, the very first `openconvert recipe save` died with a bare
    // "The system cannot find the path specified. (os error 3)" and wrote
    // nothing. `journal.rs` already does this before appending; the asymmetry
    // was invisible to the tests because every one of them handed `save_at` a
    // directory it had just created.
    std::fs::create_dir_all(dir)?;
    let filename = format!("{}.recipe.toml", recipe.name);
    let contents = toml::to_string_pretty(recipe)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Through [`crate::write`] — this crate's only permitted creation path
    // (I12/SR-15). `Fail`, not `Suffix`: silently growing a second copy of a
    // pin the user asked to update is its own kind of surprise, so the safe
    // answer is the refusal, with what to do about it.
    let (placed, file) = match crate::write::create_output(dir, &filename, OnConflict::Fail) {
        Ok(ok) => ok,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "recipe '{}' already exists ({}) — remove it first to replace it",
                    recipe.name,
                    dir.join(&filename).display()
                ),
            ));
        }
        Err(e) => return Err(e),
    };
    let path = match placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p,
    };
    let mut file = file.ok_or_else(|| {
        std::io::Error::other("recipe placement succeeded but produced no handle")
    })?;
    file.write_all(contents.as_bytes())?;
    Ok(path)
}

/// Read a recipe from an explicit path.
///
/// # Errors
///
/// A string, not an io::Error: every failure here is a message for the user
/// ("could not read", "corrupt"), not a condition to match on.
pub fn load(path: &std::path::Path) -> Result<Recipe, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("could not read recipe: {e}"))?;
    toml::from_str(&contents).map_err(|e| format!("corrupt recipe: {e}"))
}

/// Today's date, UTC, as `YYYY-MM-DD`.
///
/// Fills [`Recipe::created`] at save time. No calendar dependency: eleven
/// lines of Howard Hinnant's `civil_from_days` beat a crate whose only other
/// job would be to exist. Tested against known epochs below, including a
/// leap day.
#[must_use]
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_date(secs / 86_400)
}

/// Calendar date from days since 1970-01-01. Hinnant's algorithm, verbatim.
fn civil_date(days_since_epoch: u64) -> String {
    let z = days_since_epoch as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = era * 400 + yoe + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    /// The date arithmetic, pinned against epochs chosen to hurt:
    /// day zero, a leap day, and a century boundary year.
    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(1), "1970-01-02");
        // 2000-02-29 — the leap day the naive div-by-4 misses.
        assert_eq!(civil_date(951_782_400 / DAY), "2000-02-29");
        // 2000-03-01 — the day after, where February must have ended.
        assert_eq!(civil_date(951_782_400 / DAY + 1), "2000-03-01");
        // 2100-03-01 — a century year that is NOT a leap year.
        assert_eq!(civil_date(4_107_542_400 / DAY), "2100-03-01");
    }

    /// Today parses back as a real date in the expected decade.
    #[test]
    fn today_is_a_plausible_date() {
        let t = today_utc();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20"), "got {t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[7..8], "-");
    }

    /// A temp recipes directory, emptied first for repeatability.
    fn test_dir(name: &str) -> std::path::PathBuf {
        let tmp =
            std::env::temp_dir().join(format!("tx-recipe-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    fn sample(name: &str) -> Recipe {
        Recipe {
            name: name.into(),
            created: "2026-08-23".into(),
            plan_hash: 0x1234_5678_9abc_def0,
            input_format: "heic".into(),
            target_format: "jpeg".into(),
        }
    }

    /// Save then load round-trips every field through TOML unchanged.
    #[test]
    fn save_then_load_round_trips() {
        let dir = test_dir("round-trip");
        let path = dir.join("heic-to-jpeg.recipe.toml");
        let contents = toml::to_string_pretty(&sample("heic-to-jpeg")).unwrap();
        std::fs::write(&path, contents).unwrap(); // openconvert-lint: allow -- test scratch

        let r = load(&path).unwrap();
        assert_eq!(r.name, "heic-to-jpeg");
        assert_eq!(r.created, "2026-08-23");
        assert_eq!(r.plan_hash, 0x1234_5678_9abc_def0);
        assert_eq!(r.input_format, "heic");
        assert_eq!(r.target_format, "jpeg");
    }

    /// First save on a machine with no state directory yet. Every other test
    /// creates the directory first, which is precisely why this one exists.
    #[test]
    fn save_creates_the_recipes_directory() {
        let dir = test_dir("fresh-machine")
            .join("OpenConvert")
            .join("recipes");
        assert!(!dir.exists());

        let path = save_at(&dir, &sample("first-ever")).unwrap();

        assert!(path.exists(), "recipe not written to {}", path.display());
        assert_eq!(load(&path).unwrap().name, "first-ever");
    }

    /// Corrupt state degrades to a message, never a panic (SR-19).
    #[test]
    fn corrupt_recipe_is_an_error_not_a_crash() {
        let dir = test_dir("corrupt");
        let path = dir.join("bad.recipe.toml");
        std::fs::write(&path, "[[[ not toml").unwrap(); // openconvert-lint: allow -- test scratch
        let err = load(&path).unwrap_err();
        assert!(err.starts_with("corrupt recipe:"), "got {err}");
    }

    /// A missing file says so, distinctly from corruption.
    #[test]
    fn missing_recipe_names_the_problem() {
        let dir = test_dir("missing");
        let err = load(&dir.join("nope.recipe.toml")).unwrap_err();
        assert!(err.starts_with("could not read recipe:"), "got {err}");
    }

    /// A re-save of an existing name refuses rather than replaces (I12):
    /// destruction is not a side effect of saving, and the refusal says what
    /// to do about it. The original survives untouched.
    #[test]
    fn resaving_an_existing_recipe_refuses_and_preserves_it() {
        let dir = test_dir("resave");
        let r = sample("dup");
        save_at(&dir, &r).expect("first save");
        let original = load(&dir.join("dup.recipe.toml")).unwrap();

        let err = save_at(&dir, &r).expect_err("second save must refuse");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("remove it first"), "got {err}");

        let after = load(&dir.join("dup.recipe.toml")).unwrap();
        assert_eq!(original.created, after.created);
    }
}
