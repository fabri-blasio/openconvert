//! Confinement syscalls, process creation, and read-back.
//! The only host-side crate permitted `unsafe`.
//!
//! Since v0.6 this crate holds **one invariant of its own**: it constructs the
//! `SandboxProfile` by reading back what actually engaged, rather than
//! recording what was requested (I11, `03` section 9.5). The v0.5 audit brief
//! said `-os` holds no invariants; that is no longer true, and `09` section 9
//! says so.
//!
//! Why read-back rather than request -- measured, and the error ran in **both**
//! directions (spikes S17b, S20b): ACG engages in an AppContainer whether or not
//! it is requested, and CIG does not engage even when it is.

#![deny(missing_docs)]

pub mod available;
pub mod gpu;
pub mod readback;

#[cfg(windows)]
pub mod appcontainer;

#[cfg(windows)]
pub mod job_object;

#[cfg(windows)]
pub mod spawn_windows;

#[cfg(not(windows))]
pub mod spawn_posix;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

/// The platform's piped spawn, under one name.
///
/// `-run` calls `spawn::spawn_piped` and never learns which one it got. The
/// two differ in what they *guarantee*, not in what they expose — and the
/// difference is recorded where it belongs, in `available::probe()`, which is
/// what decides whether a given profile may run at all.
#[cfg(windows)]
pub use spawn_windows as spawn;

#[cfg(not(windows))]
pub use spawn_posix as spawn;
