//! The shell. Detection, the handle table, engine adapters, execution,
//! receipts and on-disk state.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod avidemux;
pub mod container;
pub mod detect;
pub mod engine_dir;
pub mod engines;
pub mod exec;
pub mod handles;
pub mod matroska;
pub mod models;
pub mod mp4demux;
pub mod mp4mux;
pub mod naming;
pub mod pdf;
pub mod pdfops;
pub mod pool;
pub mod probe;
pub mod receipt;
pub mod remux;
pub mod state;
pub mod strip;
pub mod tabular;
pub mod tools;
pub mod video;
pub mod worker_client;
pub mod write;
