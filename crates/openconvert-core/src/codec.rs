//! Which codecs a container may carry — the fact a remux turns on.
//!
//! # Why this is core logic and not an engine's answer
//!
//! `Properties::Video` used to carry `stream_copyable: bool`, documented as
//! *"the single most consequential boolean an engine reports"*. It cannot be
//! reported, and the reason is structural rather than a matter of effort: a
//! stream is copyable **into a particular container**, and phase-2 detection
//! runs before a destination is chosen. `probe()` takes a file; `route()` takes
//! a target. The boolean was asked of the half of the system that cannot know.
//!
//! So an engine reports what it read — the codecs — and the decision moves
//! here, next to the route table that already knows the destination. That is
//! the same split the rest of this design uses: engines observe, the pure core
//! decides.
//!
//! # The table errs toward refusing
//!
//! A wrong `false` costs a re-encode: the route falls back to the Class B row
//! and the user is told fidelity is lost. A wrong `true` produces a file that
//! muxes cleanly and **will not play**, with a receipt attesting it was
//! lossless. Those are not symmetric, so anything not clearly carryable is
//! omitted, and formats standardised-but-poorly-supported are treated as not
//! carryable and say so in a comment rather than being quietly included.

use crate::format::FormatId;

/// A video codec, as identified from a container header.
///
/// Closed, like every other identity in this crate. A codec we cannot name is
/// [`VideoCodec::Other`], which is never carryable — an unknown stream is
/// exactly the one not to promise a lossless copy of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    H265,
    /// AV1.
    Av1,
    /// VP8.
    Vp8,
    /// VP9.
    Vp9,
    /// Apple ProRes.
    ///
    /// Carried by Matroska. There is no `Mov` in the format table yet, so the
    /// container it actually ships in is not addressable here — listed because
    /// identifying it is what stops it being reported as [`VideoCodec::Other`]
    /// and silently refused for the wrong reason.
    ProRes,
    /// Identified as video, not identified as one of the above.
    Other,
}

/// An audio codec, as identified from a container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// AAC.
    Aac,
    /// MP3.
    Mp3,
    /// Opus.
    Opus,
    /// Vorbis.
    Vorbis,
    /// FLAC.
    Flac,
    /// Apple Lossless.
    Alac,
    /// Uncompressed PCM.
    Pcm,
    /// Identified as audio, not identified as one of the above.
    Other,
}

impl VideoCodec {
    /// Whether `container` may carry this codec without re-encoding.
    #[must_use]
    pub const fn carried_by(self, container: FormatId) -> bool {
        use FormatId as F;
        match container {
            // Matroska is a general-purpose container and carries every codec
            // in this enum -- but not `Other`, because "we could not identify
            // it" is not the same claim as "anything fits".
            F::Mkv => !matches!(self, Self::Other),
            F::Mp4 => matches!(self, Self::H264 | Self::H265 | Self::Av1),
            // WebM is a strict Matroska subset. Putting H.264 in one produces a
            // file that muxes and does not play.
            F::Webm => matches!(self, Self::Vp8 | Self::Vp9 | Self::Av1),
            _ => false,
        }
    }
}

impl AudioCodec {
    /// Whether `container` may carry this codec without re-encoding.
    #[must_use]
    pub const fn carried_by(self, container: FormatId) -> bool {
        use FormatId as F;
        match container {
            F::Mkv | F::Mka => !matches!(self, Self::Other),
            // Opus in MP4 is standardised and still unevenly supported by
            // players. Omitted deliberately: the cost of leaving it out is a
            // re-encode the user is told about, and the cost of putting it in
            // is a silent unplayable file.
            F::Mp4 => matches!(self, Self::Aac | Self::Mp3 | Self::Alac),
            F::Webm => matches!(self, Self::Opus | Self::Vorbis),
            _ => false,
        }
    }
}

/// Whether every stream present can move into `container` untouched.
///
/// A missing stream is not an obstacle: a video-only file remuxes into any
/// container that takes its video codec. `None` for both is `true` — a file
/// with no streams we identified has nothing that would need re-encoding, and
/// the route's other requirements still have to hold.
#[must_use]
pub const fn streams_carryable(
    container: FormatId,
    video: Option<VideoCodec>,
    audio: Option<AudioCodec>,
) -> bool {
    let v = match video {
        Some(c) => c.carried_by(container),
        None => true,
    };
    let a = match audio {
        Some(c) => c.carried_by(container),
        None => true,
    };
    v && a
}

/// Whether the AUDIO side alone can move into `container` untouched.
///
/// This is the audio-extraction requirement, and it deliberately IGNORES the
/// video codec: extraction drops the video track entirely, so asking whether
/// H.264 "fits" in an audio-only Matroska would be a category error — the
/// same shape as the removed `stream_copyable` boolean, asked of a predicate
/// that only makes sense once the destination is known.
#[must_use]
pub const fn audio_carryable(container: FormatId, audio: Option<AudioCodec>) -> bool {
    match audio {
        Some(c) => c.carried_by(container),
        // No audio track means extraction has nothing to extract. Refusing is
        // honest: a "conversion" producing zero tracks is not a conversion.
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::FormatId as F;

    /// **The flagship, decided correctly.**
    ///
    /// `01` §154 promises `MKV → MP4` completes in seconds and is bit-identical.
    /// That is true for the common case — H.264 video with AAC audio — and this
    /// is the check that selects it.
    #[test]
    fn h264_aac_mkv_remuxes_into_mp4() {
        assert!(streams_carryable(
            F::Mp4,
            Some(VideoCodec::H264),
            Some(AudioCodec::Aac)
        ));
    }

    /// **The case that must NOT be called lossless.**
    ///
    /// VP9 does not go into MP4 in any form worth shipping, so the Class A row
    /// has to fail and the Class B transcode row has to win — which is the
    /// user being told fidelity is lost, rather than being handed a file that
    /// will not play.
    #[test]
    fn vp9_in_mkv_does_not_remux_into_mp4() {
        assert!(!streams_carryable(
            F::Mp4,
            Some(VideoCodec::Vp9),
            Some(AudioCodec::Opus)
        ));
    }

    /// WebM is a strict subset, and the table treats it that way.
    ///
    /// H.264 in a `.webm` muxes cleanly and plays nowhere.
    #[test]
    fn webm_refuses_everything_outside_its_subset() {
        assert!(!VideoCodec::H264.carried_by(F::Webm));
        assert!(!VideoCodec::H265.carried_by(F::Webm));
        assert!(!AudioCodec::Aac.carried_by(F::Webm));
        assert!(!AudioCodec::Mp3.carried_by(F::Webm));
        // The control: what WebM does carry.
        assert!(VideoCodec::Vp9.carried_by(F::Webm));
        assert!(VideoCodec::Av1.carried_by(F::Webm));
        assert!(AudioCodec::Opus.carried_by(F::Webm));
    }

    /// An unidentified codec is never carryable — including into Matroska.
    ///
    /// "Matroska carries anything" is true of codecs that exist and false as a
    /// statement about a stream nobody identified. Promising a lossless copy of
    /// something unrecognised is the one direction with no recovery.
    #[test]
    fn an_unidentified_stream_is_never_carryable() {
        for container in [F::Mkv, F::Mp4, F::Webm] {
            assert!(
                !VideoCodec::Other.carried_by(container),
                "unidentified video was accepted by {container}"
            );
            assert!(
                !AudioCodec::Other.carried_by(container),
                "unidentified audio was accepted by {container}"
            );
        }
    }

    /// A missing stream is not an obstacle.
    #[test]
    fn a_file_without_audio_still_remuxes() {
        assert!(streams_carryable(F::Mp4, Some(VideoCodec::H264), None));
        assert!(streams_carryable(F::Webm, None, Some(AudioCodec::Opus)));
        assert!(streams_carryable(F::Mp4, None, None));
    }

    /// A non-container destination carries nothing.
    ///
    /// The fallback arm is `false`, so adding a container to the format table
    /// without adding it here refuses remuxes into it — which costs a re-encode
    /// and never a broken file.
    #[test]
    fn a_non_container_format_carries_nothing() {
        assert!(!VideoCodec::H264.carried_by(F::Png));
        assert!(!AudioCodec::Aac.carried_by(F::Jpeg));
        assert!(!streams_carryable(
            F::Png,
            Some(VideoCodec::H264),
            Some(AudioCodec::Aac)
        ));
    }
}
