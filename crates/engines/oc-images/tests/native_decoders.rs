//! Compile-and-test gate for the RAW-decoder and ICC-colour modules.
//!
//! `main.rs` does not declare these modules yet — the orchestrator wires the
//! RAW dispatch arms at merge time (see private implementation notes) — so this harness pulls
//! each file in by path. Their own `#[cfg(test)]` units run from here, which
//! is what keeps the modules compiled and linted between now and that wiring.
//!
//! Gated on `has_heif` because build.rs sets exactly that cfg when the vcpkg
//! import libraries are present — and a machine that can link the worker's
//! existing native engine can link these two as well (`raw_r.lib` /
//! `lcms2.lib` come from the same tree; each module carries its own
//! `#[link]` attribute). Everywhere else this target compiles empty and the
//! suite stays green.
//!
//! Runtime note until build.rs copies `raw_r.dll` / `lcms2-2.dll` / `z.dll`
//! beside the test binaries (private implementation notes item 1): the test processes need the
//! vcpkg bin directory on PATH to load their DLLs,
//!
//! ```powershell
//! $env:PATH = "$env:USERPROFILE\vcpkg\installed\x64-windows\bin;$env:PATH"
//! cargo test -p oc-images
//! ```

#![cfg(all(windows, has_heif))]

#[path = "../src/raw.rs"]
mod raw;

#[path = "../src/colour.rs"]
mod colour;
