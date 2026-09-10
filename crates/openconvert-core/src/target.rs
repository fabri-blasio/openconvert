//! What the user asked for.
//!
//! Deliberately *not* a format. "Convert this to PDF" and "strip the location
//! from this" are both targets, and only one of them names a format — a design
//! that models the target as a `FormatId` cannot express the second without a
//! parallel mechanism, which is how metadata operations end up bolted on.

use crate::format::FormatId;
use crate::plan::Class;

/// An operation that changes a file without changing its format.
///
/// These are the Class A candidates: nothing is re-encoded, so the output is
/// bit-for-bit correct by construction rather than by measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Remove metadata: GPS, camera serial, author, edit history.
    ///
    /// The first Class A operation in the build (week 3), and the reason the
    /// Class A gate has a subject at week 4 instead of week 30. It also gives
    /// **A8/SR-11** — metadata leakage — its first real test; until this landed
    /// that requirement had a written policy and nothing exercising it.
    StripMetadata,
    /// Rewrap streams into a different container without re-encoding.
    ///
    /// Week 27–30. `mkv → mp4` where the codecs are already MP4-compatible.
    Remux,
    /// Keep only the samples inside `[start_ms, end_ms)`.
    ///
    /// Matroska-family inputs: the cut is container surgery, so sample bytes
    /// move verbatim and only presence and timestamps change — Class A by
    /// construction, like everything else in this enum.
    Trim {
        /// Start of the kept range, in milliseconds.
        start_ms: u64,
        /// End of the kept range, in milliseconds.
        end_ms: u64,
    },
    /// Cut the background away, leaving the subject on transparency.
    ///
    /// # Why this is an Operation and not a Tool that skips the plan
    ///
    /// The tools registry calls pixel effects `preview_only` because "a pixel
    /// operation has no `Target` to route through, and SR-11 means an output
    /// without a receipt must not exist". That reasoning is right and the
    /// conclusion was a limitation, not a law: giving the effect an
    /// `Operation` gives it a route, a `Plan`, and therefore a receipt — so it
    /// can be SAVED rather than only previewed.
    ///
    /// It changes the format like [`Self::RenderPage`] does, and for a
    /// related reason: the result needs an alpha channel, so a JPEG in cannot
    /// stay a JPEG out. The destination is named rather than inherited.
    ///
    /// **Class D, alone in this enum.** Every other operation here is Class A
    /// container surgery. This one asks a neural network to decide which
    /// pixels are the subject, and it can be wrong in ways no checksum
    /// catches — the receipt records the model's name and hash for exactly
    /// that reason.
    RemoveBackground {
        /// Destination raster format; must carry alpha.
        to: FormatId,
        /// Which segmenter to use.
        ///
        /// Three exist and none is interchangeable with the others in
        /// post-processing: u2netp emits an unnormalised saliency map that
        /// must be stretched, MODNet emits a finished alpha that must not be,
        /// and BiRefNet emits a 1024x1024 map that may be logits. The choice
        /// therefore travels with the step rather than being inferred from
        /// whatever happens to be downloaded.
        ///
        /// It was a `bool` while there were two. A third would have made it a
        /// bool plus a special case, which is how the second-best model ends up
        /// running because a condition was written the wrong way round.
        quality: Quality,
    },
    /// Strip background noise out of speech.
    ///
    /// Class D like the other model operations: what counts as noise is the
    /// network's judgement, and it can take quiet speech with it. Measured on
    /// this build it adds roughly 8.6 dB to a noisy recording and leaves an
    /// already-clean one alone — but "leaves it alone" is a measurement, not a
    /// guarantee.
    ///
    /// The output is always 48 kHz mono WAV: the model's own rate, and one
    /// channel because that is what it enhances. Both are changes to the file
    /// and both are disclosed.
    Denoise,
    /// Enlarge an image ×4 with a super-resolution model.
    ///
    /// Class D like [`Self::RemoveBackground`], and for a blunter reason: the
    /// pixels at the new size were never in the file. A bicubic resize invents
    /// nothing and looks soft; this invents plausible detail, which is more
    /// useful and much easier to mistake for recovered information. The
    /// receipt says so in those words.
    Upscale {
        /// Destination raster format.
        to: FormatId,
    },
    /// Render one page of a document to a raster format.
    ///
    /// The only operation here that CHANGES the format, because "page 3 as a
    /// JPEG" is a single user sentence that neither `Format` nor the other
    /// operations can say: it names both a parameter and a destination.
    RenderPage {
        /// Zero-based page index. 0 renders exactly what the plain
        /// `pdf -> png` / `pdf -> jpeg` routes have always rendered.
        page: u32,
        /// Where the pixels land.
        to: FormatId,
    },
    /// Invert every channel: `v -> 255 - v`, alpha untouched.
    ///
    /// **Class A.** Applying it twice returns the original bytes exactly, and
    /// the operation reads and writes the same pixel grid — nothing is
    /// resampled, quantised or discarded. The destination format decides
    /// whether the FILE round-trips; the operation itself does.
    Invert,
    /// Convert to greyscale by luminance.
    ///
    /// **Class B, and the distinction from `Invert` is the point.** Colour is
    /// discarded and cannot be recovered, so calling it lossless would be
    /// exactly the kind of receipt this project must never write. Deterministic
    /// and reproducible, which is what keeps it in B rather than C.
    Greyscale,
}

/// How much time and disk the user is willing to spend on a cut-out.
///
/// Ordered by COST, which is what the tier ladder in `models.toml` is ordered
/// by: roughly 1 s, 5 s and 46 s per image, at 4.5 MB, 6.6 MB and 224 MB. On
/// the one subject all three have been measured against, quality happens to
/// rise with cost as well -- but cost is the thing being chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    /// u2netp. Small, quick, and good on a clearly-separated subject.
    Standard,
    /// MODNet. A portrait matting network; better on hair, weaker on
    /// everything it was not trained for, which is why it carries a fallback.
    Better,
    /// BiRefNet-lite. The most accurate here and much the slowest.
    Best,
}

/// What the user wants to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Convert to a named format.
    Format(FormatId),
    /// Apply an operation, leaving the format alone.
    Operation(Operation),
}

impl Target {
    /// The format this target produces, given the input's format.
    ///
    /// An operation preserves the input format — which is what makes it Class A
    /// eligible in the first place. [`Operation::RenderPage`] is the exception
    /// that names its own destination.
    #[must_use]
    pub const fn output_format(self, input: FormatId) -> FormatId {
        match self {
            Self::Format(f) => f,
            Self::Operation(Operation::RenderPage { to, .. }) => to,
            // WHAT THE USER ASKED FOR, WHICH IS NOT ALWAYS WHAT THEY GET.
            //
            // An operation leaves the format alone, so this answers with the
            // input — and that is the right answer to the question this
            // function asks. It is the wrong thing to NAME A FILE with, and
            // for a long time it was what named every file.
            //
            // Noise removal is where the two came apart. The user asks for the
            // noise gone; the target says the format is unchanged; the plan
            // ends wherever it can reach. When the encoder is present the plan
            // gets back to the original format and the two agree. When it is
            // not, the plan ends on WAV — and naming that file `.mp3` produced
            // a WAV that every player trusting the extension refuses.
            //
            // So the NAME comes from `Plan::output_format`, which is what will
            // be in the file. This stays as the fallback for a caller with no
            // plan in hand, where "the format is unchanged" is all that can
            // honestly be said.
            Self::Operation(_) => input,
        }
    }

    /// The auto-class ceiling this target's NAME is itself a request for.
    ///
    /// # Why a target can raise its own ceiling
    ///
    /// `Policy::max_auto_class` defaults to B so nothing lossier than a
    /// re-encode ever happens *because a route was convenient* (I8). The
    /// refusal it produces says "ask for the operation by name to run it" —
    /// and this function is what recognises the asking.
    ///
    /// **It lives here because it was written twice.** The CLI had it in
    /// `policy_for` and the tool path did not have it at all, which is how
    /// every model-backed tool in the desktop reported "That did not produce a
    /// file" on a machine where all four models worked: the ceiling refused
    /// the plan before an adapter was ever reached. Two copies of a rule this
    /// consequential is one copy too many.
    ///
    /// # The three answers
    ///
    /// - **D** for an operation only a model can perform, and for `-t txt`,
    ///   which on an image or a recording has exactly one meaning: read it.
    ///   A `.docx` also reaches `txt`, on a Class B row, and gets it — arming
    ///   raises the ceiling, it does not change which route wins, and the
    ///   lowest-class route still does.
    /// - **C** for any other named format. Naming a destination is not
    ///   "convenient routing", it is a sentence; and the only thing this
    ///   admits is a destination that nothing but a rebuild can reach. Where a
    ///   B route exists it is still the one chosen, so on all but a handful of
    ///   pairs this changes nothing at all. `pdf -> docx` is the pair it
    ///   exists for.
    /// - **None** for the operations that transform without inventing —
    ///   they are Class A or B already and have nothing to ask for.
    ///
    /// What this never does is hide the cost: the class reaches the plan, the
    /// receipt and the UI either way. The ceiling decides whether a conversion
    /// happens silently, not whether it is disclosed.
    #[must_use]
    pub const fn arms(self) -> Option<Class> {
        match self {
            Self::Operation(
                Operation::RemoveBackground { .. } | Operation::Upscale { .. } | Operation::Denoise,
            )
            | Self::Format(FormatId::Txt) => Some(Class::D),
            Self::Format(_) => Some(Class::C),
            Self::Operation(_) => None,
        }
    }
}

// `changes_format` and `FormatId::eq_id` lived here until mutation testing
// found them: 8 surviving mutants, because nothing called either one. They were
// written in anticipation of a caller that never arrived.
//
// Deleted rather than tested. Spike S30 predicted exactly this -- one of the
// things cargo-mutants found in the spike work was dead code, and the right
// response to dead code is deletion, not coverage. A test for an uncalled
// function is a test that keeps it alive.
