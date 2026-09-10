//! `stream_copy` — the Class A video path, end to end.
//!
//! This is where `01` §154's flagship promise lives or dies: `MKV → MP4`
//! completes in seconds and is bit-identical, because the coded samples are
//! never touched — only their bookkeeping moves between containers.
//!
//! Two mechanisms, chosen by destination:
//!
//! - **EBML → MP4** demuxes into the graph ([`crate::matroska::demux`]) and
//!   re-boxes through [`crate::mp4mux`]. Samples cross verbatim; timestamps
//!   transfer exactly because the output media timescale is DERIVED from the
//!   segment's own TimecodeScale rather than guessed from a frame rate.
//! - **EBML → EBML** (WebM ⇄ MKV, → MKA) performs segment surgery: original
//!   elements splice through verbatim, so even timestamps' BYTES survive.
//!
//! Everything runs against `Limits::memory_bytes` as a retained-byte budget;
//! a hostile container stops mid-walk with an error naming the bound instead
//! of materialising.

use crate::matroska::{self, DemuxError};
use crate::mp4mux::{self, MuxError};
use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;

/// Why a stream copy could not happen.
#[derive(Debug, thiserror::Error)]
pub enum RemuxError {
    /// The source container refused to be parsed within its bounds.
    #[error(transparent)]
    Demux(#[from] DemuxError),
    /// The destination container could not be assembled from what was read.
    #[error(transparent)]
    Mux(#[from] MuxError),
    /// An MP4 source refused to be parsed within its bounds.
    #[error(transparent)]
    Mp4(#[from] crate::mp4demux::Mp4Error),
    /// The AVI reader refused.
    #[error(transparent)]
    Avi(#[from] crate::avidemux::AviError),
    /// This pair does not belong here. Structurally unreachable through
    /// `route()`, which only produces `StreamCopy` for rows this module
    /// supports — kept as an error so a drifted table fails loudly instead
    /// of re-encoding silently somewhere else.
    #[error("no stream-copy path exists for {from} to {to}")]
    UnsupportedPair {
        /// Where the bytes claim to be from.
        from: FormatId,
        /// Where they were asked to go.
        to: FormatId,
    },
}

/// Copy every coded sample from `from` into `to`, untouched.
///
/// Returns the new container plus disclosures — the things that existed in
/// the source and do not exist in the output. An empty list means nothing
/// was lost, which is exactly what Class A means and what a lossless remux
/// must be able to say.
///
/// # Errors
///
/// See [`RemuxError`].
pub fn stream_copy(
    bytes: &[u8],
    from: FormatId,
    to: FormatId,
    limits: &Limits,
) -> Result<(Vec<u8>, Vec<String>), RemuxError> {
    let budget = limits.memory_bytes;

    if matroska::supports_surgery(from, to) {
        let out = matroska::ebml_to_ebml(bytes, to == FormatId::Mka, budget)?;
        let removed = if to == FormatId::Mka {
            vec!["non-audio tracks removed (audio extraction)".to_string()]
        } else {
            // WebM ⇄ MKV: same elements, new header. Nothing to disclose.
            Vec::new()
        };
        return Ok((out, removed));
    }

    if matches!(from, FormatId::Mkv | FormatId::Webm) && to == FormatId::Mp4 {
        let graph = matroska::demux(bytes, budget)?;
        return mp4mux::mux(&graph).map_err(RemuxError::from);
    }

    // QUICKTIME IN, MP4 OUT. Both ends are ISO base media, so this is a demux
    // and a re-mux of the same coded samples -- the `ftyp` brand changes and
    // the payloads do not. The spike that preceded the route row ran real
    // exports through `mp4demux` with no QuickTime-specific handling at all.
    // AVI IN. The demuxer reframes Annex-B into AVCC and builds the `avcC`
    // from the stream's own parameter sets; from there it is the same graph
    // every other source produces, so both destinations are the writers that
    // already exist.
    if from == FormatId::Avi {
        let (graph, dropped) = crate::avidemux::demux(bytes, budget)?;
        let mut removed = dropped;
        removed.push(
            "the AVI index and its chunk padding; the coded pictures are carried \
             byte for byte and only their framing changed"
                .to_string(),
        );
        // SAID OUTRIGHT, because it is the one thing this route cannot carry
        // and the one a viewer might notice.
        //
        // AVI records decode order and nothing about display order. H.264 with
        // B-frames is stored in a different order from the one it is shown in,
        // and the container has nowhere to put the difference — so the output
        // has no `ctts`, exactly as ffmpeg's own AVI-to-MP4 stream copy
        // produces none. Players reorder from the bitstream, which is what
        // they already do when playing the AVI.
        removed.push(
            "the display order, which an AVI does not record: frames are carried in \
             decode order and a player reorders them from the bitstream, as it does \
             when playing the AVI itself"
                .to_string(),
        );
        return match to {
            FormatId::Mp4 => {
                let (out, mut more) = mp4mux::mux(&graph)?;
                more.extend(removed);
                Ok((out, more))
            }
            FormatId::Mkv | FormatId::Webm | FormatId::Mka => {
                let mut graph = graph;
                if to == FormatId::Mka {
                    let before = graph.tracks.len();
                    graph
                        .tracks
                        .retain(|t| t.kind == crate::matroska::TrackKind::Audio);
                    if graph.tracks.len() < before {
                        removed.push("non-audio tracks removed (audio extraction)".to_string());
                    }
                }
                if graph.tracks.is_empty() {
                    return Err(RemuxError::UnsupportedPair { from, to });
                }
                Ok((matroska::graph_to_ebml(&graph)?, removed))
            }
            _ => Err(RemuxError::UnsupportedPair { from, to }),
        };
    }

    if from == FormatId::Mov && to == FormatId::Mp4 {
        let (graph, dropped) = crate::mp4demux::demux(bytes, budget)?;
        let (out, mut removed) = mp4mux::mux(&graph)?;
        // WHAT THE DEMUXER COULD NOT CARRY, in the receipt. A professional
        // `.mov` is H.264 beside LPCM audio and a timecode track; without
        // these lines the audio left the file under a Class A receipt saying
        // nothing was lost.
        removed.extend(dropped);
        return Ok((out, removed));
    }

    // The direction that did not exist. `mp4mux` muxed from the day it landed
    // and nothing read one back, so MP4 was a destination and never a source.
    if matches!(from, FormatId::Mp4 | FormatId::Mov)
        && matches!(to, FormatId::Mkv | FormatId::Webm | FormatId::Mka)
    {
        let (mut graph, dropped) = crate::mp4demux::demux(bytes, budget)?;
        // As above, and this direction had the same silence: `mp4 -> mkv` on a
        // file with an LPCM track produced a video-only Matroska and disclosed
        // nothing.
        let mut removed = dropped;
        if to == FormatId::Mka {
            let before = graph.tracks.len();
            graph
                .tracks
                .retain(|t| t.kind == crate::matroska::TrackKind::Audio);
            if graph.tracks.len() < before {
                removed.push("non-audio tracks removed (audio extraction)".to_string());
            }
        }
        if graph.tracks.is_empty() {
            return Err(RemuxError::UnsupportedPair { from, to });
        }
        let out = matroska::graph_to_ebml(&graph)?;
        return Ok((out, removed));
    }

    Err(RemuxError::UnsupportedPair { from, to })
}
