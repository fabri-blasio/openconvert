//! Routing, as an **ordered table** rather than a graph search.
//!
//! `route()` returns the first route whose requirements hold, attaches the
//! `Class`, `Limits` and `Isolation` that policy and environment imply, and
//! records the requirement that eliminated each rejected route â€” which is where
//! the explanation comes from. Thirty lines instead of a search.
//!
//! # Why a table beats A\*
//!
//! | | Route table | A\* search |
//! |---|---|---|
//! | Inspectable | `openconvert routes mkv mp4` prints it | you cannot print a search |
//! | Explainable | falls out of the requirement that failed | reconstruct a heuristic |
//! | Debuggable | a wrong choice is a wrong requirement, one line | a wrong weight, somewhere |
//! | Tunable | nothing to tune | weights need data you do not have |
//!
//! And the strongest argument, which v0.4 did not make: **one artefact, four
//! consumers.** The docs, the website's `/formats`, `openconvert routes` and the
//! property test all read this table, so a wrong route is a one-line diff
//! visible in four places.
//!
//! Add a search when the table cannot express something. Not before.

use crate::environment::Environment;
use crate::facts::Properties;
use crate::format::FormatId;
use crate::isolation::Isolation;
use crate::limits::{LimitCeiling, Limits};
use crate::plan::{Class, NoRoute, Plan, PlanRequest, Step, StepKind, Warning};
use crate::policy::Policy;
use crate::target::{Operation, Quality, Target};

/// A condition a route needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Container streams are already compatible with the destination.
    StreamsCompatible,
    /// The audio stream alone is carryable â€” the audio-extraction
    /// requirement. Video codecs are irrelevant to it by construction.
    AudioCompatible,
    /// A named engine must be present.
    Engine(&'static str),
    /// Always satisfied. The fallback row.
    Always,
}

impl Requirement {
    /// Whether this requirement holds for a conversion into `output`.
    ///
    /// `output` is here because `StreamsCompatible` is meaningless without it:
    /// H.264 is carryable into MP4 and not into WebM, and the same file gives
    /// opposite answers depending on where it is going.
    #[must_use]
    pub fn holds(self, env: &Environment, props: Properties, output: FormatId) -> bool {
        match self {
            Self::Always => true,
            Self::Engine(name) => env.engine(name).is_some_and(|e| e.available),
            Self::StreamsCompatible => match props {
                Properties::Video { video, audio, .. } => {
                    crate::codec::streams_carryable(output, video, audio)
                }
                // Not a video file, or not probed. Either way nothing has said
                // the streams are carryable, and this requirement is the one
                // that must never be satisfied by absence of evidence: a
                // failure here costs a re-encode the user is told about, and a
                // false pass produces a file that will not play under a receipt
                // calling it lossless.
                _ => false,
            },
            Self::AudioCompatible => match props {
                // Extraction is a CONTAINER operation, so this rides the same
                // video-file gate as `StreamsCompatible`. Pure audio inputs
                // have their own routes and never consult it.
                Properties::Video { audio, .. } => crate::codec::audio_carryable(output, audio),
                _ => false,
            },
        }
    }

    /// Human-readable, for the explanation of a rejected route.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Always => "always applies",
            Self::Engine(name) => name,
            Self::StreamsCompatible => "codecs are compatible with the destination container",
            Self::AudioCompatible => "the audio codec is compatible with the destination container",
        }
    }
}

/// One way to get from A to B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Source format.
    pub from: FormatId,
    /// Destination format.
    pub to: FormatId,
    /// The steps, in order.
    pub steps: &'static [StepKind],
    /// What this route costs in fidelity.
    pub class: Class,
    /// Conditions, all of which must hold.
    pub requires: &'static [Requirement],
}

/// The ordered route table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteTable {
    routes: Vec<Route>,
}

impl RouteTable {
    /// Build from an ordered list. **Order is meaningful**: the first route
    /// whose requirements hold is the one taken, so lossless routes come first.
    #[must_use]
    pub fn new(routes: Vec<Route>) -> Self {
        Self { routes }
    }

    /// Every route for a pair, in preference order.
    pub fn routes_for(&self, from: FormatId, to: FormatId) -> impl Iterator<Item = &Route> {
        self.routes
            .iter()
            .filter(move |r| r.from == from && r.to == to)
    }

    /// The whole table, for `openconvert routes` and the property tests.
    #[must_use]
    pub fn all(&self) -> &[Route] {
        &self.routes
    }
}

/// Why a route was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected {
    /// The route.
    pub route: Route,
    /// The first requirement that failed.
    pub failed: Requirement,
}

/// Formats the inference adapters decode directly.
///
/// `oc-ai` builds on the `image` crate, which handles these and not the
/// camera formats beside them. That is a fact about the adapter, not about
/// what the user is allowed to bring.
const INFER_NATIVE: &[FormatId] = &[
    FormatId::Png,
    FormatId::Jpeg,
    FormatId::Webp,
    FormatId::Bmp,
    FormatId::Tiff,
    FormatId::Gif,
];

/// Raster formats this build can DECODE, including the ones the inference
/// adapters cannot read for themselves.
///
/// HEIC, AVIF and JXL are here and not in `INFER_NATIVE`: `oc-images` opens
/// all three, `oc-ai` opens none of them.
fn decodable_raster(id: FormatId) -> bool {
    INFER_NATIVE.contains(&id) || matches!(id, FormatId::Heic | FormatId::Avif | FormatId::Jxl)
}

/// The step that gets `input` into a form the inference adapter can read, or
/// nothing when it already can.
///
/// **THE PHOTOGRAPH ON THE USER'S PHONE IS A HEIC.** Background removal and
/// upscaling both refused every HEIC, AVIF and JXL outright, with
/// "background removal cannot be applied to this file" -- which reads as a
/// statement about the picture and is actually a statement about which
/// decoder the model happens to sit behind. The file is perfectly readable;
/// something just has to open it first.
///
/// A normalising step costs one extra decode and makes the operation work on
/// the formats people actually have. PNG because it is lossless: the model
/// sees what the camera recorded, not a generation of JPEG artifacts
/// introduced on the way in.
///
/// This is only possible because the executor hands each step the format the
/// PREVIOUS step produced. It used to tell every step the format of the
/// original file, and a two-step plan would have picked the wrong worker for
/// the second one.
fn normalise_for_infer(input: FormatId, env: &Environment, props: Properties) -> Option<StepKind> {
    if INFER_NATIVE.contains(&input) {
        return None;
    }
    // The decoder for the camera formats lives in oc-images; without it there
    // is nothing to normalise WITH, and the caller refuses as it always did.
    Requirement::Engine("oc-images")
        .holds(env, props, FormatId::Png)
        .then_some(StepKind::Transcode {
            from: input,
            to: FormatId::Png,
        })
}

/// What the best matting tier needs, per conversion.
///
/// BiRefNet-lite peaked at 6,465 MB on one 320x240 image: the fixed 1024x1024
/// input means a transformer backbone's activations dominate its 224 MB of
/// weights entirely. 8 GiB is the next size the Settings control offers above
/// that measurement -- refusing at 6.5 would mean refusing at a number nobody
/// can select.
pub const BEST_MATTING_MEMORY: u64 = 8 << 30;

/// Whether a policy allowing `memory_bytes` can run the best matting tier.
///
/// **Public because `auto` must not choose a tier this machine will refuse.**
/// `tier_for_tool` resolves an absent preference to the best tier that is
/// INSTALLED, and installing BiRefNet is not the same as being able to run it:
/// at the default 1 GiB the route below refuses, correctly, and the user sees
/// "that did not produce a file" for a tool that worked before they downloaded
/// a better model.
///
/// An EXPLICIT choice of `best` still refuses, and must: someone who believes
/// they got the better model and silently got the small one has been told
/// something untrue. This exists so the automatic path can decline to pick it,
/// not so the refusal can be skipped.
#[must_use]
pub const fn best_matting_fits(memory_bytes: u64) -> bool {
    memory_bytes >= BEST_MATTING_MEMORY
}

/// Plan a conversion. **Pure**: no I/O, no clock, no environment.
///
/// # The order of the checks is the security model
///
/// Each of these produces *no steps*, not a flagged plan â€” refusal is the
/// absence of a plan, so there is nothing for a caller to run anyway.
///
/// 1. **Unconditional refusals** â€” a polyglot (A6). No machine configuration
///    makes a file that is validly two formats safe, so this is refused before
///    anything machine-dependent. A message the user can act on beats one that
///    is merely accurate.
/// 2. **Isolation floor** (I9, SR-1). For sandboxed steps,
///    including `network: Denied`, which has no config key anywhere in this
///    crate. A machine that cannot deny network yields no steps regardless of
///    everything below.
/// 3. **Route exists** â€” otherwise an explained `NoRoute`.
/// 4. **`max_auto_class`** (I8). A Class D route is reachable only from a
///    `Target` naming the operation, never from a suggestion, never from
///    prediction.
/// 5. **Limits are clamped** against the policy ceiling (I14). Engine-reported
///    facts may narrow and never widen.
/// 6. **Isolation is assigned** from the format table (I10). A format without
///    a pure-Rust parser is `Sandboxed`, and there is no branch that produces
///    `InProcess` for one.
#[must_use]
pub fn route(request: PlanRequest, props: Properties, policy: &Policy, env: &Environment) -> Plan {
    let mut warnings = Vec::new();

    // (1) Refusals that no machine configuration can lift, first.
    //
    // A polyglot is a property of the FILE. No amount of sandboxing makes a
    // file that is validly two formats safe to convert, because the ambiguity
    // is the attack: detection cannot tell which format the author meant, and
    // picking by preference order is precisely what a polyglot is built to
    // exploit. So we refuse and ask (A6).
    //
    // The ORDER matters, and an earlier version had it backwards. Checking the
    // machine's isolation floor first meant a polyglot ZIP was refused with
    // "this machine cannot deny network access" -- true, unhelpful, and
    // actively misleading, because it invites the user to go and fix their
    // machine for a file that would still be refused afterwards.
    //
    // The rule: unconditional refusals before conditional ones. A message the
    // user can act on beats a message that happens to be accurate.
    if request.polyglot {
        warnings.push(Warning::Polyglot);
        return Plan::new(request, Vec::new(), warnings);
    }

    // (2) The floor â€” for steps that will actually be sandboxed.
    //
    // `03` Â§9.1 scopes this precisely: "route() emits no executable steps for
    // any **Sandboxed** step whose profile does not denies_network()". An
    // earlier version of this function checked it unconditionally, which
    // refused a pure-Rust PNG conversion on any machine without a probed
    // profile â€” found by running the CLI, not by reading the code.
    //
    // The floor exists because a third-party C parser must be confined or not
    // run at all. An `InProcess` step uses no such parser: I10 and the format
    // table guarantee it, and a property test asserts it. There is nothing to
    // confine, so there is nothing for the floor to be about.
    //
    // SR-1 still binds the in-process path â€” by `net::no_outbound_during_
    // conversion` and the single-call-site scan (SR-9), which are the right
    // instruments for code we compile ourselves.
    // This must answer the same question as the step construction below.
    // Looking only at the input format misses routes whose decoder is safe in
    // process but whose encoder exists only in a worker (PNG/JPEG -> AVIF).
    let will_sandbox = matches!(
        isolation_for(request.input, request.target, policy, env),
        Isolation::Sandboxed(_)
    );
    if will_sandbox {
        if let Some(refusal) = policy.floor().refusal(env.profile()) {
            warnings.push(Warning::BelowFloor(refusal));
            return Plan::new(request, Vec::new(), warnings);
        }
    }

    if request.input == FormatId::Unknown {
        warnings.push(Warning::NoRoute(NoRoute::UnknownInput));
        return Plan::new(request, Vec::new(), warnings);
    }

    let output = request.target.output_format(request.input);

    // (2) A route, or an explanation.
    //
    // The rejections ARE the explanation. An earlier version of this function
    // computed them into a throwaway `Vec::new()` and discarded them, leaving
    // `Rejected` and `Requirement::describe` unreachable — found by mutation
    // testing, not by reading the code, and not by any of the 17 property
    // tests that were passing at the time.
    let mut rejected = Vec::new();

    // Limits and isolation come from the input and policy alone, never from
    // which row was chosen, so they are computed once here — before selection.
    let ceiling = LimitCeiling::new(policy.limit_ceiling());
    let limits = ceiling.clamp(limits_for(props, policy));
    let isolation = isolation_for(request.input, request.target, policy, env);

    // Operations whose steps carry runtime parameters cannot be rows in a
    // static table: their step values are built here, under exactly the same
    // class-ceiling and warning rules the table path applies below.
    enum Selected {
        /// A table row.
        Table(Route),
        /// Steps built for a parameterized operation.
        Steps(Vec<StepKind>, Class),
    }

    let selected = match request.target {
        Target::Operation(Operation::Trim { start_ms, end_ms }) => {
            if !matches!(
                request.input,
                FormatId::Mkv | FormatId::Webm | FormatId::Mka
            ) {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "trim",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            Selected::Steps(vec![StepKind::Trim { start_ms, end_ms }], Class::A)
        }
        Target::Operation(Operation::RenderPage { page, to }) => {
            if !matches!(request.input, FormatId::Pdf)
                || !matches!(to, FormatId::Png | FormatId::Jpeg)
            {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "page render",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let req = Requirement::Engine("oc-pdf");
            if !req.holds(env, props, to) {
                warnings.push(Warning::NoRoute(NoRoute::RequirementUnmet {
                    requirement: req,
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            Selected::Steps(vec![StepKind::RenderPage { page }], Class::B)
        }
        Target::Operation(Operation::RemoveBackground { to, quality }) => {
            // Raster in, and a destination that HAS an alpha channel: the
            // whole output of this operation is a matte, and writing it to
            // JPEG would silently composite the subject onto a colour nobody
            // chose. Refusing is the only honest answer.
            // Any raster this build can decode, not only the ones the model's
            // own decoder handles -- see `normalise_for_infer`.
            if !decodable_raster(request.input) || !matches!(to, FormatId::Png | FormatId::Webp) {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "background removal",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let normalise = normalise_for_infer(request.input, env, props);
            if normalise.is_none() && !INFER_NATIVE.contains(&request.input) {
                // A camera format with no decoder in this build. Refusing is
                // still the honest answer; it is just no longer the answer for
                // every HEIC on every machine.
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "background removal",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            // The MODEL is the requirement, not the worker. A oc-ai that
            // loaded fine still cannot do this without weights, and naming
            // the runtime in that refusal would send the user to fix the
            // wrong thing.
            // THE QUALITY TIER NEEDS BOTH MODELS, and both are requirements.
            //
            // MODNet is a portrait matting network: better than u2netp on a
            // person, and capable of returning an essentially empty matte on a
            // subject it was not trained for -- which reaches the user as a PNG
            // that opens blank. The adapter falls back to u2netp when that
            // happens, so u2netp is not an alternative to MODNet here, it is
            // part of the same step, and a plan that promised the quality tier
            // without it would promise a step that cannot complete.
            // Every tier carries its fallback with it, because the adapter
            // decides between them by looking at the matte it got rather than
            // by asking the registry a second time.

            // THE BEST TIER NEEDS THE MEMORY LIMIT RAISED, AND SAYS SO HERE.
            //
            // BiRefNet-lite runs at a fixed 1024x1024 and peaks at **6,465 MB**
            // for one 320x240 image -- measured, recorded in `models.toml`. At
            // the default 1 GiB it does not run slowly or cut worse: it dies
            // with an allocation failure inside the worker.
            //
            // This is in `route()` and not in the tool because there are two
            // front doors. `openconvert convert --remove-background=best` comes
            // through here without touching `tools.rs` at all, and a check on
            // the tool side would hold for the app and miss the command line --
            // which is exactly how `force_sandbox` came to be half-wired.
            //
            // It reads `base_limits` rather than the ceiling: the ceiling is
            // what a step may have, the base is what it will be given, and
            // `Policy::with_worker_memory` moves both together so this is the
            // effective number either way. Note this does NOT widen anything --
            // `limits_for` still never widens. It refuses early instead.
            if quality == Quality::Best {
                let allowed = policy.base_limits().memory_bytes;
                if allowed < BEST_MATTING_MEMORY {
                    warnings.push(Warning::NoRoute(NoRoute::NeedsMoreMemory {
                        needed: BEST_MATTING_MEMORY,
                        allowed,
                    }));
                    return Plan::new(request, Vec::new(), warnings);
                }
            }

            let needed: &[&str] = match quality {
                Quality::Best => &["birefnet-lite", "u2netp"],
                Quality::Better => &["modnet", "u2netp"],
                Quality::Standard => &["u2netp"],
            };
            for name in needed {
                let req = Requirement::Engine(name);
                if !req.holds(env, props, to) {
                    warnings.push(Warning::NoRoute(NoRoute::RequirementUnmet {
                        requirement: req,
                    }));
                    return Plan::new(request, Vec::new(), warnings);
                }
            }
            let mut steps = Vec::new();
            steps.extend(normalise);
            steps.push(StepKind::Infer {
                models: needed,
                task: match quality {
                    Quality::Best => "remove-background-best",
                    Quality::Better => "remove-background-quality",
                    Quality::Standard => "remove-background",
                },
                to,
            });
            Selected::Steps(
                steps,
                // Class D: a network decided which pixels were the subject.
                // Nothing else in this file makes a judgement that can be
                // WRONG rather than merely lossy.
                Class::D,
            )
        }
        Target::Operation(Operation::Denoise) => {
            // Every audio source this build can decode, plus the containers a
            // speech recording actually arrives in.
            let audio_in = matches!(
                request.input,
                FormatId::Wav
                    | FormatId::Flac
                    | FormatId::Mp3
                    | FormatId::Ogg
                    | FormatId::M4a
                    | FormatId::Mkv
                    | FormatId::Webm
                    | FormatId::Mp4
            );
            if !audio_in {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "noise removal",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let req = Requirement::Engine("deepfilternet");
            if !req.holds(env, props, FormatId::Wav) {
                warnings.push(Warning::NoRoute(NoRoute::RequirementUnmet {
                    requirement: req,
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            // THE RECORDING COMES BACK IN THE FORMAT IT ARRIVED IN.
            //
            // The network emits samples, so the inference step can only
            // produce WAV — and for a long time that was the whole plan, which
            // meant "take the noise out of this mp3" handed back a WAV five
            // times the size. Worse, `Target::output_format` names an
            // operation's output after its INPUT, so the file was called
            // `.mp3` and was not one.
            //
            // A second step re-encodes it back. This was not possible until
            // the executor stopped telling every step the format of the
            // ORIGINAL file (see `exec.rs`); with that fixed, the encoder that
            // handles `wav -> mp3` for an ordinary conversion handles it here.
            //
            // CONTAINERS ARE EXCLUDED DELIBERATELY. Denoising an MKV and
            // returning an MKV means rebuilding a container around one
            // replaced track, which is a different job. Those keep WAV, and
            // `Plan::output_format` makes sure the name says so.
            let mut steps = vec![StepKind::Infer {
                models: DENOISE_MODELS,
                task: "denoise",
                to: FormatId::Wav,
            }];
            let back = match request.input {
                FormatId::Mp3 => Some((FormatId::Mp3, Requirement::Engine("libmp3lame"))),
                FormatId::Flac => Some((FormatId::Flac, Requirement::Engine("oc-audio"))),
                FormatId::Ogg => Some((FormatId::Ogg, Requirement::Engine("libopus"))),
                // WAV in, WAV out: already there, and a second step that
                // re-encoded WAV to WAV would be a decode and encode for
                // nothing.
                _ => None,
            };
            if let Some((to, req)) = back {
                if req.holds(env, props, to) {
                    steps.push(StepKind::Transcode {
                        from: FormatId::Wav,
                        to,
                    });
                }
                // If the encoder is not in this build, the plan simply ends on
                // WAV and the output is NAMED `.wav`. That is not a silent
                // downgrade: the file says what it is, which is the property
                // that was broken before any of this.
            }
            Selected::Steps(steps, Class::D)
        }
        Target::Operation(Operation::Upscale { to }) => {
            if !decodable_raster(request.input)
                || !matches!(to, FormatId::Png | FormatId::Jpeg | FormatId::Webp)
            {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "upscaling",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let normalise = normalise_for_infer(request.input, env, props);
            if normalise.is_none() && !INFER_NATIVE.contains(&request.input) {
                warnings.push(Warning::NoRoute(NoRoute::OperationInputMismatch {
                    operation: "upscaling",
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let req = Requirement::Engine("realesrgan-x4");
            if !req.holds(env, props, to) {
                warnings.push(Warning::NoRoute(NoRoute::RequirementUnmet {
                    requirement: req,
                }));
                return Plan::new(request, Vec::new(), warnings);
            }
            let mut steps = Vec::new();
            steps.extend(normalise);
            steps.push(StepKind::Infer {
                models: &["realesrgan-x4"],
                task: "upscale",
                to,
            });
            Selected::Steps(steps, Class::D)
        }
        _ => match pick(request, props, env, &mut rejected) {
            Some(chosen) => Selected::Table(chosen),
            None => {
                warnings.push(Warning::NoRoute(explain_no_route(
                    request.input,
                    output,
                    &rejected,
                )));
                return Plan::new(request, Vec::new(), warnings);
            }
        },
    };

    // (3) The auto-class ceiling. I8. Applies to BOTH selections above; a
    // parameterized operation gets no exemption from it.
    let (class, kinds): (Class, Vec<StepKind>) = match selected {
        Selected::Table(r) => (r.class, r.steps.to_vec()),
        Selected::Steps(kinds, class) => (class, kinds),
    };
    if class > policy.max_auto_class() {
        warnings.push(Warning::AboveAutoClass {
            needed: class,
            allowed: policy.max_auto_class(),
        });
        return Plan::new(request, Vec::new(), warnings);
    }

    if class > Class::A {
        warnings.push(Warning::Lossy { class });
    }

    // (4) and (5): every step gets limits and isolation, non-optionally.
    //
    // Isolation is per STEP, not per request. It was one value for the whole
    // plan, which held while every row for a given pair ran in the same place
    // -- and stopped holding when `mp4 -> mkv` gained a second row: the Class A
    // stream copy is in-process container surgery, and the Class B transcode
    // is the Video module in its own container. One value for both marked the
    // stream copy sandboxed and it failed with "no engine handles a stream
    // copy".
    //
    // Container surgery is always ours and always in-process. Everything else
    // takes the computed value.
    let steps = kinds
        .iter()
        .map(|&kind| Step {
            kind,
            class,
            limits,
            isolation: match kind {
                StepKind::StreamCopy | StepKind::Trim { .. } => Isolation::InProcess,
                _ => isolation,
            },
        })
        .collect();

    Plan::new(request, steps, warnings)
}

/// The first route whose requirements all hold, recording the rejections.
fn pick(
    request: PlanRequest,
    props: Properties,
    env: &Environment,
    rejected: &mut Vec<Rejected>,
) -> Option<Route> {
    let output = request.target.output_format(request.input);

    // An operation that preserves the format is its own route and does not
    // need a table entry. Metadata stripping is the case: it is Class A by
    // construction, because nothing is decoded.
    //
    // The parameterized operations (`Trim`, `RenderPage`) are NOT this case:
    // their steps carry runtime values no static slice can hold, so `route()`
    // builds them before ever calling this function.
    if let Target::Operation(op) = request.target {
        return Some(match op {
            Operation::StripMetadata => Route {
                from: request.input,
                to: request.input,
                steps: &[StepKind::StripMetadata],
                class: Class::A,
                requires: &[],
            },
            Operation::Remux => Route {
                from: request.input,
                to: request.input,
                steps: &[StepKind::StreamCopy],
                class: Class::A,
                requires: &[],
            },
            // Both keep the format and the pixel grid; only the class differs,
            // and it differs because greyscale throws colour away. Naming that
            // here, next to each other, is what stops the two being treated as
            // one "pixel op" with a shared class later.
            Operation::Invert => Route {
                from: request.input,
                to: request.input,
                steps: &[StepKind::Pixel { op: "invert" }],
                class: Class::A,
                requires: &[],
            },
            Operation::Greyscale => Route {
                from: request.input,
                to: request.input,
                steps: &[StepKind::Pixel { op: "greyscale" }],
                class: Class::B,
                requires: &[],
            },
            Operation::Trim { .. }
            | Operation::RenderPage { .. }
            | Operation::RemoveBackground { .. }
            | Operation::Upscale { .. }
            | Operation::Denoise => {
                unreachable!("parameterized operations are selected before pick()")
            }
        });
    }

    for r in env.routes().routes_for(request.input, output) {
        match r.requires.iter().find(|req| !req.holds(env, props, output)) {
            None => return Some(*r),
            Some(&failed) => rejected.push(Rejected { route: *r, failed }),
        }
    }
    None
}

/// Why nothing matched â€” generated, not enumerated.
///
/// Cross-kind pairs get a **generated** reason rather than a table entry. That
/// is what keeps adding a format O(1): v0.4 required every pair in the
/// catalogue to have an explicit row, so format 21 owed 40 new decisions.
fn explain_no_route(from: FormatId, to: FormatId, rejected: &[Rejected]) -> NoRoute {
    // Rows existed and every one failed. Report the requirement that stopped
    // the most-preferred route -- which is the first, because the table is
    // ordered by preference.
    if let Some(first) = rejected.first() {
        return NoRoute::RequirementUnmet {
            requirement: first.failed,
        };
    }
    match (from.kind(), to.kind()) {
        (Some(a), Some(b)) if a != b => NoRoute::CrossKind,
        _ => NoRoute::UnsupportedPair,
    }
}

/// Limits for this input, before clamping.
fn limits_for(props: Properties, policy: &Policy) -> Limits {
    let mut l = policy.base_limits();
    // Narrow where the content tells us we can. Never widen: that direction is
    // the clamp's job to refuse, and this function never attempts it.
    if let Properties::Image { width, height, .. } = props {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > 0 {
            l.decode_pixels = l.decode_pixels.min(pixels.saturating_mul(2));
        }
    }
    l
}

/// Where this step's parser runs. I10.
///
/// There is still exactly one branch producing `InProcess`, and it is still
/// guarded by the format table's `pure_rust_parser` flag. A second branch is
/// how the v0.4 hole reopens, so this adds a CONJUNCT rather than a branch:
/// in-process now requires both a pure-Rust parser for the input AND a step
/// our in-process code can actually perform.
///
/// # Why the flag alone stopped being enough
///
/// `pure_rust_parser` is a property of a FORMAT, and it answers "can we read
/// this container without a C library". For Matroska the answer is yes, and
/// that is what makes stream copy, trim and concat run here. It does NOT mean
/// we can decode the audio CODEC inside — that is symphonia, in oc-audio.
///
/// The moment `Mkv -> Wav` became a route, the flag sent it in-process, where
/// the transcode arm handed a Matroska file to image-rs and got "the image
/// format could not be determined". The row was right; the isolation rule was
/// answering a question one level too coarse.
///
/// The target comes from the REQUEST, not from the row that gets selected, so
/// computing this before row selection is still sound.
fn isolation_for(
    format: FormatId,
    target: Target,
    policy: &Policy,
    env: &Environment,
) -> Isolation {
    // Defence in depth, opted into. The in-process branch below is guarded by
    // the format table and is sound on its own; this narrows it further for a
    // user who would rather pay a process launch than trust a memory-safe
    // decoder with the host's address space.
    //
    // It only applies where there is a worker to run the step. `has_worker` is
    // the same fact `EngineBin::for_media` encodes, and forcing a step into a
    // kind that has none would produce a plan that cannot execute — the exact
    // failure "available means it finishes" exists to prevent.
    if policy.force_sandbox() {
        // The predicate has to be the one *execution* uses to pick the engine,
        // which keys on the input kind. Asking whether the output kind has a
        // worker gets the same answer for most routes and the wrong one where
        // the two kinds differ, and the wrong answer here is not a missed
        // confinement -- it is a plan that cannot run.
        let to = target.output_format(format);
        if crate::format::MediaKind::conversion_has_worker(format, to) {
            return Isolation::Sandboxed(*env.profile());
        }
    }

    if env.can_run_in_process(format) && !needs_codec_worker(format, target) {
        Isolation::InProcess
    } else {
        Isolation::Sandboxed(*env.profile())
    }
}

/// Whether this conversion needs a worker despite a pure-Rust input parser.
///
/// Two cases, and they are unrelated except in what they answer:
///
/// 1. **Audio extraction out of a video container.** Our own Matroska code
///    moves sample payloads verbatim and never looks inside them, so the codec
///    work belongs to oc-audio. Rewrapping (`Mkv -> Mka`) is not this — that
///    stays a stream copy, in process, Class A.
/// 2. **Anything that runs a model.** A PNG has a pure-Rust parser, so
///    background removal on one was planned `InProcess` and reached the
///    in-process transcode arm, which has no adapter for it. More importantly
///    it MUST not: inference is a large C++ runtime executing weights, and
///    running that in the host process would put the least trustworthy code in
///    the build outside the sandbox everything else is held to.
fn needs_codec_worker(from: FormatId, target: Target) -> bool {
    use crate::format::MediaKind as K;
    match target {
        Target::Operation(
            Operation::RemoveBackground { .. } | Operation::Upscale { .. } | Operation::Denoise,
        ) => true,
        // A DESTINATION OF PLAIN TEXT NEEDS A WORKER EITHER WAY.
        //
        // It used to say text was "inference by construction", which stopped
        // being true when `docx -> txt` and `pdf -> txt` landed: those read
        // text the file already contains. The conclusion is unchanged and the
        // reason is now simpler -- nothing produces text in our own address
        // space. It is whisper in oc-ai, paddleocr in oc-ai, or a parser in
        // oc-pdf, and all three are workers.
        Target::Format(FormatId::Txt) => true,
        // DOCUMENTS ARE READ AND WRITTEN BY oc-pdf, NEVER IN OUR OWN PROCESS.
        //
        // The one in-process step with a document destination GENERATES a PDF
        // around an image and parses nothing, which is what makes it safe
        // without a sandbox. It is not a general document writer. Everything
        // else with a document on either side -- txt, markdown, html, docx,
        // odt, epub, a notebook, a deck -- has to be parsed and typeset, and
        // that is the worker.
        //
        // Without this, `txt -> pdf` and `txt -> docx` planned `InProcess`
        // because Txt has a pure-Rust parser and nothing said otherwise, then
        // handed a text file to the image embedder and died with "the image
        // format could not be determined". Both routes were declared, survived
        // planning, and failed at the last step -- the exact class
        // `every_declared_document_route_runs` exists to catch. It caught them.
        //
        // The image exception is written as an exception, so a new document
        // format is confined by default and only an image source can opt out.
        Target::Format(to)
            if (from.kind() == Some(K::Document) || to.kind() == Some(K::Document))
                && from.kind() != Some(K::Image) =>
        {
            true
        }
        // PIXEL OPERATIONS LIVE IN oc-images, AND ONLY THERE.
        //
        // Inverting and greyscaling a PNG were planned `InProcess`, because
        // PNG has a pure-Rust parser and nothing said otherwise -- and the
        // in-process engine handles `Transcode` and nothing else, so every one
        // of those plans reached `Unsupported(Pixel { .. })` and failed. Two
        // advertised tools that could not run on the most ordinary input they
        // have.
        //
        // `pure_rust_parser` answers "can this be DECODED without a C
        // library". Whether we have an implementation of the operation is a
        // different question, and this is where it is answered.
        Target::Operation(Operation::Invert | Operation::Greyscale) => true,
        // SVG's parser IS pure Rust, and it is still sandboxed. The flag
        // answers "can this be parsed without a C library"; the question here
        // is "should this be parsed in our own address space", and for a
        // renderer that follows a document's instructions the answer is no.
        // A far larger attack surface than a PNG decoder, for the price of one
        // process launch.
        _ if from == FormatId::Svg => true,
        Target::Format(to) => {
            // Audio extraction that decodes, and video transcode. Neither runs
            // in our address space: the first is symphonia in oc-audio, the
            // second is the Video module in its own container. Matroska's
            // `pure_rust_parser` flag is about reading the CONTAINER, and
            // saying "in-process" of either would put a false claim on the
            // receipt.
            let a =
                from.kind() == Some(K::Video) && to.kind() == Some(K::Audio) && to != FormatId::Mka;
            let b = from.kind() == Some(K::Video) && to.kind() == Some(K::Video);
            // AVIF encoding is provided by ravif in oc-images. The host's
            // deliberately smaller image-rs build does not carry that encoder,
            // so a pure-Rust PNG/JPEG decoder does not make the whole route an
            // in-process route.
            let c = to == FormatId::Avif;
            a || b || c
        }
        Target::Operation(_) => false,
    }
}

/// OCR's three artifacts, in the order the adapter expects them.
///
/// Named once because six route rows share it, and a row that listed them in a
/// different order would hand the recogniser to the detector with no error at
/// all — just nonsense.
const OCR_MODELS: &[&str] = &["paddleocr-det", "paddleocr-rec", "paddleocr-dict"];

/// Enhancement's graph and the constants that go with it.
const DENOISE_MODELS: &[&str] = &["deepfilternet", "deepfilternet-aux"];

/// Whisper's three artifacts, in the order the adapter expects them.
const ASR_MODELS: &[&str] = &[
    "whisper-encoder",
    "whisper-decoder",
    "whisper-tokenizer",
    // Part of the pipeline, not an optional extra: without it a silent
    // stretch produces confident invented text.
    "silero-vad",
];

macro_rules! route {
    ($from:ident -> $to:ident, $class:ident, [$($step:expr),*], [$($req:expr),*]) => {
        Route {
            from: FormatId::$from,
            to: FormatId::$to,
            steps: &[$($step),*],
            class: Class::$class,
            requires: &[$($req),*],
        }
    };
}

impl RouteTable {
    /// The v1 route table.
    ///
    /// **Order within a pair is preference order.** Lossless first, always: the
    /// `MKV -> MP4` pair has stream-copy above transcode, so a file whose codecs
    /// already fit the container is copied rather than re-encoded, and the user
    /// is told *why* â€” "stream copy, because the codecs are compatible; the
    /// alternative was re-encode" falls out of the requirement that failed.
    #[must_use]
    pub fn v1() -> Self {
        use StepKind::Extract as E;
        use StepKind::Transcode as T;
        Self::new(vec![
            // ---- containers: the case the whole ordering exists for ----
            //
            // Class A only. The Class B re-encode rows were removed, not
            // deferred silently: video transcoding needs the separately
            // downloaded FFmpeg module (`02` Â§3.2), and a row whose execution
            // cannot finish must not be routable â€” "available means a plan
            // routed here finishes" is what keeps the table honest. The rows
            // return when the module does.
            route!(Mkv  -> Mp4,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Webm -> Mp4,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            // EBML â†’ EBML: same elements, new header. WebM's strict subset
            // rule still applies to Mkv â†’ Webm; the reverse is unconditional,
            // because Matroska carries everything WebM does by definition.
            route!(Webm -> Mkv,  A, [StepKind::StreamCopy], []),
            route!(Mkv  -> Webm, A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            // Audio extraction: the video track is dropped (and disclosed),
            // so only the AUDIO codec must fit the destination.
            route!(Mkv  -> Mka,  A, [StepKind::StreamCopy], [Requirement::AudioCompatible]),
            route!(Webm -> Mka,  A, [StepKind::StreamCopy], [Requirement::AudioCompatible]),
            // THE DIRECTION THAT DID NOT EXIST. `mp4mux` muxed from the day it
            // landed and nothing read one back, so the most common video
            // container in the world was a destination and never a source.
            //
            // Gated exactly like its mirror images: Matroska carries anything
            // whose codec we can identify, WebM is the strict subset, and an
            // unidentified codec is never carryable in either direction.
            route!(Mp4  -> Mkv,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Mp4  -> Webm, A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Mp4  -> Mka,  A, [StepKind::StreamCopy], [Requirement::AudioCompatible]),
            // QUICKTIME, WHICH IS THE SAME CONTAINER. `mov -> mp4` is the one
            // people ask for most, and it is a stream copy: the spike ran real
            // exports and synthesised files through `mp4demux` unchanged and
            // got the coded samples back byte-identical.
            //
            // `StreamsCompatible` is what makes the CODEC question honest.
            // ProRes and QuickTime RLE have no MP4 representation, so a file
            // carrying them fails this requirement and is refused with the
            // reason rather than re-encoded under a Class A receipt. The three
            // other destinations mirror MP4's own rows for the same reasons
            // those carry.
            route!(Mov  -> Mp4,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Mov  -> Mkv,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Mov  -> Webm, A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Mov  -> Mka,  A, [StepKind::StreamCopy], [Requirement::AudioCompatible]),
            // AVI, WHICH IS A REFRAMING RATHER THAN A REPACK — and is still
            // Class A, because the coded pictures are byte-identical.
            //
            // AVI stores H.264 as Annex-B: NAL units behind start codes, with
            // the parameter sets in band. MP4 wants AVCC: the same NALs behind
            // their lengths, with the parameter sets in the sample entry. Not
            // one macroblock changes, so nothing about the picture is lost —
            // what changes is where the file says each unit ends.
            //
            // `StreamsCompatible` still gates it. An AVI carrying Motion JPEG
            // or MPEG-4 Part 2 fails that requirement and is refused with the
            // reason, rather than re-encoded under a lossless receipt.
            route!(Avi  -> Mp4,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Avi  -> Mkv,  A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Avi  -> Webm, A, [StepKind::StreamCopy], [Requirement::StreamsCompatible]),
            route!(Avi  -> Mka,  A, [StepKind::StreamCopy], [Requirement::AudioCompatible]),
            // ---- Class B video: the transcode module ----
            //
            // These are the rows `02-FEATURES` §3.2 describes, and they are
            // the ONLY rows in this table gated on a separate download. Core
            // ships no video encoder on purpose -- a patent posture and a CVE
            // posture, not a scheduling gap -- so `available` here means "the
            // module is installed", and without it these plan nothing.
            //
            // Class B, and the ordering matters: the Class A stream-copy rows
            // above are tried FIRST, so a file whose codecs already fit is
            // rewrapped in seconds rather than re-encoded for minutes. These
            // are what happens when that fails, which is the case the old
            // `StreamsCompatible` refusal had no answer for.
            //
            // AV1 + Opus (AAC into MP4). No H.264 encoder: that needs x264,
            // which is GPL and which `cargo xtask engines` refuses whatever
            // the linkage.
            route!(Mp4  -> Webm, B, [T { from: FormatId::Mp4,  to: FormatId::Webm }], [Requirement::Engine("ffmpeg")]),
            route!(Mp4  -> Mkv,  B, [T { from: FormatId::Mp4,  to: FormatId::Mkv  }], [Requirement::Engine("ffmpeg")]),
            route!(Mkv  -> Mp4,  B, [T { from: FormatId::Mkv,  to: FormatId::Mp4  }], [Requirement::Engine("ffmpeg")]),
            route!(Mkv  -> Webm, B, [T { from: FormatId::Mkv,  to: FormatId::Webm }], [Requirement::Engine("ffmpeg")]),
            route!(Webm -> Mp4,  B, [T { from: FormatId::Webm, to: FormatId::Mp4  }], [Requirement::Engine("ffmpeg")]),
            route!(Webm -> Mkv,  B, [T { from: FormatId::Webm, to: FormatId::Mkv  }], [Requirement::Engine("ffmpeg")]),
            // Extraction that DECODES, as opposed to the stream copies above.
            //
            // `-> Mka` rewraps the audio track untouched and is Class A.
            // These rows decode it and re-encode into a real audio format,
            // which is Class B even into FLAC: the source track is whatever
            // the container held, and calling a decode/re-encode lossless
            // because the DESTINATION is lossless would be a claim about the
            // wrong end. oc-audio reads Matroska through symphonia's `mkv`
            // feature -- enabled when M4A decode landed, and until now not
            // reachable by any route.
            //
            // The video track is dropped, and the receipt lists it: the worker
            // counts every track it did not take.
            //
            // There are no `Mka ->` rows on purpose. An `.mka` has MKV's
            // signature and MKV's DocType, so `sniff` resolves it to `Mkv`
            // every time -- Mka is a destination this table can reach and not
            // a source it can ever be handed. A row for it would be dead.
            route!(Mkv  -> Wav,  B, [T { from: FormatId::Mkv,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Mkv  -> Flac, B, [T { from: FormatId::Mkv,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Mkv  -> Mp3,  B, [T { from: FormatId::Mkv,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Mkv  -> Ogg,  B, [T { from: FormatId::Mkv,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(Webm -> Wav,  B, [T { from: FormatId::Webm, to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Webm -> Flac, B, [T { from: FormatId::Webm, to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Webm -> Mp3,  B, [T { from: FormatId::Webm, to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Webm -> Ogg,  B, [T { from: FormatId::Webm, to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            // MP4 AUDIO. The decoder has been in oc-audio since symphonia's
            // `isomp4` feature was enabled for M4A, and no route ever reached
            // it -- so the most common video container in the world was
            // output-only: this build could WRITE an mp4 by remuxing and
            // could not read one back.
            //
            // These are the audio half only. `Mp4 -> Mkv` and friends need a
            // demuxer that does not exist yet (`mp4mux.rs` muxes and nothing
            // more), and a row whose execution cannot finish must not be
            // routable.
            route!(Mp4  -> Wav,  B, [T { from: FormatId::Mp4,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Mp4  -> Flac, B, [T { from: FormatId::Mp4,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Mp4  -> Mp3,  B, [T { from: FormatId::Mp4,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Mp4  -> Ogg,  B, [T { from: FormatId::Mp4,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(Mov  -> Wav,  B, [T { from: FormatId::Mov,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Mov  -> Flac, B, [T { from: FormatId::Mov,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Mov  -> Mp3,  B, [T { from: FormatId::Mov,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Mov  -> Ogg,  B, [T { from: FormatId::Mov,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            // ---- images: in-process, pure Rust, no engine required ----
            //
            // The full raster matrix among the six formats image-rs both
            // reads and writes. Everything here is Class B: a decode/re-encode
            // round trip is not assumed lossless until measured as such (the
            // lone pre-existing Class A row, Bmp -> Png, predates this rule
            // and keeps its class).
            route!(Png  -> Jpeg, B, [T { from: FormatId::Png,  to: FormatId::Jpeg }], []),
            route!(Png  -> Webp, B, [T { from: FormatId::Png,  to: FormatId::Webp }], []),
            route!(Png  -> Gif,  B, [T { from: FormatId::Png,  to: FormatId::Gif  }], []),
            route!(Png  -> Bmp,  B, [T { from: FormatId::Png,  to: FormatId::Bmp  }], []),
            route!(Png  -> Tiff, B, [T { from: FormatId::Png,  to: FormatId::Tiff }], []),
            route!(Png  -> Avif, B, [T { from: FormatId::Png,  to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Jpeg -> Png,  B, [T { from: FormatId::Jpeg, to: FormatId::Png  }], []),
            route!(Jpeg -> Webp, B, [T { from: FormatId::Jpeg, to: FormatId::Webp }], []),
            route!(Jpeg -> Gif,  B, [T { from: FormatId::Jpeg, to: FormatId::Gif  }], []),
            route!(Jpeg -> Bmp,  B, [T { from: FormatId::Jpeg, to: FormatId::Bmp  }], []),
            route!(Jpeg -> Tiff, B, [T { from: FormatId::Jpeg, to: FormatId::Tiff }], []),
            route!(Jpeg -> Avif, B, [T { from: FormatId::Jpeg, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Webp -> Png,  B, [T { from: FormatId::Webp, to: FormatId::Png  }], []),
            route!(Webp -> Jpeg, B, [T { from: FormatId::Webp, to: FormatId::Jpeg }], []),
            route!(Webp -> Gif,  B, [T { from: FormatId::Webp, to: FormatId::Gif  }], []),
            route!(Webp -> Bmp,  B, [T { from: FormatId::Webp, to: FormatId::Bmp  }], []),
            route!(Webp -> Tiff, B, [T { from: FormatId::Webp, to: FormatId::Tiff }], []),
            route!(Gif  -> Png,  B, [T { from: FormatId::Gif,  to: FormatId::Png  }], []),
            route!(Gif  -> Jpeg, B, [T { from: FormatId::Gif,  to: FormatId::Jpeg }], []),
            route!(Gif  -> Webp, B, [T { from: FormatId::Gif,  to: FormatId::Webp }], []),
            route!(Gif  -> Bmp,  B, [T { from: FormatId::Gif,  to: FormatId::Bmp  }], []),
            route!(Gif  -> Tiff, B, [T { from: FormatId::Gif,  to: FormatId::Tiff }], []),
            route!(Bmp  -> Png,  A, [T { from: FormatId::Bmp,  to: FormatId::Png  }], []),
            route!(Bmp  -> Jpeg, B, [T { from: FormatId::Bmp,  to: FormatId::Jpeg }], []),
            route!(Bmp  -> Webp, B, [T { from: FormatId::Bmp,  to: FormatId::Webp }], []),
            route!(Bmp  -> Gif,  B, [T { from: FormatId::Bmp,  to: FormatId::Gif  }], []),
            route!(Bmp  -> Tiff, B, [T { from: FormatId::Bmp,  to: FormatId::Tiff }], []),
            route!(Tiff -> Png,  B, [T { from: FormatId::Tiff, to: FormatId::Png  }], []),
            route!(Tiff -> Jpeg, B, [T { from: FormatId::Tiff, to: FormatId::Jpeg }], []),
            route!(Tiff -> Webp, B, [T { from: FormatId::Tiff, to: FormatId::Webp }], []),
            route!(Tiff -> Gif,  B, [T { from: FormatId::Tiff, to: FormatId::Gif  }], []),
            route!(Tiff -> Bmp,  B, [T { from: FormatId::Tiff, to: FormatId::Bmp  }], []),
            // ---- image -> PDF: structural wrapping, in-process ----
            //
            // The PDF is GENERATED here rather than parsed, which is why this
            // is safe without a sandbox on the input side too: a JPEG's bytes
            // are embedded verbatim as a DCT stream and never interpreted as
            // PostScript. Rendering directions (PDF -> image) are the ones
            // that need pdfium, and those wait for oc-pdf.
            route!(Jpeg -> Pdf,  B, [T { from: FormatId::Jpeg, to: FormatId::Pdf  }], []),
            route!(Png  -> Pdf,  B, [T { from: FormatId::Png,  to: FormatId::Pdf  }], []),
            // The other four rasters reach PDF through a lossless PNG re-encode
            // (see `pdf::wrap_image`): PDF speaks DCTDecode and FlateDecode and
            // nothing else we can reach, so TIFF, WebP, GIF and BMP have to be
            // re-expressed rather than embedded. Only the first image of an
            // animation is wrapped, and the receipt says so.
            route!(Tiff -> Pdf,  B, [T { from: FormatId::Tiff, to: FormatId::Pdf  }], []),
            route!(Webp -> Pdf,  B, [T { from: FormatId::Webp, to: FormatId::Pdf  }], []),
            route!(Gif  -> Pdf,  B, [T { from: FormatId::Gif,  to: FormatId::Pdf  }], []),
            route!(Bmp  -> Pdf,  B, [T { from: FormatId::Bmp,  to: FormatId::Pdf  }], []),
            // ---- images needing a C engine ----
            route!(Heic -> Jpeg, B, [T { from: FormatId::Heic, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Png,  B, [T { from: FormatId::Heic, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            // AVIF is back, and this is the comment it replaces:
            //
            //   "Avif row REMOVED: no AV1 decoder in engine set. Returns when
            //    dav1d/aom linked."
            //
            // It returned through neither. `rav1d` is a Rust port of dav1d
            // under BSD-2, so there is no native library, no engines.toml row
            // and no DLL beside the worker -- the same shape as jxl-oxide.
            //
            // 8-bit only: the 16-bit path roughly doubles what a full AV1
            // decoder compiles to, for images this build refuses on pixel
            // budget anyway. A 10-bit AVIF refuses by name.
            route!(Avif -> Png,  B, [T { from: FormatId::Avif, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Avif -> Jpeg, B, [T { from: FormatId::Avif, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Avif -> Webp, B, [T { from: FormatId::Avif, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Png,  B, [T { from: FormatId::Jxl,  to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Jpeg, B, [T { from: FormatId::Jxl,  to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            // ---- the rest of the raster matrix -------------------------
            //
            // Seventeen pairs were missing here, and the gaps were not
            // principled:
            // AVIF could be reached from PNG and JPEG but not from WebP, GIF,
            // BMP or TIFF; a HEIC from a phone could become a PNG or a JPEG and
            // nothing else. `webp -> avif` was reported as a bug, and it was one
            // of seventeen.
            //
            // THE RULE THE TABLE NOW FOLLOWS, WITHOUT EXCEPTION: anything
            // `oc-images` DECODES can reach anything it ENCODES.
            //
            // Decodes: the six image-rs formats, plus AVIF, HEIC, JXL, SVG and
            // the five camera-raw formats -- sixteen sources counting PDF,
            // which arrives here through a render step. Encodes: the six plus
            // AVIF. NOT HEIC or JXL, which have no encoder in this build, and
            // not SVG, because rasterised pixels cannot become a vector
            // document again. Those three stay absent as targets, because a
            // route that cannot run is worse than one that is missing.
            //
            // That is 105 pairs and the table has all of them. The rule is
            // stated here rather than left implicit precisely because the
            // seventeen missing pairs were not a decision anyone made -- they
            // were the residue of adding decoders one at a time and writing
            // only the rows each was added for. A test enumerates the matrix
            // so the next decoder cannot repeat it.
            route!(Webp -> Avif, B, [T { from: FormatId::Webp, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Gif  -> Avif, B, [T { from: FormatId::Gif,  to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Bmp  -> Avif, B, [T { from: FormatId::Bmp,  to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Tiff -> Avif, B, [T { from: FormatId::Tiff, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Avif -> Gif , B, [T { from: FormatId::Avif, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Avif -> Bmp , B, [T { from: FormatId::Avif, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Avif -> Tiff, B, [T { from: FormatId::Avif, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Webp, B, [T { from: FormatId::Heic, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Gif , B, [T { from: FormatId::Heic, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Bmp , B, [T { from: FormatId::Heic, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Tiff, B, [T { from: FormatId::Heic, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Heic -> Avif, B, [T { from: FormatId::Heic, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Webp, B, [T { from: FormatId::Jxl,  to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Gif , B, [T { from: FormatId::Jxl,  to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Bmp , B, [T { from: FormatId::Jxl,  to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Tiff, B, [T { from: FormatId::Jxl,  to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Jxl  -> Avif, B, [T { from: FormatId::Jxl,  to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            // SVG rasterisation. Class B and not D: nothing is invented — the
            // document says exactly what to draw, and what is lost is that it
            // stops being resizable. Text is not drawn (no fonts in the
            // sandbox) and the engine string on the receipt says so.
            route!(Svg  -> Png,  B, [T { from: FormatId::Svg,  to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Jpeg, B, [T { from: FormatId::Svg,  to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Webp, B, [T { from: FormatId::Svg,  to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Bmp,  B, [T { from: FormatId::Svg,  to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Tiff, B, [T { from: FormatId::Svg,  to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Gif,  B, [T { from: FormatId::Svg,  to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Svg  -> Avif, B, [T { from: FormatId::Svg,  to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            // NO `Svg -> Pdf` ROW. It would need two steps -- rasterise in
            // the worker, then wrap in-process -- and this table's steps all
            // run in one place. `svg -> png -> pdf` does it in two commands
            // and says so; a row that silently could not finish would not.

            // ---- PDF rendering (oc-pdf) ----
            //
            // Page selection (`--page N`) rides the RenderPage operation; these
            // plain-format rows render page 0, which is what they always did.
            route!(Pdf  -> Png,  B, [T { from: FormatId::Pdf,  to: FormatId::Png  }], [Requirement::Engine("oc-pdf")]),
            route!(Pdf  -> Jpeg, B, [T { from: FormatId::Pdf,  to: FormatId::Jpeg }], [Requirement::Engine("oc-pdf")]),
            // oc-pdf renders a lossless PNG first; oc-images then encodes the
            // requested raster. Multi-step execution tracks the format emitted
            // by each step, so the second worker is handed PNG bytes as PNG.
            route!(Pdf  -> Webp, B, [T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Webp }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Pdf  -> Gif,  B, [T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Gif  }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Pdf  -> Bmp,  B, [T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Bmp  }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Pdf  -> Tiff, B, [T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Tiff }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Pdf  -> Avif, B, [T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Avif }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            // ---- Camera RAW decode (libraw via oc-images) ----
            //
            // CR2 was sniffed long before it routed; these rows are what made
            // that honest. Decode runs through libraw exactly as CR3/NEF/ARW/
            // DNG already did.
            //
            // ALL SEVEN TARGETS, not the two these started with. A raw file
            // decodes to the same `DynamicImage` every other source produces,
            // and `oc-images` encodes it with the same `encode_image` -- there
            // was never a technical reason `cr2 -> jpeg` worked and
            // `cr2 -> tiff` did not, only the order the rows were written in.
            // A photographer archiving to TIFF or shrinking to AVIF was told
            // the conversion did not exist while the code to do it ran on the
            // next row down.
            route!(Cr2 -> Jpeg, B, [T { from: FormatId::Cr2, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Png,  B, [T { from: FormatId::Cr2, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Webp, B, [T { from: FormatId::Cr2, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Gif,  B, [T { from: FormatId::Cr2, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Bmp,  B, [T { from: FormatId::Cr2, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Tiff, B, [T { from: FormatId::Cr2, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Avif, B, [T { from: FormatId::Cr2, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Jpeg, B, [T { from: FormatId::Cr3, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Png,  B, [T { from: FormatId::Cr3, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Webp, B, [T { from: FormatId::Cr3, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Gif,  B, [T { from: FormatId::Cr3, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Bmp,  B, [T { from: FormatId::Cr3, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Tiff, B, [T { from: FormatId::Cr3, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Avif, B, [T { from: FormatId::Cr3, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Jpeg, B, [T { from: FormatId::Nef, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Png,  B, [T { from: FormatId::Nef, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Webp, B, [T { from: FormatId::Nef, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Gif,  B, [T { from: FormatId::Nef, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Bmp,  B, [T { from: FormatId::Nef, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Tiff, B, [T { from: FormatId::Nef, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Avif, B, [T { from: FormatId::Nef, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Jpeg, B, [T { from: FormatId::Arw, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Png,  B, [T { from: FormatId::Arw, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Webp, B, [T { from: FormatId::Arw, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Gif,  B, [T { from: FormatId::Arw, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Bmp,  B, [T { from: FormatId::Arw, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Tiff, B, [T { from: FormatId::Arw, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Avif, B, [T { from: FormatId::Arw, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Jpeg, B, [T { from: FormatId::Dng, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Png,  B, [T { from: FormatId::Dng, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Webp, B, [T { from: FormatId::Dng, to: FormatId::Webp }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Gif,  B, [T { from: FormatId::Dng, to: FormatId::Gif  }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Bmp,  B, [T { from: FormatId::Dng, to: FormatId::Bmp  }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Tiff, B, [T { from: FormatId::Dng, to: FormatId::Tiff }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Avif, B, [T { from: FormatId::Dng, to: FormatId::Avif }], [Requirement::Engine("oc-images")]),
            // ---- audio ----
            //
            // Lossless pairs first; lossy-source decodes after.
            //
            // MP3 and Opus OUTPUT rows name the linked encoder libraries
            // directly, exactly as engines.toml does. The registry reports a
            // library available only when its DLL sits beside the worker, so
            // "routed" still means "finishes": an install without the encoder
            // DLLs is refused with the missing library named, not failed at
            // the last step.
            route!(Wav  -> Flac, A, [T { from: FormatId::Wav,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Flac -> Wav,  A, [T { from: FormatId::Flac, to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Mp3  -> Wav,  B, [T { from: FormatId::Mp3,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Ogg  -> Wav,  B, [T { from: FormatId::Ogg,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(Wav  -> Mp3,  B, [T { from: FormatId::Wav,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Flac -> Mp3,  B, [T { from: FormatId::Flac, to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Mp3  -> Mp3,  B, [T { from: FormatId::Mp3,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Ogg  -> Mp3,  B, [T { from: FormatId::Ogg,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(M4a  -> Mp3,  B, [T { from: FormatId::M4a,  to: FormatId::Mp3  }], [Requirement::Engine("libmp3lame")]),
            route!(Wav  -> Ogg,  B, [T { from: FormatId::Wav,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(Flac -> Ogg,  B, [T { from: FormatId::Flac, to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(Mp3  -> Ogg,  B, [T { from: FormatId::Mp3,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(Ogg  -> Ogg,  B, [T { from: FormatId::Ogg,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            route!(M4a  -> Ogg,  B, [T { from: FormatId::M4a,  to: FormatId::Ogg  }], [Requirement::Engine("libopus")]),
            // M4A/AAC/ALAC decode arrived with symphonia's isomp4/aac/alac
            // features; these rows are what make that honest.
            route!(M4a  -> Wav,  B, [T { from: FormatId::M4a,  to: FormatId::Wav  }], [Requirement::Engine("oc-audio")]),
            route!(M4a  -> Flac, B, [T { from: FormatId::M4a,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            // MP3 AND OGG TO FLAC. These were the only two decodable sources
            // that could not reach FLAC, and the asymmetry was an accident,
            // not a policy: `m4a -> flac` is the same conversion from the same
            // kind of source and it was already here.
            //
            // A lossy source cannot be made lossless again and this row does
            // not pretend otherwise -- it is Class B, like every other lossy
            // decode, because the loss happened upstream and re-encoding
            // cannot undo it. What FLAC gives is a stop to FURTHER loss, which
            // is the reason to want it when a file is about to be edited or
            // archived. Refusing the row does not protect anyone; it just
            // makes them route through WAV and lose the metadata on the way.
            route!(Mp3  -> Flac, B, [T { from: FormatId::Mp3,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            route!(Ogg  -> Flac, B, [T { from: FormatId::Ogg,  to: FormatId::Flac }], [Requirement::Engine("oc-audio")]),
            // ---- archives ----
            //
            // Zip -> Tar was the first repack; Tar -> Zip is its reverse, and
            // Tar -> Gzip compresses without repacking members at all. Both
            // run in oc-archive under the same SR-5 running-total caps.
            route!(Zip      -> Tar,  A, [E { from: FormatId::Zip, to: FormatId::Tar }], [Requirement::Engine("oc-archive")]),
            route!(Tar      -> Zip,  A, [E { from: FormatId::Tar, to: FormatId::Zip }], [Requirement::Engine("oc-archive")]),
            route!(Tar      -> Gzip, A, [E { from: FormatId::Tar, to: FormatId::Gzip }], [Requirement::Engine("oc-archive")]),
            // The inverse, and Class A for the same reason: the bytes that
            // went in come back out. The cap is applied to the OUTPUT, since
            // a gzip's ratio is unbounded and bounding the input bounds
            // nothing.
            route!(Gzip     -> Tar,  A, [E { from: FormatId::Gzip, to: FormatId::Tar }], [Requirement::Engine("oc-archive")]),
            // 7z READS but is never written. `sevenz-rust2` is Apache-2.0 with
            // a pure-Rust LZMA, so this is a dependency rather than a native
            // library -- but the repack is Class B, not A: 7z members are
            // LZMA-compressed and both destinations re-compress or store them
            // differently, so the MEMBERS survive and their encoding does not.
            route!(SevenZip -> Tar,  B, [E { from: FormatId::SevenZip, to: FormatId::Tar }], [Requirement::Engine("oc-archive")]),
            route!(SevenZip -> Zip,  B, [E { from: FormatId::SevenZip, to: FormatId::Zip }], [Requirement::Engine("oc-archive")]),
            // THE CHAINS THROUGH TAR, which is what `.tar.gz` needs.
            //
            // `gzip -> zip` did not exist while both its legs did, and
            // oc-archive refused it deliberately: "gzip to zip would hide a
            // decompress and a repack behind one step." That objection was
            // right about a ONE-step row and is answered by a two-step one.
            // Both steps appear in the plan, both in the receipt, and each is
            // executed by the leg that was already tested.
            //
            // A chain is only as lossless as its worst step, so the classes
            // are not copied from each other: gzip/zip/tar legs are all A, and
            // anything through 7z inherits that row's B — 7z members are
            // LZMA-compressed and the destination stores them differently.
            route!(Gzip     -> Zip,  A, [E { from: FormatId::Gzip, to: FormatId::Tar }, E { from: FormatId::Tar, to: FormatId::Zip }], [Requirement::Engine("oc-archive")]),
            route!(Zip      -> Gzip, A, [E { from: FormatId::Zip,  to: FormatId::Tar }, E { from: FormatId::Tar, to: FormatId::Gzip }], [Requirement::Engine("oc-archive")]),
            route!(SevenZip -> Gzip, B, [E { from: FormatId::SevenZip, to: FormatId::Tar }, E { from: FormatId::Tar, to: FormatId::Gzip }], [Requirement::Engine("oc-archive")]),
            // ---- CAD ----
            //
            // ONE FORMAT, TWO TARGETS, AND A CLOSED SET OF ENTITIES. Forty-five
            // real drawings were read before these rows existed, and between
            // them they hold five entity types: LWPOLYLINE (with bulges, which
            // are arcs), CIRCLE, LINE, ARC and SPLINE. That is what `dxf.rs`
            // reads, and anything else is counted and named in the receipt.
            //
            // **Class C.** The curves are evaluated and flattened into
            // polylines, so what comes out is a faithful drawing of the same
            // geometry rather than the geometry itself — and everything that is
            // not geometry (layers, line types, text, dimensions, blocks) is
            // gone. B would call that the usual cost of changing format; it is
            // a redrawing.
            //
            // PDF is the one people ask for and SVG is the more useful: SVG
            // keeps the drawing's own units, where a PDF has to scale it onto a
            // page — and the receipt says by how much.
            route!(Dxf  -> Pdf,      C, [T { from: FormatId::Dxf, to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            route!(Dxf  -> Svg,      C, [T { from: FormatId::Dxf, to: FormatId::Svg }], [Requirement::Engine("oc-pdf")]),
            // ---- decks, books and workbooks: the other ZIPs of XML ----
            //
            // PPTX, ODP and EPUB are the same shape as the DOCX and ODT above:
            // a ZIP holding XML, scanned by the same bounded scanner, feeding
            // the same `Paragraph` model and the same six writers. That is why
            // these are eighteen rows and three reader modules rather than a
            // new engine.
            //
            // **Class C, not B.** `docx -> markdown` is B because a Word file
            // states its paragraphs and its heading levels, and what is lost is
            // formatting. A deck states neither: a slide is shapes at
            // positions, and turning it into a title and a bulleted list is a
            // reading of the file rather than a translation of it. The same is
            // true of a book's chapters becoming pages. C says "rebuilt", which
            // is what happened.
            //
            // `-> txt` is C for the same reason and not B, which is the class
            // `docx -> txt` carries: there is no structure to lose in plain
            // text, but there was structure INVENTED on the way to it.
            route!(Pptx  -> Txt,      C, [T { from: FormatId::Pptx, to: FormatId::Txt }], [Requirement::Engine("oc-pdf")]),
            route!(Pptx  -> Markdown, C, [T { from: FormatId::Pptx, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Pptx  -> Html,     C, [T { from: FormatId::Pptx, to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            route!(Pptx  -> Pdf,      C, [T { from: FormatId::Pptx, to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            route!(Pptx  -> Docx,     C, [T { from: FormatId::Pptx, to: FormatId::Docx }], [Requirement::Engine("oc-pdf")]),
            route!(Pptx  -> Odt,      C, [T { from: FormatId::Pptx, to: FormatId::Odt }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Txt,      C, [T { from: FormatId::Odp, to: FormatId::Txt }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Markdown, C, [T { from: FormatId::Odp, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Html,     C, [T { from: FormatId::Odp, to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Pdf,      C, [T { from: FormatId::Odp, to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Docx,     C, [T { from: FormatId::Odp, to: FormatId::Docx }], [Requirement::Engine("oc-pdf")]),
            route!(Odp   -> Odt,      C, [T { from: FormatId::Odp, to: FormatId::Odt }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Txt,      C, [T { from: FormatId::Epub, to: FormatId::Txt }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Markdown, C, [T { from: FormatId::Epub, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Html,     C, [T { from: FormatId::Epub, to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Pdf,      C, [T { from: FormatId::Epub, to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Docx,     C, [T { from: FormatId::Epub, to: FormatId::Docx }], [Requirement::Engine("oc-pdf")]),
            route!(Epub  -> Odt,      C, [T { from: FormatId::Epub, to: FormatId::Odt }], [Requirement::Engine("oc-pdf")]),
            // A WORKBOOK IS NOT A DOCUMENT, and these rows say so by going
            // nowhere near one. `xlsx -> pdf` would mean rendering a grid,
            // which is a layout engine this build does not have; what a
            // workbook converts to here is its data.
            //
            // Class C: one sheet of the several, values in place of formulas,
            // and a date left as the serial number the cell holds because the
            // epoch is the workbook's own. Every one of those is in the
            // receipt. B would claim this is the usual cost of changing format
            // and it is not -- a CSV is a different thing from a workbook.
            route!(Xlsx  -> Csv,      C, [T { from: FormatId::Xlsx, to: FormatId::Csv }], [Requirement::Engine("oc-pdf")]),
            route!(Xlsx  -> Json,     C, [T { from: FormatId::Xlsx, to: FormatId::Json }], [Requirement::Engine("oc-pdf")]),
            route!(Ods   -> Csv,      C, [T { from: FormatId::Ods, to: FormatId::Csv }], [Requirement::Engine("oc-pdf")]),
            route!(Ods   -> Json,     C, [T { from: FormatId::Ods, to: FormatId::Json }], [Requirement::Engine("oc-pdf")]),
            // THE ONE WORKBOOK-TO-WORKBOOK ROW, and the one that keeps every
            // sheet. CSV and JSON hold a single grid, so those rows take the
            // first sheet and the receipt names what was left; XLSX holds as
            // many as the source did, and the cell TYPES survive with them --
            // a number written as text is a cell Excel will not sum, which is
            // a defect in the file rather than a cosmetic loss.
            //
            // Still Class C. The output is rebuilt from cached values: a sheet
            // whose point was its formulas comes out as the answers without
            // the working, and B would call that the usual cost of changing
            // format.
            //
            // There is no `Xlsx -> Ods`. It would need an OpenDocument writer,
            // which is a second dependency for the mirror of a row that
            // already exists, and nobody has asked for it.
            route!(Ods   -> Xlsx,     C, [T { from: FormatId::Ods, to: FormatId::Xlsx }], [Requirement::Engine("oc-pdf")]),
            // ---- fonts ----
            //
            // SEVEN ROWS, AND THE TWO THAT ARE MISSING ARE THE POINT.
            //
            // TTF and OTF are the same SFNT container carrying different
            // outlines: `glyf` quadratics in one, `CFF ` cubics in the other.
            // WOFF and WOFF2 wrap that container and compress it -- zlib per
            // table, Brotli over the lot -- so every row here is a repack, the
            // tables come out byte-identical, and the class is A.
            //
            // **There is no `otf -> ttf` and no `ttf -> otf`**, though every
            // converter on the web offers them and CloudConvert calls the first
            // one common. What those tools do is convert the OUTLINES, cubic
            // Béziers to quadratic, which is an approximation of every curve in
            // the font and cannot be undone. Renaming the container instead --
            // shipping CFF outlines in a file called `.ttf` -- is the other
            // thing that gets done under that name, and it produces a file half
            // the world will not render. Offering either under a Class A label
            // would be a lie about the more important half of the conversion.
            //
            // **WOFF2 is a destination only.** Writing one needs a Brotli
            // encoder and the null transform, which is exact. Reading an
            // arbitrary one needs the `glyf` transform -- a bespoke
            // point-encoding that must be reconstructed exactly -- and that is
            // a different size of job. A row for it would be a route that
            // refuses almost every real file.
            //
            // `Requirement::Engine("oc-archive")` because a font's tables are a
            // compressed container, and that is the worker that owns those.
            // See `EngineBin::for_media` for why that is not an accident of
            // where there was room.
            route!(Ttf   -> Woff,  A, [T { from: FormatId::Ttf,   to: FormatId::Woff  }], [Requirement::Engine("oc-archive")]),
            route!(Ttf   -> Woff2, A, [T { from: FormatId::Ttf,   to: FormatId::Woff2 }], [Requirement::Engine("oc-archive")]),
            route!(Otf   -> Woff,  A, [T { from: FormatId::Otf,   to: FormatId::Woff  }], [Requirement::Engine("oc-archive")]),
            route!(Otf   -> Woff2, A, [T { from: FormatId::Otf,   to: FormatId::Woff2 }], [Requirement::Engine("oc-archive")]),
            route!(Woff  -> Woff2, A, [T { from: FormatId::Woff,  to: FormatId::Woff2 }], [Requirement::Engine("oc-archive")]),
            // Unwrapping gives back whatever flavour was wrapped, so these two
            // are ONE operation with two names, and the engine refuses the
            // mismatch by name -- a WOFF holding CFF outlines is an OTF, and
            // calling it a TTF is the container rename above.
            route!(Woff  -> Ttf,   A, [T { from: FormatId::Woff,  to: FormatId::Ttf   }], [Requirement::Engine("oc-archive")]),
            route!(Woff  -> Otf,   A, [T { from: FormatId::Woff,  to: FormatId::Otf   }], [Requirement::Engine("oc-archive")]),
            // ---- AI: rows exist exactly where an adapter does ----
            //
            // Six rows lived here once and were REMOVED, because they named
            // models whose adapters had never been written: `oc-ai` linked the
            // runtime, queried its version, and could not run anything. A row
            // gated on "the model is not installed" describes a machine; that
            // was a program without an adapter, and the two deserve different
            // sentences.
            //
            // OCR is back because that stopped being true. Transcription is
            // still absent for the same reason it was removed -- no adapter --
            // and will return the same way, with one.
            //
            // Each row names ITS OWN artifacts, so a machine with the detector
            // but not the dictionary plans nothing and says which is missing.
            //
            // Class D, not B: OCR does not extract text the file contains, it
            // GUESSES text from pixels, and it can be confidently wrong in
            // ways no checksum catches. Calling that Class B would put a
            // judgement under a lossy-conversion label.
            route!(Png  -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            route!(Jpeg -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            route!(Webp -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            route!(Bmp  -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            route!(Tiff -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            route!(Gif  -> Txt, D, [StepKind::Infer { models: OCR_MODELS, task: "ocr", to: FormatId::Txt }], [Requirement::Engine("paddleocr")]),
            // ---- office documents: text extraction, no model involved ----
            //
            // Class B, not D: the text is IN the file and is being read out,
            // not guessed from pixels. What is lost is everything around it --
            // formatting, tables, images, notes -- which is a lot, and is why
            // this is not Class A either.
            route!(Docx -> Txt, B, [T { from: FormatId::Docx, to: FormatId::Txt }], [Requirement::Engine("oc-pdf")]),
            route!(Odt  -> Txt, B, [T { from: FormatId::Odt,  to: FormatId::Txt }], [Requirement::Engine("oc-pdf")]),
            // ---- The document matrix -------------------------------------
            //
            // A text document should reach the other text formats and a page
            // image, and until now `.docx` reached plain text and stopped.
            //
            // CLASSES, and why they differ across a row that looks uniform:
            //
            // * `-> markdown` is **B**. The heading levels come from the file
            //   itself -- `w:pStyle`, `text:outline-level` -- so the structure
            //   in the output was read, not guessed. Everything else is lost,
            //   which is what B means.
            // * `-> odt` and `-> docx` are **B** for the same reason: the same
            //   paragraphs and the same named headings, re-expressed in the
            //   other family's XML.
            // * `-> pdf` is **C**. Nothing about the source's geometry
            //   survives: it is re-typeset on A4 in Helvetica. The text and its
            //   order are preserved and the *page* is rebuilt, which is the
            //   distinction between C and B.
            // * `-> png/jpeg/...` is **C** for the same reason, one step
            //   further on -- and it is a two-step route through PDF rather
            //   than a new capability, exactly as `Pdf -> Webp` is a render
            //   followed by a re-encode.
            route!(Docx -> Markdown, B, [T { from: FormatId::Docx, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Odt  -> Markdown, B, [T { from: FormatId::Odt,  to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Pdf  -> Markdown, B, [T { from: FormatId::Pdf,  to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            // ---- Text as a SOURCE ----------------------------------------
            //
            // `md -> pdf` did not exist, and neither did `txt -> anything`:
            // Markdown was a destination with no way out of it, so the writers
            // that produce PDF, DOCX and ODT could not be reached from the one
            // format most likely to want them.
            //
            // THE SAME ROWS FOR `txt` AND `markdown`, and that is the design.
            // Detection is content-only and a `.md` file is plain text with no
            // signature, so the two cannot be told apart by content. Rather
            // than guess -- and be wrong in both directions -- every text input
            // is parsed as CommonMark, which plain text trivially is. A `.txt`
            // containing no markup yields body paragraphs; a `.md` yields its
            // headings and lists. Neither is refused for being mistaken for
            // the other, and the cost of a wrong guess is formatting rather
            // than a failed conversion.
            //
            // CLASS C for `-> pdf`: the source has no page geometry at all, so
            // the page is built rather than translated. B for the rest, which
            // re-express the same paragraphs in another text format.
            // NO ROUTES *FROM* MARKDOWN, and that is not an omission.
            //
            // Detection is content-only, and a `.md` file is plain text with no
            // signature — so `sniff` reports `Txt` for one and there is no path
            // anywhere that produces `FormatId::Markdown` as a *detected*
            // format. Rows written from it therefore cannot fire, whatever they
            // say. Seven of them shipped here, and `docs/ROUTES.md` advertised
            // all seven as capabilities: `markdown -> pdf`, listed as available,
            // reachable by nobody.
            //
            // The `Txt ->` rows below are the real capability and always were.
            // A `.md` arrives as `Txt`, and `mdread` parses every text input as
            // CommonMark, so the headings and lists are read either way — that
            // is the whole of the text-is-Markdown design.
            //
            // Markdown remains a perfectly good TARGET (`Pdf -> Markdown`,
            // `Docx -> Markdown`, and the rest below): those fire, because the
            // target is asked for by name rather than detected.
            //
            // `source_formats_are_reachable_by_detection` in the property tests
            // is what stops this class of row coming back.
            route!(Txt -> Pdf,      C, [T { from: FormatId::Txt, to: FormatId::Pdf      }], [Requirement::Engine("oc-pdf")]),
            route!(Txt -> Docx,     B, [T { from: FormatId::Txt, to: FormatId::Docx     }], [Requirement::Engine("oc-pdf")]),
            route!(Txt -> Odt,      B, [T { from: FormatId::Txt, to: FormatId::Odt      }], [Requirement::Engine("oc-pdf")]),
            route!(Txt -> Html,     B, [T { from: FormatId::Txt, to: FormatId::Html     }], [Requirement::Engine("oc-pdf")]),
            route!(Txt -> Markdown, B, [T { from: FormatId::Txt, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            // ---- Jupyter notebooks ---------------------------------------
            //
            // A notebook is prose, code and output, and every writer for those
            // already exists — so this is a reader and a set of rows, not a
            // new capability.
            //
            // CLASS C throughout, including to the text formats, and that is
            // not the usual B. A notebook is not a document that happens to be
            // in JSON: it is a program with its results attached, and what
            // comes out is a *rendering* of it. Figures are named rather than
            // included, rich output is reduced to its plain-text form, and
            // nothing that comes out can be run. That is a rebuild.
            route!(Ipynb -> Markdown, C, [T { from: FormatId::Ipynb, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Html,     C, [T { from: FormatId::Ipynb, to: FormatId::Html     }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Txt,      C, [T { from: FormatId::Ipynb, to: FormatId::Txt      }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Pdf,      C, [T { from: FormatId::Ipynb, to: FormatId::Pdf      }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Docx,     C, [T { from: FormatId::Ipynb, to: FormatId::Docx     }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Odt,      C, [T { from: FormatId::Ipynb, to: FormatId::Odt      }], [Requirement::Engine("oc-pdf")]),
            route!(Ipynb -> Png,      C, [T { from: FormatId::Ipynb, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }], [Requirement::Engine("oc-pdf")]),
            // HTML in. The tags are stripped rather than interpreted -- see
            // `run_text` -- so this yields the prose without the structure, and
            // the receipt says so. Class C for that reason even where the
            // target is another text format: what comes out is a rebuilt
            // document, not a re-expressed one.
            route!(Html -> Pdf,      C, [T { from: FormatId::Html, to: FormatId::Pdf      }], [Requirement::Engine("oc-pdf")]),
            route!(Html -> Docx,     C, [T { from: FormatId::Html, to: FormatId::Docx     }], [Requirement::Engine("oc-pdf")]),
            route!(Html -> Odt,      C, [T { from: FormatId::Html, to: FormatId::Odt      }], [Requirement::Engine("oc-pdf")]),
            route!(Html -> Markdown, C, [T { from: FormatId::Html, to: FormatId::Markdown }], [Requirement::Engine("oc-pdf")]),
            route!(Html -> Txt,      C, [T { from: FormatId::Html, to: FormatId::Txt      }], [Requirement::Engine("oc-pdf")]),
            // And the other document sources reach HTML too, through the same
            // writer.
            route!(Pdf  -> Html, B, [T { from: FormatId::Pdf,  to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            // `pdf -> docx` shipped and `pdf -> odt` did not, though both come
            // off the same reconstructed paragraphs and both writers exist.
            // Nobody wrote the row.
            route!(Pdf  -> Odt,  C, [T { from: FormatId::Pdf,  to: FormatId::Odt  }], [Requirement::Engine("oc-pdf")]),
            route!(Docx -> Html, B, [T { from: FormatId::Docx, to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            route!(Odt  -> Html, B, [T { from: FormatId::Odt,  to: FormatId::Html }], [Requirement::Engine("oc-pdf")]),
            // Text to a page image: typeset first, then render, exactly as the
            // office formats do.
            route!(Txt -> Png,  C, [T { from: FormatId::Txt, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }], [Requirement::Engine("oc-pdf")]),
            route!(Txt -> Jpeg, C, [T { from: FormatId::Txt, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Jpeg }], [Requirement::Engine("oc-pdf")]),
            route!(Docx -> Odt,  B, [T { from: FormatId::Docx, to: FormatId::Odt  }], [Requirement::Engine("oc-pdf")]),
            route!(Odt  -> Docx, B, [T { from: FormatId::Odt,  to: FormatId::Docx }], [Requirement::Engine("oc-pdf")]),
            route!(Docx -> Pdf, C, [T { from: FormatId::Docx, to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            route!(Odt  -> Pdf, C, [T { from: FormatId::Odt,  to: FormatId::Pdf }], [Requirement::Engine("oc-pdf")]),
            // A page image of a text document: typeset it, then render the
            // page. Two steps and two engines, declared as such.
            route!(Docx -> Png,  C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }], [Requirement::Engine("oc-pdf")]),
            route!(Docx -> Jpeg, C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Jpeg }], [Requirement::Engine("oc-pdf")]),
            route!(Docx -> Webp, C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Webp }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Docx -> Avif, C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Avif }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Docx -> Gif,  C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Gif  }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Docx -> Bmp,  C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Bmp  }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Docx -> Tiff, C, [T { from: FormatId::Docx, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }, T { from: FormatId::Png, to: FormatId::Tiff }], [Requirement::Engine("oc-pdf"), Requirement::Engine("oc-images")]),
            route!(Odt -> Png,  C, [T { from: FormatId::Odt, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Png }], [Requirement::Engine("oc-pdf")]),
            route!(Odt -> Jpeg, C, [T { from: FormatId::Odt, to: FormatId::Pdf }, T { from: FormatId::Pdf, to: FormatId::Jpeg }], [Requirement::Engine("oc-pdf")]),
            // ---- PDF text: read out, not recognised ----
            //
            // CLASS B AND EMPHATICALLY NOT D. The six rows above this section
            // run OCR, and OCR guesses characters from pixels. This does not:
            // a PDF stores its text as text, and pdfium's text page hands back
            // the characters the file already contains, in the order the
            // content stream draws them. Filing that under the same class as
            // a recogniser would say the words might be wrong when they cannot
            // be.
            //
            // What IS lost is large and is why this is not Class A: every
            // glyph position, every font, columns, tables, headers, footers,
            // and reading order wherever the draw order is not the reading
            // order. A scanned PDF -- pixels with no text objects -- yields
            // nothing at all, and the worker says so by name rather than
            // writing an empty file, because an empty .txt looks like success.
            route!(Pdf  -> Txt,  B, [T { from: FormatId::Pdf,  to: FormatId::Txt  }], [Requirement::Engine("oc-pdf")]),
            // ---- PDF to Word: the first Class C row in the table ----
            //
            // **Class C is what this is for, and until now nothing used it.**
            // A and B both promise the output is the input, exactly or
            // approximately. This promises less: the text is read out
            // faithfully, and then a DOCUMENT STRUCTURE THAT THE PDF DOES NOT
            // CONTAIN is inferred over it. A PDF has no paragraphs. It has
            // glyphs at coordinates. Paragraph breaks, headings and the
            // reading order across columns are all conclusions drawn from
            // geometry, and a conclusion can be wrong in a way a re-encode
            // cannot.
            //
            // That is exactly the distinction C was defined to carry --
            // rebuilt, not converted -- so the honest thing is to name it and
            // ship it, rather than either refusing a conversion people plainly
            // want or filing it under B and letting the receipt imply a
            // fidelity nobody can deliver.
            //
            // Class C is above the default auto-class ceiling, so naming
            // `docx` as the destination is what arms it (`Target::arms`). The
            // class reaches the receipt and the UI either way.
            route!(Pdf  -> Docx, C, [T { from: FormatId::Pdf,  to: FormatId::Docx }], [Requirement::Engine("oc-pdf")]),
            // Transcription. The same shape as OCR one level up: the model
            // guesses words from a signal rather than extracting text the
            // file contains, so it is Class D for the same reason.
            //
            // Every audio format we can DECODE is a source, plus the three
            // containers a recording arrives in -- oc-ai runs the same
            // symphonia front end oc-audio does, so "can we read it" has one
            // answer, and that answer covers every row below.
            //
            // NO `Mka` ROW, for the reason the container section below spells
            // out: an `.mka` carries MKV's signature and MKV's DocType, so
            // `sniff` resolves it to `Mkv` every time. An audio-only Matroska
            // therefore arrives here AS `Mkv` and transcribes on that row. A
            // `Mka -> Txt` row would never be reached, and a dead row that
            // makes the list look more complete is worse than the gap it
            // appears to close.
            route!(Wav  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Flac -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Mp3  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Ogg  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(M4a  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Mkv  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Webm -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            route!(Mp4  -> Txt, D, [StepKind::Infer { models: ASR_MODELS, task: "transcribe", to: FormatId::Txt }], [Requirement::Engine("whisper")]),
            // ---- tabular: pure Rust both sides, in-process ----
            route!(Csv  -> Json, A, [T { from: FormatId::Csv,  to: FormatId::Json }], []),
            route!(Json -> Csv,  B, [T { from: FormatId::Json, to: FormatId::Csv  }], []),
        ])
    }
}
