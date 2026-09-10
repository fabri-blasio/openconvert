//! THE BOUNDARY. See `03-ARCHITECTURE.md` section 5.
//!
//! This crate holds the compile-time invariants -- `OutputName`, `BrokeredPath`,
//! `BrokeredOutput`, `EngineBin`, `Argv`, `WorkerSession`, `sweep` -- and is
//! expected to stay under ~800 lines so that a third-party audit of it is both
//! affordable and pointed.
//!
//! **It holds no syscalls.** v0.5 put process creation here and described it as
//! "pure logic over std::fs and std::process". It cannot be: handing a worker
//! pre-opened descriptors and nothing else requires `unsafe` on both platforms,
//! by two independent routes (spikes S3, S12c). `spawn()` lives in
//! `openconvert-os`; this crate calls one typed entry point.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod argv;
pub mod broker;
pub mod display;
pub mod job;
pub mod protocol;
pub mod session;
pub mod sweep;
