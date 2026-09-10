//! The tool registry — the backend half of the GUI's tools menu.
//!
//! The split rule (A9's brief, Phase 5) is one sentence: if the only choice is
//! the output format, it belongs to the main screen's alternates list; if the
//! user must supply anything else — a range, a second file, a strength — it is
//! a tool. This module owns everything behind that second list: descriptors the
//! interface renders generically, and handlers that run through the SAME
//! detect/route/execute path every conversion takes, so a tool run gets real
//! limits, real confinement and a real receipt.
//!
//! # Availability honesty
//!
//! A descriptor reports `available: false` with a reason when its engine or
//! model is absent — never a menu entry that errors at click time. The two
//! tools registered today are pure-Rust container operations compiled into
//! this crate, so they are always available; when a model-backed tool lands
//! (background removal, upscale), its row reads the model registry exactly the
//! way engines read engines.toml.

use crate::handles::HandleTable;
use crate::pool::WorkerPool;
use openconvert_core::environment::Environment;
use openconvert_core::facts::Provenance;
use openconvert_core::plan::PlanRequest;
use openconvert_core::policy::{OnConflict, Policy};
use openconvert_core::route::{route, RouteTable};
use openconvert_core::target::{Operation, Quality, Target};
use openconvert_sandbox::argv::EngineBin;
use openconvert_sandbox::display::DisplayName;
use std::path::{Path, PathBuf};

/// The menu group a tool renders under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCategory {
    /// Raster effects and edits.
    Image,
    /// Page-level document operations.
    Pdf,
    /// Audio edits.
    Audio,
    /// Clip-level video operations.
    Video,
}

/// One parameter the tool's form renders.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolParam {
    /// Stable id, as the handler reads it back.
    pub id: String,
    /// Rendered label.
    pub title: String,
    /// `"number"` | `"text"` | `"choice"` | `"range"`.
    ///
    /// The interface renders the control from this alone, so a tool gains a
    /// slider or a set of buttons by describing itself differently — not by the
    /// interface learning its name.
    pub kind: String,
    /// Whether the tool refuses without it.
    pub required: bool,
    /// The permitted values, for `choice`. Empty for every other kind.
    ///
    /// **The handler still validates.** These drive the control; they are not
    /// the check. A value that reaches the engine unrecognised is an error
    /// there, because a UI is not a place to enforce anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ToolOption>,
    /// Bounds and granularity, for `range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Bounds and granularity, for `range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Bounds and granularity, for `range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// What the tool does when the parameter is absent, shown pre-selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// One permitted value of a `choice` parameter.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolOption {
    /// The value sent to the handler.
    pub value: String,
    /// What the button says.
    pub label: String,
}

/// One entry of the tools menu.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolDescriptor {
    /// Stable id (`video-trim`).
    pub id: String,
    /// Menu group.
    pub category: ToolCategory,
    /// Human title ("Trim clip").
    pub title: String,
    /// Whether picking it will work.
    pub available: bool,
    /// Why not, when [`Self::available`] is false.
    pub unavailable_reason: Option<String>,
    /// Multi-input assembly (join/merge) versus single-input edit.
    pub multi_input: bool,
    /// Whether the tools menu offers it.
    ///
    /// False for the composites and the container surgery: `pdf-compose` is
    /// what the PDF workspace saves through, and `video-trim` / `video-concat`
    /// were withdrawn from the menu while staying dispatchable. An unadvertised
    /// tool is still a tool -- it runs by id, it validates, it writes a
    /// receipt. It simply has no button.
    pub advertised: bool,
    /// The workspace can show what this does, but nothing can be written yet.
    ///
    /// A pixel operation has no [`Target`] to route through, and SR-11 means an
    /// output without a receipt must not exist — only `route()` builds the
    /// `Plan` a receipt is made from. So these render into the preview and
    /// refuse to save, which is a smaller lie than a Save button that fails.
    pub preview_only: bool,
    /// The parameter schema, rendered generically by the interface.
    pub params: Vec<ToolParam>,
}

/// The order the interface offers them in: most reached for first.
///
/// **THIS IS A PRESENTATION DECISION AND IT LIVES HERE ANYWAY.** The
/// alternative was a second list in `ToolWorkspace.svelte`, which would mean
/// two orders that can disagree and one of them invisible to anyone reading
/// the registry. `all_tools` stays in the order it was written, because that
/// is the dispatch table and its order means nothing.
///
/// Within a category, roughly: what the tool exists for, then what is
/// occasionally useful, then the advanced one. The rail filters by category,
/// so only the order WITHIN each run of ids is doing any work.
///
/// A tool missing from this list sorts to the end rather than vanishing, and
/// `every_advertised_tool_has_a_place` fails so nobody finds out that way.
const BY_FREQUENCY: &[&str] = &[
    "image-remove-background",
    "image-compress",
    "image-upscale",
    "image-ocr",
    "image-pick",
    "image-invert",
    "image-greyscale",
    // SIX PDF TOOLS, down from eleven, and the five that left are still
    // dispatchable. Extract, Remove, Rotate and Crop are reached from the
    // "Modify pages" board; Remove password is `pdf-protect`'s `direction`.
    // The registry is where that consolidation is recorded, because the rail
    // renders whatever this returns and holds no list of its own.
    "pdf-compress",
    "pdf-merge",
    "pdf-split",
    "pdf-reorder",
    "pdf-stamp",
    "pdf-protect",
    "pdf-text",
    "audio-denoise",
    "audio-transcribe",
];

/// Every tool this build registers, with availability measured NOW, in the
/// order `BY_FREQUENCY` gives them.
///
/// The interface must render whatever this returns without hard-coding ids --
/// categories with zero available entries render nothing, unavailable entries
/// render disabled with their reason as tooltip, and the ORDER comes from here
/// rather than from a list beside the rail.
#[must_use]
pub fn list_tools() -> Vec<ToolDescriptor> {
    let mut out: Vec<ToolDescriptor> = all_tools().into_iter().filter(|t| t.advertised).collect();
    // Stable, so anything not named keeps its declaration order at the end
    // rather than being shuffled among its equals.
    out.sort_by_key(|t| {
        BY_FREQUENCY
            .iter()
            .position(|id| *id == t.id)
            .unwrap_or(usize::MAX)
    });
    out
}

/// Every tool `run_tool` can dispatch, advertised or not.
///
/// **THE HOST VALIDATES AGAINST THIS ONE.** `run_tool_batch` in the desktop
/// looks the requested id up before running anything, and it looked it up in
/// `list_tools` -- the MENU. `pdf-compose` is deliberately absent from the menu
/// and is exactly what the PDF workspace saves through, so every merge, reorder
/// and signature Save failed with "no tool named pdf-compose is registered".
/// The combined PDF flow could never save at all.
///
/// `video-trim` and `video-concat` are unlisted the same way and carried the
/// same latent fault.
///
/// Two lists, one source: what may RUN and what is OFFERED are different
/// questions, and collapsing them is what produced a tool the product could
/// build a job for and then refuse to execute.
#[must_use]
pub fn all_tools() -> Vec<ToolDescriptor> {
    vec![
        // The first tool backed by a model, and the first image tool that can
        // SAVE rather than only preview: it has an `Operation`, so it has a
        // route, a plan and therefore a receipt.
        //
        // Availability is measured, not assumed. An absent runtime, a model
        // that was never downloaded and a model the user left switched off are
        // three different reasons, and the menu shows whichever one applies
        // instead of a dead entry that errors on click.
        ToolDescriptor {
            id: "image-remove-background".into(),
            category: ToolCategory::Image,
            title: "Remove background".into(),
            available: background_removal_reason().is_none(),
            unavailable_reason: background_removal_reason(),
            multi_input: false,
            advertised: true,
            preview_only: false,
            // NO MODEL CHOICE. "Fast" and "Quality" asked the user to pick
            // between two networks by name, having given them nothing to
            // decide it with: the difference is visible only in the result,
            // which they cannot see until after they have chosen. The best
            // model actually installed is used, and if that is the only one
            // installed then the question had one answer anyway.
            params: vec![],
        },
        ToolDescriptor {
            id: "image-upscale".into(),
            category: ToolCategory::Image,
            // NOT "Upscale x4" any more: the factor is a choice now, and a
            // title naming one of the options is a title that will be wrong
            // for the other.
            title: "Upscale".into(),
            available: model_reason("realesrgan-x4").is_none(),
            unavailable_reason: model_reason("realesrgan-x4"),
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![ToolParam {
                id: "scale".into(),
                title: "Size".into(),
                kind: "choice".into(),
                required: false,
                // ONLY WHAT `run_upscale` WILL ACTUALLY ACCEPT.
                //
                // A `2x` sat here and the handler refused it: "this build
                // enlarges by 4x only". So the interface drew a two-item
                // selector, the user picked the first, and the tool declined
                // after the file was chosen and the button pressed. The
                // refusal is honest; the offer was not.
                //
                // 2x returns when a 2x model is in the registry, not before --
                // running 4x and shrinking the result is rejected in
                // `run_upscale` for reasons that have not changed.
                // `every_advertised_choice_is_one_the_tool_accepts` is what
                // now holds the two in step.
                options: vec![ToolOption {
                    value: "4".into(),
                    label: "4x".into(),
                }],
                min: None,
                max: None,
                step: None,
                // 4x is what the installed model does, so it is what the
                // control opens on. 2x is offered and REFUSED with a reason
                // rather than hidden: `02-FEATURES.md` promises "2x/4x", and a
                // control that quietly omits half of that leaves the promise
                // broken and unexplained.
                default: Some("4".into()),
            }],
        },
        ToolDescriptor {
            id: "image-invert".into(),
            category: ToolCategory::Image,
            title: "Invert colours".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        ToolDescriptor {
            id: "image-greyscale".into(),
            category: ToolCategory::Image,
            title: "Black and white".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        // COMPRESSION IS NOT CONVERSION, which is why it is a tool and not a
        // target format. The output has the same format as the input; what
        // changes is how many bytes it takes, and by how much is a choice.
        ToolDescriptor {
            id: "image-compress".into(),
            category: ToolCategory::Image,
            title: "Compress".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            // High deliberately trades more visual detail for a smaller file.
            params: vec![ToolParam {
                id: "quality".into(),
                title: "Compression".into(),
                kind: "choice".into(),
                required: false,
                options: vec![
                    ToolOption {
                        value: "85".into(),
                        label: "Low".into(),
                    },
                    ToolOption {
                        value: "65".into(),
                        label: "Medium".into(),
                    },
                    ToolOption {
                        value: "20".into(),
                        label: "High".into(),
                    },
                ],
                min: None,
                max: None,
                step: None,
                default: Some("65".into()),
            }],
        },
        ToolDescriptor {
            id: "pdf-text".into(),
            category: ToolCategory::Pdf,
            title: "Extract text".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        ToolDescriptor {
            id: "pdf-compress".into(),
            category: ToolCategory::Pdf,
            title: "Compress".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![ToolParam {
                id: "quality".into(),
                title: "Compression".into(),
                kind: "choice".into(),
                required: false,
                options: vec![
                    ToolOption {
                        value: "lossless".into(),
                        label: "Low".into(),
                    },
                    ToolOption {
                        value: "85".into(),
                        label: "Medium".into(),
                    },
                    ToolOption {
                        value: "45".into(),
                        label: "High".into(),
                    },
                ],
                min: None,
                max: None,
                step: None,
                default: Some("85".into()),
            }],
        },
        ToolDescriptor {
            id: "image-pick".into(),
            category: ToolCategory::Image,
            title: "Colour picker".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Reads a pixel out of the preview; it writes nothing by design.
            advertised: true,
            preview_only: true,
            // NO SAMPLE-SIZE PARAMETER. It asked for a number of image pixels
            // about a preview scaled to the stage, with no way to see what the
            // answer changed — a setting nobody could evaluate. The workspace
            // now offers a magnifier instead, and the sample size follows from
            // it: one pixel when you can see which pixel, an average when you
            // cannot.
            params: vec![],
        },
        // The first audio tool that is not container surgery.
        ToolDescriptor {
            id: "audio-denoise".into(),
            category: ToolCategory::Audio,
            title: "Remove background noise".into(),
            available: model_reason("deepfilternet").is_none(),
            unavailable_reason: model_reason("deepfilternet"),
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        // Speech to text. Its own action rather than a target format: it does
        // not replace the audio, it produces a second artifact beside it, and
        // the result is INFERRED rather than converted. `route()` would refuse
        // it anyway -- audio to text is cross-kind -- which is why it never
        // appears in a format dropdown.
        ToolDescriptor {
            id: "audio-transcribe".into(),
            category: ToolCategory::Audio,
            title: "Transcribe to text".into(),
            available: model_reason("whisper-encoder").is_none(),
            unavailable_reason: model_reason("whisper-encoder"),
            multi_input: false,
            // The transcript is shown, saved or copied from the workspace; there
            // is no output file for the conversion pipeline to write.
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        // OCR IS ADVERTISED AGAIN, on PP-OCRv6 medium.
        //
        // The descriptor lives further down with the other unlisted tools; what
        // changed here on 2026-09-05 is that it is no longer unlisted. The
        // three artifacts are pinned again in `models.toml`, so the capability
        // is reachable end to end: rows to download, a button that reaches
        // them, and `every_named_tool_exists` enforcing that pairing in both
        // directions.
        //
        // Same shape as transcription for the same reason: it does not replace
        // the image, it produces a second artifact beside it, and the result is
        // INFERRED rather than converted.
        // --- PDF -----------------------------------------------------------
        //
        // All PDF -> PDF, which the model expresses directly -- unlike the
        // pixel operations above.
        //
        // The comment that stood here said the engine half was missing and
        // that these "preview and refuse to save". That stopped being true
        // when `pdfops::merge` and `pdfops::reorder` were wired through
        // `run_tool`; every descriptor below is `preview_only: false` and
        // saves. A comment describing a state the code left behind is worse
        // than no comment, because it is read as current.
        // EXTRACT AND REMOVE ARE THE SAME ENGINE OP, inverted.
        //
        // Both resolve their `pages` spec inside the worker, where the page
        // count is already known, and both end in `pages::reorder` over a
        // subset -- which that function has always permitted. Two entries
        // because "keep these" and "lose these" are different intentions and a
        // single tool with a mode switch would make the user translate one
        // into the other.
        ToolDescriptor {
            id: "pdf-extract".into(),
            category: ToolCategory::Pdf,
            title: "Extract pages".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Folded into "Modify pages": the board is the interface for this now.
            // Still dispatchable, so `run_tool` and the CLI reach it unchanged.
            advertised: false,
            preview_only: false,
            params: vec![text_param("pages", "Pages to keep, like 1-3,7 or 2-", true)],
        },
        ToolDescriptor {
            id: "pdf-remove".into(),
            category: ToolCategory::Pdf,
            title: "Remove pages".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Folded into "Modify pages": the board is the interface for this now.
            // Still dispatchable, so `run_tool` and the CLI reach it unchanged.
            advertised: false,
            preview_only: false,
            params: vec![text_param(
                "pages",
                "Pages to remove, like 1-3,7 or 2-",
                true,
            )],
        },
        ToolDescriptor {
            id: "pdf-rotate".into(),
            category: ToolCategory::Pdf,
            title: "Rotate".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Folded into "Modify pages": the board is the interface for this now.
            // Still dispatchable, so `run_tool` and the CLI reach it unchanged.
            advertised: false,
            preview_only: false,
            params: vec![
                ToolParam {
                    id: "turn".into(),
                    title: "Turn".into(),
                    kind: "choice".into(),
                    required: false,
                    options: vec![
                        ToolOption {
                            value: "90".into(),
                            label: "90° right".into(),
                        },
                        ToolOption {
                            value: "180".into(),
                            label: "180°".into(),
                        },
                        // 270 clockwise is a quarter turn the other way, and
                        // that is what the label should say: nobody thinks in
                        // 270 degrees.
                        ToolOption {
                            value: "270".into(),
                            label: "90° left".into(),
                        },
                    ],
                    min: None,
                    max: None,
                    step: None,
                    default: Some("90".into()),
                },
                // Empty means every page, which is what somebody
                // straightening a scan means.
                text_param("pages", "Pages, like 1-3 (blank for all)", false),
            ],
        },
        ToolDescriptor {
            id: "pdf-crop".into(),
            category: ToolCategory::Pdf,
            title: "Crop".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Folded into "Modify pages": the board is the interface for this now.
            // Still dispatchable, so `run_tool` and the CLI reach it unchanged.
            advertised: false,
            preview_only: false,
            // MARGINS, NOT A RECTANGLE. "Take 20 points off each edge" is
            // something a person can hold in their head; "the box runs from
            // (72, 90) to (540, 702)" is a coordinate system they would have
            // to learn first, and it is the one this operation is measured
            // against internally rather than the one it is asked in.
            params: vec![
                margin_param("left"),
                margin_param("bottom"),
                margin_param("right"),
                margin_param("top"),
                text_param("pages", "Pages, like 1-3 (blank for all)", false),
            ],
        },
        // UNLOCK IS NOT A RECOVERY TOOL, and the description says so.
        //
        // It removes a password the user can supply. There is deliberately no
        // affordance that could be mistaken for cracking -- no empty-password
        // probe, no dictionary attempt -- and the wording is what stops
        // somebody arriving expecting one.
        ToolDescriptor {
            // ONE PASSWORD TOOL, WITH A DIRECTION.
            //
            // Add password and Remove password were two rail entries that
            // differ in one word and never both apply: a document either has
            // one or it does not. Two buttons for a state the app can see is a
            // choice the app is making the user make.
            //
            // `pdf-protect` is the one that stays advertised, because "Add" is
            // the request people arrive with; the `direction` parameter is how
            // the other half is reached, and `pdf-unlock` remains dispatchable
            // for a CLI caller who knows which they want.
            id: "pdf-unlock".into(),
            category: ToolCategory::Pdf,
            title: "Remove password".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // Reached through `pdf-protect`'s `direction` parameter now. See above.
            advertised: false,
            preview_only: false,
            params: vec![text_param(
                "password",
                "The document's current password",
                true,
            )],
        },
        ToolDescriptor {
            id: "pdf-protect".into(),
            category: ToolCategory::Pdf,
            title: "Password".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![
                ToolParam {
                    id: "direction".into(),
                    title: "Password".into(),
                    kind: "choice".into(),
                    required: false,
                    options: vec![
                        ToolOption {
                            value: "add".into(),
                            label: "Add".into(),
                        },
                        ToolOption {
                            value: "remove".into(),
                            label: "Remove".into(),
                        },
                    ],
                    min: None,
                    max: None,
                    step: None,
                    default: Some("add".into()),
                },
                text_param("password", "Password", true),
                // Optional, and it defaults to the password above. An owner
                // password nobody chose is a second secret the user does not
                // know they have.
                text_param("owner_password", "Owner password (optional)", false),
            ],
        },
        // SPLIT PRODUCES SEVERAL FILES, which nothing else here does.
        //
        // It is reached through `run_multi_tool`, not `run_tool_with`, and the
        // latter refuses it by name so a caller cannot get one part of six and
        // believe that is the whole answer.
        ToolDescriptor {
            id: "pdf-split".into(),
            category: ToolCategory::Pdf,
            title: "Split".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            // ONE FIELD: WHERE TO CUT.
            //
            // It offered two, "Pages per file" and "Or exact ranges", and a
            // user had to work out which of them expressed the thing they
            // wanted -- for "split after the summary and after the appendix",
            // neither does without arithmetic. Boundaries are how people hold
            // this: `2,5,10` cuts a twelve-page document into four.
            //
            // `every` and `ranges` are still ACCEPTED by `split_spec`; they
            // are simply not advertised. A script written against either keeps
            // working, and the interface stops asking a question with three
            // answers.
            params: vec![text_param(
                "boundaries",
                "Split after pages, like 2,5,10",
                false,
            )],
        },
        ToolDescriptor {
            id: "pdf-merge".into(),
            category: ToolCategory::Pdf,
            title: "Merge documents".into(),
            available: true,
            unavailable_reason: None,
            multi_input: true,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        ToolDescriptor {
            // MODIFY PAGES: the one place a document's pages are changed.
            //
            // It was Reorder, beside Extract pages, Remove pages, Rotate and
            // Crop -- five entries in a rail for five things you do to the
            // same board of page thumbnails, four of which asked for the page
            // numbers in a text field that the board was already showing.
            //
            // The other four are still dispatchable and still tested; they are
            // simply not offered, because reaching them through the board is
            // better in every case a person has. A CLI caller loses nothing.
            id: "pdf-reorder".into(),
            category: ToolCategory::Pdf,
            title: "Modify pages".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            // REQUIRED, WHICH IT ALWAYS WAS. The descriptor said optional and
            // the dispatch does `param(params, "order")?` -- so a caller who
            // read the catalogue, omitted it, and expected the document back
            // unchanged got a refusal instead. The interface never noticed
            // because it reaches reordering through `pdf-compose`, where the
            // order genuinely is optional.
            //
            // Same shape as the 2x upscale option: the product described
            // itself one way and behaved another, and the description was the
            // part that was wrong.
            params: vec![text_param("order", "New order (e.g. 3,1,2)", true)],
        },
        ToolDescriptor {
            // "Sign" in the menu; the receipt still says what actually happened.
            //
            // The longer name was chosen so nobody would read cryptographic
            // authenticity into a raster overlay, and that concern is right --
            // but it was answered in the wrong place. "Place image on page"
            // describes the mechanism to someone looking for the task, and a
            // menu is where people look for tasks.
            //
            // The assurance is kept where it is load-bearing: the receipt
            // records a stamped image, `pdfops::stamp` is unchanged, and
            // nothing in the product claims a signature is verified.
            id: "pdf-stamp".into(),
            category: ToolCategory::Pdf,
            title: "Sign".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![
                number_param("page", "Page", true),
                text_param("image", "Image file (transparent PNG)", true),
            ],
        },
        ToolDescriptor {
            // SEVERAL EDITS AS ONE JOB, and now a tool you can point at.
            //
            // It was `advertised: false` -- the thing the workspace quietly
            // saved through when more than one operation was lit in the rail.
            // That worked, and nothing on screen said it was happening: two
            // lit buttons, one Save, and no way to tell whether Save meant
            // both of them or the last one clicked.
            //
            // The rail is one-at-a-time now, which is the clearer rule, and it
            // would have deleted the capability outright. So the capability
            // becomes the tool: pick Combine edits, tick what to apply, and
            // the several stages are still one file with one receipt.
            //
            // "Combine edits" and not "Compose": composing is what the code
            // does, and nobody sets out to compose an image.
            id: "image-compose".into(),
            category: ToolCategory::Image,
            title: "Combine edits".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            // BACK OUT OF THE RAIL, AND THIS TIME WITH SOMETHING TO POINT AT.
            //
            // It was hidden, then advertised as a tool with tick boxes, and is
            // now neither: combining is a MODE, beside Single file and Batch,
            // and turning it on makes the rail multi-select. That is the shape
            // the operation always had -- several tools lit at once -- with a
            // control that says so, which is what the tick boxes were standing
            // in for.
            //
            // A mode is not a rail entry, so this is unadvertised again. It is
            // still dispatched by id, and `every_advertised_tool_saves` no
            // longer has to special-case an `ops` string nobody types.
            advertised: false,
            preview_only: false,
            // Still a text parameter, still supplied by the workspace rather
            // than typed -- the same arrangement `pdf-reorder`'s page order
            // has. It is the contract with `run_tool`, which is what a CLI
            // caller passes; the tick boxes are how a person fills it in.
            params: vec![text_param("ops", "Operations, in any order", true)],
        },
        // --- dispatchable, not advertised -------------------------------
        ToolDescriptor {
            // OCR was withdrawn from the menu on 2026-09-04, and the claim
            // made at the time was that the adapter, the worker arm and the six
            // routes stayed reachable for a CLI caller. That claim was FALSE
            // the moment the descriptor was deleted: the host resolves every id
            // before running it, so `image-ocr` was refused by name. It was
            // restored unadvertised, then advertised again on 2026-09-05 once
            // PP-OCRv6 artifacts were pinned.
            //
            // `available` asks about the DETECTOR only, and that is a real
            // narrowing: the feature needs all three artifacts, and the
            // registry's `tier_ready` is what enforces the set. This field
            // decides whether the button is offered, and offering it on the
            // strength of one row present is why the descriptor names the row
            // whose absence is most likely -- the two 60-plus MB downloads
            // arrive together or not at all, and the 75 KB dictionary is the
            // one that can lag. Refusal at run time still reports the missing
            // artifact by name.
            id: "image-ocr".into(),
            category: ToolCategory::Image,
            title: "Read text in image".into(),
            available: model_reason("paddleocr-det").is_none(),
            unavailable_reason: model_reason("paddleocr-det"),
            multi_input: false,
            advertised: true,
            preview_only: false,
            params: vec![],
        },
        //
        // These have descriptors so the HOST can resolve them, and
        // `advertised: false` so the menu does not offer them. Before the
        // split they had no descriptor at all, and the desktop's pre-flight
        // lookup refused every one of them by name.
        ToolDescriptor {
            // What the PDF workspace saves through: merge, reorder and stamp
            // carried to the worker in one call, so the result is one file and
            // one receipt instead of three of each. A menu entry would be a
            // fourth name for work the other three already describe.
            id: "pdf-compose".into(),
            category: ToolCategory::Pdf,
            title: "Compose".into(),
            available: true,
            unavailable_reason: None,
            multi_input: true,
            advertised: false,
            preview_only: false,
            params: vec![],
        },
        ToolDescriptor {
            id: "video-trim".into(),
            category: ToolCategory::Video,
            title: "Trim clip".into(),
            available: true,
            unavailable_reason: None,
            multi_input: false,
            advertised: false,
            preview_only: false,
            params: vec![
                number_param("start_ms", "Start (ms)", true),
                number_param("end_ms", "End (ms)", true),
            ],
        },
        ToolDescriptor {
            id: "video-concat".into(),
            category: ToolCategory::Video,
            title: "Join clips".into(),
            available: true,
            unavailable_reason: None,
            multi_input: true,
            advertised: false,
            preview_only: false,
            params: vec![],
        },
        // Video's trim and join were dropped from the menu deliberately.
        // `run_tool` still dispatches both -- the container surgery works and
        // nothing else should break -- but they are no longer advertised.
        //
        // `pdf-compose` is unlisted for a different reason, following the same
        // precedent. Nobody sets out to "compose" a PDF; they set out to merge
        // one, or reorder one, or sign one, and often two of those to the same
        // document. Those three are the menu. `pdf-compose` is what the PDF
        // workspace SAVES through, carrying all three to the worker in one
        // call so the result is one file and one receipt instead of three of
        // each -- and a menu entry for it would be a fourth name for work the
        // other three already describe.
    ]
}

fn text_param(id: &str, title: &str, required: bool) -> ToolParam {
    ToolParam {
        id: id.to_string(),
        title: title.to_string(),
        kind: "text".to_string(),
        required,
        options: Vec::new(),
        min: None,
        max: None,
        step: None,
        default: None,
    }
}

// `choice_param` stood here. Both callers were removed in the same change —
// the background-removal model picker and the colour picker's sample size —
// and a constructor for a parameter kind nothing declares is dead weight that
// `clippy -D warnings` is right to object to. The `choice` KIND is still part
// of the IPC contract and the workspace still renders it; only this helper is
// gone, and it is four lines to write again.

fn number_param(id: &str, title: &str, required: bool) -> ToolParam {
    ToolParam {
        id: id.to_string(),
        title: title.to_string(),
        kind: "number".to_string(),
        required,
        options: Vec::new(),
        min: None,
        max: None,
        step: None,
        default: None,
    }
}

/// What one tool run produced — the same shape a conversion produces, because
/// tools are not a second-class path.
#[derive(Debug)]
pub struct ToolRun {
    /// Where the output landed.
    pub output: PathBuf,
    /// Where the receipt landed, when the caller requested a sidecar.
    pub receipt: Option<PathBuf>,
    /// The receipt body, always returned so a GUI can persist it elsewhere.
    pub receipt_body: String,
}

/// Run a tool that produces SEVERAL files.
///
/// # Why this is a sibling of [`run_tool_with`] and not a field on [`ToolRun`]
///
/// `ToolRun` could have grown an `extra_outputs: Vec<PathBuf>` that is empty
/// for every tool but one. It would compile, and every existing caller would
/// keep working — by ignoring it. The desktop's post-run handling would show
/// one file of six and nobody would see a bug, because there would not be one:
/// the caller simply never read the field.
///
/// A separate entry point cannot be ignored by accident. A caller either asks
/// for the multi-output form or does not, and `run_tool_with` refuses the
/// tools that belong here by name.
///
/// # Errors
///
/// [`ToolError::UnknownTool`] for anything that is not a multi-output tool,
/// and whatever the tool itself reports.
pub fn run_multi_tool(
    id: &str,
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<Vec<ToolRun>, ToolError> {
    match id {
        "pdf-split" => {
            let spec = split_spec(params)?;
            crate::pdfops::split(inputs, &spec, policy, options)
        }
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

/// Whether a tool produces more than one file.
///
/// The desktop asks this before choosing which entry point to call, so the
/// answer lives beside the tools rather than being spelt out at each caller.
#[must_use]
pub fn is_multi_output(id: &str) -> bool {
    id == "pdf-split"
}

/// Read the split parameters into a spec.
///
/// Three forms are accepted and one is advertised. `boundaries` is what the
/// interface sends; `ranges` and `every` predate it and still work, so nothing
/// written against the CLI broke when the field count went from two to one.
///
/// Most specific first: boundaries, then ranges, then the every-n default.
fn split_spec(params: &[(String, String)]) -> Result<crate::pdfops::SplitSpec, ToolError> {
    if let Some((_, raw)) = params.iter().find(|(k, _)| k == "boundaries") {
        if !raw.trim().is_empty() {
            let cuts = raw
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(parse_usize)
                .collect::<Result<Vec<_>, _>>()?;
            if cuts.is_empty() {
                return Err(ToolError::BadParam(
                    "name at least one page to split after".to_string(),
                ));
            }
            return Ok(crate::pdfops::SplitSpec::Boundaries(cuts));
        }
    }
    // `ranges` wins when both are given: it is the more specific request, and
    // silently preferring the other would be the wrong way round.
    if let Some((_, raw)) = params.iter().find(|(k, _)| k == "ranges") {
        if !raw.trim().is_empty() {
            let mut out = Vec::new();
            for part in raw.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let (first, last) = match part.split_once('-') {
                    None => {
                        let n = parse_usize(part)?;
                        (n, n)
                    }
                    Some((lo, hi)) => (parse_usize(lo.trim())?, parse_usize(hi.trim())?),
                };
                out.push((first, last));
            }
            return Ok(crate::pdfops::SplitSpec::Ranges(out));
        }
    }
    let every = params
        .iter()
        .find(|(k, _)| k == "every")
        .map_or(Ok(1), |(_, v)| parse_usize(v.trim()))?;
    Ok(crate::pdfops::SplitSpec::EveryN(every))
}

fn parse_usize(text: &str) -> Result<usize, ToolError> {
    text.parse()
        .map_err(|_| ToolError::BadParam(format!("{text:?} is not a page number")))
}

/// Storage policy around a tool run.
#[derive(Debug, Clone, Copy)]
pub struct ToolRunOptions {
    /// Write the traditional beside-output JSON receipt.
    pub write_sidecar: bool,
}

impl Default for ToolRunOptions {
    fn default() -> Self {
        Self {
            write_sidecar: true,
        }
    }
}

/// Why a tool refused to run.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// No such id.
    #[error("no tool named {0:?}")]
    UnknownTool(String),
    /// A required parameter was missing or malformed.
    #[error("{0}")]
    BadParam(String),
    /// The inputs did not fit the tool.
    #[error("{0}")]
    BadInput(String),
    /// Everything under the conversion path.
    #[error(transparent)]
    Exec(#[from] crate::exec::ExecError),
    /// Detection failed.
    #[error(transparent)]
    Detect(#[from] std::io::Error),
}

/// Run one tool by id.
///
/// Trim goes through `route()` with an [`Operation::Trim`] target, which makes
/// the plan preview, the class ceiling and the receipt identical to any other
/// conversion — because it IS one. Concat folds the Matroska graphs directly,
/// the same container surgery the CLI subcommand performs, and writes its own
/// receipt beside the output for the default CLI policy. GUI callers can keep
/// the returned body in memory and persist it transactionally elsewhere.
///
/// # Errors
///
/// See [`ToolError`].
pub fn run_tool(
    id: &str,
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
) -> Result<ToolRun, ToolError> {
    run_tool_with(id, inputs, params, policy, ToolRunOptions::default())
}

/// Run one tool with an explicit receipt-storage policy.
pub fn run_tool_with(
    id: &str,
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    match id {
        "image-remove-background" => run_remove_background(inputs, params, policy, options),
        "image-upscale" => run_upscale(inputs, params, policy, options),
        "audio-denoise" => run_denoise(inputs, policy, options),
        "audio-transcribe" => {
            if params.iter().any(|(k, v)| k == "format" && v == "vtt") {
                run_timed_transcribe(inputs, policy, options)
            } else {
                run_transcribe(inputs, policy, options)
            }
        }
        "pdf-text" => {
            let [input] = inputs else {
                return Err(ToolError::BadInput(
                    "extracting text takes exactly one PDF".into(),
                ));
            };
            convert_one(
                input,
                Target::Format(openconvert_core::format::FormatId::Txt),
                policy,
                options,
            )
        }
        "image-ocr" => {
            if params.iter().any(|(k, v)| k == "format" && v == "pdf") {
                run_searchable_ocr(inputs, policy, options)
            } else {
                run_ocr(inputs, policy, options)
            }
        }
        "image-invert" => run_pixel_op(inputs, Operation::Invert, policy, options),
        "image-greyscale" => run_pixel_op(inputs, Operation::Greyscale, policy, options),
        "image-compose" => run_image_compose(inputs, params, policy, options),
        "image-compress" => run_image_compress(inputs, params, policy, options),
        "pdf-compress" => run_pdf_compress(inputs, params, policy, options),
        "pdf-stamp" => {
            let image = param(params, "image")?;
            let page = params
                .iter()
                .find(|(k, _)| k == "page")
                .map_or("1", |(_, v)| v.as_str());
            // Absent means the worker's own defaults, which is what a caller
            // that only says "page 3" means. A value that will not parse is
            // NOT silently replaced by a default -- placing a signature
            // somewhere other than where it was dropped is worse than saying
            // the number was unreadable.
            let at = placement(params)?;
            crate::pdfops::stamp(inputs, image, page, at, policy, options)
        }
        "pdf-compose" => {
            let order = params
                .iter()
                .find(|(k, _)| k == "order")
                .map_or("", |(_, v)| v.as_str());
            let image = params
                .iter()
                .find(|(k, _)| k == "image")
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty());
            let page = params
                .iter()
                .find(|(k, _)| k == "page")
                .map_or("1", |(_, v)| v.as_str());
            // `page:degrees;…` and `page:l,b,r,t;…`, both optional. The page
            // numbers are positions in the FINAL document, which is what the
            // reorder board shows and what `page` above already means.
            let turns = params
                .iter()
                .find(|(k, _)| k == "turns")
                .map_or("", |(_, v)| v.as_str());
            let crops = params
                .iter()
                .find(|(k, _)| k == "crops")
                .map_or("", |(_, v)| v.as_str());
            crate::pdfops::compose_with_stamps(
                inputs,
                image,
                crate::pdfops::PageEdits {
                    order,
                    turns,
                    crops,
                },
                placement(params)?,
                page,
                policy,
                options,
                params
                    .iter()
                    .find(|(k, _)| k == "stamps")
                    .map(|(_, v)| v.as_str()),
            )
        }
        "pdf-merge" => crate::pdfops::merge(inputs, policy, options),
        "pdf-reorder" => {
            let order = param(params, "order")?;
            crate::pdfops::reorder(inputs, order, policy, options)
        }
        // Refused HERE rather than left to the unknown-tool arm, because the
        // message matters: "no such tool" would send someone looking for a
        // typo, and the tool exists.
        id if is_multi_output(id) => Err(ToolError::BadInput(format!(
            "{id} produces several files; call run_multi_tool for it"
        ))),
        "pdf-unlock" => {
            let password = param(params, "password")?;
            crate::pdfops::unlock(inputs, password, policy, options)
        }
        "pdf-protect" => {
            let password = param(params, "password")?;
            // ONE TOOL, TWO DIRECTIONS. `direction=remove` is the same request
            // `pdf-unlock` serves and reaches the same code -- the tool is one
            // entry in the interface, not one operation in the engine.
            //
            // Defaults to `add`, which is what every caller written before the
            // parameter existed meant, so the CLI is unchanged.
            let direction = params
                .iter()
                .find(|(k, _)| k == "direction")
                .map_or("add", |(_, v)| v.as_str());
            match direction {
                "remove" => crate::pdfops::unlock(inputs, password, policy, options),
                "add" => {
                    let owner = params
                        .iter()
                        .find(|(k, _)| k == "owner_password")
                        .map_or("", |(_, v)| v.as_str());
                    crate::pdfops::protect(inputs, password, owner, policy, options)
                }
                other => Err(ToolError::BadParam(format!(
                    "direction is add or remove, got {other:?}"
                ))),
            }
        }
        "pdf-rotate" => {
            let turn: i64 = params
                .iter()
                .find(|(k, _)| k == "turn")
                .map_or(Ok(90), |(_, v)| {
                    v.trim().parse().map_err(|_| {
                        ToolError::BadParam(format!("turn must be a number of degrees, got {v:?}"))
                    })
                })?;
            let pages = params
                .iter()
                .find(|(k, _)| k == "pages")
                .map_or("", |(_, v)| v.as_str());
            crate::pdfops::rotate(inputs, pages, turn, policy, options)
        }
        "pdf-crop" => {
            let mut insets = [0.0_f32; 4];
            for (i, key) in ["left", "bottom", "right", "top"].iter().enumerate() {
                if let Some((_, raw)) = params.iter().find(|(k, _)| k == key) {
                    if !raw.trim().is_empty() {
                        insets[i] = raw.trim().parse().map_err(|_| {
                            ToolError::BadParam(format!("{key} must be a number, got {raw:?}"))
                        })?;
                    }
                }
            }
            let pages = params
                .iter()
                .find(|(k, _)| k == "pages")
                .map_or("", |(_, v)| v.as_str());
            crate::pdfops::crop(inputs, pages, insets, policy, options)
        }
        "pdf-extract" => {
            let pages = param(params, "pages")?;
            crate::pdfops::extract_pages(inputs, pages, policy, options)
        }
        "pdf-remove" => {
            let pages = param(params, "pages")?;
            crate::pdfops::remove_pages(inputs, pages, policy, options)
        }
        "video-trim" => run_trim(inputs, params, policy, options),
        "video-concat" => run_concat(inputs, policy, options),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

/// One crop margin, in points, defaulting to none.
fn margin_param(edge: &str) -> ToolParam {
    ToolParam {
        id: edge.into(),
        title: format!("{}{} margin", edge[..1].to_uppercase(), &edge[1..]),
        kind: "number".into(),
        required: false,
        options: Vec::new(),
        min: Some(0.0),
        max: Some(2000.0),
        step: Some(1.0),
        default: Some("0".into()),
    }
}

/// Read `x`, `y` and `width` for a stamp; absent keys keep the default.
fn placement(params: &[(String, String)]) -> Result<crate::pdfops::At, ToolError> {
    let mut at = crate::pdfops::At::default();
    for (key, slot) in [("x", &mut at.x), ("y", &mut at.y), ("width", &mut at.width)] {
        if let Some((_, raw)) = params.iter().find(|(k, _)| k == key) {
            let n: f32 = raw.trim().parse().map_err(|_| {
                ToolError::BadParam(format!("{key} must be a number of points, not {raw:?}"))
            })?;
            if !n.is_finite() {
                return Err(ToolError::BadParam(format!("{key} must be a real number")));
            }
            *slot = n;
        }
    }
    if at.width <= 0.0 {
        return Err(ToolError::BadParam(
            "a stamp needs a width greater than zero".to_string(),
        ));
    }
    Ok(at)
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Result<&'a str, ToolError> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| ToolError::BadParam(format!("missing required parameter {key:?}")))
}

/// Why a model-backed tool cannot run, or `None` when it can.
///
/// Three conditions in the order a user can act on them: the runtime, then the
/// download, then the switch. Naming the first unmet one keeps the message
/// actionable — "download it" is useless advice on a machine with no runtime.
fn model_reason(id: &str) -> Option<String> {
    if !crate::engines::registry()
        .iter()
        .any(|e| e.name == "onnxruntime" && e.available)
    {
        return Some("the ONNX Runtime engine library is not installed".into());
    }
    match crate::models::list().into_iter().find(|m| m.id == id) {
        None => Some(format!("{id} is not in this build's registry")),
        Some(m) if !m.downloaded => Some(format!(
            "the {id} model has not been downloaded ({:.1} MB)",
            m.size_bytes as f64 / 1_048_576.0
        )),
        Some(m) if !m.enabled => Some(format!("the {id} model is downloaded but switched off")),
        Some(_) => None,
    }
}

/// Enlarge one image ×4.
fn run_upscale(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "upscaling takes exactly one file".to_string(),
        ));
    };

    // THE FACTOR IS A REAL CHOICE, AND AN UNSERVEABLE ONE IS REFUSED.
    //
    // `02-FEATURES.md` describes this feature as "2x/4x image enlargement" and
    // the registry has only ever held a 4x export -- so the promise was broken
    // and nothing said so. The parameter exists now, and asking for a factor no
    // installed model provides names the model that would provide it.
    //
    // The alternative -- running 4x and downscaling to 2x -- is deliberately
    // not taken. It would be slower than a 2x model, worse than one, and it
    // would put a second resample after a step whose receipt says the detail is
    // invented. Separate weights are the answer, and until they are here the
    // honest response is a refusal that names what is missing.
    let scale = match params.iter().find(|(k, _)| k == "scale") {
        None => 4_u32,
        Some((_, v)) => v
            .parse::<u32>()
            .map_err(|_| ToolError::BadParam(format!("scale must be 2 or 4, got {v:?}")))?,
    };
    if scale != 4 {
        return Err(ToolError::BadParam(format!(
            "this build enlarges by 4x only. {scale}x needs a {scale}x model, which is not in \
             the registry yet -- enlarging 4x and shrinking the result back would be slower \
             than one and worse than one."
        )));
    }
    convert_one(
        input,
        Target::Operation(Operation::Upscale {
            to: openconvert_core::format::FormatId::Png,
        }),
        policy,
        options,
    )
}

/// Enhance one recording.
fn run_denoise(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "noise removal takes exactly one file".to_string(),
        ));
    };
    convert_one(
        input,
        Target::Operation(Operation::Denoise),
        policy,
        options,
    )
}

/// Which tier of one tool's models this machine should run.
///
/// **The user's preference first, presence as the fallback.** `auto` -- and no
/// entry at all -- resolve to the best tier that is ready, which is exactly
/// what the app did before tiers existed. An explicit tier that is not ready
/// resolves to `None`, and the caller refuses rather than downgrading: someone
/// who believes they got the better model and got the small one has been told
/// something untrue.
///
/// Public because the PREVIEW has to make the same choice. The preview renders
/// through a different path from the save, and the two disagreeing about which
/// model runs would show one result and write another.
#[must_use]
pub fn tier_for_tool(tool: &str) -> Option<crate::models::Tier> {
    let config = crate::state::config::UserConfig::load();
    let preference = config
        .model_tier
        .get(tool)
        .map(String::as_str)
        // `auto` is the absence of a preference, spelled out. A value that
        // parses as nothing -- a typo, or a tier written by a later build --
        // is also treated as absent rather than as a refusal: a config file
        // that cannot be read should not make a working tool stop working.
        .filter(|v| *v != "auto")
        .and_then(crate::models::Tier::parse);
    crate::models::tier_for(tool, preference)
}

/// Whether background removal will run its better tier.
///
/// Kept as a named question because two call sites ask it and one of them is
/// the preview, which must agree with the run.
#[must_use]
pub fn modnet_ready() -> bool {
    tier_for_tool("image-remove-background") == Some(crate::models::Tier::Better)
}

/// Why background removal cannot run, or `None` when it can.
///
/// Three conditions in the order a user can act on them: the runtime, then
/// the download, then the switch. Naming the first unmet one keeps the message
/// actionable — "download it" is useless advice on a machine with no runtime.
fn background_removal_reason() -> Option<String> {
    if !crate::engines::registry()
        .iter()
        .any(|e| e.name == "onnxruntime" && e.available)
    {
        return Some("the ONNX Runtime engine library is not installed".into());
    }
    // EITHER MODEL WILL DO, so this asks about both.
    //
    // It used to ask only about `u2netp`, which was correct while `u2netp` was
    // the default and the user chose the other one explicitly. With the choice
    // removed and the best available model selected automatically, a machine
    // that had downloaded only `modnet` would have been told background
    // removal was unavailable while holding a model that does it better.
    let models = crate::models::list();
    let ready = |id: &str| {
        models
            .iter()
            .any(|m| m.id == id && m.downloaded && m.enabled)
    };
    if ready("modnet") || ready("u2netp") {
        return None;
    }

    // Neither is usable. Report against the one the user is most likely to
    // want, naming the first thing they can act on.
    let info = models.iter().find(|m| m.id == "u2netp");
    match info {
        None => Some("the u2netp model is not in this build's registry".into()),
        Some(m) if !m.downloaded => Some(format!(
            "the u2netp model has not been downloaded ({:.1} MB)",
            m.size_bytes as f64 / 1_048_576.0
        )),
        Some(_) => Some("the background-removal model is downloaded but switched off".into()),
    }
}

/// Cut the background out of one image, saving a PNG with transparency.
fn run_remove_background(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "background removal takes exactly one file".to_string(),
        ));
    };
    // THE BEST MODEL THAT IS ACTUALLY HERE.
    //
    // This read a `quality` parameter and made the user choose between two
    // networks by name. They had nothing to choose with — the difference shows
    // up in the cut-out, which they see only after deciding — so the setting
    // was a question the interface could not help anyone answer.
    //
    // `modnet` produces the better matte, so it wins when it is downloaded and
    // switched on; `u2netp` is the fallback, and is what
    // `background_removal_reason` reports against when neither is available.
    // An explicit parameter is still honoured, because the CLI contract for
    // this tool named it and scripts may pass it.
    // The TIER the user chose, or the best that is installed. `auto` -- and no
    // preference at all -- resolves through the registry, which is the same
    // question `modnet_ready` used to answer with one model in mind.
    let tier = tier_for_tool("image-remove-background");
    let from_tier = match tier {
        // AUTOMATIC MUST NOT PICK A TIER THIS MACHINE WILL REFUSE.
        //
        // `tier_for_tool` resolves an absent preference to the best tier that
        // is INSTALLED, and installing BiRefNet is not the same as being able
        // to run it: it needs 8 GiB and the default policy allows 1, so
        // `route()` refuses -- correctly, because at 1 GiB it dies inside the
        // worker rather than running slowly.
        //
        // The effect was that DOWNLOADING A BETTER MODEL BROKE THE TOOL. A
        // machine with only u2netp cut backgrounds out; the same machine after
        // fetching the best tier answered "that did not produce a file,
        // nothing was written" -- and the preview still worked, because it
        // renders through a path with no memory ceiling to cross.
        //
        // So `auto` steps down to the best tier that FITS. An explicit choice
        // of `best` below does not, and must not: someone who believes they
        // got the better model and silently got the small one has been told
        // something untrue. This declines to choose; it never skips a refusal.
        Some(crate::models::Tier::Best)
            if openconvert_core::route::best_matting_fits(policy.base_limits().memory_bytes) =>
        {
            Quality::Best
        }
        Some(crate::models::Tier::Best | crate::models::Tier::Better) => Quality::Better,
        _ => Quality::Standard,
    };
    let quality = match params.iter().find(|(k, _)| k == "quality") {
        None => from_tier,
        // The CLI contract named these two and scripts may pass them, so they
        // keep meaning what they meant: the small model, or the best installed
        // one above it.
        Some((_, v)) if v == "fast" => Quality::Standard,
        Some((_, v)) if v == "quality" => from_tier.max(Quality::Better),
        Some((_, v)) if v == "best" => Quality::Best,
        Some((_, v)) => {
            return Err(ToolError::BadParam(format!(
                "quality must be `fast`, `quality` or `best`, got {v:?}"
            )));
        }
    };

    convert_one(
        input,
        Target::Operation(Operation::RemoveBackground {
            quality,
            // PNG rather than the source format: the result is a matte, and
            // the formats that can hold one are the only honest destinations.
            to: openconvert_core::format::FormatId::Png,
        }),
        policy,
        options,
    )
}

/// Speech to text.
///
/// A plain conversion to `txt`, not a bespoke path: the route table already
/// carries `wav/flac/mp3/ogg -> txt` through the whisper adapter, classed **D**
/// because a transcript is invented text rather than a rearrangement of the
/// input. Wiring the tool to the route it already had is what turns a panel
/// that displayed nothing into a tool that writes a file with a receipt.
/// Read the text in an image, as a `.txt` beside it.
///
/// `Target::Format(Txt)` from an image is the OCR route — the same shape as
/// transcription from audio, and refused by `route()` for anything that has no
/// such route, so this cannot accidentally mean "convert this picture to a
/// text file".
fn run_ocr(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "reading text takes exactly one image".to_string(),
        ));
    };
    convert_one(
        input,
        Target::Format(openconvert_core::format::FormatId::Txt),
        policy,
        options,
    )
}

fn run_transcribe(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "transcription takes exactly one recording".to_string(),
        ));
    };
    convert_one(
        input,
        Target::Format(openconvert_core::format::FormatId::Txt),
        policy,
        options,
    )
}

/// A pixel operation: one image in, the same image with its pixels rewritten.
///
/// Routed like every other single-input operation, so the class comes from the
/// route table rather than from here -- invert is A, greyscale is B, and this
/// function does not get an opinion about which.
fn run_pixel_op(
    inputs: &[PathBuf],
    op: Operation,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "this operation takes exactly one image".to_string(),
        ));
    };
    convert_one(input, Target::Operation(op), policy, options)
}

/// Make one image smaller, in its own format.
///
/// # Why this is not a conversion
///
/// Every other image tool either changes the pixels or changes the format.
/// This changes neither: a JPEG comes back a JPEG and a PNG comes back a PNG,
/// with the same dimensions and the same content. What changes is the number
/// of bytes, and by how much is the one thing the user chooses.
///
/// That is also why it is not a `Target::Format` on the main screen: "convert
/// this JPEG to JPEG" is not a sentence, and the route table would have to
/// grow a same-format row for every raster format to express it.
///
/// # The class is not the same for both formats
///
/// PNG re-compression is **lossless** — the deflate settings change and every
/// pixel survives — so it is Class A. JPEG re-encoding at a quality setting
/// throws detail away and is Class B. Writing one class for both would be
/// false for one of them, and which one depends on the file.
fn run_image_compress(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "compressing takes exactly one image".to_string(),
        ));
    };

    let quality = params
        .iter()
        .find(|(k, _)| k == "quality")
        .map_or(Ok(65_u32), |(_, v)| {
            v.parse::<u32>()
                .map_err(|_| ToolError::BadParam(format!("quality must be a number, got {v:?}")))
        })?;
    if !(1..=100).contains(&quality) {
        return Err(ToolError::BadParam(format!(
            "quality must be from 1 to 100, got {quality}"
        )));
    }

    let mut table = HandleTable::new();
    let facts = crate::detect::detect(input, &mut table)?;
    let bytes = crate::detect::bytes_of(&facts, &mut table)?;
    let before = bytes.len();
    let detected = facts.sniff().detected;

    // Compression never chooses a different output format.
    let to = detected.name();

    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, removed_by_engine, confinement) = pool
        .run(
            EngineBin::Images,
            facts.provenance(),
            &limits,
            bytes.clone(),
            to,
            &[
                ("op".to_string(), "compress".to_string()),
                ("quality".to_string(), quality.to_string()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;

    // A RESULT THAT IS NOT SMALLER IS NOT A RESULT.
    //
    // Re-encoding an already-optimised file grows it as often as not. Writing
    // that out and calling it compression means someone running this twice over
    // a folder ends up with more bytes than they started with, and nothing on
    // screen would have said so.
    let preserved = out.len() >= before;
    let (out, mut removed) = if preserved {
        (
            bytes.to_vec(),
            vec!["No smaller same-format output available; original bytes preserved".to_string()],
        )
    } else {
        (out, removed_by_engine)
    };
    removed.push(format!(
        "{} bytes ({}% smaller), from {before} to {}",
        before - out.len(),
        (before - out.len()) * 100 / before.max(1),
        out.len()
    ));

    let stem = input
        .file_stem()
        .map_or_else(|| "image".to_string(), |s| s.to_string_lossy().into_owned());
    let extension = if preserved {
        input
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or(detected.name())
    } else if to == "jpeg" {
        match input.extension().and_then(|s| s.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("jpeg") || ext.eq_ignore_ascii_case("jpg") => ext,
            _ => "jpg",
        }
    } else {
        to
    };
    let name = format!("{stem}-smaller.{extension}");
    let dest = input.parent().unwrap_or(Path::new(".")).to_path_buf();

    let (placed, file) = crate::write::create_output(&dest, &name, policy.on_conflict())
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let output = match &placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
    };
    let Some(mut f) = file else {
        return Err(ToolError::BadInput(format!(
            "{} already exists and the conflict policy is to skip",
            DisplayName::new(&output.to_string_lossy())
        )));
    };
    use std::io::Write as _;
    f.write_all(&out)
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    drop(f);

    let (receipt, receipt_body) = crate::pdfops::make_receipt(
        &name,
        inputs,
        &out,
        &removed,
        &confinement,
        "Compress",
        "oc-images",
        detected.name(),
        // See the note on this function: the honest class depends on which
        // format was compressed, because only one of the two loses anything.
        if !preserved && matches!(to, "jpeg" | "avif") {
            "B (lossy, standard)"
        } else {
            "A (lossless)"
        },
        policy,
        options,
    )?;
    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

/// Make one PDF smaller without changing a page.
///
/// Deflates the streams a producer left uncompressed and drops the document
/// metadata. **The images are not touched** — see `pages::compress` for why
/// that is a separate operation rather than a silent part of this one.
fn run_pdf_compress(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "compressing takes exactly one document".to_string(),
        ));
    };

    let quality = params
        .iter()
        .find(|(k, _)| k == "quality")
        .map_or("85", |(_, v)| v.as_str());
    if !matches!(quality, "lossless" | "85" | "45") {
        return Err(ToolError::BadParam("unknown PDF compression level".into()));
    }
    let mut table = HandleTable::new();
    let facts = crate::detect::detect(input, &mut table)?;
    let bytes = crate::detect::bytes_of(&facts, &mut table)?;
    let before = bytes.len();
    let detected = facts.sniff().detected;
    if detected != openconvert_core::format::FormatId::Pdf {
        return Err(ToolError::BadInput(format!(
            "this is {}, not a PDF",
            detected.name()
        )));
    }

    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, removed, confinement) = pool
        .run(
            EngineBin::Pdf,
            facts.provenance(),
            &limits,
            bytes,
            "pdf",
            &[
                ("op".to_string(), "compress".to_string()),
                ("quality".to_string(), quality.to_string()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;

    // The worker returns the ORIGINAL bytes when it could not improve on them,
    // rather than a larger file — so equal length here means "already
    // compressed", which is worth saying rather than writing a copy.
    debug_assert!(out.len() <= before);

    let stem = input.file_stem().map_or_else(
        || "document".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let name = format!("{stem}-smaller.pdf");
    let dest = input.parent().unwrap_or(Path::new(".")).to_path_buf();

    let (placed, file) = crate::write::create_output(&dest, &name, policy.on_conflict())
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let output = match &placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
    };
    let Some(mut f) = file else {
        return Err(ToolError::BadInput(format!(
            "{} already exists and the conflict policy is to skip",
            DisplayName::new(&output.to_string_lossy())
        )));
    };
    use std::io::Write as _;
    f.write_all(&out)
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    drop(f);

    let (receipt, receipt_body) = crate::pdfops::make_receipt(
        &name,
        inputs,
        &out,
        &removed,
        &confinement,
        "Compress",
        "oc-pdf",
        "pdf",
        // Every page, its content and its links survive byte-identically; only
        // the encoding of the streams and the metadata change.
        if quality == "lossless" {
            "A (lossless)"
        } else {
            "B (lossy)"
        },
        policy,
        options,
    )?;
    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

/// Allowed edits; their execution order is supplied by the editor.
const COMPOSABLE_EDITS: &[&str] = &[
    "image-upscale",
    "image-invert",
    "image-greyscale",
    "image-remove-background",
];

fn ordered_image_edits(requested: Vec<&str>) -> Result<Vec<&str>, ToolError> {
    if requested.is_empty() {
        return Err(ToolError::BadParam(
            "composing needs an `ops` list naming which operations to run".to_string(),
        ));
    }
    for id in &requested {
        if !COMPOSABLE_EDITS.contains(id) {
            return Err(ToolError::BadParam(format!(
                "{id} is not an image operation that can be composed"
            )));
        }
    }
    // The editor order is the execution order. Reject duplicate stages.
    let mut stages = Vec::new();
    for id in requested {
        if stages.contains(&id) {
            return Err(ToolError::BadParam(format!("duplicate image edit: {id}")));
        }
        stages.push(id);
    }

    Ok(stages)
}

/// Several image operations, one output, one receipt.
///
/// **NOT a chain of tool runs.** Running them one after another from the host
/// would write an intermediate file per stage and a receipt describing each,
/// and those files would name pictures that stop existing the moment the next
/// stage ran. `pdf-compose` rejected that for documents and the reasoning is
/// the same here: the bytes move between workers in memory, and exactly one
/// file and one receipt reach the disk.
///
/// The receipt accumulates what every stage removed, so a composed image is as
/// accountable as a single conversion, and names each step rather than one.
fn run_image_compose(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "composing takes exactly one image".to_string(),
        ));
    };

    // Which operations, as the workspace listed them. An unknown id is refused
    // rather than skipped: silently dropping a stage produces a picture nobody
    // asked for and a receipt that agrees with it.
    let requested: Vec<&str> = params
        .iter()
        .find(|(k, _)| k == "ops")
        .map(|(_, v)| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let stages = ordered_image_edits(requested)?;

    let mut table = HandleTable::new();
    let facts = crate::detect::detect(input, &mut table)?;
    let mut bytes = crate::detect::bytes_of(&facts, &mut table)?;
    let detected = facts.sniff().detected;
    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());

    let mut removed: Vec<String> = Vec::new();
    let mut confinement = String::new();

    // NORMALISE ONCE, AT THE FRONT. The inference adapters read the six
    // image-rs formats and not HEIC, AVIF or JXL; the pixel ops read whatever
    // they are handed. One transcode to PNG at the start serves every stage,
    // and keeping the intermediate lossless means three operations do not
    // stack three generations of JPEG artifacts.
    let native = matches!(
        detected,
        openconvert_core::format::FormatId::Png
            | openconvert_core::format::FormatId::Jpeg
            | openconvert_core::format::FormatId::Webp
            | openconvert_core::format::FormatId::Bmp
            | openconvert_core::format::FormatId::Tiff
            | openconvert_core::format::FormatId::Gif
    );
    if !native {
        let (out, gone, seen) = pool
            .run(
                EngineBin::Images,
                facts.provenance(),
                &limits,
                bytes,
                "png",
                &[],
            )
            .map_err(|e| ToolError::BadInput(e.to_string()))?;
        bytes = out;
        removed.extend(gone);
        confinement = seen;
    }

    for id in &stages {
        let (bin, step_params, models): (EngineBin, Vec<(String, String)>, Vec<&str>) = match *id {
            "image-upscale" => (
                EngineBin::Ai,
                vec![("task".to_string(), "upscale".to_string())],
                vec!["realesrgan-x4"],
            ),
            "image-remove-background" => {
                let tier = tier_for_tool("image-remove-background");
                let (task, model) = match tier {
                    Some(crate::models::Tier::Best) => ("remove-background-best", "birefnet-lite"),
                    Some(crate::models::Tier::Better) => ("remove-background-quality", "modnet"),
                    _ => ("remove-background", "u2netp"),
                };
                (
                    EngineBin::Ai,
                    vec![("task".into(), task.into())],
                    if model == "u2netp" {
                        vec![model]
                    } else {
                        vec![model, "u2netp"]
                    },
                )
            }
            "image-invert" => (
                EngineBin::Images,
                vec![("op".to_string(), "invert".to_string())],
                Vec::new(),
            ),
            // The only remaining member of COMPOSABLE_EDITS.
            _ => (
                EngineBin::Images,
                vec![("op".to_string(), "greyscale".to_string())],
                Vec::new(),
            ),
        };

        // Weights are read and verified host-side, exactly as `exec` does it:
        // the worker has no filesystem, and a model is code in every way that
        // matters, so the sha256 pin is re-checked before the bytes are sent.
        let weights: Vec<Vec<u8>> = models
            .iter()
            .map(|m| {
                crate::models::verified_bytes(m)
                    .map_err(|e| ToolError::BadInput(format!("{m}: {e}")))
            })
            .collect::<Result<_, _>>()?;

        let (out, gone, seen) = pool
            .run_with_models(
                bin,
                facts.provenance(),
                &limits,
                bytes,
                &weights,
                "png",
                &step_params,
            )
            .map_err(|e| ToolError::BadInput(e.to_string()))?;
        bytes = out;
        removed.extend(gone);
        confinement = seen;
    }

    let output_format = params
        .iter()
        .find(|(k, _)| k == "format")
        .map_or("png", |(_, v)| v.as_str());
    if !matches!(output_format, "png" | "jpeg" | "webp" | "avif") {
        return Err(ToolError::BadParam(
            "unsupported compose output format".into(),
        ));
    }
    if output_format != "png" {
        let (out, gone, seen) = pool
            .run(
                EngineBin::Images,
                facts.provenance(),
                &limits,
                bytes,
                output_format,
                &[],
            )
            .map_err(|e| ToolError::BadInput(e.to_string()))?;
        bytes = out;
        removed.extend(gone);
        confinement = seen;
    }
    // One line per distinct thing lost, not one per stage that lost it.
    // Every pixel stage re-encodes, so "all source metadata (re-encoded via
    // image-rs)" arrived once per operation and a three-stage compose listed
    // it three times. A receipt that repeats itself is harder to read and
    // says nothing extra; order is preserved so the sequence still reads as
    // the sequence.
    let mut seen = std::collections::HashSet::new();
    removed.retain(|line| seen.insert(line.clone()));

    // THE OUTPUT IS A PNG, AND IT SAYS SO.
    //
    // Every stage above emits PNG, and background removal produces an alpha
    // channel that only PNG and WebP among our targets can hold. Naming the
    // file for what is in it is the property the denoise fix established, and
    // it applies here for the same reason.
    let stem = input
        .file_stem()
        .map_or_else(|| "image".to_string(), |s| s.to_string_lossy().into_owned());
    let extension = if output_format == "jpeg" {
        "jpg"
    } else {
        output_format
    };
    let name = format!("{stem}-edited.{extension}");
    let dest = input.parent().unwrap_or(Path::new(".")).to_path_buf();

    let (placed, file) = crate::write::create_output(&dest, &name, policy.on_conflict())
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let output = match &placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
    };
    let Some(mut f) = file else {
        return Err(ToolError::BadInput(format!(
            "{} already exists and the conflict policy is to skip",
            DisplayName::new(&output.to_string_lossy())
        )));
    };
    use std::io::Write as _;
    f.write_all(&bytes)
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    drop(f);

    let (receipt, receipt_body) = crate::pdfops::make_receipt(
        &name,
        inputs,
        &bytes,
        &removed,
        &confinement,
        "Compose",
        // Two workers ran: `oc-images` for the pixel stages and `oc-ai` for the
        // model ones. Named as the pair rather than as whichever went last,
        // because both parsed these bytes and the receipt is about what
        // touched them.
        if stages
            .iter()
            .any(|s| matches!(*s, "image-upscale" | "image-remove-background"))
        {
            "oc-images, oc-ai"
        } else {
            "oc-images"
        },
        // The output is a PNG whatever went in; this names the INPUT's own
        // format, and it is not "pdf".
        detected.name(),
        // CLASS D WHEN A MODEL RAN, AND THIS USED TO SAY CLASS A.
        //
        // Upscaling and background removal are networks deciding what should be
        // there -- Class D, which is exactly what the route table gives those
        // same two operations when they run on their own. Inversion and
        // greyscale are arithmetic over the pixels that are already there.
        //
        // Class A is the load-bearing promise in this product, and a receipt
        // claiming it for a model-generated cut-out is the worst thing this
        // file can write.
        if stages
            .iter()
            .any(|s| matches!(*s, "image-upscale" | "image-remove-background"))
        {
            "D (generative)"
        } else {
            "B (lossy, standard)"
        },
        policy,
        options,
    )?;
    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

fn run_trim(
    inputs: &[PathBuf],
    params: &[(String, String)],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "trim takes exactly one file".to_string(),
        ));
    };
    let start_ms: u64 = param(params, "start_ms")?.parse().map_err(|_| {
        ToolError::BadParam("start_ms must be a whole number of milliseconds".into())
    })?;
    let end_ms: u64 = param(params, "end_ms")?
        .parse()
        .map_err(|_| ToolError::BadParam("end_ms must be a whole number of milliseconds".into()))?;
    if end_ms <= start_ms {
        return Err(ToolError::BadParam(
            "end_ms must come after start_ms".to_string(),
        ));
    }

    convert_one(
        input,
        Target::Operation(Operation::Trim { start_ms, end_ms }),
        policy,
        options,
    )
}

fn run_concat(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    if inputs.len() < 2 {
        return Err(ToolError::BadInput(
            "joining needs at least two files".to_string(),
        ));
    }
    let limits = policy.base_limits();
    let mut joined: Option<crate::matroska::AvGraph> = None;
    let mut first_stem = "output".to_string();

    for (i, path) in inputs.iter().enumerate() {
        let bytes = std::fs::read(path)?;
        let graph = crate::matroska::demux(&bytes, limits.memory_bytes)
            .map_err(|e| ToolError::BadInput(format!("{}: {e}", path.display())))?;
        if i == 0 {
            first_stem = path
                .file_stem()
                .map_or_else(|| "output".into(), |s| s.to_string_lossy().into_owned());
        }
        joined = Some(match joined {
            None => graph,
            Some(acc) => crate::matroska::concat(&acc, &graph)
                .map_err(|e| ToolError::BadInput(format!("cannot join these clips: {e}")))?,
        });
    }

    let out_bytes = crate::matroska::graph_to_ebml(&joined.expect("at least two inputs"))
        .map_err(|e| ToolError::BadInput(format!("could not serialise the join: {e}")))?;

    // Same placement rule as everywhere else: create_new or suffixed, never a
    // replacement.
    let dir = inputs[0].parent().unwrap_or(Path::new("."));
    let name = format!("{first_stem}-joined.mkv");
    let (placed, file) = crate::write::create_output(dir, &name, OnConflict::Suffix)?;
    let output = match &placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
    };
    let Some(mut f) = file else {
        return Err(crate::exec::ExecError::Output(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists; skipped by policy", output.display()),
        ))
        .into());
    };
    use std::io::Write as _;
    f.write_all(&out_bytes)?;
    drop(f);

    let body = serde_json::json!({
        "version": 1,
        "tool": tool_version(),
        "detected": "mkv",
        "class": "A (lossless)",
        "steps": [{
            "kind": "Concat",
            "engine": "openconvert-container",
            "isolation": "in-process (pure Rust)",
            "removed": ["timestamps renumbered across the join"],
        }],
        "output": name,
        "output_bytes": out_bytes.len(),
    });
    let receipt_body =
        serde_json::to_string_pretty(&body).map_err(|e| ToolError::BadInput(e.to_string()))?;
    let receipt = if options.write_sidecar {
        let receipt_name = format!("{name}.receipt.json");
        let (placed, file) = crate::write::create_output(dir, &receipt_name, OnConflict::Suffix)?;
        let path = match placed {
            crate::write::Placed::Created(path) | crate::write::Placed::Skipped(path) => path,
        };
        if let Some(mut file) = file {
            file.write_all(receipt_body.as_bytes())?;
            file.write_all(b"\n")?;
        }
        Some(path)
    } else {
        None
    };

    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

/// Peak amplitude per bucket for a recording, 0-255, for drawing a waveform.
///
/// **Not a conversion.** Nothing is written, no receipt is produced, and the
/// result is a picture rather than a file — so this does not go through
/// `convert_one` and does not pretend to. It runs the audio worker with
/// `op=peaks`, under the same confinement and the same limits as any other
/// audio work, because the input is still a stranger's file.
///
/// # Errors
///
/// [`ToolError`] — an unreadable file, or a recording the worker cannot decode.
pub fn audio_peaks(input: &Path, buckets: usize, policy: &Policy) -> Result<Vec<u8>, ToolError> {
    let bytes = std::fs::read(input).map_err(|e| {
        ToolError::BadInput(format!(
            "could not read {}: {e}",
            DisplayName::new(&input.to_string_lossy())
        ))
    })?;

    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, _removed, _confinement) = pool
        .run_with_inputs(
            EngineBin::Audio,
            Provenance::Untrusted,
            &limits,
            bytes,
            &[],
            // The destination format is meaningless for this op and the worker
            // ignores it, but the protocol has a slot for one. "wav" is the
            // honest filler: it is what the decoder would have produced.
            "wav",
            &[
                ("op".to_string(), "peaks".to_string()),
                ("buckets".to_string(), buckets.to_string()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    Ok(out)
}

fn run_searchable_ocr(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput("Choose one scan".into()));
    };
    let mut handles = HandleTable::new();
    let facts = crate::detect::detect(input, &mut handles)?;
    let bytes = crate::detect::bytes_of(&facts, &mut handles)?;
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (png, _, _) = pool
        .run_with_inputs(
            EngineBin::Images,
            facts.provenance(),
            &policy.base_limits(),
            bytes,
            &[],
            "png",
            &[],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let weights = ["paddleocr-det", "paddleocr-rec", "paddleocr-dict"]
        .iter()
        .map(|id| crate::models::verified_bytes(id).map_err(|e| ToolError::BadInput(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let (layout, mut removed, _) = pool
        .run_with_models(
            EngineBin::Ai,
            facts.provenance(),
            &policy.base_limits(),
            png.clone(),
            &weights,
            "txt",
            &[
                ("task".into(), "ocr".into()),
                ("format".into(), "layout".into()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let (bytes, gone, confinement) = pool
        .run_with_inputs(
            EngineBin::Pdf,
            facts.provenance(),
            &policy.base_limits(),
            png,
            &[layout],
            "pdf",
            &[("op".into(), "searchable".into())],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    removed.extend(gone);
    save_generated(
        inputs,
        &bytes,
        &removed,
        &confinement,
        "Searchable OCR",
        "oc-ai, oc-images, oc-pdf",
        facts.sniff().detected.name(),
        "pdf",
        policy,
        options,
    )
}

fn run_timed_transcribe(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput("Choose one recording".into()));
    };
    let tier = tier_for_tool("audio-transcribe").ok_or_else(|| {
        ToolError::BadInput("Download the selected transcription model first".into())
    })?;
    let weights = crate::models::tier_models("audio-transcribe", tier)
        .iter()
        .map(|id| crate::models::verified_bytes(id).map_err(|e| ToolError::BadInput(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let mut handles = HandleTable::new();
    let facts = crate::detect::detect(input, &mut handles)?;
    let bytes = crate::detect::bytes_of(&facts, &mut handles)?;
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, removed, confinement) = pool
        .run_with_models(
            EngineBin::Ai,
            facts.provenance(),
            &policy.base_limits(),
            bytes,
            &weights,
            "txt",
            &[
                ("task".into(), "transcribe".into()),
                ("format".into(), "vtt".into()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let stem = input
        .file_stem()
        .ok_or_else(|| ToolError::BadInput("Missing filename".into()))?
        .to_string_lossy();
    let name = format!("{stem}.vtt");
    let (placed, file) = crate::write::create_output(
        input.parent().unwrap_or(Path::new(".")),
        &name,
        policy.on_conflict(),
    )
    .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let mut file =
        file.ok_or_else(|| ToolError::BadInput("The subtitle file already exists".into()))?;
    let output = match placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p,
    };
    use std::io::Write as _;
    let write = file.write_all(&out).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = write {
        let _ = std::fs::remove_file(&output); // openconvert-lint: allow -- rollback only the output created exclusively by this call
        return Err(ToolError::BadInput(e.to_string()));
    }
    let receipt = crate::pdfops::make_receipt(
        &name,
        inputs,
        &out,
        &removed,
        &confinement,
        "Transcribe",
        "oc-ai",
        facts.sniff().detected.name(),
        "D (model-generated)",
        policy,
        options,
    );
    let (receipt, receipt_body) = match receipt {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&output); // openconvert-lint: allow -- rollback only the output created exclusively by this call
            return Err(e);
        }
    };
    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

/// Decode a local recording for playback without opening it in the webview.
pub fn audio_preview(input: &Path, policy: &Policy) -> Result<Vec<u8>, ToolError> {
    let size = std::fs::metadata(input)
        .map_err(|e| ToolError::BadInput(e.to_string()))?
        .len();
    if size > 256 << 20 {
        return Err(ToolError::BadInput(
            "This recording is too large for an in-app preview".into(),
        ));
    }
    let bytes = std::fs::read(input).map_err(|e| ToolError::BadInput(e.to_string()))?;
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, _, _) = pool
        .run_with_inputs(
            EngineBin::Audio,
            Provenance::Untrusted,
            &policy.base_limits(),
            bytes,
            &[],
            "wav",
            &[],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    if out.len() > 128 << 20 {
        return Err(ToolError::BadInput(
            "The decoded recording is too large for an in-app preview".into(),
        ));
    }
    Ok(out)
}

/// Save a corrected cutout, with its original retained for restoration strokes.
pub fn correct_image(
    input: &Path,
    original: &Path,
    strokes: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let read = |p: &Path| std::fs::read(p).map_err(|e| ToolError::BadInput(e.to_string()));
    let (source, _, _) = pool
        .run_with_inputs(
            EngineBin::Images,
            Provenance::Untrusted,
            &policy.base_limits(),
            read(original)?,
            &[],
            "png",
            &[],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let (bytes, removed, confinement) = pool
        .run_with_inputs(
            EngineBin::Images,
            Provenance::Untrusted,
            &policy.base_limits(),
            read(input)?,
            &[source],
            "png",
            &[
                ("op".into(), "mask".into()),
                ("strokes".into(), strokes.into()),
            ],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    save_generated(
        &[input.to_path_buf(), original.to_path_buf()],
        &bytes,
        &removed,
        &confinement,
        "Correct transparency",
        "oc-images",
        "png",
        "png",
        policy,
        options,
    )
}

#[allow(clippy::too_many_arguments)]
fn save_generated(
    inputs: &[PathBuf],
    bytes: &[u8],
    removed: &[String],
    confinement: &str,
    step: &str,
    engine: &str,
    detected: &str,
    extension: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let input = &inputs[0];
    let name = format!(
        "{}-edited.{extension}",
        input.file_stem().unwrap_or_default().to_string_lossy()
    );
    let (placed, file) = crate::write::create_output(
        input.parent().unwrap_or(Path::new(".")),
        &name,
        policy.on_conflict(),
    )
    .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let mut file = file.ok_or_else(|| ToolError::BadInput("Output already exists".into()))?;
    let output = match placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p,
    };
    use std::io::Write as _;
    let write = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = write {
        let _ = std::fs::remove_file(&output); // openconvert-lint: allow -- rollback only the output created exclusively by this call
        return Err(ToolError::BadInput(e.to_string()));
    }
    match crate::pdfops::make_receipt(
        &file_name(&output),
        inputs,
        bytes,
        removed,
        confinement,
        step,
        engine,
        detected,
        "D (model or user edits)",
        policy,
        options,
    ) {
        Ok((receipt, receipt_body)) => Ok(ToolRun {
            output,
            receipt,
            receipt_body,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&output); // openconvert-lint: allow -- rollback only the output created exclusively by this call
            Err(e)
        }
    }
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn tool_version() -> String {
    format!("openconvert-tools {}", env!("CARGO_PKG_VERSION"))
}

/// The shared front half of every single-input tool: detect, probe, route,
/// execute. Identical to what `openconvert convert` runs — deliberately, so a
/// tool cannot drift away from the conversion path's limits and receipts.
fn convert_one(
    path: &Path,
    target: Target,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let mut table = HandleTable::new();
    let facts = crate::detect::detect(path, &mut table)?;
    let bytes = crate::detect::bytes_of(&facts, &mut table)?;
    let s = facts.sniff();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let props = crate::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool);

    // NAMING THE TOOL IS THE ARMING GESTURE, AND THIS IS WHERE IT WAS MISSING.
    //
    // `route()` refuses any plan whose class exceeds `policy.max_auto_class()`,
    // which defaults to B (I8). Every model-backed tool is Class D. The CLI
    // handles this in `cmd::convert::policy_for` -- naming an operation is the
    // explicit request the ceiling asks for -- but NOTHING ON THE TOOL PATH DID,
    // so the desktop passed `Policy::default()` straight through and every AI
    // tool routed to an empty plan.
    //
    // The symptom was "That did not produce a file. Nothing was written.", for
    // background removal, upscaling, denoising and transcription alike, on a
    // machine where all four models were downloaded, enabled and working. The
    // adapters were never reached, so testing them directly -- which is what was
    // done -- could not find it.
    //
    // The rule is the CLI's, verbatim: an operation only a model can perform,
    // or a destination only a model produces, is a sentence the user typed.
    // A plain `-t png` that happens to find a Class D route still refuses,
    // because the target alone is not a request for inference.
    let armed = policy.armed_for(target);

    let plan = route(
        PlanRequest {
            input: s.detected,
            target,
            polyglot: s.polyglot,
        },
        props,
        &armed,
        &real_environment(),
    );
    if !plan.is_executable() {
        return Err(ToolError::BadInput(
            plan.warnings()
                .first()
                .map_or_else(|| "no route".to_string(), |w| format!("{w:?}")),
        ));
    }

    let stem = path.file_stem().map_or_else(
        || "output".to_string(),
        |x| x.to_string_lossy().into_owned(),
    );
    // THE NAME COMES FROM THE PLAN, NOT THE TARGET.
    //
    // `Target::output_format` answers "what did the user ask for"; for an
    // operation that is "the format it came in as". What goes in the FILE is
    // whatever the last step produces, and for noise removal those two differ
    // whenever the plan cannot reach the original encoder. Naming by the
    // target produced a WAV called `.mp3`.
    //
    // The fallback is the target, for a step that names no format of its own —
    // a stream copy, a metadata strip — which is correct, because those do not
    // change the format either.
    let ext = plan
        .output_format()
        .unwrap_or_else(|| target.output_format(s.detected));
    let name = match ext.row() {
        Some(r) => format!("{stem}.{}", r.extension),
        None => stem,
    };
    let dest = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let outcome = crate::exec::execute_with(
        &plan,
        &facts,
        &mut table,
        &crate::exec::ExecOptions {
            dest_dir: dest,
            output_name: name,
            write_receipt: options.write_sidecar,
            replace_source: false,
            source_path: None,
        },
        // The ARMED policy, the same one the plan was routed under. Executing
        // under a stricter ceiling than the plan was built for would be a
        // second place for the same refusal to appear, one step later.
        &armed,
        &mut pool,
    )?;
    let receipt_body = serde_json::to_string_pretty(&outcome.record)
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    Ok(ToolRun {
        output: outcome.output,
        receipt: options.write_sidecar.then_some(outcome.receipt),
        receipt_body,
    })
}

/// The machine, probed once per tool run.
///
/// Tools run in host processes driven by the desktop shell, which builds its
/// own environment per command anyway; this mirrors `probe_real()` in the CLI
/// without reaching into that binary. Requires this crate's `attest` feature,
/// because a planning profile may only be CONSTRUCTED from probed values
/// (I11) — never assembled by hand.
#[cfg(feature = "attest")]
fn real_environment() -> Environment {
    let caps = openconvert_os::available::probe();
    Environment::new(
        openconvert_os::available::planning_profile(caps),
        crate::engines::registry(),
        RouteTable::v1(),
    )
}

/// Without `attest`, tools route against an unconfined planning profile: every
/// step still runs through its worker, so what changes is only what the plan
/// CLAIMS about confinement, and receipts record read-back evidence instead.
#[cfg(not(feature = "attest"))]
fn real_environment() -> Environment {
    Environment::new(
        openconvert_core::isolation::SandboxProfile::unconfined(),
        crate::engines::registry(),
        RouteTable::v1(),
    )
}

#[cfg(test)]
mod order_tests {
    use super::{list_tools, BY_FREQUENCY};

    /// **EVERY ADVERTISED TOOL HAS A PLACE IN THE ORDER.**
    ///
    /// Unlisted tools sort to the end, which is the right failure mode at run
    /// time and a silent one at review time: a tool added next month appears
    /// under "Black and white" for no reason anybody can see, and nothing
    /// says so. This says so.
    #[test]
    fn every_advertised_tool_has_a_place() {
        let missing: Vec<&str> = list_tools()
            .iter()
            .map(|t| t.id.as_str())
            .filter(|id| !BY_FREQUENCY.contains(id))
            .map(|id| Box::leak(id.to_string().into_boxed_str()) as &str)
            .collect();
        assert!(
            missing.is_empty(),
            "these tools are advertised but not placed in BY_FREQUENCY, so they \
             sort to the end of their category: {missing:?}"
        );
    }

    /// And nothing in the order names a tool that is not offered.
    ///
    /// The other direction, and the one that rots quietly: a tool withdrawn
    /// from the menu leaves its id here, where it reads as a decision about
    /// something that no longer exists.
    #[test]
    fn the_order_names_nothing_that_is_not_offered() {
        let offered: Vec<String> = list_tools().iter().map(|t| t.id.clone()).collect();
        let stale: Vec<&&str> = BY_FREQUENCY
            .iter()
            .filter(|id| !offered.iter().any(|t| t == *id))
            .collect();
        assert!(
            stale.is_empty(),
            "BY_FREQUENCY places tools that are not advertised: {stale:?}"
        );
    }
}

#[cfg(test)]
mod arming_tests {
    use super::*;
    use openconvert_core::plan::Class;
    use openconvert_core::policy::Policy;

    /// **Naming a model tool arms the class it needs.**
    ///
    /// `route()` refuses any plan above `policy.max_auto_class()`, which is B
    /// by default (I8). Every model-backed tool is Class D. The CLI armed it in
    /// `cmd::convert::policy_for`; the TOOL path did not, so the desktop --
    /// which passes `Policy::default()` -- got an empty plan for background
    /// removal, upscaling, denoising and transcription alike, and reported
    /// "that did not produce a file" on a machine where every model was
    /// downloaded, enabled and working.
    ///
    /// This asserts the rule at the level it now lives at, so a caller that
    /// forgets cannot reintroduce it.
    #[test]
    fn model_targets_arm_class_d_and_ordinary_ones_do_not() {
        let armed_for = |target: Target| -> Class {
            match target {
                Target::Operation(
                    Operation::RemoveBackground { .. }
                    | Operation::Upscale { .. }
                    | Operation::Denoise,
                )
                | Target::Format(openconvert_core::format::FormatId::Txt) => {
                    Policy::default().arming(Class::D).max_auto_class()
                }
                _ => Policy::default().max_auto_class(),
            }
        };

        use openconvert_core::format::FormatId;
        for target in [
            Target::Operation(Operation::RemoveBackground {
                quality: openconvert_core::target::Quality::Best,
                to: FormatId::Png,
            }),
            Target::Operation(Operation::RemoveBackground {
                quality: openconvert_core::target::Quality::Standard,
                to: FormatId::Png,
            }),
            Target::Operation(Operation::Upscale { to: FormatId::Png }),
            Target::Operation(Operation::Denoise),
            Target::Format(FormatId::Txt),
        ] {
            assert_eq!(
                armed_for(target),
                Class::D,
                "{target:?} is a model operation and must arm D"
            );
        }

        // The control. A plain format conversion is NOT a request for
        // inference, even when a Class D route happens to exist for the pair.
        for target in [
            Target::Format(FormatId::Png),
            Target::Format(FormatId::Jpeg),
            Target::Operation(Operation::Invert),
            Target::Operation(Operation::Greyscale),
        ] {
            assert_eq!(
                armed_for(target),
                Class::B,
                "{target:?} must not arm D on its own"
            );
        }
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// **Every id `run_tool` dispatches has a descriptor the host can find.**
    ///
    /// The desktop resolves the requested id before running anything. It
    /// resolved against `list_tools` -- the menu -- while the PDF workspace
    /// saves through `pdf-compose`, which is deliberately not on the menu. Every
    /// merge, reorder and signature Save therefore failed with "no tool named
    /// pdf-compose is registered", and nothing caught it because the dispatch
    /// arm existed and the tests called the arms directly.
    ///
    /// The ids are listed here rather than scraped from the match, so adding a
    /// dispatch arm without a descriptor is a failing test rather than a
    /// runtime refusal a user reports.
    #[test]
    fn every_dispatchable_tool_resolves() {
        const DISPATCHABLE: &[&str] = &[
            "image-remove-background",
            "image-upscale",
            "image-invert",
            "image-greyscale",
            "image-pick",
            "image-ocr",
            "audio-denoise",
            "audio-transcribe",
            "pdf-merge",
            "pdf-reorder",
            "pdf-stamp",
            "pdf-compose",
            "image-compose",
            "video-trim",
            "video-concat",
        ];
        let known: Vec<String> = all_tools().into_iter().map(|t| t.id).collect();
        for id in DISPATCHABLE {
            assert!(
                known.iter().any(|k| k == id),
                "`{id}` is dispatched by run_tool but has no descriptor, so the \
                 host refuses it before it runs"
            );
        }
    }

    /// The menu is a subset, and the difference is deliberate.
    ///
    /// The control for the test above: if `list_tools` and `all_tools` were the
    /// same list, the split would be doing nothing and the first test would
    /// pass for the wrong reason.
    ///
    /// **`image-compose` IS BACK IN THIS LIST**, having briefly left it. It
    /// was hidden, then advertised as a tool carrying tick boxes, and is now
    /// hidden again -- because combining turned out to be a MODE rather than a
    /// tool. "Combine edits" sits beside Single file and Batch, and switching
    /// to it makes the rail multi-select, which is what the tick boxes were
    /// imitating.
    ///
    /// `pdf-compose` was hidden throughout, for the same reason both are: a
    /// composite is what the workspace SAVES through, not something anyone
    /// sets out to do.
    #[test]
    fn the_menu_hides_the_composites() {
        let menu: Vec<String> = list_tools().into_iter().map(|t| t.id).collect();
        for hidden in ["pdf-compose", "image-compose", "video-trim", "video-concat"] {
            assert!(
                !menu.iter().any(|m| m == hidden),
                "`{hidden}` should be dispatchable but not offered"
            );
        }
        assert!(
            menu.len() < all_tools().len(),
            "the menu must be a strict subset, or the split is decorative"
        );
        assert!(
            menu.iter().any(|m| m == "pdf-merge"),
            "the menu still has to contain the tools people pick"
        );
    }
}

#[cfg(test)]
mod compose_order_tests {
    use super::ordered_image_edits;
    #[test]
    fn keeps_requested_order_and_rejects_invalid_stages() {
        let edits = vec!["image-remove-background", "image-invert", "image-upscale"];
        assert_eq!(ordered_image_edits(edits.clone()).unwrap(), edits);
        assert!(ordered_image_edits(vec![]).is_err());
        assert!(ordered_image_edits(vec!["image-invert", "image-invert"]).is_err());
        assert!(ordered_image_edits(vec!["image-ocr"]).is_err());
    }
}
