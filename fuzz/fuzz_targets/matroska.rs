//! The Matroska/WebM demuxer.
//!
//! Phase 4 put a full container parser in the HOST process — Class A video is
//! pure-Rust surgery by design (`02` §3.1), which means this parser, like
//! `sniff`, runs outside any sandbox. Every bound it enforces (depth, element
//! count, byte budget, advancing cursor) must hold for arbitrary input, and
//! this target is what checks that no input finds a path around them.
//!
//! Both entry points are exercised: demux into the graph, and EBML→EBML
//! surgery in both audio-only and full-copy modes — they walk different
//! code paths over the same hostile structure.

#![no_main]
use libfuzzer_sys::fuzz_target;
use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;

fuzz_target!(|data: &[u8]| {
    let budget = Limits::defaults().memory_bytes;
    // Demux may fail; panicking is the only crime.
    if let Ok(graph) = openconvert_run::matroska::demux(data, budget) {
        // The graph's invariants are part of the contract.
        for track in &graph.tracks {
            debug_assert!(track.number <= u64::from(u32::MAX));
        }
        // Muxing whatever demuxed must also never panic.
        let _ = openconvert_run::mp4mux::mux(&graph);
    }
    let _ = openconvert_run::matroska::ebml_to_ebml(data, false, budget);
    let _ = openconvert_run::matroska::ebml_to_ebml(data, true, budget);
    let _ = openconvert_run::remux::stream_copy(
        data,
        FormatId::Mkv,
        FormatId::Mp4,
        &Limits::defaults(),
    );
});
