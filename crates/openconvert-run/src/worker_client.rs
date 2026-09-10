//! The host side of the worker protocol.
//!
//! Spawns an engine binary, drives one session, and shuts it down. This is the
//! piece that turns `EngineBin` from a closed enum of *names* into a closed
//! enum of *programs that run*.
//!
//! # What the host trusts, and what it does not
//!
//! Nothing coming back. Every value the worker sends is treated as attacker
//! input, because a compromised engine is the threat this whole architecture is
//! shaped around. Limits are clamped before they are sent and **not** re-read
//! from the reply; the read-back mitigations are recorded, never acted on.
//!
//! # Why the worker gets bytes rather than a path
//!
//! It never learns a filename. That closes the filename-as-attack-vector class
//! more completely than renaming does, and it is what lets the Linux ladder
//! work without a mount namespace (`03` Â§5.2).
//!
//! Those bytes travel as a declared length followed by raw frames, each within
//! `MAX_FRAME_BYTES`. The first version put them inside the `Run` message and
//! capped every sandboxed conversion at one frame â€” roughly 300 KB after
//! serde's byte-array encoding â€” against `Limits` promising a 4 GiB output.
//!
//! `03` Â§5.2 specifies pre-opened descriptors, and they are still the better
//! answer: no copies, and an engine can map the file. They are not built,
//! because `execute()` reads whole files into memory anyway, so descriptors
//! would add a job directory, its ACLs and a sweep to deliver a benefit the
//! shell cannot take. They land when the shell streams.

use openconvert_core::facts::Properties;
use openconvert_core::limits::Limits;
use openconvert_os::spawn::PipedChild;
use openconvert_sandbox::argv::EngineBin;
use openconvert_sandbox::display::DisplayName;
use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};
use std::io::Write;
use std::path::PathBuf;

type ProgressListener = Box<dyn FnMut(u64, u64)>;
thread_local! {
    static PROGRESS_LISTENER: std::cell::RefCell<Option<ProgressListener>> = const { std::cell::RefCell::new(None) };
}

/// Observe work-unit progress for synchronous conversions on this thread.
/// Restores the previous listener even if the conversion unwinds.
pub fn with_progress<T>(listener: impl FnMut(u64, u64) + 'static, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<ProgressListener>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROGRESS_LISTENER.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(PROGRESS_LISTENER.with(|slot| slot.replace(Some(Box::new(listener)))));
    run()
}

/// Why a worker session failed.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// The engine binary is not installed next to us.
    #[error("{engine} is not installed at {path}. Reinstall to repair the engine set.")]
    NotInstalled {
        /// Which engine.
        engine: &'static str,
        /// Where we looked.
        path: String,
    },
    /// The worker could not be started.
    #[error("could not start {engine}: {message}")]
    Spawn {
        /// Which engine.
        engine: &'static str,
        /// Why, in the platform spawn's own words.
        message: String,
    },
    /// The worker started but could not be confined.
    ///
    /// **Fatal.** An unconfined worker is a subprocess running an attacker's
    /// file with this user's authority, which is worse than not converting.
    #[error(
        "{engine} started but could not be confined ({message}); it was stopped \
         and nothing was written"
    )]
    Unconfinable {
        /// Which engine.
        engine: &'static str,
        /// Why.
        message: String,
    },
    /// The worker died mid-session.
    ///
    /// Distinguished from a `Failed` reply on purpose: a worker that *answers*
    /// has a reason to give, and one that stops talking does not. `03` Â§13
    /// needs to tell those apart, because only the first can name a file and a
    /// step.
    #[error("{engine} stopped responding")]
    Crashed {
        /// Which engine.
        engine: &'static str,
    },
    /// The worker answered something the protocol does not allow here.
    #[error("{engine} sent an unexpected reply")]
    Protocol {
        /// Which engine.
        engine: &'static str,
    },
    /// The worker reported a structured failure.
    #[error("{engine}: {message}")]
    Engine {
        /// Which engine.
        engine: String,
        /// What it said, in the user's terms.
        message: String,
        /// Whether raising a limit and retrying could help.
        retryable: bool,
    },
    /// Underlying I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A running worker, mid-session.
pub struct Worker {
    child: PipedChild,
    engine: &'static str,
    /// The AppContainer this worker runs in. Empty off Windows.
    ///
    /// Recorded for the receipt: "confined" is a claim, and a claim with a
    /// name is one a user can check with `Get-AppxPackage` or an ACL viewer.
    container_name: String,
    /// The Job Object holding the child, on Windows.
    ///
    /// Kept alive for exactly as long as the worker is: the job's memory cap
    /// and its kill-on-close behaviour both end when the last handle closes,
    /// so dropping this early would silently un-limit a running engine.
    #[cfg(windows)]
    _job: openconvert_os::job_object::LimitedJob,
    /// What the worker read back about its own confinement.
    ///
    /// Recorded for the receipt. **Never acted on** â€” the host does not decide
    /// anything from a value the worker chose.
    mitigations: Vec<(String, bool)>,
}

impl Worker {
    /// Start an engine, confine it, and complete the handshake.
    ///
    /// The `Hello` round trip costs a message and buys the receipt its
    /// evidence: what confinement actually engaged, read back in the child
    /// rather than echoed from our request.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`].
    pub fn start(bin: EngineBin, limits: &Limits) -> Result<Self, WorkerError> {
        let path = engine_path(bin);
        if !path.exists() {
            return Err(WorkerError::NotInstalled {
                engine: bin.file_stem(),
                path: path.display().to_string(),
            });
        }
        Self::start_path(&path, bin.file_stem(), limits)
    }

    /// Start the engine binary at an exact path. **Tests only.**
    ///
    /// Behind the `test-support` feature, which no shipped build enables, so
    /// the production guarantee is untouched: [`Worker::start`] takes a closed
    /// enum and resolves the path itself, because an engine chosen by a
    /// caller-supplied path is an engine chosen by whoever controls that
    /// string.
    ///
    /// It is public at all because the alternative was worse. These tests began
    /// life inside this crate, resolving the binary beside `current_exe()` and
    /// skipping when it was not there â€” and under `cargo mutants` it never was,
    /// so 33 mutants survived in the code they were written to cover. Only a
    /// test in the `oc-images` package gets `CARGO_BIN_EXE_oc-images`.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`].
    #[cfg(feature = "test-support")]
    pub fn start_at(
        path: &std::path::Path,
        engine: &'static str,
        limits: &Limits,
    ) -> Result<Self, WorkerError> {
        Self::start_path(path, engine, limits)
    }

    /// Start the engine binary at an exact path.
    fn start_path(
        path: &std::path::Path,
        engine: &'static str,
        limits: &Limits,
    ) -> Result<Self, WorkerError> {
        // Through `-os`, never `Command`. The inherit list is the difference
        // between a worker holding two pipe ends and a worker holding every
        // inheritable handle in this process (spike S3: 64 of them, two
        // writable).
        //
        // On Windows the spawn also places the child in an AppContainer, which
        // is the only mechanism here that bounds what the worker can *reach*
        // rather than what it can consume. Everything else -- the Job Object,
        // the handle list -- caps resources.
        #[cfg(windows)]
        let (child, container_name) = {
            let container = container_for(engine, path)?;
            // An AppContainer cannot launch a program it cannot read, and
            // during development the engines sit under a user profile that
            // grants app packages nothing.
            //
            // ONCE PER PROCESS, under a lock. Converting a folder starts
            // several workers at once, and rewriting one file's DACL from
            // several threads makes it transiently unreadable -- so the spawn
            // that lost the race fails with ACCESS_DENIED on a binary that was
            // fine a millisecond earlier. Found as a flake in the workspace
            // test run: six failures in one pass, six passes in the next four.
            let child =
                openconvert_os::spawn_windows::spawn_piped_in_container(path, &[], &container)
                    .map_err(|e| WorkerError::Spawn {
                        engine,
                        message: e.to_string(),
                    })?;
            (child, container.name().to_string())
        };

        #[cfg(not(windows))]
        let (child, container_name) = (
            // The one thing the POSIX host applies itself: the address-space
            // cap, set before exec. Landlock and seccomp are the worker's own
            // first act, applied before it reads a byte â€” which is why they
            // do not appear here.
            openconvert_os::spawn::spawn_piped_limited(path, &[], Some(limits.memory_bytes))
                .map_err(|e| WorkerError::Spawn {
                    engine,
                    message: e.to_string(),
                })?,
            String::new(),
        );

        #[cfg(windows)]
        let job =
            {
                // The child is already running here, so this window is real. It is
                // bounded by the fact that the worker's first act is to read a
                // frame -- it touches no attacker bytes until the host sends some,
                // and the host sends none until this succeeds.
                let job = openconvert_os::job_object::LimitedJob::new(limits.memory_bytes)
                    .map_err(|e| WorkerError::Unconfinable {
                        engine,
                        message: e.to_string(),
                    })?;
                job.assign(child.child())
                    .map_err(|e| WorkerError::Unconfinable {
                        engine,
                        message: e.to_string(),
                    })?;
                job
            };

        let mut worker = Self {
            child,
            engine,
            container_name,
            #[cfg(windows)]
            _job: job,
            mitigations: Vec::new(),
        };

        match worker.exchange(&Request::Hello, &[])? {
            // The labels are strings the worker chose, and they end up in a
            // receipt. Rendered on arrival so no later caller has to remember.
            Response::Ready { mitigations, .. } => {
                worker.mitigations = mitigations
                    .into_iter()
                    .map(|(name, on)| (DisplayName::new(&name).to_string(), on))
                    .collect();
            }
            _ => return Err(WorkerError::Protocol { engine }),
        }
        Ok(worker)
    }

    /// What the worker read back about its own confinement.
    #[must_use]
    pub fn mitigations(&self) -> &[(String, bool)] {
        &self.mitigations
    }

    /// The AppContainer this worker runs in. Empty off Windows.
    #[must_use]
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Convert bytes.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Engine`] carries the worker's own message, because the
    /// engine knows what went wrong and the host does not. It carries the
    /// **host's** name for that engine, not the worker's â€” see below.
    pub fn run(
        &mut self,
        bytes: Vec<u8>,
        to: &str,
        limits: &Limits,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>), WorkerError> {
        self.run_with_models(bytes, &[], to, limits, params)
    }

    /// Convert bytes with a model artifact alongside them.
    ///
    /// The weights travel on the pipe because a worker has no filesystem — see
    /// `Request::Run::model_len`. The caller has already verified them against
    /// the registry's sha256 pin; this function neither checks nor interprets
    /// them, it moves them.
    ///
    /// Run with FURTHER INPUTS alongside the first.
    ///
    /// Merging documents is what this exists for. The extras ride the same
    /// framing as model artifacts and are drained after them, which is why
    /// they are a separate list rather than appended to that one: the two mean
    /// different things, and a worker must be able to tell which it received.
    ///
    /// # Errors
    ///
    /// As [`Self::run`].
    pub fn run_with_inputs(
        &mut self,
        bytes: Vec<u8>,
        extras: &[Vec<u8>],
        to: &str,
        limits: &Limits,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>), WorkerError> {
        let reply = self.exchange_with_models(
            &Request::Run {
                input_len: bytes.len() as u64,
                to: to.to_string(),
                limits: run_limits(limits),
                params: params.to_vec(),
                model_lens: Vec::new(),
                extra_lens: extras.iter().map(|e| e.len() as u64).collect(),
            },
            &bytes,
            &[],
            extras,
        )?;
        self.finish(reply, limits)
    }

    /// # Errors
    ///
    /// As [`Self::run`].
    pub fn run_with_models(
        &mut self,
        bytes: Vec<u8>,
        models: &[Vec<u8>],
        to: &str,
        limits: &Limits,
        params: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<String>), WorkerError> {
        let reply = self.exchange_with_models(
            &Request::Run {
                input_len: bytes.len() as u64,
                to: to.to_string(),
                // Already clamped against `LimitCeiling` (I14). The worker may
                // narrow further and cannot widen, and we never read a limit
                // back out of its reply.
                limits: run_limits(limits),
                params: params.to_vec(),
                model_lens: models.iter().map(|m| m.len() as u64).collect(),
                extra_lens: Vec::new(),
            },
            &bytes,
            models,
            &[],
        )?;

        self.finish(reply, limits)
    }

    /// Read a header and report what it claims. Phase 2 of detection.
    ///
    /// # Errors
    ///
    /// See [`WorkerError`]. A caller that only wants defence in depth should
    /// treat any error as "no properties" rather than as a refusal â€” see
    /// [`crate::probe`].
    pub fn probe(&mut self, bytes: Vec<u8>, limits: &Limits) -> Result<Properties, WorkerError> {
        let reply = self.exchange(
            &Request::Probe {
                input_len: bytes.len() as u64,
            },
            &bytes,
        )?;
        match reply {
            // The worker's answer arrives in the shape of what it probed.
            // An archive reports an entry count and an image reports pixels,
            // and neither borrows the other's fields to do it.
            Response::Properties(p) => Ok(match p {
                openconvert_worker::Probed::Image {
                    width,
                    height,
                    has_alpha,
                    frames,
                } => {
                    // The worker chose these numbers, so nothing downstream may
                    // widen a limit from them. `route::limits_for` only ever
                    // takes the MINIMUM of the policy ceiling and what they
                    // imply, so an engine inflating its dimensions cannot buy
                    // itself a larger budget -- only shrink its own.
                    let _ = limits;
                    Properties::Image {
                        width,
                        height,
                        has_alpha,
                        frames,
                    }
                }
                openconvert_worker::Probed::Audio {
                    sample_rate,
                    channels,
                    duration_ms,
                } => Properties::Audio {
                    sample_rate,
                    channels,
                    duration_ms,
                },
                openconvert_worker::Probed::Document { pages, encrypted } => {
                    Properties::Document { pages, encrypted }
                }
                openconvert_worker::Probed::Archive { entries } => Properties::Archive {
                    entries,
                    // Not determined by a probe. Reaching either would mean
                    // touching member bodies, which is a decode -- and the
                    // nesting cap is enforced during the conversion, where the
                    // bytes are already in hand.
                    depth: 0,
                    declared_total_bytes: None,
                },
            }),
            Response::Failed {
                message, retryable, ..
            } => Err(WorkerError::Engine {
                engine: self.engine.to_string(),
                message: DisplayName::sentence(&message).to_string(),
                retryable,
            }),
            _ => Err(WorkerError::Protocol {
                engine: self.engine,
            }),
        }
    }

    /// Send one request plus its content, and read one reply.
    fn exchange(&mut self, request: &Request, content: &[u8]) -> Result<Response, WorkerError> {
        self.exchange_with_models(request, content, &[], &[])
    }

    /// As [`Self::exchange`], with further content streams: model weights, then
    /// any additional inputs.
    ///
    /// The streams are written in the order the worker drains them, in one
    /// write turn, for the same reason the first one is: a length the worker
    /// can act on before its bytes arrive would make them separable.
    fn exchange_with_models(
        &mut self,
        request: &Request,
        content: &[u8],
        models: &[Vec<u8>],
        extras: &[Vec<u8>],
    ) -> Result<Response, WorkerError> {
        let engine = self.engine;
        let body = serde_json::to_vec(request).map_err(std::io::Error::other)?;
        {
            let stdin = self.child.stdin().ok_or(WorkerError::Crashed { engine })?;
            write_frame(stdin, &body).map_err(|e| std::io::Error::other(e.to_string()))?;
            // Content follows the request that declared its length, always in
            // the same write turn. A flush between them would let a worker act
            // on a length whose bytes are still in flight -- harmless here, but
            // it would make the two separable, and the next person to touch
            // this would not know they must not be.
            write_content(stdin, content).map_err(|e| std::io::Error::other(e.to_string()))?;
            for model in models {
                write_content(stdin, model).map_err(|e| std::io::Error::other(e.to_string()))?;
            }
            // Extras AFTER the models, which is the order the worker drains
            // them in. Two lists that agree only by convention would be a
            // desynchronisation waiting to happen, so both ends read this one
            // sentence.
            for extra in extras {
                write_content(stdin, extra).map_err(|e| std::io::Error::other(e.to_string()))?;
            }
            stdin.flush()?;
        }

        let stdout = self.child.stdout().ok_or(WorkerError::Crashed { engine })?;
        // A worker killed by its Job Object memory cap dies mid-frame, so a
        // read failure here means "stopped talking" rather than "said no".
        // Those are different rows in `03` Â§13 and only one of them has a
        // message the user can act on.
        loop {
            let frame = read_frame(stdout).map_err(|_| WorkerError::Crashed { engine })?;
            let reply: Response =
                serde_json::from_slice(&frame).map_err(|_| WorkerError::Protocol { engine })?;
            if let Response::Progress { completed, total } = reply {
                if total > 0 && completed <= total {
                    PROGRESS_LISTENER.with(|listener| {
                        if let Some(callback) = listener.borrow_mut().as_mut() {
                            callback(completed, total);
                        }
                    });
                }
            } else {
                return Ok(reply);
            }
        }
    }

    /// Turn a worker's reply into bytes, or into an error that names the right
    /// component.
    ///
    /// Extracted when a second run path appeared: two copies of this would be
    /// two places to decide whether a worker gets to say who it is, and only
    /// one of them would keep saying no.
    fn finish(
        &mut self,
        reply: Response,
        limits: &Limits,
    ) -> Result<(Vec<u8>, Vec<String>), WorkerError> {
        match reply {
            Response::Done {
                output_len,
                removed,
            } => Ok((
                // `output_len` is the worker's number, so it is checked against
                // OUR ceiling before a byte is reserved. A compromised engine
                // answering `u64::MAX` must cost the host an error, not an
                // allocation (I14).
                self.take_content(output_len, limits.output_bytes)?,
                // The removed-metadata list goes into the receipt, and a
                // receipt is a document a user reads. Field names come from a
                // process assumed compromised, so they are rendered, not
                // trusted.
                removed
                    .iter()
                    .map(|r| DisplayName::new(r).to_string())
                    .collect(),
            )),
            Response::Failed {
                engine: claimed,
                message,
                retryable,
            } => {
                // THE WORKER DOES NOT GET TO SAY WHO IT IS.
                //
                // `Failed` carries an engine name, and taking it would let a
                // compromised oc-images sign its errors "oc-pdf" -- so a user
                // reporting a bug, and a maintainer reading the report, both
                // look at the wrong component. The host started this process
                // and knows its name; the worker's claim is only worth
                // recording when it DISAGREES, which is a symptom in itself.
                // `sentence`, NOT `new`: this is prose, and `new` elides the
                // middle of what it is given. A worker's refusal came back as
                // "...any of the 99 languages t...ich is not close enough to
                // transcribe", which is two readable halves joined into
                // nonsense. The engine NAME beside it is still a name.
                let message = if claimed == self.engine {
                    DisplayName::sentence(&message).to_string()
                } else {
                    format!(
                        "{} (this worker identified itself as {}, which it is not)",
                        DisplayName::sentence(&message),
                        DisplayName::new(&claimed)
                    )
                };
                Err(WorkerError::Engine {
                    engine: self.engine.to_string(),
                    message,
                    retryable,
                })
            }
            _ => Err(WorkerError::Protocol {
                engine: self.engine,
            }),
        }
    }

    /// Read content the worker declared, bounded by **our** ceiling.
    ///
    /// `declared` crosses from a process this design assumes is compromised, so
    /// it is a claim, not a length. `read_content` refuses it while it is still
    /// a number.
    fn take_content(&mut self, declared: u64, cap: u64) -> Result<Vec<u8>, WorkerError> {
        let engine = self.engine;
        if declared == 0 {
            return Ok(Vec::new());
        }
        let stdout = self.child.stdout().ok_or(WorkerError::Crashed { engine })?;
        read_content(stdout, declared, cap).map_err(|e| match e {
            // An over-claim is the engine misbehaving, and it deserves the
            // variant that names it and says whether retrying helps -- not
            // `Crashed`, which is for a worker that stopped talking.
            openconvert_sandbox::protocol::FrameError::ContentTooLarge { declared, cap } => {
                WorkerError::Engine {
                    engine: engine.to_string(),
                    message: format!(
                        "produced {declared} bytes, which exceeds the {cap}-byte output limit"
                    ),
                    retryable: true,
                }
            }
            _ => WorkerError::Crashed { engine },
        })
    }
}

// NO `Drop` HERE, deliberately.
//
// There was one, and mutation testing showed replacing its body with `()`
// changed nothing observable: `PipedChild`'s own `Drop` already closes stdin
// and reaps the child, and on Windows the Job Object terminates anything that
// ignores both. Teardown belongs to the type that owns the process, and a
// second place that is merely redundant today is a second place to keep right
// forever.
//
// Field order still matters: `child` is declared before `_job`, so the child is
// waited on while the job still holds it.

/// Grant the container execute on one engine binary, **at most once per
/// process**.
///
/// The ACL is a property of the file, not of a worker, so doing it per start
/// was both wasteful and wrong: `SetNamedSecurityInfoW` replaces a DACL, and a
/// concurrent `CreateProcessW` against a file mid-replacement fails with
/// `ERROR_ACCESS_DENIED`. The lock is held across the call so two starts cannot
/// overlap, and the set means the second start does no ACL work at all.
///
/// A poisoned lock is recovered rather than propagated. The only thing this
/// mutex guards is a set of paths; a panic elsewhere holding it leaves that set
/// perfectly valid, and refusing every future conversion over it would turn one
/// unrelated bug into a dead product.
/// Get this engine's AppContainer, creating the profile and ACLing the binary
/// **at most once per process**.
///
/// # Why the whole thing is serialised, not just the ACL
///
/// Both halves race, and the first fix only addressed the second half. Profile
/// creation was the louder one: two threads calling `CreateAppContainerProfile`
/// with the same name get `E_UNEXPECTED`, not "already exists". The ACL is the
/// quieter one: `SetNamedSecurityInfoW` replaces a DACL, and a concurrent
/// `CreateProcessW` against a file mid-replacement fails `ACCESS_DENIED` on a
/// binary that was fine a millisecond earlier.
///
/// Converting a folder starts several workers of one engine together, so this
/// is a product bug the tests happened to find first â€” six of six failing in
/// one pass and none in the next four, which is the signature of a race and the
/// reason a single green run proves nothing.
///
/// **One profile per engine, stable across runs.** Profiles persist per user,
/// so a fresh name each time would leave a growing pile behind. Per engine
/// rather than one shared profile because the ACLs differ: only `oc-images`
/// should ever be granted execute on `oc-images`.
///
/// Failing here is fatal (`Unconfinable`). A conversion that does not happen is
/// strictly better than one that happens unprotected.
///
/// A poisoned lock is recovered rather than propagated: it guards a set of
/// paths, which a panic elsewhere leaves perfectly valid, and refusing every
/// future conversion over an unrelated bug would be the worse failure.
#[cfg(windows)]
fn container_for(
    engine: &'static str,
    path: &std::path::Path,
) -> Result<openconvert_os::appcontainer::AppContainer, WorkerError> {
    container_named(&format!("openconvert.{engine}"), engine, path)
}

/// The body of [`container_for`], with the profile name supplied.
///
/// Split so a test can pass a name Windows refuses and watch `Unconfinable`
/// actually happen. It was the only branch in the confinement path with no test
/// that executed it, because production derives the name from a closed enum and
/// every derived name works.
///
/// The name stays out of `container_for`'s signature: a container chosen by a
/// caller-supplied string is a container chosen by whoever controls that
/// string, which is the same reasoning that keeps `EngineBin` closed.
#[cfg(windows)]
fn container_named(
    name: &str,
    engine: &'static str,
    path: &std::path::Path,
) -> Result<openconvert_os::appcontainer::AppContainer, WorkerError> {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static PREPARED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let prepared = PREPARED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut done = prepared.lock().unwrap_or_else(|e| e.into_inner());

    let unconfinable =
        |e: openconvert_os::appcontainer::ContainerError| WorkerError::Unconfinable {
            engine,
            message: e.to_string(),
        };

    let container =
        openconvert_os::appcontainer::AppContainer::create(name).map_err(unconfinable)?;

    if !done.contains(path) {
        // Read and execute only: an engine binary the worker could rewrite is
        // one it could replace, which would turn a single compromised
        // conversion into a permanent one.
        container.grant_execute(path).map_err(unconfinable)?;

        // §5.3's packaging trap, caught here: dynamically-linked engine
        // libraries are loaded INSIDE the sandbox under the container's
        // identity, so every `*.dll` beside the engine needs read+execute or
        // the worker dies before `main` with a loader error naming a missing
        // dependency rather than a permission problem.
        if let Some(dir) = path.parent() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                        container.grant_execute(&p).map_err(unconfinable)?;
                    }
                }
            }
        }
        done.insert(path.to_path_buf());
    }
    Ok(container)
}

/// Every ceiling a worker may need, from the step's limits.
///
/// One place builds this, so adding a limit to `Limits` and forgetting to send
/// it is a compile error rather than a worker running unbounded on a field
/// nobody wired.
fn run_limits(l: &Limits) -> RunLimits {
    RunLimits {
        decode_pixels: l.decode_pixels,
        memory_bytes: l.memory_bytes,
        archive_depth: l.archive_depth,
        archive_entries: l.archive_entries,
        archive_total_bytes: l.archive_total_bytes,
        use_gpu: l.use_gpu,
    }
}

/// Where an engine binary lives.
///
/// **In one of our own directories, never on `PATH`.** A `PATH` lookup is a
/// program name resolved by the environment, which is the environment choosing
/// our engine for us -- and `EngineBin` being a closed enum would then
/// guarantee nothing about what actually runs.
///
/// "Our own directories" was a single directory until installed copies proved
/// it could not be: a `.deb` separates the binary from its resources, and so
/// does a macOS bundle. [`crate::engine_dir`] holds the two-entry search and
/// the reasoning; when the engine is in neither, this returns where a repair
/// would put it, so the error names a real path.
fn engine_path(bin: EngineBin) -> PathBuf {
    let name = format!("{}{}", bin.file_stem(), std::env::consts::EXE_SUFFIX);
    crate::engine_dir::locate(&name).unwrap_or_else(|| crate::engine_dir::expected_path(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing engine is reported by **name and path**, not as a generic
    /// failure.
    ///
    /// `03` Â§13's rule: name the file, the step and the engine, and say what to
    /// do. "Reinstall to repair the engine set" is actionable; "spawn failed"
    /// is not.
    #[test]
    fn a_missing_engine_names_itself_and_where_we_looked() {
        // oc-pdf is genuinely not built, which makes this the real case rather
        // than a simulated one.
        let Err(err) = Worker::start(EngineBin::Pdf, &Limits::defaults()) else {
            panic!("oc-pdf is not built; starting it should have failed");
        };
        match err {
            WorkerError::NotInstalled { engine, path } => {
                assert_eq!(engine, "oc-pdf");
                assert!(path.contains("oc-pdf"), "the path should name it: {path}");
            }
            other => panic!("wrong error: {other}"),
        }
    }

    /// Engines resolve beside our executable, never through `PATH`.
    #[test]
    fn engines_resolve_beside_us_not_through_path() {
        let p = engine_path(EngineBin::Images);
        assert!(
            p.is_absolute(),
            "engine path should be absolute: {}",
            p.display()
        );
        assert!(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("oc-images"),
            "wrong file name: {}",
            p.display()
        );
    }

    /// **`Unconfinable` happens**, and it is fatal rather than a warning.
    ///
    /// The only branch in the confinement path with no test that ran it.
    /// Production derives the profile name from a closed enum, and every
    /// derived name works, so nothing had ever driven `AppContainer::create`
    /// to fail here.
    ///
    /// An empty name is refused by Windows with `E_INVALIDARG`, and refused by
    /// derivation too â€” so this is the real OS failure, not a simulated one.
    /// What matters is that it becomes `Unconfinable` and stops the worker: an
    /// unconfined worker is a subprocess running an attacker's file with this
    /// user's authority, which is worse than not converting.
    #[test]
    #[cfg(windows)]
    fn a_container_that_cannot_be_created_is_fatal_and_names_the_engine() {
        let exe = std::env::current_exe().expect("current exe");
        let Err(err) = container_named("", "tx-test", &exe) else {
            panic!("an empty container name should be refused by Windows");
        };
        match err {
            WorkerError::Unconfinable { engine, message } => {
                assert_eq!(engine, "tx-test", "the message must name the engine");
                assert!(
                    message.contains("AppContainer"),
                    "the message should say what could not be set up: {message}"
                );
            }
            other => panic!("expected Unconfinable, got {other}"),
        }
    }

    /// The control: a name Windows accepts confines successfully.
    ///
    /// Without it, the test above is satisfied by a `container_named` that
    /// refuses every name â€” which would refuse every conversion.
    #[test]
    #[cfg(windows)]
    fn a_valid_container_name_confines_rather_than_failing() {
        let exe = std::env::current_exe().expect("current exe");
        let name = format!("openconvert.test.{}", std::process::id());
        assert!(
            container_named(&name, "tx-test", &exe).is_ok(),
            "a valid name should produce a container"
        );
    }

    /// Every engine resolves to a distinct program.
    ///
    /// A mapping that collapsed two variants onto one name would send PDF work
    /// to the image worker, and the closed enum would still look correct.
    #[test]
    fn every_engine_resolves_to_a_distinct_program() {
        let paths: Vec<_> = EngineBin::ALL.iter().map(|b| engine_path(*b)).collect();
        for (i, a) in paths.iter().enumerate() {
            for b in &paths[i + 1..] {
                assert_ne!(a, b, "two engines resolve to the same program");
            }
        }
    }

    // The tests that DRIVE a worker live in `crates/engines/oc-images/tests/`,
    // where `CARGO_BIN_EXE_oc-images` guarantees the binary exists. They lived
    // here first, skipped when it did not, and 33 mutants survived in the code
    // they were written to cover. A test that can skip is not coverage.
}
