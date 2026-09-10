//! The host↔worker wire format: framing, and the bound that has to precede it.
//!
//! Private to one job, one client, and unversioned as a public contract — but
//! it is a real format, so it is specified rather than assumed.
//!
//! ```text
//! host                                            worker (confined)
//!   spawn EngineBin::Images with Argv
//!     fd 3 = requests (r)   fd 4 = responses (w)
//!     fd 5 = job-dir fd     fd 6.. = pre-opened read-only input fds
//!   ── Probe { sniff }              ──▶
//!   ◀── Properties(..) | Failed{..} ───
//!   ── Run { step, inputs, outputs }──▶
//!   ◀── Progress { done, total }    ───   (repeated, optional)
//!   ◀── Wrote(OutputName)           ───   (repeated)
//!   ◀── Done | Failed { .. }        ───
//!   finish(): copy out, remove job dir
//! ```
//!
//! # Why the bound lives here and not in the decoder
//!
//! A length prefix of `u32::MAX` is a **correct decode**. There is nothing
//! malformed about it: the parser reads four bytes, gets a number, and does
//! exactly what it was asked. A fuzzer looking for panics will never flag it —
//! 7.4 million cases over this project's other parser found nothing of the kind
//! (spike S29), and it could not have, because there is no bug in the parsing.
//!
//! The bug is the allocation that follows. So the check happens **before** any
//! buffer is reserved, on the session rather than in the decoder, and it is the
//! reason this module exists separately from the value types.
//!
//! This module deliberately contains **no `Deserialize`**. Framing is bytes in,
//! bytes out; turning those bytes into values is `openconvert_core::wire`'s job,
//! where every conversion clamps. A lint asserts the split.

use std::io::{self, Read, Write};

/// Wire protocol version, sent in every frame header.
///
/// Bumped when the *framing* changes, not when a message is added. A worker
/// that speaks a different framing version cannot be talked to at all, and
/// finding that out on frame one is better than on message forty.
pub const PROTOCOL_VERSION: u16 = 1;

/// The largest single frame either side will read.
///
/// **1 MiB.** A probe response describing an image is a few hundred bytes; the
/// largest legitimate *message* is a `Failed` carrying an engine's error text.
///
/// # Content is chunked, not exempt
///
/// This comment used to say content never crosses the protocol at all, because
/// it "moves through pre-opened descriptors". The descriptor path is `03` §5.2's
/// design and it is not built. What was built instead put a `Vec<u8>` inside the
/// `Run` message — and serde encodes a byte vector as an array of decimal
/// numbers, so a 4 MB PNG became a **13 MB frame** against this 1 MiB limit.
/// Every image over roughly 300 KB was refused by the sandboxed path while
/// `Limits` advertised a 4 GiB output and a 256 Mpx decode.
///
/// So content crosses as a declared length followed by raw frames, each at most
/// this size. The bound is unchanged and still the point: a peer cannot make
/// this side reserve a buffer from a number it chose. Descriptors remain the
/// better answer for a host that streams — this one reads whole files into
/// memory in `execute()`, so they would add a job directory, ACLs and a sweep
/// to deliver a benefit the shell cannot yet take.
pub const MAX_FRAME_BYTES: u32 = 1 << 20;

/// The most frames a worker may send in response to one request.
///
/// `Progress` is repeatable and `Wrote` is repeatable, so "how many" is not
/// structurally bounded by the message grammar. An engine that emits progress
/// in a tight loop is a denial of service against the host's own event queue,
/// whether it means to be or not.
pub const MAX_FRAMES_PER_REQUEST: usize = 100_000;

/// Frame header: version, then length. Six bytes, big-endian.
const HEADER_LEN: usize = 6;

/// Why a frame could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The peer announced a frame larger than [`MAX_FRAME_BYTES`].
    ///
    /// **Refused before allocating.** This is the whole point of the module:
    /// the number is rejected while it is still a number, not after it has
    /// become a `Vec::with_capacity`.
    #[error(
        "frame of {announced} bytes exceeds the {MAX_FRAME_BYTES}-byte limit; \
         refused before allocating"
    )]
    FrameTooLarge {
        /// What the peer claimed.
        announced: u32,
    },
    /// The peer declared more content than this side will hold.
    ///
    /// The same rule as `FrameTooLarge`, one level up: a length is refused
    /// while it is still a number.
    #[error(
        "content of {declared} bytes exceeds this side's {cap}-byte ceiling; \
         refused before allocating"
    )]
    ContentTooLarge {
        /// What they claimed.
        declared: u64,
        /// What we allow.
        cap: u64,
    },
    /// The peer sent more content than it declared.
    ///
    /// Refused rather than truncated. Truncating leaves the extra bytes in the
    /// pipe, where they become the next message — so a desync would be repaired
    /// into a protocol confusion instead of an error.
    #[error("peer declared {declared} bytes of content and sent at least {got}")]
    ContentOverrun {
        /// What they claimed.
        declared: u64,
        /// What arrived.
        got: u64,
    },
    /// The peer sent more frames than one request may answer with.
    #[error("worker sent more than {MAX_FRAMES_PER_REQUEST} frames for one request")]
    TooManyFrames,
    /// The peer speaks a different framing version.
    #[error("protocol version {theirs}, expected {PROTOCOL_VERSION}")]
    VersionMismatch {
        /// What arrived.
        theirs: u16,
    },
    /// A frame of zero bytes, which carries nothing and means nothing.
    #[error("zero-length frame")]
    EmptyFrame,
    /// Underlying I/O, including a peer that closed mid-frame.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Write one frame.
///
/// # Errors
///
/// [`FrameError::FrameTooLarge`] if the payload exceeds the limit — checked on
/// our own output too, so a bug on this side cannot produce a frame the far
/// side is obliged to refuse.
pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge { announced: len });
    }
    if payload.is_empty() {
        return Err(FrameError::EmptyFrame);
    }
    w.write_all(&PROTOCOL_VERSION.to_be_bytes())?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

/// Read one frame, refusing an oversized one **before allocating**.
///
/// # Errors
///
/// See [`FrameError`].
pub fn read_frame<R: Read>(r: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut header = [0_u8; HEADER_LEN];
    r.read_exact(&mut header)?;

    let version = u16::from_be_bytes([header[0], header[1]]);
    if version != PROTOCOL_VERSION {
        return Err(FrameError::VersionMismatch { theirs: version });
    }

    let announced = u32::from_be_bytes([header[2], header[3], header[4], header[5]]);

    // THE LINE THIS MODULE EXISTS FOR.
    //
    // Everything above is four bytes on the stack. Everything below allocates.
    // A `u32::MAX` here is a perfectly well-formed number that would reserve
    // four gigabytes, and no parser is going to object -- it was asked for four
    // gigabytes and it is obliging.
    if announced > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge { announced });
    }
    if announced == 0 {
        return Err(FrameError::EmptyFrame);
    }

    let mut payload = vec![0_u8; announced as usize];
    r.read_exact(&mut payload)?;
    Ok(payload)
}

/// Read a bounded sequence of frames — one request's worth of replies.
///
/// `Progress` and `Wrote` repeat, so the grammar does not bound the count. This
/// does.
///
/// # Errors
///
/// [`FrameError::TooManyFrames`] once the budget is spent, plus anything
/// [`read_frame`] can return.
pub fn read_frames<R: Read>(
    r: &mut R,
    mut is_terminal: impl FnMut(&[u8]) -> bool,
) -> Result<Vec<Vec<u8>>, FrameError> {
    let mut out = Vec::new();
    loop {
        if out.len() >= MAX_FRAMES_PER_REQUEST {
            return Err(FrameError::TooManyFrames);
        }
        let frame = read_frame(r)?;
        let done = is_terminal(&frame);
        out.push(frame);
        if done {
            return Ok(out);
        }
    }
}

// ---------------------------------------------------------------------------
// Content transfer
//
// A `Run` request names a length; the bytes follow as raw frames. Nothing about
// the frame bound changes -- each chunk is still at most MAX_FRAME_BYTES, still
// refused before allocating -- but a conversion is no longer capped at one
// frame's worth of file.
//
// THE DEFECT THIS CLOSES. `Request::Run` carried a `Vec<u8>` inside the JSON,
// which meant every file crossed as one frame, and serde encodes a byte vector
// as an array of decimal numbers -- so a 4 MB PNG became a 13 MB frame against
// a 1 MiB limit. Every image over roughly 300 KB failed, while `Limits`
// advertised a 4 GiB output and a 256 Mpx decode. The module header opposite
// already said content does not travel this way; the host did it anyway.
//
// Why not simply raise MAX_FRAME_BYTES: the bound exists so a hostile peer
// cannot make this side allocate an arbitrary buffer from a number it chose.
// Raising it to fit the largest legal file would trade a real control for a
// formality, which is exactly what the header warns against.
// ---------------------------------------------------------------------------

/// Write `bytes` as a sequence of content frames.
///
/// # Errors
///
/// Any underlying I/O failure. Never [`FrameError::FrameTooLarge`]: chunking is
/// what this function is for.
pub fn write_content<W: Write>(w: &mut W, bytes: &[u8]) -> Result<(), FrameError> {
    // An empty payload is not writable as a frame (`EmptyFrame`), and an empty
    // file is a legitimate input, so zero chunks is the encoding for zero bytes
    // -- which the reader recovers from the declared length, not from a
    // terminator. A terminator would be a second way to end a stream, and two
    // ways to end a stream is how a desync becomes a hang.
    for chunk in bytes.chunks(MAX_FRAME_BYTES as usize) {
        write_frame(w, chunk)?;
    }
    Ok(())
}

/// Read exactly `declared` bytes of content, refusing an oversized claim
/// **before allocating**.
///
/// `cap` is the reader's own ceiling — `Limits::output_bytes` on the host,
/// the memory limit in a worker. The declared length comes from the far side
/// and is therefore attacker-controlled whenever the far side is: a compromised
/// engine answering `output_len: u64::MAX` must cost this process an error, not
/// an allocation.
///
/// # Errors
///
/// See [`FrameError`].
pub fn read_content<R: Read>(r: &mut R, declared: u64, cap: u64) -> Result<Vec<u8>, FrameError> {
    if declared > cap {
        return Err(FrameError::ContentTooLarge { declared, cap });
    }
    // Only reached once `declared` is known to be within a limit this side
    // chose, so the reservation is bounded by our own policy rather than by
    // their number.
    let want =
        usize::try_from(declared).map_err(|_| FrameError::ContentTooLarge { declared, cap })?;
    let mut out = Vec::with_capacity(want);
    while out.len() < want {
        let frame = read_frame(r)?;
        out.extend_from_slice(&frame);
        // A peer that sends more than it declared is desynchronising the
        // stream, deliberately or otherwise. Refused rather than truncated:
        // truncating leaves the extra bytes in the pipe, where they become the
        // next message.
        if out.len() > want {
            return Err(FrameError::ContentOverrun {
                declared,
                got: out.len() as u64,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_frame_round_trips() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").expect("write");
        let got = read_frame(&mut Cursor::new(buf)).expect("read");
        assert_eq!(got, b"hello");
    }

    /// **The test this module exists for.**
    ///
    /// A header announcing `u32::MAX` is refused, and refused *cheaply* — the
    /// reader has seen six bytes and there is no four-gigabyte payload behind
    /// them. If the bound were inside the decoder instead, this test would need
    /// four gigabytes to fail.
    #[test]
    fn an_oversized_announcement_is_refused_before_allocating() {
        let mut header = Vec::new();
        header.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        // Deliberately no payload. A reader that allocated first would block or
        // die here; one that checks first returns an error immediately.
        let err = read_frame(&mut Cursor::new(header)).expect_err("must refuse");
        match err {
            FrameError::FrameTooLarge { announced } => assert_eq!(announced, u32::MAX),
            other => panic!("wrong error, and the allocation may have happened: {other}"),
        }
    }

    /// The boundary, both sides.
    #[test]
    fn the_frame_limit_is_exact() {
        let mut at_limit = Vec::new();
        at_limit.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        at_limit.extend_from_slice(&MAX_FRAME_BYTES.to_be_bytes());
        at_limit.extend(std::iter::repeat_n(0_u8, MAX_FRAME_BYTES as usize));
        assert!(
            read_frame(&mut Cursor::new(at_limit)).is_ok(),
            "a frame of exactly the limit was refused"
        );

        let mut over = Vec::new();
        over.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        over.extend_from_slice(&(MAX_FRAME_BYTES + 1).to_be_bytes());
        assert!(read_frame(&mut Cursor::new(over)).is_err());
    }

    /// Our own writer is bounded too.
    ///
    /// A bug on this side must not produce a frame the far side is obliged to
    /// refuse — that turns our defect into their protocol error, which is the
    /// hardest kind to diagnose.
    #[test]
    fn we_refuse_to_send_what_we_would_refuse_to_receive() {
        let oversized = vec![0_u8; MAX_FRAME_BYTES as usize + 1];
        let mut sink = Vec::new();
        assert!(matches!(
            write_frame(&mut sink, &oversized),
            Err(FrameError::FrameTooLarge { .. })
        ));
        assert!(sink.is_empty(), "a refused frame still wrote bytes");
    }

    #[test]
    fn a_version_mismatch_is_caught_on_the_first_frame() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(PROTOCOL_VERSION + 1).to_be_bytes());
        buf.extend_from_slice(&4_u32.to_be_bytes());
        buf.extend_from_slice(b"data");
        match read_frame(&mut Cursor::new(buf)) {
            Err(FrameError::VersionMismatch { theirs }) => {
                assert_eq!(theirs, PROTOCOL_VERSION + 1);
            }
            other => panic!("wrong result: {other:?}"),
        }
    }

    #[test]
    fn zero_length_frames_are_refused_in_both_directions() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        buf.extend_from_slice(&0_u32.to_be_bytes());
        assert!(matches!(
            read_frame(&mut Cursor::new(buf)),
            Err(FrameError::EmptyFrame)
        ));
        assert!(matches!(
            write_frame(&mut Vec::new(), b""),
            Err(FrameError::EmptyFrame)
        ));
    }

    /// A truncated frame is an error, not a short read.
    ///
    /// A worker killed by its Job Object memory cap dies mid-frame — measured,
    /// and it does so *without* sending `Failed` (spike S14). The host has to
    /// tell "the peer stopped talking" from "the peer said nothing", because
    /// only one of those has a message to show the user.
    #[test]
    fn a_truncated_frame_is_an_error() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        buf.extend_from_slice(&100_u32.to_be_bytes());
        buf.extend_from_slice(b"only ten b");
        assert!(matches!(
            read_frame(&mut Cursor::new(buf)),
            Err(FrameError::Io(_))
        ));
    }

    /// A repeated message type cannot run forever.
    #[test]
    fn a_flood_of_frames_is_bounded() {
        let mut buf = Vec::new();
        for _ in 0..(MAX_FRAMES_PER_REQUEST + 10) {
            write_frame(&mut buf, b"progress").expect("write");
        }
        // Never terminal: exactly the shape an engine emitting progress in a
        // tight loop produces.
        let err = read_frames(&mut Cursor::new(buf), |_| false).expect_err("must stop");
        assert!(matches!(err, FrameError::TooManyFrames));
    }

    /// The control: a normal reply sequence terminates on its own.
    ///
    /// Without it, the bound above is satisfied by a reader that always refuses.
    #[test]
    fn a_normal_sequence_terminates() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"progress").expect("w");
        write_frame(&mut buf, b"progress").expect("w");
        write_frame(&mut buf, b"done").expect("w");
        let frames = read_frames(&mut Cursor::new(buf), |f| f == b"done").expect("read");
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2], b"done");
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    /// Content survives a round trip at every awkward size.
    ///
    /// The boundaries are where chunking breaks: exactly one frame, one byte
    /// over, and an exact multiple — where an off-by-one in the loop emits a
    /// trailing empty frame, which `write_frame` refuses outright.
    #[test]
    fn content_round_trips_across_chunk_boundaries() {
        let max = MAX_FRAME_BYTES as usize;
        for len in [0, 1, max - 1, max, max + 1, 2 * max, 2 * max + 7] {
            let src: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut wire = Vec::new();
            write_content(&mut wire, &src).expect("write");
            let back =
                read_content(&mut std::io::Cursor::new(wire), len as u64, u64::MAX).expect("read");
            assert_eq!(back, src, "round trip failed at {len} bytes");
        }
    }

    /// **A declared length beyond our ceiling is refused before allocating.**
    ///
    /// The whole module's reason for existing, one level up from
    /// `FrameTooLarge`. A compromised worker answering `u64::MAX` must cost the
    /// host an error, not sixteen exabytes of `with_capacity`.
    #[test]
    fn an_absurd_declared_length_is_refused_without_allocating() {
        let mut empty = std::io::Cursor::new(Vec::new());
        let err = read_content(&mut empty, u64::MAX, 4 << 30).expect_err("must refuse");
        assert!(
            matches!(err, FrameError::ContentTooLarge { .. }),
            "wrong error: {err}"
        );
    }

    /// The control: a length just inside the ceiling is accepted.
    ///
    /// Without it, the test above is satisfied by a `read_content` that refuses
    /// everything.
    #[test]
    fn a_length_within_the_ceiling_is_accepted() {
        let src = vec![9_u8; 100];
        let mut wire = Vec::new();
        write_content(&mut wire, &src).expect("write");
        let back =
            read_content(&mut std::io::Cursor::new(wire), 100, 100).expect("exactly at the cap");
        assert_eq!(back, src);
    }

    /// A peer that sends more than it declared is refused, not truncated.
    ///
    /// Truncating would leave the surplus in the pipe, where the next
    /// `read_frame` would parse attacker-chosen bytes as a message — turning a
    /// desync into a protocol confusion instead of an error.
    #[test]
    fn sending_more_than_declared_is_refused() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &[1, 2, 3, 4, 5, 6, 7, 8]).expect("write");
        let err =
            read_content(&mut std::io::Cursor::new(wire), 4, 1 << 20).expect_err("must refuse");
        assert!(
            matches!(
                err,
                FrameError::ContentOverrun {
                    declared: 4,
                    got: 8
                }
            ),
            "wrong error: {err}"
        );
    }

    /// Zero bytes is zero frames, and reads back as empty.
    ///
    /// An empty file is a legitimate input, and `write_frame` refuses an empty
    /// payload — so the encoding for "nothing" has to be "no frames", recovered
    /// from the declared length rather than from a terminator. A terminator
    /// would be a second way to end a stream, and two ways to end a stream is
    /// how a desync becomes a hang.
    #[test]
    fn zero_bytes_is_zero_frames() {
        let mut wire = Vec::new();
        write_content(&mut wire, &[]).expect("write");
        assert!(wire.is_empty(), "an empty payload should emit no frames");
        let back = read_content(&mut std::io::Cursor::new(wire), 0, 1 << 20).expect("read");
        assert!(back.is_empty());
    }
}
