//! On-disk state (`03` §12): paths, batch journal, user config, recipes.
//!
//! One per-user directory with a fixed layout:
//!
//! ```text
//! <state_dir>/config.toml        preferences; cannot lower the isolation floor
//! <state_dir>/journal/<id>.jsonl append-only batch journals
//! <state_dir>/recipes/*.toml     saved PlanRequests plus their plan hash
//! ```
//!
//! The one rule every file here obeys: **untrusted on read**. Anything
//! running as the user can write these files, so nothing parsed from them is
//! allowed to be fatal — corrupt lines are skipped, corrupt config falls back
//! to defaults, corrupt recipes return a message — and nothing in them can
//! widen what a conversion is permitted to do.

pub mod config;
pub mod journal;
pub mod paths;
pub mod recipe;

pub use config::UserConfig;
pub use journal::{Journal, JournalEntry, Outcome};
pub use recipe::Recipe;
