//! The pure core. See `03-ARCHITECTURE.md` section 4.
//!
//! No I/O, no clock, no environment, no `unsafe`. Enforced by `clippy.toml`,
//! denied at crate level in `Cargo.toml`, and by an xtask scan that refuses an
//! `#[allow(clippy::disallowed_*)]` anywhere in this crate.
//!
//! What that buys, concretely: every test in here runs without a filesystem,
//! and the plan preview is not a feature -- it is `route()` without `execute()`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod codec;
pub mod environment;
pub mod facts;
pub mod format;
pub mod isolation;
pub mod limits;
pub mod plan;
pub mod policy;
pub mod predict;
pub mod route;
pub mod sniff;
pub mod target;
pub mod wire;
