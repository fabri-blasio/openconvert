//! Test inputs for OpenConvert.
//!
//! Two halves, and the second matters as much as the first:
//!
//! - [`evil`] — hostile names, generated from checked-in code rather than
//!   fetched, so the SR-13 and SR-15 gates cannot skip on a fork PR.
//! - `benign/` — ordinary files, on disk, for round-trip and golden tests.
//!
//! Real exploit samples never enter this repository. They run in a separate
//! nightly job that is permitted to depend on a secret, because it is not the
//! gate. See `09` section 9.

pub mod evil;
