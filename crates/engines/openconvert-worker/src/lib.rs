//! The shared worker-side runtime.
//!
//! Every engine binary links this and implements one trait. What the runtime
//! owns is the part that must be identical across all four workers: reading
//! framed requests, applying confinement to itself, reading back what actually
//! engaged, and answering.
//!
//! # Why the workers are separate binaries at all
//!
//! `EngineBin` is a closed enum of *programs*, and a closed enum of programs is
//! only meaningful if the programs exist. libvips, libheif, libavif, libjxl,
//! libraw and lcms2 are **libraries**; pdfium has no usable CLI; and none of
//! them emit the structured `Properties` two-phase detection needs. Depending
//! on stock upstream CLIs would mean shipping ten third-party executables we do
//! not control and parsing their human-readable output.
//!
//! So we write four small binaries instead — grouped by engine family, so the
//! count of things to sign, notarize and confine stays at four rather than ten.
//!
//! **FFI `unsafe` lives in the engine crates, never here and never in the
//! boundary.** That is the point of the split: `unsafe` outside the trust
//! boundary, in a process that is already confined.
//!
//! # The worker confines itself — through [`run_real`], never through [`serve`]
//!
//! Not the host. A worker applies its own confinement after `main` starts and
//! **before** it reads the first request — so the window between "process
//! exists" and "process is confined" contains no attacker-supplied bytes.
//!
//! It then *reads back* what engaged and reports it. The host records that,
//! never the request, because measurement found that error running in both
//! directions (S17b, S20b).
//!
//! **Why two entry points.** Both mechanisms the worker applies on Linux are
//! one-way for the process that applies them, and cargo runs a crate's tests
//! in one process on many threads. If `serve()` confined, the first unit test
//! to drive it would deny filesystem access to every sibling test mid-run —
//! and the failure would look like dozens of unrelated breakages. So:
//!
//! - [`serve`] is the **protocol loop only**, safe to run in-process. It
//!   performs whatever *non-destructive* read-back the platform offers, which
//!   is what lets the Windows handshake be exercised without spawning anything.
//! - [`run_real`] is what a worker binary's `main` calls: it applies
//!   confinement — fatally refusing to continue if it cannot — and then serves
//!   standard streams until the host closes them.

pub mod net;

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// What the host asked for.
///
/// **Deliberately small, and content is not in it.** A message declares how
/// many bytes follow; the bytes then arrive as raw frames. Putting them inside
/// the message is what capped the sandboxed path at one frame per file — see
/// `MAX_FRAME_BYTES`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Report what confinement actually engaged, and who we are.
    ///
    /// The first message on every session. It costs a round trip and buys the
    /// receipt its evidence.
    Hello,
    /// Extract format-specific facts. Phase 2 of detection.
    Probe {
        /// How many bytes of content follow this frame.
        input_len: u64,
    },
    /// Do the conversion.
    Run {
        /// How many bytes of content follow this frame.
        ///
        /// The bytes themselves arrive as raw content frames, not inside this
        /// message. A `Vec<u8>` here would be serialised as an array of decimal
        /// numbers -- a 4 MB PNG became a 13 MB frame against a 1 MiB limit --
        /// so every file over roughly 300 KB was refused while `Limits`
        /// advertised a 4 GiB output.
        input_len: u64,
        /// Target format, by its table name.
        to: String,
        /// Every ceiling this engine may need, already clamped by the host.
        limits: RunLimits,
        /// Operation parameters, when the plan's step carried any (a page
        /// render's page index is the only one today). Defaulted so messages
        /// without it still parse; engines that understand none of it take
        /// the default [`Engine::run`] path unchanged.
        #[serde(default)]
        params: Vec<(String, String)>,
        /// The length of each MODEL artifact following the input, in order.
        ///
        /// # Why the weights travel on the pipe
        ///
        /// A worker has no filesystem. On Linux Landlock denies all of it,
        /// and that is stated one screen down as the reason the protocol
        /// needs none. Handing `oc-ai` a path to `<state>/models/<hash>.bin`
        /// would have meant punching a hole in exactly the confinement this
        /// design earns its receipts from -- and the AI worker is the last
        /// one that should get an exemption, since its input is a file the
        /// user was told we would not trust.
        ///
        /// So the host reads the verified artifacts and sends them, the same
        /// way it sends the file being converted, in that many more content
        /// frames after the input's.
        ///
        /// # Why a list and not one
        ///
        /// A "model" is frequently several files. PaddleOCR is a detection
        /// network, a recognition network and a character dictionary; Whisper
        /// is an encoder, a decoder and a tokeniser. Pinning and shipping them
        /// as one blob would mean inventing a container format and a hash over
        /// it, when the registry already pins artifacts individually — so the
        /// step names the artifacts it needs and they arrive in that order.
        #[serde(default)]
        model_lens: Vec<u64>,
        /// The length of each ADDITIONAL INPUT following the models, in order.
        ///
        /// Separate from `model_lens` on purpose. The framing is identical --
        /// a length here, a content frame there -- but the meaning is not, and
        /// a field called `model_lens` carrying a second PDF would be a lie in
        /// the one place a reader goes to learn what crosses the boundary.
        ///
        /// Merging documents is the operation that needs this: N files in, one
        /// out. Everything else sends none and is unchanged.
        #[serde(default)]
        extra_lens: Vec<u64>,
    },
}

/// The ceilings a worker runs under.
///
/// # Why a struct and not the two numbers it started as
///
/// `Run` carried `decode_pixels` and `memory_bytes`, which is the whole limit
/// vocabulary of an image engine and none of an archive one. `oc-archive`
/// enforces SR-5's bomb caps -- depth, entry count, total extracted bytes --
/// and a protocol that cannot express them is a protocol that cannot carry the
/// archive worker's entire security story.
///
/// The `Engine` trait was deliberately deferred until a second implementation
/// existed, so its shape would be known rather than imagined. This is that
/// shape: an engine takes the ceilings it needs and ignores the rest, and
/// adding a limit is one field rather than a new message.
///
/// **Every value here is already clamped against `LimitCeiling` by the host
/// (I14).** A worker may narrow further and can never widen -- and the host
/// never reads a limit back out of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    /// Pixels a decoder may allocate.
    pub decode_pixels: u64,
    /// Memory ceiling.
    ///
    /// Doubles as the worker's ceiling on `input_len`: a host asking it to hold
    /// more than the Job Object will allow is asking it to be killed mid-read,
    /// which reaches the host as `Crashed` and names nothing.
    pub memory_bytes: u64,
    /// Nesting depth for archives. SR-5.
    pub archive_depth: u8,
    /// Entry count for archives. SR-5.
    pub archive_entries: u32,
    /// Total extracted bytes for archives.
    ///
    /// The control that actually works. `expansion_ratio` cannot separate a
    /// bomb from real data -- a legitimate disk image compresses 1029x and the
    /// bomb 1028x (spike S23) -- so the absolute total is what bounds it.
    pub archive_total_bytes: u64,
    /// Whether this run may use a GPU.
    ///
    /// `#[serde(default = "yes")]` rather than a plain `default`, because
    /// `bool::default()` is **false** and this has to default to true. A host
    /// built before this field existed sends no value, and reading that as
    /// "the user turned the GPU off" would silently disable hardware
    /// acceleration for every older caller.
    ///
    /// Like every other value here it is already clamped by the host, and the
    /// clamp for a boolean is `&&`: a layer may withdraw the GPU and none may
    /// grant it.
    #[serde(default = "yes")]
    pub use_gpu: bool,
}

/// `serde`'s default for [`RunLimits::use_gpu`]. See the field.
const fn yes() -> bool {
    true
}

/// Whether the request in flight may use a GPU.
///
/// # Why a global rather than an argument
///
/// `Engine::run` already receives the limits, so an engine *can* read this
/// from its parameter. The code that builds inference sessions sits several
/// calls below that, in modules shared with examples and tests that have no
/// request at all, so threading a boolean down to two call sites would add the
/// field to a dozen signatures to say the same thing at each one.
///
/// Set once per request, before the engine is called. A worker handles one
/// request at a time, so there is nothing to race with.
static USE_GPU: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Record what the request in flight permits. Called by the runtime.
pub fn set_use_gpu(allowed: bool) {
    USE_GPU.store(allowed, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the request in flight may use a GPU.
#[must_use]
pub fn use_gpu() -> bool {
    USE_GPU.load(std::sync::atomic::Ordering::Relaxed)
}

/// What the worker answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// Completed work units within the current request.
    Progress { completed: u64, total: u64 },
    /// Identity and **read-back** confinement.
    Ready {
        /// Which engine this is.
        engine: String,
        /// Version string for the receipt.
        version: String,
        /// Mitigations that actually engaged, as `(name, engaged)`.
        ///
        /// Read back from the OS in this process, not echoed from the request.
        mitigations: Vec<(String, bool)>,
    },
    /// Facts extracted, in the shape of whatever was probed.
    ///
    /// Carries a [`Probed`] rather than flat image fields, so an archive can
    /// report an entry count without borrowing a field named for animation
    /// frames.
    Properties(Probed),
    /// The conversion succeeded.
    Done {
        /// How many bytes of content follow this frame.
        ///
        /// Chosen by the worker, so the host checks it against
        /// `Limits::output_bytes` before allocating anything (I14).
        output_len: u64,
        /// What was removed, itemised for the receipt.
        removed: Vec<String>,
    },
    /// It failed, structurally.
    ///
    /// **Structured, so `03` §13 can name the file, the step and the engine.**
    /// A worker that answers with a string leaves the host guessing.
    Failed {
        /// Which engine.
        engine: String,
        /// What went wrong, in the user's terms.
        message: String,
        /// Whether the host may retry — a limit is not a crash.
        retryable: bool,
    },
}

/// What a header says, without decoding anything.
///
/// # Why this is an enum and not one flat struct
///
/// It was a struct with `width`, `height`, `has_alpha` and `frames` — the whole
/// vocabulary of an image and none of an archive. `oc-archive` had nowhere to
/// put an entry count, so it rode in `frames` with the dimensions left at zero.
/// That worked, and a field named for animation frames carrying a member count
/// is a fact nobody reading a receipt could interpret.
///
/// `Properties` in the core has had an `Archive` variant all along. The gap was
/// on the wire, which is the half a worker can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Probed {
    /// A raster image, from its header.
    Image {
        /// Pixel width.
        width: u32,
        /// Pixel height.
        height: u32,
        /// Whether the header declares an alpha channel.
        has_alpha: bool,
        /// Frame count, or 0 for "not counted".
        ///
        /// A still format is 1 and known from the header. Counting an animated
        /// format's frames means walking the file, which is a decode, and a
        /// probe that decodes is not a probe.
        frames: u32,
    },
    /// An archive, from its index.
    Archive {
        /// Members named in the index.
        ///
        /// From the index, never from unpacking — a file declaring a million
        /// members costs a million index reads to refuse rather than a million
        /// inflations. SR-5's `archive_entries` cap is checked against it.
        entries: u32,
    },
    /// An audio stream, from its container header.
    Audio {
        /// Sample rate in Hz.
        sample_rate: u32,
        /// Channel count.
        channels: u8,
        /// Duration in milliseconds, or **0 for "not determined"**.
        ///
        /// The same sentinel the video fields use: some containers carry a
        /// total-sample count in their headers and some do not, and a probe
        /// that decodes to find out is not a probe.
        duration_ms: u64,
    },
    /// A PDF document, from its header/trailer.
    Document {
        /// Number of pages reported by the container.
        pages: u32,
        /// Whether the document is password-protected.
        encrypted: bool,
    },
}

/// What a conversion produced.
///
/// # Why `removed` comes from the engine
///
/// The runtime used to attach `"all source metadata (re-encoded)"` to every
/// `Done`. That is true of `oc-images`, which decodes and re-encodes, and false
/// of `oc-archive`, which copies each member through byte for byte — so the
/// first archive repack produced a **Class A receipt claiming a re-encode**.
///
/// A receipt is the product's second pillar. A sentence the runtime asserts on
/// an engine's behalf is a sentence no engine is responsible for keeping true,
/// and SR-11 wants the itemised list rather than a boilerplate line.
pub struct Converted {
    /// The output bytes.
    pub bytes: Vec<u8>,
    /// What this conversion discarded, itemised for the receipt.
    ///
    /// Empty is a legitimate answer and means nothing was lost — which is what
    /// Class A means, and what a lossless repack should say.
    pub removed: Vec<String>,
}

/// What an engine crate implements.
///
/// One trait, four implementations. Everything else — framing, confinement,
/// read-back, error shaping — is the runtime's.
pub trait Engine {
    /// Run with advisory progress; engines without work-unit reporting delegate.
    // The protocol keeps these inputs separate so every engine can opt into only
    // the data it needs without allocating an intermediate request object.
    #[allow(clippy::too_many_arguments)]
    fn run_progress(
        &self,
        bytes: &[u8],
        extras: &[Vec<u8>],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<Converted, String> {
        self.run_with_inputs(bytes, extras, models, to, params, limits)
    }
    /// Stable name for the receipt.
    fn name(&self) -> &'static str;
    /// Version string for the receipt.
    fn version(&self) -> String;
    /// Extract facts without converting.
    ///
    /// **From the header only.** A probe that decodes is not a probe: its whole
    /// purpose is to let the host narrow `decode_pixels` *before* anything
    /// allocates, and a 4 KB file declaring 500 megapixels has to cost nothing
    /// to refuse.
    ///
    /// # Errors
    ///
    /// A message the host can show the user directly.
    fn probe(&self, bytes: &[u8]) -> Result<Probed, String>;
    /// Convert.
    ///
    /// # Errors
    ///
    /// As above. Limits are **already clamped by the host** — an engine may
    /// narrow them further but must never widen them, and the host would refuse
    /// the attempt anyway (I14).
    fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String>;

    /// Convert with operation parameters.
    ///
    /// The default implementation ignores `params` and delegates to [`Self::run`],
    /// so an engine that takes no parameters changes nothing. An engine that
    /// DOES take them overrides this; a parameter it does not recognise must be
    /// an error rather than silence, because silent parameter loss converts
    /// something other than what was asked.
    ///
    /// # Errors
    ///
    /// As [`Self::run`]. An unrecognised parameter is an error here too.
    fn run_params(
        &self,
        bytes: &[u8],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        if params.is_empty() {
            return self.run(bytes, to, limits);
        }
        Err(format!(
            "this engine takes no parameters, was asked {params:?}"
        ))
    }

    /// Convert with a model artifact alongside the input.
    ///
    /// The default ignores `model` and delegates, so every engine that does
    /// not do inference is unchanged — and an engine handed weights it never
    /// asked for says so rather than dropping them, on the same reasoning as
    /// [`Self::run_params`]: silence about an ignored input converts
    /// something other than what was asked.
    ///
    /// # Errors
    ///
    /// A message the host can show the user directly.
    fn run_model(
        &self,
        bytes: &[u8],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        if models.is_empty() {
            return self.run_params(bytes, to, params, limits);
        }
        Err(format!(
            "this engine does not take models, was handed {} of them",
            models.len()
        ))
    }

    /// Convert with FURTHER INPUTS alongside the first.
    ///
    /// Merging documents is what this is for: N files in, one out. The default
    /// delegates when there are none, so every engine that takes one input is
    /// unchanged — and an engine handed extra inputs it never asked for says
    /// so, on the same reasoning as [`Self::run_params`]. Silently converting
    /// only the first of five documents someone asked to merge is the worst
    /// available outcome: it succeeds, writes a receipt, and loses four files'
    /// worth of content without saying a word.
    ///
    /// # Errors
    ///
    /// A message the host can show the user directly.
    fn run_with_inputs(
        &self,
        bytes: &[u8],
        extras: &[Vec<u8>],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        if extras.is_empty() {
            return self.run_model(bytes, models, to, params, limits);
        }
        Err(format!(
            "this engine takes one input, was handed {} more",
            extras.len()
        ))
    }
}

/// Run a real engine binary: confine this process, then serve until the host
/// closes the pipe.
///
/// **This is the production entry point.** Every `tx-*` binary's `main` calls
/// it; nothing else should. Confinement is applied to *this* process before
/// the first byte of attacker input is read, and what actually engaged is
/// reported in every `Ready` answer.
///
/// # Failing to confine is fatal, not a warning
///
/// On Linux the worker applies its own Landlock + seccomp. If that application
/// fails — a kernel too old for Landlock, seccomp refused by policy — this
/// process stops before reading any frame. The host sees a session that never
/// started and reports it as a crash naming the engine. An unconfined worker
/// is a subprocess running an attacker's file with the user's authority, which
/// is worse than not converting; the host-side twin of this rule is
/// `WorkerError::Unconfinable`.
///
/// # Errors
///
/// Any framing or I/O failure after confinement succeeded.
pub fn run_real<E: Engine>(engine: &E) -> std::io::Result<()> {
    let mitigations = match confine_self() {
        Ok(m) => m,
        Err(e) => {
            // stderr, never stdout: stdout is the protocol, and a stray line
            // on it desynchronises the host's framing.
            eprintln!("this engine could not be confined and was stopped: {e}");
            return Err(std::io::Error::other(e));
        }
    };
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    serve_with(engine, &mitigations, &mut stdin, &mut stdout)
}

/// Apply confinement to this process, and read back what engaged.
///
/// - **Linux** — the worker is its own warden: Landlock denies all filesystem
///   access (this protocol needs none; bytes arrive on stdin and leave on
///   stdout), and seccomp denies outbound network at the syscall filter. Both
///   verdicts are earned by attempting the forbidden operation afterwards.
/// - **Windows** — the host confined us at spawn time (AppContainer, handle
///   list); there is nothing to apply here, so we only ask the OS what is in
///   force, exactly as before.
/// - **macOS** — the worker is its own warden as on Linux: a Seatbelt
///   `pure-computation` profile denies both filesystem and network, and both
///   verdicts are earned by attempting the forbidden operation afterwards.
///   **Never executed on a Mac** — see the module docs on `openconvert_os::macos`.
fn confine_self() -> Result<Vec<(String, bool)>, String> {
    #[cfg(target_os = "linux")]
    {
        openconvert_os::linux::confine_worker().map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        let back = openconvert_os::readback::read_back(&["ACG", "CIG", "AppContainer"]);
        Ok(back
            .mitigations
            .iter()
            .map(|m| (m.name.to_string(), m.engaged))
            .collect())
    }
    #[cfg(target_os = "macos")]
    {
        openconvert_os::macos::confine_worker().map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        // A platform with no mechanism implemented reports nothing, which
        // lowers the reported tier rather than claiming a sandbox that is not
        // there.
        Ok(Vec::new())
    }
}

/// What the platform can report about this process without applying anything.
///
/// Used by [`serve`], whose contract is "protocol loop only". On Windows this
/// is the same mitigation query `run_real` performs, because there the host
/// did the confining and the query is read-only. On Linux it returns nothing:
/// the honest alternative would be reporting mechanisms nobody applied.
#[must_use]
fn platform_readback() -> Vec<(String, bool)> {
    #[cfg(windows)]
    {
        let back = openconvert_os::readback::read_back(&["ACG", "CIG", "AppContainer"]);
        back.mitigations
            .iter()
            .map(|m| (m.name.to_string(), m.engaged))
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Serve one session over the given pipes, answering from the mitigations
/// supplied.
///
/// Shared by both entry points; the caller decides where the mitigation list
/// came from.
fn serve_with<E: Engine, R: Read, W: Write>(
    engine: &E,
    mitigations: &[(String, bool)],
    input: &mut R,
    output: &mut W,
) -> std::io::Result<()> {
    loop {
        let frame = match openconvert_sandbox::protocol::read_frame(input) {
            Ok(f) => f,
            // A closed pipe is how a session ends, not a failure.
            Err(openconvert_sandbox::protocol::FrameError::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(());
            }
            Err(e) => {
                return Err(std::io::Error::other(e.to_string()));
            }
        };

        let (response, content) = match serde_json::from_slice::<Request>(&frame) {
            Ok(req) => {
                // Content is drained from the pipe HERE, before dispatch, and on
                // every path including the failing ones. A request whose bytes
                // are left unread desynchronises the stream: the next
                // `read_frame` returns the middle of a file and the session is
                // lost. Answering a bad request without draining it is worse
                // than not answering at all.
                match take_content(input, &req) {
                    Ok((bytes, models, extras)) => handle(
                        engine,
                        req,
                        &bytes,
                        &models,
                        &extras,
                        mitigations,
                        &mut |completed, total| {
                            if let Ok(body) =
                                serde_json::to_vec(&Response::Progress { completed, total })
                            {
                                let _ = openconvert_sandbox::protocol::write_frame(output, &body);
                                let _ = output.flush();
                            }
                        },
                    ),
                    // A content failure IS fatal. Unlike a malformed request,
                    // there is no way to know how many bytes are still in the
                    // pipe, so there is no way to resynchronise -- and guessing
                    // would hand the next `read_frame` attacker-chosen bytes to
                    // parse as a message.
                    Err(e) => return Err(std::io::Error::other(e.to_string())),
                }
            }
            // A request we cannot parse is answered, not fatal. The host learns
            // which engine rejected it and whether to retry.
            //
            // Safe to continue only because an unparseable request declares no
            // length, so there is nothing following it to leave behind.
            Err(e) => (
                Response::Failed {
                    engine: engine.name().to_string(),
                    message: format!("could not parse the request: {e}"),
                    retryable: false,
                },
                Vec::new(),
            ),
        };

        let body = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
        openconvert_sandbox::protocol::write_frame(output, &body)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        // Content follows the reply that declared it, and only that reply.
        // `Done` is the sole variant carrying an `output_len`, so a `Failed`
        // must never be followed by bytes -- the host reads exactly what the
        // message it just parsed told it to expect.
        if !content.is_empty() {
            openconvert_sandbox::protocol::write_content(output, &content)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
    }
}

/// Run the protocol loop over pipes you supply, without confining anything.
///
/// **Tests and harnesses only.** This exists so the protocol can be exercised
/// in-process — which is what lets a malformed request, a desynced stream or a
/// hostile length prefix be studied without spawning anything. It performs
/// whatever *non-destructive* read-back the platform offers (on Windows, the
/// same mitigation query a real worker answers with; elsewhere nothing) and
/// applies nothing.
///
/// A worker binary must call [`run_real`], which confines first. The two are
/// deliberately different names: there is no signature that would let one
/// quietly become the other.
///
/// # Errors
///
/// Any framing or I/O failure. A malformed request is answered with `Failed`
/// rather than terminating the session — one bad message must not cost the
/// whole batch.
pub fn serve<E: Engine, R: Read, W: Write>(
    engine: &E,
    input: &mut R,
    output: &mut W,
) -> std::io::Result<()> {
    let mitigations = platform_readback();
    serve_with(engine, &mitigations, input, output)
}

/// Everything a request carried after its header: the input, the model
/// artifacts, and any further inputs.
///
/// Named rather than written out because it is three lists of bytes whose
/// ORDER is the protocol — input, then models, then extras, exactly as the host
/// wrote them. A bare tuple made that order something a reader had to infer
/// from a signature.
type RequestContent = (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>);

/// Read the content a request declared, bounded by the limit it carries.
///
/// The ceiling is the request's own `memory_bytes`, which the host already
/// clamped against `LimitCeiling` (I14). A worker asked to hold more than its
/// Job Object permits would be killed mid-read, and the host would see
/// `Crashed` -- an outcome that names nothing and suggests nothing.
fn take_content<R: Read>(
    input: &mut R,
    req: &Request,
) -> Result<RequestContent, openconvert_sandbox::protocol::FrameError> {
    match *req {
        Request::Hello => Ok((Vec::new(), Vec::new(), Vec::new())),
        Request::Probe { input_len } => {
            // A probe reads a header. It is capped far below a conversion
            // because nothing legitimate needs more, and phase-2 detection runs
            // against files that have not yet been judged.
            Ok((
                openconvert_sandbox::protocol::read_content(input, input_len, PROBE_CEILING)?,
                Vec::new(),
                Vec::new(),
            ))
        }
        Request::Run {
            input_len,
            limits,
            ref model_lens,
            ref extra_lens,
            ..
        } => {
            let content =
                openconvert_sandbox::protocol::read_content(input, input_len, limits.memory_bytes)?;
            // Drained in the SAME order the host wrote them, and
            // unconditionally: a declared stream left in the pipe
            // desynchronises everything after it, which is the reason the
            // input is drained before dispatch too. Extras follow the models,
            // for the same reason and with the same rule.
            let mut models = Vec::with_capacity(model_lens.len());
            for &len in model_lens {
                models.push(openconvert_sandbox::protocol::read_content(
                    input,
                    len,
                    limits.memory_bytes,
                )?);
            }
            let mut extras = Vec::with_capacity(extra_lens.len());
            for &len in extra_lens {
                extras.push(openconvert_sandbox::protocol::read_content(
                    input,
                    len,
                    limits.memory_bytes,
                )?);
            }
            Ok((content, models, extras))
        }
    }
}

/// The most content a `Probe` may carry.
///
/// Detection reads magic bytes and a header; 16 MiB is far more than any format
/// in the table needs and far less than a conversion. Two different questions
/// deserve two different ceilings -- reusing the conversion limit here would let
/// an unjudged file claim a gigabyte before anything had decided it was safe to
/// look at.
pub const PROBE_CEILING: u64 = 16 << 20;

/// Answer one request, returning the reply and any content that follows it.
fn handle<E: Engine>(
    engine: &E,
    req: Request,
    bytes: &[u8],
    models: &[Vec<u8>],
    extras: &[Vec<u8>],
    mitigations: &[(String, bool)],
    progress: &mut dyn FnMut(u64, u64),
) -> (Response, Vec<u8>) {
    let failed = |message: String, retryable: bool| {
        (
            Response::Failed {
                engine: engine.name().to_string(),
                message,
                retryable,
            },
            Vec::new(),
        )
    };

    match req {
        Request::Hello => (
            Response::Ready {
                engine: engine.name().to_string(),
                version: engine.version(),
                mitigations: mitigations.to_vec(),
            },
            Vec::new(),
        ),
        Request::Probe { .. } => match engine.probe(bytes) {
            Ok(p) => (Response::Properties(p), Vec::new()),
            Err(message) => failed(message, false),
        },
        Request::Run {
            to,
            ref limits,
            ref params,
            ..
        } => {
            // Recorded before the engine runs, because the adapters that build
            // inference sessions are far below this call and share code with
            // examples that have no request at all.
            set_use_gpu(limits.use_gpu);
            match engine.run_progress(bytes, extras, models, &to, params, limits, progress) {
                Ok(out) => (
                    Response::Done {
                        output_len: out.bytes.len() as u64,
                        removed: out.removed,
                    },
                    out.bytes,
                ),
                Err(message) => {
                    // A limit is not a crash. The host may retry with a higher
                    // ceiling if policy allows; a decode failure it may not.
                    let retryable = message.contains("limit");
                    failed(message, retryable)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Engine for Fake {
        #[allow(clippy::too_many_arguments)]
        fn run_progress(
            &self,
            bytes: &[u8],
            extras: &[Vec<u8>],
            models: &[Vec<u8>],
            to: &str,
            params: &[(String, String)],
            limits: &RunLimits,
            progress: &mut dyn FnMut(u64, u64),
        ) -> Result<Converted, String> {
            if to == "progress" {
                progress(1, 2);
                progress(2, 2);
            }
            self.run_with_inputs(bytes, extras, models, to, params, limits)
        }
        fn name(&self) -> &'static str {
            "fake"
        }
        fn version(&self) -> String {
            "0.0.1".into()
        }
        fn probe(&self, bytes: &[u8]) -> Result<Probed, String> {
            if bytes.is_empty() {
                return Err("empty input".into());
            }
            Ok(Probed::Image {
                width: bytes[0] as u32,
                height: bytes.len() as u32,
                has_alpha: false,
                frames: 1,
            })
        }
        fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
            if bytes.len() as u64 > limits.decode_pixels {
                return Err(format!(
                    "input exceeds the decode_pixels limit of {}",
                    limits.decode_pixels
                ));
            }
            Ok(Converted {
                bytes: format!("{to}:{}", bytes.len()).into_bytes(),
                removed: Vec::new(),
            })
        }
    }

    /// Test limits, with one field varied.
    ///
    /// Named rather than inline so a new ceiling added to `RunLimits` appears
    /// in one place here instead of every call site.
    fn lim(decode_pixels: u64) -> RunLimits {
        RunLimits {
            decode_pixels,
            memory_bytes: 1 << 20,
            archive_depth: 32,
            archive_entries: 100_000,
            archive_total_bytes: 8 << 30,
            use_gpu: true,
        }
    }

    /// Drive a full session through in-memory pipes.
    ///
    /// Each request carries its content, and each answer is decoded together
    /// with the content that follows it — so the helper exercises the same
    /// length-then-bytes discipline the real host does, rather than a simpler
    /// protocol that happens to work in memory.
    fn session(requests: &[(Request, Vec<u8>)]) -> Vec<(Response, Vec<u8>)> {
        let mut input = Vec::new();
        for (r, content) in requests {
            let body = serde_json::to_vec(r).unwrap();
            openconvert_sandbox::protocol::write_frame(&mut input, &body).unwrap();
            openconvert_sandbox::protocol::write_content(&mut input, content).unwrap();
        }
        let mut output = Vec::new();
        serve(&Fake, &mut std::io::Cursor::new(input), &mut output).expect("serve");

        let mut cursor = std::io::Cursor::new(output);
        let mut out: Vec<(Response, Vec<u8>)> = Vec::new();
        while let Ok(frame) = openconvert_sandbox::protocol::read_frame(&mut cursor) {
            let response: Response = serde_json::from_slice(&frame).unwrap();
            // Only `Done` is followed by bytes. Reading content after any other
            // variant would consume the NEXT answer's frame, which is exactly
            // the desync this shape has to make impossible.
            let content = match response {
                Response::Done { output_len, .. } => {
                    openconvert_sandbox::protocol::read_content(&mut cursor, output_len, u64::MAX)
                        .expect("content")
                }
                _ => Vec::new(),
            };
            out.push((response, content));
        }
        out
    }

    /// One request, one content payload.
    fn req(r: Request, content: &[u8]) -> (Request, Vec<u8>) {
        (r, content.to_vec())
    }

    #[test]
    fn progress_frames_do_not_consume_output_or_desynchronize_next_request() {
        let out = session(&[
            req(
                Request::Run {
                    input_len: 2,
                    to: "progress".into(),
                    limits: lim(1000),
                    params: vec![],
                    model_lens: vec![],
                    extra_lens: vec![],
                },
                &[1, 2],
            ),
            req(Request::Hello, &[]),
        ]);
        assert!(matches!(
            out[0].0,
            Response::Progress {
                completed: 1,
                total: 2
            }
        ));
        assert!(matches!(
            out[1].0,
            Response::Progress {
                completed: 2,
                total: 2
            }
        ));
        assert_eq!(out[2].1, b"progress:2");
        assert!(matches!(out[3].0, Response::Ready { .. }));
    }

    /// The whole protocol round-trips: hello, probe, run.
    #[test]
    fn a_session_answers_every_request_in_order() {
        let out = session(&[
            req(Request::Hello, &[]),
            req(Request::Probe { input_len: 3 }, &[7, 8, 9]),
            req(
                Request::Run {
                    input_len: 2,
                    to: "jpeg".into(),
                    limits: lim(1000),
                    params: Vec::new(),
                    model_lens: Vec::new(),
                    extra_lens: Vec::new(),
                },
                &[1, 2],
            ),
        ]);
        assert_eq!(out.len(), 3, "one answer per request");
        assert!(matches!(out[0].0, Response::Ready { .. }));
        assert!(matches!(
            out[1].0,
            Response::Properties(Probed::Image {
                width: 7,
                height: 3,
                ..
            })
        ));
        let (Response::Done { output_len, .. }, content) = &out[2] else {
            panic!("expected Done, got {:?}", out[2].0);
        };
        assert_eq!(
            *output_len as usize,
            content.len(),
            "the declared length must match the bytes that followed"
        );
    }

    /// **`Hello` reports read-back, not the request.**
    ///
    /// The worker asks the OS what engaged. This test cannot assert *which*
    /// mitigations are on — that depends on whether the process is contained —
    /// but it can assert the shape is populated on a platform we probe, which
    /// is what the receipt consumes.
    #[test]
    fn hello_carries_read_back_mitigations() {
        let out = session(&[req(Request::Hello, &[])]);
        let (
            Response::Ready {
                engine,
                version,
                mitigations,
            },
            _,
        ) = &out[0]
        else {
            panic!("expected Ready");
        };
        assert_eq!(engine, "fake");
        assert_eq!(version, "0.0.1");
        if cfg!(windows) {
            assert_eq!(
                mitigations.len(),
                3,
                "ACG, CIG and AppContainer should all be queried"
            );
            // Values, not just names. This harness is NOT confined -- it is
            // `cargo test`, not a spawned worker -- so every one must read
            // false. A `Hello` that reported confinement here would be a
            // `Hello` reporting it everywhere, which is the one failure that
            // makes the receipt worse than having none.
            assert!(
                mitigations.iter().all(|(_, on)| !on),
                "an unconfined test process claimed confinement: {mitigations:?}"
            );
        }
    }

    /// A failure is **structured**, so the host can name the engine.
    #[test]
    fn a_failure_names_the_engine_and_says_whether_to_retry() {
        let out = session(&[req(Request::Probe { input_len: 0 }, &[])]);
        let (
            Response::Failed {
                engine,
                message,
                retryable,
            },
            _,
        ) = &out[0]
        else {
            panic!("expected Failed");
        };
        assert_eq!(engine, "fake");
        assert_eq!(message, "empty input");
        assert!(!retryable, "a malformed input is not worth retrying");
    }

    /// A limit failure **is** retryable, and a decode failure is not.
    ///
    /// The distinction is what lets the host offer "raise the limit and try
    /// again" for one and not the other. Collapsing them would make the offer
    /// appear on failures where it cannot help.
    #[test]
    fn a_limit_failure_is_retryable_and_a_decode_failure_is_not() {
        let out = session(&[req(
            Request::Run {
                input_len: 5,
                to: "png".into(),
                limits: lim(2),
                params: Vec::new(),
                model_lens: Vec::new(),
                extra_lens: Vec::new(),
            },
            &[1, 2, 3, 4, 5],
        )]);
        let (Response::Failed { retryable, .. }, _) = &out[0] else {
            panic!("expected Failed");
        };
        assert!(*retryable, "a limit is not a crash");
    }

    /// A malformed frame is answered, not fatal.
    ///
    /// One bad message must not cost the whole batch — under `worker_reuse:
    /// Balanced` a session serves several files, and killing it on a parse
    /// error would take the rest down with it.
    #[test]
    fn a_malformed_request_does_not_end_the_session() {
        let mut input = Vec::new();
        openconvert_sandbox::protocol::write_frame(&mut input, b"{not json").unwrap();
        let body = serde_json::to_vec(&Request::Hello).unwrap();
        openconvert_sandbox::protocol::write_frame(&mut input, &body).unwrap();

        let mut output = Vec::new();
        serve(&Fake, &mut std::io::Cursor::new(input), &mut output).expect("serve");

        let mut cursor = std::io::Cursor::new(output);
        let mut answers = 0;
        while let Ok(frame) = openconvert_sandbox::protocol::read_frame(&mut cursor) {
            let _: Response = serde_json::from_slice(&frame).unwrap();
            answers += 1;
        }
        assert_eq!(answers, 2, "the session stopped after the bad frame");
    }

    /// A closed pipe ends the session cleanly.
    #[test]
    fn an_empty_input_is_a_clean_exit() {
        let mut output = Vec::new();
        serve(&Fake, &mut std::io::Cursor::new(Vec::new()), &mut output).expect("clean exit");
        assert!(output.is_empty());
    }
}
