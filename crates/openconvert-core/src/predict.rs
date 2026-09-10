//! Prediction: hand-weighted signals over a derived prior, **pure**.
//!
//! The scoring lives here and the gathering lives in `openconvert-run`. That split
//! is not tidiness. v0.4 put prediction inside the *CLI binary*, while the GUI —
//! which is where "the guess is the product" actually happens — is a different
//! binary that cannot link it. One implementation would have become two, then
//! three, and they would have diverged on the one behaviour the product is
//! named for.
//!
//! Being pure also makes the `p99 < 10 ms` gate a criterion benchmark over
//! vectors rather than an integration test over a filesystem.
//!
//! # What prediction may not do
//!
//! It ranks **targets**, never plans. It cannot arm a Class D operation, because
//! `route()` filters on `Policy::max_auto_class` regardless of what suggested
//! the target (I8). It cannot influence isolation, limits or the conflict
//! policy — all of which are `route()`'s, and none of which take a suggestion
//! as an input.
//!
//! # Why there is still no learned model
//!
//! Because there is still no data. Nothing recorded what was *suggested*
//! against what was *chosen* until the journal gained `suggested`,
//! `suggested_score` and `chosen_rank`, so every weight below is reasoned
//! rather than fitted, and every one of them is a table someone can read and
//! argue with. When those fields have accumulated, a logistic regression over
//! exactly these signals is the next step — and it will have something to be
//! right or wrong about.

use crate::format::FormatId;
use crate::target::Target;

/// The evidence available for one prediction, gathered by the shell.
///
/// Every field is a **value**, supplied by the caller. Nothing here reads a
/// file, a clock or an environment variable — which is what makes the whole
/// module testable without fixtures.
///
/// # Why the history fields are `f32`
///
/// They are decayed weights, not counts. A conversion done a year ago is
/// weaker evidence than one done yesterday, and a plain tally cannot say so:
/// it takes as many conversions to overturn a habit as it took to establish
/// one, so somebody who moved from JPEG to WebP last month keeps being offered
/// JPEG. The shell applies the decay because it is the half that knows the
/// clock; this half just receives a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    /// What the input actually is.
    pub input: FormatId,
    /// Decayed weight of every conversion this user has made FROM this format.
    pub history_count: f32,
    /// How much of that weight went to the candidate target.
    pub history_to_target: f32,
    /// The same two numbers, restricted to the folder this file came from.
    ///
    /// **Format choice is workflow-local**, and a single global counter cannot
    /// represent it: someone who sends `~/Scans` to PDF and `~/Web` to WebP has
    /// two habits, and one tally averages them into a third that is neither.
    /// Folder evidence is blended over the global evidence for that reason,
    /// with the same cold-start curve, so a folder seen once does not
    /// immediately outrank a hundred conversions elsewhere.
    pub folder_history_count: f32,
    /// How much of the folder's weight went to the candidate target.
    pub folder_history_to_target: f32,
    /// How many OTHER files in this folder already have an output of the
    /// candidate format beside them.
    ///
    /// This is the signal `has_sibling_output` was reaching for and did not
    /// implement. A folder holding `a.heic a.jpg b.heic b.jpg` predicts what
    /// to do with `c.heic`, which is the moment a suggestion is worth having.
    pub folder_pattern: u32,
    /// Whether **this exact file** already has an output of the candidate
    /// format beside it — `photo.jpg` next to `photo.heic`.
    ///
    /// # This is weaker evidence than it looks, and it used to be the strongest
    ///
    /// It was floored at 0.90, which is above [`ARM_THRESHOLD`], on the
    /// reasoning that "someone has already done this exact conversion in this
    /// exact folder". But it matches on the *stem*, so what it actually
    /// observes is that **this file has already been converted** — and
    /// converting it again either fails or writes `photo (2).jpg`, depending
    /// on the conflict policy.
    ///
    /// Both readings are defensible: re-dropping a file to redo it is a real
    /// thing people do. So this is demoted rather than inverted — it still
    /// leads, it no longer auto-arms — and [`Signals::folder_pattern`] carries
    /// the generalising half. `chosen_rank` in the journal is what will
    /// eventually settle which reading is right.
    pub sibling_same_stem: bool,
    /// Number of files in this drop. Separates single-file tools from batch
    /// export, which want different things.
    pub batch_size: u32,
    /// What kind of folder the input came from, if we recognised one.
    pub folder: FolderCategory,
}

/// A recognised source folder, which carries intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderCategory {
    /// `Screenshots` and similar — usually wants compression.
    Screenshots,
    /// `Scans` — usually wants OCR or PDF.
    Scans,
    /// `Downloads` — no strong prior, and worth noting that these are also the
    /// files most likely to carry untrusted provenance.
    Downloads,
    /// Nothing recognised.
    Unknown,
}

/// A scored candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Suggestion {
    /// What to convert to.
    pub target: Target,
    /// Confidence in `0.0..=1.0`.
    pub score: f32,
}

/// The cold-start blend: `w(n) = n / (n + 3)`.
///
/// One formula gives the whole curve. At the first drop the global prior
/// dominates; by the sixth, personal history does. The constant 3 is the
/// half-way point and is the only tunable number in the blend — deliberately,
/// because a model with many knobs needs calibration data that does not exist
/// until the product ships.
///
/// Takes an `f32` because the evidence it weighs is a decayed weight rather
/// than a count. Negative and non-finite inputs return 0: this is called with
/// numbers derived from files on disk, and a `NaN` weight must not become a
/// `NaN` score.
#[must_use]
pub fn personal_weight(n: f32) -> f32 {
    if !n.is_finite() || n <= 0.0 {
        return 0.0;
    }
    n / (n + 3.0)
}

/// How usual it is to convert *to* this format at all, ignoring the source.
///
/// **One number per format instead of one per pair.** The pair table below
/// covers 13 of them; the route table now carries over a hundred, so every
/// pair nobody had hand-written scored a flat 0.05 — under
/// [`CHOOSER_THRESHOLD`], which is why a new user met the format picker for
/// almost everything. Ranking by destination costs 36 numbers rather than
/// 36×36, and it grows when the format table does instead of falling behind
/// it.
///
/// The scale is deliberately coarse: 1.0 is a format people convert to on
/// purpose, 0.5 is one they accept, 0.1 is one that is almost always a source.
#[must_use]
pub fn destination_weight(f: FormatId) -> f32 {
    match f {
        // The formats a conversion is usually *for*.
        FormatId::Jpeg | FormatId::Png | FormatId::Pdf | FormatId::Mp4 | FormatId::Mp3 => 1.0,
        // Common, modern, chosen deliberately by people who know why.
        FormatId::Webp | FormatId::Docx | FormatId::Flac | FormatId::Json => 0.8,
        FormatId::Avif | FormatId::M4a | FormatId::Mkv | FormatId::Wav | FormatId::Txt => 0.6,
        // A destination people choose deliberately when they want the
        // structure kept, which is most of what makes it worth offering.
        // Both are destinations people choose deliberately when they want the
        // structure kept, which is most of what makes them worth offering.
        FormatId::Markdown | FormatId::Html => 0.6,
        // Essentially never a destination: notebooks are written by Jupyter,
        // and converting *to* one would mean inventing cells.
        FormatId::Ipynb => 0.05,
        // Real destinations, but niche ones.
        FormatId::Gif | FormatId::Ogg | FormatId::Webm | FormatId::Csv | FormatId::Zip => 0.5,
        FormatId::Tiff | FormatId::Odt | FormatId::Jxl | FormatId::Svg => 0.3,
        FormatId::Mka | FormatId::Gzip | FormatId::Tar | FormatId::SevenZip => 0.2,
        // WOFF2 is where a web font is GOING: it is the delivery format, and
        // "make this smaller for the web" is the only reason most people
        // convert a font at all. WOFF is the older answer to the same
        // question and still a real destination.
        FormatId::Woff2 => 0.8,
        FormatId::Woff => 0.4,
        // A desktop font is what somebody HAS. Converting to one is unwrapping
        // a web font back for editing -- real, and much rarer than the reverse.
        FormatId::Ttf | FormatId::Otf => 0.2,
        // SOURCES, ALL FIVE. A deck, a workbook and a book are things people
        // convert OUT of -- to a PDF to send, to a CSV to work with, to text to
        // read. This build cannot write any of them, so the weight would be
        // consulted for nothing; it is stated rather than defaulted so that
        // adding a writer later is a deliberate change to this line.
        // A drawing joins them: a `.dxf` is what CAD exported, and this build
        // cannot write one.
        FormatId::Pptx
        | FormatId::Odp
        | FormatId::Epub
        | FormatId::Xlsx
        | FormatId::Ods
        | FormatId::Dxf => 0.1,
        // Almost always a source and almost never a target. Nobody converts
        // *to* a camera raw: the point of raw is that it is what the sensor
        // gave you, and a converted one is a worse version of a JPEG.
        // MOV joins these rather than MP4's 1.0. It is what a camera, a phone
        // and an NLE hand you, and `mov -> mp4` is the conversion people ask
        // for -- almost nobody asks for the reverse, and this build cannot
        // write one anyway.
        // AVI joins MOV for the same reason: it is what somebody HAS, and
        // `avi -> mp4` is the conversion they want. This build cannot write
        // one anyway.
        FormatId::Bmp | FormatId::Heic | FormatId::PostScript | FormatId::Mov | FormatId::Avi => {
            0.1
        }
        FormatId::Cr3 | FormatId::Cr2 | FormatId::Nef | FormatId::Arw | FormatId::Dng => 0.05,
        FormatId::Unknown => 0.0,
    }
}

/// Roughly what a format does to the data it holds.
///
/// Used for one asymmetry only — see [`direction_multiplier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fidelity {
    /// Keeps every sample: PNG, FLAC, WAV, raw.
    Lossless,
    /// Discards some on purpose: JPEG, MP3, most video.
    Lossy,
    /// Neither applies — documents, archives, tabular data, vectors.
    Neither,
}

const fn fidelity(f: FormatId) -> Fidelity {
    match f {
        FormatId::Png
        | FormatId::Bmp
        | FormatId::Tiff
        | FormatId::Flac
        | FormatId::Wav
        | FormatId::Cr3
        | FormatId::Cr2
        | FormatId::Nef
        | FormatId::Arw
        | FormatId::Dng => Fidelity::Lossless,
        FormatId::Jpeg
        | FormatId::Webp
        | FormatId::Avif
        | FormatId::Jxl
        | FormatId::Heic
        | FormatId::Gif
        | FormatId::Mp3
        | FormatId::Ogg
        | FormatId::M4a
        | FormatId::Mp4
        | FormatId::Webm => Fidelity::Lossy,
        _ => Fidelity::Neither,
    }
}

/// Going lossy → lossless is unusual; going lossless → lossy is the normal
/// reason to convert at all.
///
/// A PNG of a JPEG is bigger than the JPEG and contains not one extra bit of
/// detail. People do it — to edit, to satisfy a tool that demands PNG — so it
/// is discouraged rather than refused. The hand table already encoded exactly
/// this asymmetry for one pair (`png→jpeg` 0.55 against `jpeg→png` 0.30); this
/// is that observation applied to every pair instead of one.
fn direction_multiplier(from: FormatId, to: FormatId) -> f32 {
    match (fidelity(from), fidelity(to)) {
        (Fidelity::Lossy, Fidelity::Lossless) => 0.55,
        (Fidelity::Lossless, Fidelity::Lossy) => 1.15,
        _ => 1.0,
    }
}

/// The global prior: what people convert this format to, absent any history.
///
/// **A tuned pair overrides the derived value.** The pairs below are ones
/// somebody reasoned about individually, and a formula that scaled to the
/// whole route table should not quietly overwrite them; everything the table
/// does not mention falls through to [`derived_prior`] instead of the flat
/// 0.05 it used to get.
#[must_use]
pub fn global_prior(input: FormatId, candidate: FormatId) -> f32 {
    match (input, candidate) {
        (FormatId::Heic, FormatId::Jpeg) => 0.85,
        (FormatId::Png, FormatId::Jpeg) => 0.55,
        (FormatId::Jpeg, FormatId::Png) => 0.30,
        (FormatId::Webp, FormatId::Png) => 0.45,
        (FormatId::Webp, FormatId::Jpeg) => 0.40,
        (FormatId::Mkv, FormatId::Mp4) => 0.80,
        (FormatId::Webm, FormatId::Mp4) => 0.70,
        (FormatId::Flac, FormatId::Mp3) => 0.65,
        (FormatId::Wav, FormatId::Mp3) => 0.60,
        (FormatId::Wav, FormatId::Flac) => 0.35,
        (FormatId::Docx, FormatId::Pdf) => 0.75,
        (FormatId::Odt, FormatId::Pdf) => 0.70,
        (FormatId::Csv, FormatId::Json) => 0.50,
        _ => derived_prior(input, candidate),
    }
}

/// The prior for an operation on the file as it stands.
///
/// Deliberately the flat 0.05 that operations used to get by falling through
/// the pair table, so replacing that table changed the ranking of formats
/// without silently re-ranking every operation beside them. It is low enough
/// that an operation never leads on the prior alone: history, a folder habit,
/// or the user asking for it by name is what raises one.
///
/// It is a number nobody has evidence for, which is precisely what
/// `chosen_rank` in the journal is now accumulating.
pub const OPERATION_PRIOR: f32 = 0.05;

/// The prior for a pair nobody hand-tuned.
///
/// Structure, not enumeration: the kinds either match or they do not, the
/// destination has a weight, and the direction has an asymmetry. Three terms,
/// each of which can be read on its own.
///
/// Cross-kind is low but never zero — `pdf → png` and the OCR and transcription
/// routes are all real, and a hard zero would rank them below pairs that do not
/// route at all.
#[must_use]
pub fn derived_prior(input: FormatId, candidate: FormatId) -> f32 {
    if input == candidate || input == FormatId::Unknown || candidate == FormatId::Unknown {
        return 0.0;
    }
    let same_kind = match (input.kind(), candidate.kind()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    let base = if same_kind { 0.42 } else { 0.10 };
    (base * destination_weight(candidate) * direction_multiplier(input, candidate)).clamp(0.0, 1.0)
}

/// Score one candidate target against the signals.
///
/// Deterministic and total: the same signals always give the same number, and
/// there is no input for which this panics or returns `NaN`. Both matter — the
/// first because a suggestion that changes between two identical drops is worse
/// than no suggestion, and the second because this is fuzzed.
#[must_use]
pub fn score(signals: &Signals, candidate: Target) -> f32 {
    // An operation on the file we already have is a different kind of
    // suggestion: it needs no target format, and its prior is the folder.
    // The FORMAT it reports is the input's, so `folder_multiplier` still sees
    // a real format rather than nothing.
    let candidate_format = match candidate {
        Target::Format(f) => f,
        Target::Operation(_) => signals.input,
    };

    // OPERATIONS DO NOT GO THROUGH THE PAIR PRIOR, and routing them through it
    // was a live bug for exactly as long as the prior had a flat fallback.
    // `global_prior(input, input)` used to return 0.05 because no row matched;
    // the derived prior returns **0.0** for a pair whose ends are equal, which
    // is right for a format conversion and fatal for an operation -- every
    // `Target::Operation` scored zero and no operation could ever be
    // suggested. The two cases are separated here so the distinction cannot be
    // lost again.
    let prior = match candidate {
        Target::Format(f) => global_prior(signals.input, f),
        Target::Operation(_) => OPERATION_PRIOR,
    };

    // TWO LEVELS OF PERSONAL EVIDENCE, INNERMOST LAST.
    //
    // The global habit is blended over the prior, and the folder habit is
    // blended over that. Nesting rather than averaging is what lets a folder
    // with its own settled habit win without a folder seen twice erasing a
    // hundred conversions elsewhere: each blend is weighted by how much
    // evidence that level actually has.
    let mut s = blend(prior, signals.history_to_target, signals.history_count);
    s = blend(
        s,
        signals.folder_history_to_target,
        signals.folder_history_count,
    );

    // The folder's own file listing, which needs no history at all. This is
    // what makes the first drop into an already-organised folder good.
    if let Some(floor) = pattern_floor(signals.folder_pattern) {
        s = s.max(floor);
    }

    // This exact file, already converted once. Leads, does not arm -- see
    // `Signals::sibling_same_stem` for why it is no longer 0.90.
    if signals.sibling_same_stem {
        s = s.max(0.70);
    }

    // A large batch favours format conversion over one-off operations.
    if signals.batch_size > 1 && matches!(candidate, Target::Operation(_)) {
        s *= 0.7;
    }

    // Folder intent.
    s *= folder_multiplier(signals.folder, candidate_format);

    if s.is_finite() {
        s.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Blend `base` towards the ratio `hit / total`, weighted by how much evidence
/// `total` represents.
///
/// Returns `base` unchanged when there is no evidence, which is what makes the
/// two levels composable: a user with no folder history gets exactly the score
/// their global history earned.
fn blend(base: f32, hit: f32, total: f32) -> f32 {
    let w = personal_weight(total);
    if w == 0.0 {
        return base;
    }
    // `total` is positive and finite here, because `personal_weight` returned
    // non-zero, so this cannot divide by zero.
    let ratio = (hit / total).clamp(0.0, 1.0);
    base * (1.0 - w) + ratio * w
}

/// The confidence floor earned by a folder that already contains this exact
/// conversion, done to other files.
///
/// One example is suggestive, three is a habit. Arming at three is the point
/// where "this folder is organised this way" stops being a guess — and the
/// floor never exceeds what a sibling used to grant unconditionally.
fn pattern_floor(pattern: u32) -> Option<f32> {
    match pattern {
        0 => None,
        1 => Some(0.70),
        2 => Some(0.80),
        _ => Some(0.90),
    }
}

fn folder_multiplier(folder: FolderCategory, candidate: FormatId) -> f32 {
    match (folder, candidate) {
        (FolderCategory::Screenshots, FormatId::Jpeg | FormatId::Webp) => 1.2,
        (FolderCategory::Scans, FormatId::Pdf) => 1.3,
        _ => 1.0,
    }
}

/// Rank candidates, best first.
///
/// Ties break by the candidate order given, which the caller controls — so the
/// result is deterministic even when two scores are equal. A ranking that
/// reorders on equal scores makes the UI flicker between identical drops.
///
/// # Why this takes a function and not one `Signals`
///
/// **Four of the fields in [`Signals`] are per-candidate**, not per-file:
/// `history_to_target`, `folder_history_to_target`, `folder_pattern` and
/// `sibling_same_stem` all answer "…for THIS target". One `Signals` shared
/// across a candidate list therefore claims the same history, the same folder
/// pattern and the same existing sibling for every format at once — which
/// gives every candidate an identical floor and produces a ranking that is
/// flat where it should be decisive.
///
/// It used to take `&Signals`, and it was only ever right because the two
/// per-candidate fields it had then were supplied by a caller that rebuilt
/// them per candidate anyway. Passing the builder makes the correct usage the
/// only usage.
#[must_use]
pub fn rank(candidates: &[Target], signals_for: impl Fn(Target) -> Signals) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = candidates
        .iter()
        .map(|&target| Suggestion {
            target,
            score: score(&signals_for(target), target),
        })
        .collect();
    // `sort_by` is stable, so equal scores keep their input order.
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// The confidence at which the top suggestion is **armed** — the UI may run it
/// on Enter without asking.
///
/// `02` §12.2 states the contract this implements: ≥85% arms the top
/// suggestion; below 60% the chooser shows instead of guessing badly; between
/// the two, suggestions are shown and ranked but never auto-run. A wrong armed
/// suggestion costs more than an absent one: the user has to notice it, undo
/// it, and then stop trusting the feature.
///
/// This is a **contract value**, not a tuning knob: callers read it through
/// [`ARM_THRESHOLD`] and ship it to the interface layer rather than
/// hard-coding their own number beside it.
pub const ARM_THRESHOLD: f32 = 0.85;

/// Below this, no suggestion is trusted far enough even to rank first
/// prominently: the UI shows a chooser instead of guessing badly.
///
/// Kept beside [`ARM_THRESHOLD`] so the whole gating contract lives in one
/// place and ships together.
pub const CHOOSER_THRESHOLD: f32 = 0.60;

/// Whether a suggestion is confident enough to arm.
#[must_use]
pub fn should_arm(s: &Suggestion) -> bool {
    s.score >= ARM_THRESHOLD
}

/// Whether a suggestion is confident enough to present as more than a choice:
/// above this line it may lead; below it, the chooser takes over.
#[must_use]
pub fn should_show_lead(s: &Suggestion) -> bool {
    s.score >= CHOOSER_THRESHOLD
}
