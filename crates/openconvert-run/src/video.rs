//! Class B video: the transcode module.
//!
//! # Why this is a module and not an engine
//!
//! `02-FEATURES` §3 is explicit that core ships **no** video transcoder: core
//! handles Class A container work with pure-Rust crates, and transcoding lives
//! in a separately downloaded Video module built on FFmpeg. That is a patent
//! and CVE decision, not a scheduling one — core ships no H.264/HEVC/AAC
//! encoder, and a user who never touches video never carries FFmpeg's CVE
//! stream.
//!
//! # Why a subprocess rather than linked libraries
//!
//! The spec says the module "prefers a system FFmpeg if present", and a system
//! FFmpeg is a *binary*. Linking `libavcodec` would also put LGPL code inside
//! our own address space and bring the relinking obligation with it; running
//! the binary keeps FFmpeg entirely at arm's length, so there is nothing to
//! relink and no LGPL code in anything we distribute.
//!
//! It is also enormously less code than the FFI, over an interface that is far
//! better tested than any binding.
//!
//! # Output is a pipe; input has to be seekable, and that costs a scratch file
//!
//! Every other untrusted parse here gets its bytes on stdin and no filesystem
//! at all. This one cannot: MP4 stores its index (`moov`) wherever the writer
//! chose, and in most files that is AFTER the media. Demuxing then needs to
//! seek backwards, which a pipe cannot do — measured, not assumed: `bbb.mp4`
//! has `mdat` at offset 40 and `moov` at 4,121,299, and FFmpeg reads it
//! perfectly from a file and not at all from `pipe:0`.
//!
//! **The obvious fix would break I13/SR-16.** `exec` is explicit that there is
//! no path to re-open, and that this is what makes "the bytes routed on are
//! the bytes converted" a property of the types rather than a rule to
//! remember. Handing the module the user's original path reintroduces exactly
//! the gap that invariant closes: the file could change between the hash and
//! the transcode, and the receipt would attest to bytes nobody converted.
//!
//! So the ALREADY-ROUTED bytes are written to a private file under the state
//! dir, the module reads that, and it is removed afterwards. The scratch holds
//! the same bytes the handle produced and the receipt hashes, so the invariant
//! survives; the cost is one write and one delete of our own file, in a
//! directory nothing else uses.
//!
//! Output still goes to `pipe:1` — the module never gets write access
//! anywhere — and MP4 output is asked for in its fragmented form so it can
//! stream without seeking either.

use std::io::Write;
use std::path::{Path, PathBuf};

use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;

/// Why a transcode did not happen.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// The module is not installed.
    #[error(
        "video transcoding needs the Video module, which is not installed. \
         Put ffmpeg on PATH, or run `cargo xtask deps --video`."
    )]
    NoModule,
    /// This build does not transcode into that container.
    #[error("this build does not transcode into {0}")]
    UnsupportedTarget(FormatId),
    /// FFmpeg ran and refused.
    #[error("the video module could not convert this file: {0}")]
    Failed(String),
    /// The result outgrew the caller's ceiling.
    #[error("the result exceeded the {limit}-byte output limit")]
    TooLarge {
        /// The ceiling that was passed.
        limit: u64,
    },
    /// Spawning or talking to the module failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Where the module lives, if it is installed.
///
/// `.native/ffmpeg` first — that is what `cargo xtask deps --video` stages and
/// what a developer gets by following the setup. `PATH` second, which is the
/// "prefers a system FFmpeg" the spec asks for; a user who already has one
/// should not be made to download a second.
#[must_use]
pub fn module_path() -> Option<PathBuf> {
    let staged = Path::new(".native")
        .join("ffmpeg")
        .join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
    if staged.is_file() {
        return Some(staged);
    }
    // Beside our own binary, which is where a packaged install would put it.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let beside = dir.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        if beside.is_file() {
            return Some(beside);
        }
    }
    which_on_path("ffmpeg")
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(&exe))
            .find(|p| p.is_file())
    })
}

/// Whether the module is available at all.
#[must_use]
pub fn available() -> bool {
    module_path().is_some()
}

/// The encoders and muxer for one destination.
///
/// AV1 and Opus by default, which is what an LGPL FFmpeg can encode and what
/// `02-FEATURES` §3.2 names. **No H.264 encoder**: that needs x264 (GPL) or a
/// hardware encoder, and shipping one is the patent posture this project
/// deliberately declines.
struct Recipe {
    /// FFmpeg's muxer name.
    muxer: &'static str,
    video: &'static str,
    audio: &'static str,
    /// Flags the container needs to be writable to a pipe.
    extra: &'static [&'static str],
}

fn recipe(to: FormatId) -> Option<Recipe> {
    match to {
        FormatId::Webm => Some(Recipe {
            muxer: "webm",
            video: "libsvtav1",
            audio: "libopus",
            extra: &[],
        }),
        FormatId::Mkv => Some(Recipe {
            muxer: "matroska",
            video: "libsvtav1",
            audio: "libopus",
            extra: &[],
        }),
        FormatId::Mp4 => Some(Recipe {
            muxer: "mp4",
            video: "libsvtav1",
            // AAC rather than Opus: Opus-in-MP4 is standardised and poorly
            // supported, and the codec table here already refuses to CARRY it
            // for that reason. Writing one would contradict that.
            audio: "aac",
            // MP4 normally needs a seekable output so the index can be
            // rewritten at the end. Fragmenting makes it streamable, which is
            // what lets it go to a pipe at all.
            extra: &["-movflags", "+frag_keyframe+empty_moov+default_base_moof"],
        }),
        _ => None,
    }
}

/// Transcode `bytes` into `to`, re-encoding both streams.
///
/// # Errors
///
/// [`VideoError`] — module absent, unsupported destination, a refusal from
/// FFmpeg, or a result past the output ceiling.
pub fn transcode(bytes: &[u8], to: FormatId, limits: &Limits) -> Result<Vec<u8>, VideoError> {
    let program = module_path().ok_or(VideoError::NoModule)?;
    let recipe = recipe(to).ok_or(VideoError::UnsupportedTarget(to))?;

    let scratch = Scratch::hold(bytes)?;
    let input_arg = scratch
        .path
        .to_str()
        .ok_or_else(|| VideoError::Failed("the work path is not valid UTF-8".into()))?;

    let mut args: Vec<&str> = vec![
        "-hide_banner",
        // Never prompt. Without this a malformed input that makes FFmpeg ask
        // a question would hang the conversion until the wall clock kills it.
        "-nostdin",
        "-loglevel",
        "error",
        "-i",
        input_arg,
        "-c:v",
        recipe.video,
        "-c:a",
        recipe.audio,
        // Subtitles and data streams are dropped rather than re-encoded: this
        // is disclosed on the receipt, and carrying a stream nothing here
        // understands is what the container rules already refuse.
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
    ];
    args.extend_from_slice(recipe.extra);
    args.extend_from_slice(&["-f", recipe.muxer, "pipe:1"]);

    let out = run_module(&program, &args, &scratch.path, limits)?;
    if out.is_empty() {
        return Err(VideoError::Failed(
            "the module produced no output".to_string(),
        ));
    }
    Ok(out)
}

/// Spawn the module confined, feed it the input, read the result.
///
/// Windows only, like every other native path in this build. Elsewhere the
/// module reports unavailable and this is never reached.
#[cfg(windows)]
fn run_module(
    program: &Path,
    args: &[&str],
    input: &Path,
    limits: &Limits,
) -> Result<Vec<u8>, VideoError> {
    // `Read` is used by this function and by nothing else in the file, so it is
    // imported here rather than at the top: at the top it is an unused import
    // on every platform that is not Windows, and CI runs clippy with
    // `-D warnings`, so it failed the Linux job outright.
    use std::io::Read;

    // Confined exactly like a worker. FFmpeg is the largest parser anywhere
    // near this product and it is being handed a file we do not trust, so it
    // gets the sandbox every other untrusted parse gets -- not less because it
    // happens to be someone else's binary.
    let container = openconvert_os::appcontainer::AppContainer::create("tx-video")
        .map_err(|e| VideoError::Failed(format!("could not confine the video module: {e}")))?;
    container.grant_execute(program).map_err(|e| {
        VideoError::Failed(format!("could not grant the module its own binary: {e}"))
    })?;
    // AND THE DIRECTORY, because this is a SHARED build: ffmpeg.exe loads
    // avcodec, avformat, avutil, swresample and swscale from beside itself.
    // Granting only the executable produced STATUS_DLL_NOT_FOUND
    // (0xC0000135) -- a process that starts and dies before `main` with no
    // message of its own.
    if let Some(dir) = program.parent() {
        container.acl_directory(dir).map_err(|e| {
            VideoError::Failed(format!("could not grant the module its own directory: {e}"))
        })?;
    }
    // READ ON EXACTLY ONE FILE: the one the user named. `grant_execute` sets
    // FILE_GENERIC_READ alongside execute, which is what this needs. No
    // directory, no write, nothing else.
    container.grant_execute(input).map_err(|e| {
        VideoError::Failed(format!("could not grant the module the input file: {e}"))
    })?;

    let mut child =
        openconvert_os::spawn_windows::spawn_piped_in_container(program, args, &container)
            .map_err(|e| VideoError::Failed(format!("could not start the video module: {e}")))?;

    // Nothing is written to the module: it reads the file itself. Closing
    // stdin immediately is what tells it so -- left open, `-nostdin` still
    // leaves a handle nobody will ever write to.
    child.close_stdin();

    let cap = limits.output_bytes;
    let mut out = Vec::new();
    {
        let stdout = child
            .stdout()
            .ok_or_else(|| VideoError::Failed("the module exposed no output pipe".into()))?;
        // One byte past the cap is enough to know it was exceeded; reading
        // further would mean holding the whole oversized result to discover it
        // was oversized.
        let mut limited = stdout.take(cap.saturating_add(1));
        limited.read_to_end(&mut out)?;
    }
    let code = child.wait()?;

    if out.len() as u64 > cap {
        return Err(VideoError::TooLarge { limit: cap });
    }
    if code != 0 && out.is_empty() {
        return Err(VideoError::Failed(format!(
            "the module exited with status {code}; its own message is on stderr"
        )));
    }
    Ok(out)
}

/// The same signature, on every platform that is not Windows.
///
/// `_input` was `&[u8]` here and `&Path` above, so this file compiled on
/// Windows and nowhere else -- the whole workspace failed to build on Linux at
/// this one line. A `#[cfg]`-ed stub is a second implementation of a signature,
/// and nothing checks that two implementations agree until something tries to
/// compile the other one.
#[cfg(not(windows))]
fn run_module(
    _program: &Path,
    _args: &[&str],
    _input: &Path,
    _limits: &Limits,
) -> Result<Vec<u8>, VideoError> {
    Err(VideoError::NoModule)
}

/// The routed bytes, on disk for exactly as long as the module needs them.
///
/// Removed on every path out, including the error ones, which is the whole
/// reason this is a type rather than two statements.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn hold(bytes: &[u8]) -> Result<Self, VideoError> {
        let dir = crate::state::paths::work_dir();
        std::fs::create_dir_all(&dir)?;
        // Named by process and a counter: two conversions running at once must
        // not collide.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("video-{}-{n}.bin", std::process::id()));
        // `create_new`: never open something already there. The no-overwrite
        // rule applies to our own scratch as much as to a user's file.
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(Self { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // The one deletion in this module, and it only ever names a file this
        // type created moments earlier under a directory nothing else uses.
        let _ = std::fs::remove_file(&self.path); // openconvert-lint: allow -- removes only the scratch this type created
    }
}
