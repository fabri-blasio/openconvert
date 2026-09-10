//! The wire boundary -- **the only module in this crate or `openconvert-sandbox`
//! that implements `Deserialize`**, asserted by an xtask scan.
//!
//! Everything crossing the host/worker protocol arrives as a `wire::` type and
//! converts to its domain type through a fallible constructor that clamps. The
//! reason is that `Properties` crosses from a **confined, possibly compromised**
//! engine straight into routing, and a derived `Deserialize` on a validated
//! type reconstructs it without ever running its validator.
//!
//! Frame bounds live on the session, not here, and are checked **before**
//! allocation: an attacker-chosen length prefix is a *correct* decode as far as
//! any parser is concerned, so fuzzing will never find it -- 7.4M cases did not
//! (spike S29). Only a bound catches it.

use crate::limits::{LimitCeiling, Limits};

/// Limits as reported by an engine. Untrusted until clamped.
#[derive(Debug, Clone, Copy)]
pub struct WireLimits {
    /// Engine-proposed memory ceiling, in bytes.
    pub memory_bytes: u64,
    /// Engine-proposed pixel ceiling.
    pub decode_pixels: u64,
    /// Engine-proposed archive nesting depth.
    pub archive_depth: u8,
    /// Engine-proposed archive entry count.
    pub archive_entries: u32,
}

impl WireLimits {
    /// Convert into domain `Limits`, clamped against the policy ceiling.
    ///
    /// An engine may **narrow** a limit and never widen one. There is no
    /// conversion that skips the ceiling.
    #[must_use]
    pub fn into_limits(self, ceiling: &LimitCeiling, base: Limits) -> Limits {
        ceiling.clamp(Limits {
            memory_bytes: self.memory_bytes,
            decode_pixels: self.decode_pixels,
            archive_depth: self.archive_depth,
            archive_entries: self.archive_entries,
            ..base
        })
    }
}
