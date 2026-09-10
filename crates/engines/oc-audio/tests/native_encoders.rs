//! Compile-and-test gate for the native encoder modules.
//!
//! `main.rs` does not declare these modules yet — the orchestrator wires the
//! `"mp3"` / `"ogg"` arms at merge time — so this harness pulls each file in
//! by path. Their own `#[cfg(test)]` units run from here, gated on the same
//! `has_lame` / `has_opus` cfgs that build.rs sets when vcpkg provides the
//! import libraries: without them the modules are skipped rather than
//! failing to link.

#![cfg(windows)]

#[cfg(has_lame)]
#[path = "../src/lame.rs"]
mod lame;

#[cfg(has_opus)]
#[path = "../src/opus.rs"]
mod opus;
