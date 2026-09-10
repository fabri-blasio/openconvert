//! The plan: what *will* happen, before anything does.
//!
//! **The plan preview is not a feature. It is `route()` without `execute()`.**
//! The most distinctive thing in the product exists because of the
//! architecture rather than in addition to it.

use crate::format::FormatId;
use crate::isolation::{Isolation, Refusal};
use crate::limits::Limits;
use crate::target::Target;

/// How much fidelity a step costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// Lossless. Bit-for-bit correct output, by construction.
    ///
    /// The load-bearing promise. A conversion advertised as lossless that
    /// silently is not is the worst bug this product can have: invisible, and
    /// it damages exactly what the user was trying to protect.
    A,
    /// Lossy but standard, and understood as such. PNG → JPEG.
    B,
    /// Lossy and structurally transformative. Rebuilt, not converted.
    C,
    /// Generative. Content that was not in the input.
    ///
    /// **Never armed automatically** (I8): `route()` filters above
    /// `Policy::max_auto_class`, which defaults to `B`, regardless of what
    /// suggested the target.
    D,
}

/// A single operation in a plan.
///
/// `Class`, `Limits` and `Isolation` are **non-optional fields** — I3, I4 and
/// I10. A step that has not been assigned any of the three cannot be
/// constructed, which is what makes those three claims *Impossible* rather than
/// conventions someone has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// What this step does.
    pub kind: StepKind,
    /// Fidelity cost. I3.
    pub class: Class,
    /// Resource ceiling. I4.
    pub limits: Limits,
    /// Where it runs. I10.
    pub isolation: Isolation,
}

/// The operations a plan can be made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Copy container streams without re-encoding.
    StreamCopy,
    /// Decode then re-encode.
    Transcode {
        /// Source format.
        from: FormatId,
        /// Destination format.
        to: FormatId,
    },
    /// Remove metadata without decoding content.
    StripMetadata,
    /// Repack an archive: unpack its members and write them into another
    /// container, without re-encoding any of them.
    ///
    /// # Why it names both ends
    ///
    /// It did not, and the destination came from the PLAN's target instead —
    /// which is the same thing exactly while every archive route has one step.
    /// A chain does not have that property: `gzip -> tar -> zip` hands its
    /// second step the first one's output, and a step reading "the plan's
    /// target" would ask the worker for a zip while handing it a gzip, twice.
    ///
    /// `Transcode` names both ends for that reason and this now matches it.
    /// The pair is also what `step_output_format` needs to thread the
    /// intermediate format through `execute`, and what lets `docs/ROUTES.md`
    /// print the chain instead of the word "extract".
    Extract {
        /// The container that arrives.
        from: FormatId,
        /// The container to write.
        to: FormatId,
    },
    /// Keep only samples within [start_ms, end_ms), snapping to keyframes.
    Trim {
        /// Start of the trim window in milliseconds.
        start_ms: u64,
        /// End of the trim window in milliseconds.
        end_ms: u64,
    },
    /// Render one page of a document to the target raster format.
    ///
    /// Carries the one parameter any step kind has, which is why it exists:
    /// "page 3 as JPEG" is not expressible as `Transcode { from, to }` without
    /// losing the thing the user actually asked for.
    RenderPage {
        /// Zero-based page index.
        page: u32,
    },
    /// Rewrite a raster's pixels in place: no resampling, no reformatting.
    ///
    /// Carries the operation by NAME rather than as a second enum, for the
    /// reason `Infer` carries its task the same way: several pixel operations
    /// share one destination format, and a worker guessing which was meant is a
    /// worker doing something other than what was asked. `&'static str` keeps
    /// the whole step `Copy`, which is what lets `StepKind` stay the small
    /// value type the core's purity rules depend on.
    Pixel {
        /// The operation, as the worker knows it (`invert`, `greyscale`).
        op: &'static str,
    },
    /// Run a model over the content. The step names WHICH model, because a
    /// receipt that says "a model ran" is not a receipt anybody can check.
    Infer {
        /// Registry ids of every artifact this step needs, in the order the
        /// adapter expects them. Several, because a "model" is often a set:
        /// OCR is detection + recognition + character dictionary.
        models: &'static [&'static str],
        /// The adapter to run inside the worker (`remove-background`).
        task: &'static str,
        /// Destination format.
        to: FormatId,
    },
}

/// Something the user should know before running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// The extension did not match the content. Routed by content.
    TypeMismatch {
        /// What the name claimed.
        declared: FormatId,
        /// What the bytes said.
        detected: FormatId,
    },
    /// **Blocking.** Multiple format signatures matched.
    ///
    /// A file that is validly two things is a file whose author wanted two
    /// programs to disagree about it. We refuse and ask, rather than picking
    /// by preference order — which is what an attacker relies on. A6.
    Polyglot,
    /// The conversion will lose fidelity.
    Lossy {
        /// How much.
        class: Class,
    },
    /// **Blocking.** The machine cannot meet the isolation floor.
    BelowFloor(Refusal),
    /// **Blocking.** The target requires a class above `max_auto_class`.
    AboveAutoClass {
        /// The class the route would need.
        needed: Class,
        /// The ceiling policy allows without an explicit request.
        allowed: Class,
    },
    /// **Blocking.** No route exists.
    NoRoute(NoRoute),
}

impl Warning {
    /// Whether this warning prevents execution.
    ///
    /// A blocking warning means `route()` returned **no steps**. That is the
    /// mechanism behind I9 and SR-1: refusal is the absence of a plan, not a
    /// flag on one, so there is no plan sitting around for a caller to run
    /// anyway.
    #[must_use]
    pub const fn is_blocking(&self) -> bool {
        matches!(
            self,
            Self::BelowFloor(_) | Self::AboveAutoClass { .. } | Self::NoRoute(_) | Self::Polyglot
        )
    }
}

/// Why no route exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoRoute {
    /// The formats are in different kinds and the pair is not one we support.
    ///
    /// Generated rather than enumerated. That is what keeps adding a format
    /// O(1) instead of O(n): v0.4 required every pair to have an explicit
    /// entry, so format 21 owed 40 new table decisions.
    CrossKind,
    /// Within a kind, but this direction is not supported.
    UnsupportedPair,
    /// Rows exist for this pair, but every one failed a requirement.
    ///
    /// Carries the requirement that eliminated the **first** route, which is
    /// the most preferred one and therefore the most useful thing to tell the
    /// user: "HEIC to JPEG needs oc-images, which is not installed."
    RequirementUnmet {
        /// What the best available route needed and did not get.
        ///
        /// The **requirement**, not its description. Carrying the rendered
        /// string collapsed two different problems into one sentence: a missing
        /// `Engine` is a machine the user can repair, and unmet
        /// `StreamsCompatible` is a property of their file. The shell could not
        /// tell them apart, so it said "needs codecs are compatible with the
        /// destination container, which is not available on this machine" --
        /// ungrammatical, and wrong about whose fault it was.
        requirement: crate::route::Requirement,
    },
    /// The step needs more memory than this machine's limit allows.
    ///
    /// Not a missing engine and not a property of the file: everything needed
    /// is installed and the route exists, but running it would ask for more
    /// than the confinement permits, so it would die inside the worker with an
    /// allocation failure and reach the user as a conversion that broke for no
    /// stated reason.
    ///
    /// Refusing in the plan instead means the preview says it too, before
    /// anything runs -- and the numbers are carried rather than rendered so the
    /// shell can name the setting in its own words.
    NeedsMoreMemory {
        /// What the step needs, in bytes.
        needed: u64,
        /// What the current limit allows, in bytes.
        allowed: u64,
    },
    /// The input could not be identified.
    UnknownInput,
    /// The operation exists, but not for this input.
    ///
    /// `--trim` on a PNG is not a missing route; it is a request no route can
    /// ever satisfy for that file. Named separately from `UnsupportedPair`
    /// because the fix is different: there is nothing to add to the table —
    /// the user asked a question the format cannot answer.
    OperationInputMismatch {
        /// The operation that was asked for, lowercased ("trim").
        operation: &'static str,
    },
}

/// What the user asked for, as a value that can be stored and re-routed.
///
/// `PlanRequest` deserialises; [`Plan`] does not. Anything that receives a plan
/// — the server, a recipe, a resumed batch — stores one of these plus a plan
/// hash and **re-routes**. If the re-routed hash differs, the environment
/// changed and the caller is told, rather than executing a plan built against a
/// machine that no longer exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanRequest {
    /// The input's identity.
    pub input: FormatId,
    /// What the user wants.
    pub target: Target,
    /// More than one format signature matched.
    ///
    /// Carried here rather than left in `Sniff` because **`route()` has to act
    /// on it**. A6 says polyglots are quarantined and the user is asked; a flag
    /// that detection sets and routing never reads is a flag that quarantines
    /// nothing.
    pub polyglot: bool,
}

impl PlanRequest {
    /// A request for a file with a single, unambiguous format.
    #[must_use]
    pub const fn new(input: FormatId, target: Target) -> Self {
        Self {
            input,
            target,
            polyglot: false,
        }
    }
}

/// What will happen.
///
/// Private fields and **no `Deserialize`** (I2). Only [`crate::route::route`]
/// produces one — which is what makes "every step carries limits and isolation"
/// enforceable rather than aspirational: there is no other way to get a `Plan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    version: u32,
    request: PlanRequest,
    steps: Vec<Step>,
    warnings: Vec<Warning>,
}

/// Schema version, carried from commit one.
///
/// Receipts persist on disk and in customer archives, so the field has to exist
/// before there is anything to version.
pub const PLAN_VERSION: u32 = 1;

impl Plan {
    /// Construct. `pub(crate)` — only `route()` can reach it.
    pub(crate) fn new(request: PlanRequest, steps: Vec<Step>, warnings: Vec<Warning>) -> Self {
        Self {
            version: PLAN_VERSION,
            request,
            steps,
            warnings,
        }
    }

    /// The steps, in order. Empty if the plan is blocked.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Everything the user should know.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The request this plan answers.
    #[must_use]
    pub const fn request(&self) -> PlanRequest {
        self.request
    }

    /// Schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// The format the last step produces, or `None` for a refusal.
    ///
    /// **The name of an output has to come from here, not from the target.**
    /// `Target::output_format` answers "what did the user ask for", which for
    /// an operation is "the format it came in as" — and that is a different
    /// question from "what will be in the file". Noise removal is where the
    /// two came apart: the user asks for the noise gone, the target says the
    /// format is unchanged, and a plan that cannot reach the original encoder
    /// ends on WAV. Naming that file `.mp3` produced a WAV that every player
    /// trusting the extension refuses.
    ///
    /// A step that names no format of its own — a stream copy, a metadata
    /// strip — leaves this `None`, and the caller falls back to the target,
    /// which is right: those steps do not change the format.
    #[must_use]
    pub fn output_format(&self) -> Option<FormatId> {
        match self.steps.last()?.kind {
            StepKind::Transcode { to, .. } | StepKind::Infer { to, .. } => Some(to),
            _ => None,
        }
    }

    /// Whether this plan can run.
    #[must_use]
    pub fn is_executable(&self) -> bool {
        !self.steps.is_empty() && !self.warnings.iter().any(Warning::is_blocking)
    }

    /// The worst fidelity class any step costs.
    #[must_use]
    pub fn class(&self) -> Option<Class> {
        self.steps.iter().map(|s| s.class).max()
    }
}
