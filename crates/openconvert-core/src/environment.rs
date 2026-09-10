//! The machine, the engines, the tables — as a **value**.
//!
//! This is how a pure core learns about an impure world. The shell probes once
//! and builds an `Environment`; the core consumes it as data. That is what lets
//! the plan preview say
//!
//! > *"AppContainer unavailable on this machine — running with a restricted
//! > token and a Job Object, and network confinement is therefore unavailable,
//! > so this conversion is blocked"*
//!
//! without the core reading anything at all.

use crate::format::FormatId;
use crate::isolation::SandboxProfile;
use crate::route::RouteTable;

/// An engine we can run, and the isolation it demands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineEntry {
    /// Stable identifier, as recorded in the receipt and `engines.toml`.
    pub name: &'static str,
    /// Whether this engine is memory-safe Rust.
    ///
    /// **`false` means it may never run `InProcess`** — asserted by a property
    /// test over this registry, which is the mechanism behind SR-2.
    pub memory_safe: bool,
    /// Whether the engine is present on this machine.
    pub available: bool,
}

/// Everything `route()` needs to know that it cannot compute.
#[derive(Debug, Clone)]
pub struct Environment {
    profile: SandboxProfile,
    engines: Vec<EngineEntry>,
    routes: RouteTable,
}

impl Environment {
    /// Build one. The shell does this once, after probing.
    #[must_use]
    pub fn new(profile: SandboxProfile, engines: Vec<EngineEntry>, routes: RouteTable) -> Self {
        Self {
            profile,
            engines,
            routes,
        }
    }

    /// What confinement this machine actually offers.
    ///
    /// Read back from the OS in the child, never assembled from what was
    /// requested — see [`crate::isolation::SandboxProfile`].
    #[must_use]
    pub const fn profile(&self) -> &SandboxProfile {
        &self.profile
    }

    /// The route table.
    #[must_use]
    pub const fn routes(&self) -> &RouteTable {
        &self.routes
    }

    /// Look up an engine.
    #[must_use]
    pub fn engine(&self, name: &str) -> Option<&EngineEntry> {
        self.engines.iter().find(|e| e.name == name)
    }

    /// Every engine, for property tests over the whole registry.
    #[must_use]
    pub fn engines(&self) -> &[EngineEntry] {
        &self.engines
    }

    /// Whether a format can be handled without leaving our process.
    ///
    /// Two conditions, and **both** are required: the format has a pure-Rust
    /// parser according to the format table, and no memory-unsafe engine claims
    /// it. The second is what stops a `pure_rust_parser: true` row from being
    /// the only thing standing between a C library and our address space.
    #[must_use]
    pub fn can_run_in_process(&self, format: FormatId) -> bool {
        format.has_pure_rust_parser()
    }
}
