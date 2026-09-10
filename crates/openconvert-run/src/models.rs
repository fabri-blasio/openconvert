//! The model registry: what models.toml promises, made load-bearing.
//!
//! Three tables govern third-party components, and this module is the
//! consumer of the model one. Its rules are fail-closed by construction:
//!
//! - **Licence allow-list** — Apache-2.0, MIT, BSD. The same policy
//!   `deny.toml` enforces over the Rust tree; a row outside it is refused at
//!   PARSE time, not at download time.
//! - **Format allow-list** — safetensors, GGUF, ONNX only (SR-8). Pickle is
//!   refused at the file-type layer by [`refuse_pickled`], before anything
//!   opens it, because loading a pickle executes code.
//! - **Hash pinning** — a row whose sha256 is the all-zero sentinel is not
//!   pinned yet and cannot be downloaded, enabled or run. The sentinel is an
//!   honest "nobody has verified this artifact"; converting it into a real
//!   row is a human act with the file in hand.
//!
//! # The store is content-addressed (SR-7)
//!
//! Artifacts live under `<state_dir>/models/<sha256>.bin`. The filename IS
//! the claim, so `downloaded()` is one stat plus a size check, and two rows
//! naming identical weights share one copy for free.

use std::path::PathBuf;

/// One row of `models.toml`, embedded at build time — our data, not user
/// input.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelRow {
    /// Registry id (`whisper-base-int8`).
    pub name: String,
    /// Upstream version the pin refers to.
    pub version: String,
    /// SPDX-ish identifier; must clear [`LICENCE_ALLOWLIST`].
    pub licence: String,
    /// One of [`FORMAT_ALLOWLIST`]; anything else refuses.
    pub format: String,
    /// One line for the settings screen.
    #[serde(default)]
    pub purpose: String,
    /// Packed size in bytes, shown BEFORE the user commits to a download.
    #[serde(default)]
    pub size_bytes: u64,
    /// sha256 of the exact artifact. Zero = not pinned = refused everywhere.
    pub sha256: String,
    /// Where the artifact comes from; also the download URL.
    pub source: String,
    /// Ships in v1? Nothing does. Flipping this is a release decision.
    pub enabled: bool,
}

/// What the settings screen shows per row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    /// Registry id.
    pub id: String,
    /// Display title (the id, today; a friendlier name when one exists).
    pub title: String,
    /// One-line purpose.
    pub purpose: String,
    /// Packed size in bytes.
    pub size_bytes: u64,
    /// Licence identifier.
    pub licence: String,
    /// Whether the artifact sits verified in the local store.
    pub downloaded: bool,
    /// Enabled AND pinned AND present — the state in which a worker may use
    /// it. A row disabled here cannot be run; that refusal names this flag.
    pub enabled: bool,
    /// Why this row cannot be downloaded, or `None` when it can.
    ///
    /// The UI used to find this out by pressing the button and reading the
    /// error. A row whose hash is unpinned or whose licence is off the
    /// allow-list will refuse EVERY press, so the refusal is a property of the
    /// row and belongs beside it — with the button disabled, not armed and
    /// waiting to fail.
    pub blocked: Option<String>,
}

/// Why the registry refused something.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// No such id.
    #[error("no model named {0:?}")]
    UnknownModel(String),
    /// The row exists but its hash was never pinned.
    ///
    /// The payload is the MODEL ID, not a sentence. It used to be handed the
    /// finished sentence out of [`check_row`], which this `#[error]` then
    /// wrapped in a second one -- so the user saw both, joined without a
    /// separator, the second copy having lost its model name. A variant that
    /// formats its own message must be given a value, never a message.
    #[error(
        "{0} has no pinned sha256. It cannot be downloaded until someone verifies the artifact and records its real hash."
    )]
    NotPinned(String),
    /// The licence is not on the allow-list.
    #[error("{model} is licensed {licence}; only {allow} are accepted")]
    LicenceRefused {
        /// The model id.
        model: String,
        /// What it claims.
        licence: String,
        /// What we accept.
        allow: String,
    },
    /// The declared format is not one of SR-8's three.
    #[error("{model} declares format {format:?}; only {FORMAT_ALLOWLIST:?} are accepted")]
    FormatRefused {
        /// The model id.
        model: String,
        /// What it claims.
        format: String,
    },
    /// The downloaded bytes did not match the pin.
    #[error(
        "the downloaded artifact did NOT match its sha256 pin; nothing was kept. \
             Expected {expected}, got {actual}"
    )]
    HashMismatch {
        /// The pin from the row.
        expected: String,
        /// What arrived.
        actual: String,
    },
    /// The stored artifact is present but no longer matches itself.
    #[error("the stored copy of {0} is missing or the wrong size; delete it and download again")]
    StoreCorrupt(String),
    /// Transport failure from the single fetch path.
    #[error(transparent)]
    Net(#[from] openconvert_worker::net::NetError),
    /// Underlying I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

const LICENCE_ALLOWLIST: &[&str] = &["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause"];
/// What an artifact may be.
///
/// The first three are SR-8's model formats, and the rule behind them is that
/// a model is DESERIALISED by a runtime, so a format whose deserialiser can
/// execute code (pickle, and therefore most `.pt`/`.bin` checkpoints) is
/// refused however convenient it would be.
///
/// `txt` and `bin` are on the list for a different reason, and it deserves
/// saying out loud: they are not models. OCR's third artifact is its character
/// dictionary, one character per line, read as UTF-8 into a `Vec<String>` and
/// indexed. Enhancement's second is a block of little-endian `f32` matrices
/// read by offset. Nothing deserialises either, nothing executes either, and a
/// malformed one produces a length or parse error rather than behaviour.
///
/// Allowing them does not widen SR-8; it records that not every pinned
/// artifact is weights.
const FORMAT_ALLOWLIST: &[&str] = &["safetensors", "gguf", "onnx", "txt", "bin"];

/// The all-zero sha256: the "not yet pinned" sentinel, refused on sight.
const UNPINNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn parse_rows() -> Vec<ModelRow> {
    #[allow(unused_mut)]
    let mut rows = toml::from_str::<Manifest>(MODELS_TOML)
        .map(|m| m.model)
        .unwrap_or_else(|e| {
            // OUR table failing to parse is a build bug; say so loudly once
            // rather than silently shipping zero models.
            eprintln!("warning: models.toml failed to parse ({e}); treating as empty");
            Vec::new()
        });

    // The unpinned-refusal path needs a row that is unpinned, and for a long
    // time that meant leaving a real broken row in the shipped table so the
    // test had a specimen -- first `example`, then `kokoro-82m`. Both were
    // shown to users in Settings with a Download button that could never
    // succeed, which is how the "same error as the example model" report came
    // in.
    //
    // So the specimen moved into the test build. The refusal is still covered,
    // and models.toml now holds only rows we would actually ship.
    #[cfg(test)]
    rows.push(ModelRow {
        name: TEST_UNPINNED.to_string(),
        version: "0.0.0".to_string(),
        licence: "Apache-2.0".to_string(),
        format: "onnx".to_string(),
        purpose: "Test fixture: an unpinned row, to prove it is refused".to_string(),
        size_bytes: 1,
        sha256: UNPINNED.to_string(),
        source: "https://example.invalid/never-fetched".to_string(),
        enabled: false,
    });

    rows
}

/// The id of the test-only unpinned row [`parse_rows`] appends.
#[cfg(test)]
const TEST_UNPINNED: &str = "test-unpinned";

#[derive(Debug, serde::Deserialize)]
struct Manifest {
    #[serde(default)]
    model: Vec<ModelRow>,
    #[serde(default)]
    feature: Vec<Feature>,
}

/// The embedded registry, parsed once.
pub fn rows() -> &'static [ModelRow] {
    static ROWS: std::sync::OnceLock<Vec<ModelRow>> = std::sync::OnceLock::new();
    ROWS.get_or_init(parse_rows)
}

/// Where artifacts live.
#[must_use]
pub fn store_dir() -> PathBuf {
    crate::state::paths::state_dir().join("models")
}

fn store_path(sha256: &str) -> PathBuf {
    store_dir().join(format!("{sha256}.bin"))
}

fn licence_allowed(licence: &str) -> bool {
    // Dual licences ("Apache-2.0 OR MIT") are acceptable when EITHER side is.
    licence
        .split(" OR ")
        .any(|l| LICENCE_ALLOWLIST.contains(&l.trim()))
}

/// Validate one row against both allow-lists and the pin rule.
///
/// Exposed for tests and for the settings screen, which can show WHY a row is
/// inert instead of just grey.
#[must_use]
pub fn check_row(row: &ModelRow) -> Vec<String> {
    let mut problems = Vec::new();
    if !licence_allowed(&row.licence) {
        problems.push(format!(
            "{} is licensed {}; only {LICENCE_ALLOWLIST:?} are accepted",
            row.name, row.licence
        ));
    }
    if !FORMAT_ALLOWLIST.contains(&row.format.as_str()) {
        problems.push(format!(
            "{} declares format {:?}; only {FORMAT_ALLOWLIST:?} are accepted",
            row.name, row.format
        ));
    }
    if row.sha256.eq_ignore_ascii_case(UNPINNED) {
        problems.push(format!(
            "{} has no pinned sha256; it stays inert until someone verifies \
             the artifact and records its real hash",
            row.name
        ));
    }
    if row.size_bytes == 0 && !row.purpose.contains("Template") {
        problems.push(format!(
            "{} declares no size; the download prompt needs one",
            row.name
        ));
    }
    problems
}

fn find(id: &str) -> Result<&'static ModelRow, ModelError> {
    rows()
        .iter()
        .find(|r| r.name == id)
        .ok_or_else(|| ModelError::UnknownModel(id.to_string()))
}

fn user_enabled(id: &str) -> Option<bool> {
    let cfg = crate::state::config::UserConfig::load();
    // `None` means "never chosen", which is not the same as "chosen off" --
    // collapsing the two is what would make a shipped-on default impossible to
    // turn off, or an explicit off silently revert.
    let list = cfg.enabled_models.as_ref()?;
    Some(list.iter().any(|x| x == id))
}

/// Whether a row should run, given the user's choice and the row's default.
fn effective_enabled(r: &ModelRow) -> bool {
    user_enabled(&r.name).unwrap_or(r.enabled)
}

/// Every row, as the settings screen renders them.
#[must_use]
pub fn list() -> Vec<ModelInfo> {
    rows()
        .iter()
        .map(|r| ModelInfo {
            title: r.name.clone(),
            id: r.name.clone(),
            purpose: r.purpose.clone(),
            size_bytes: r.size_bytes,
            licence: r.licence.clone(),
            downloaded: downloaded(&r.name).unwrap_or(false),
            // THE USER'S SWITCH AND THE ROW'S `enabled` ARE DIFFERENT THINGS.
            //
            // This read `r.enabled && user_enabled(..) && downloaded(..)`, and
            // since every row in models.toml ships `enabled = false` — that
            // field is a RELEASE decision, "does this row ship on by default"
            // — the conjunction could never be true. `set_enabled` wrote the
            // config, returned `Ok`, printed "u2netp is now enabled", and the
            // model stayed off. A setting with no effect, which is a shape
            // this project has been caught by before.
            //
            // What actually gates running a model is: the artifact is in the
            // store (which implies pinned and hash-verified), and the user
            // asked for it. The row's own flag decides the DEFAULT for a user
            // who has never expressed a preference, which is what a ship flag
            // should do.
            enabled: downloaded(&r.name).unwrap_or(false) && effective_enabled(r),
            // One sentence, or none. `check_row` can report several problems
            // with one row; showing all of them turns a disabled button into a
            // paragraph, so the first is the reason and the rest follow from
            // fixing it.
            blocked: check_row(r).into_iter().next(),
        })
        .collect()
}

/// A CAPABILITY, as a person would choose it — not a model file.
///
/// Nobody wants "paddleocr-det"; they want to read text out of a scan, and that
/// takes three artifacts. The registry pins artifacts because an artifact is
/// what a hash covers; this is the other half, and it is what a chooser shows:
/// one row, one size, one sentence saying what it does.
///
/// Read from `models.toml` rather than written here, because the installer's
/// first page prints the same list and a second copy in a second language is a
/// second copy that will disagree.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Feature {
    /// Stable id, for the wire and for the config.
    pub id: String,
    /// What it is called in a chooser.
    pub title: String,
    /// What it does, in one line, in a user's words.
    pub does: String,
    /// The tool that uses it, when one exists.
    ///
    /// Absent means the artifacts would download and nothing in the app could
    /// reach them. A chooser must not offer those: a user who accepts a 248 MB
    /// download and then finds no button has been told something untrue.
    #[serde(default)]
    pub tool: Option<String>,
    /// Which of a tool's alternatives this is.
    ///
    /// **A TIER IS A BUDGET DECISION, and it is the user's.** Small or good;
    /// 4.5 MB or 900 MB; seconds or minutes. Several features may name the same
    /// `tool` and differ only in this.
    ///
    /// It is NOT a variant. Which BiRefNet suits a photograph of a chair versus
    /// a portrait is a question about the content in front of us, decided per
    /// file and never by a setting -- collapsing the two into one list produces
    /// a chooser nobody can reason about.
    ///
    /// `#[serde(default)]`, so every feature written before tiers existed
    /// parses unchanged as the single tier of its tool.
    #[serde(default)]
    pub tier: Tier,
    /// Whether this tier is one of the boxes on the first-run screen.
    ///
    /// **NOT EVERY TIER IS AN INSTALL-TIME QUESTION.** The screen lists
    /// features, so three tiers of background removal listed as three rows
    /// with the same title -- and the third of them is a 224 MB download
    /// offered as a tick box beside two that are 4 and 11 MB. Nobody can weigh
    /// that at the moment they are trying to start using the program, and the
    /// stated total for "add everything" was 515 MB, most of it one model most
    /// people will never want.
    ///
    /// A tier that is not offered here is not hidden and not unavailable: it
    /// is chosen inside the tool, where its size sits next to the reason for
    /// wanting it, and fetched then.
    ///
    /// Defaults to TRUE, so a feature written without thinking about this is
    /// offered -- the same behaviour every feature had before the field
    /// existed. Turning it off is the deliberate act.
    #[serde(default = "yes")]
    pub offered_at_install: bool,
    /// The artifacts, by registry id.
    pub models: Vec<String>,
}

/// `serde(default)` cannot spell `true`.
const fn yes() -> bool {
    true
}

/// How good a tool's alternatives are, in the only order that matters.
///
/// Ordered so `best_ready` can take a maximum without a table. The names are
/// the user's -- "small" and "better" are claims anyone can check against the
/// size beside them, where "v2" and "large" are not.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    serde::Deserialize,
    serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The one that fits on a laptop and finishes while you watch.
    #[default]
    Small,
    /// Larger, slower, better.
    Better,
    /// Larger and slower again. Reserved; nothing ships here yet.
    Best,
}

impl Tier {
    /// The name used in `models.toml`, the config and the receipt.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Better => "better",
            Self::Best => "best",
        }
    }

    /// Parse a config value. Unknown text is `None`, never a silent default:
    /// a typo in `model_tier.image-remove-background` must not quietly become
    /// "small" and hand someone the fast model for a year.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "small" => Some(Self::Small),
            "better" => Some(Self::Better),
            "best" => Some(Self::Best),
            _ => None,
        }
    }
}

/// Every capability the AI pack can provide, usable or not.
#[must_use]
pub fn feature_rows() -> &'static [Feature] {
    static FEATURES: std::sync::OnceLock<Vec<Feature>> = std::sync::OnceLock::new();
    FEATURES.get_or_init(|| {
        toml::from_str::<Manifest>(MODELS_TOML)
            .map(|m| m.feature)
            .unwrap_or_else(|e| {
                eprintln!("warning: models.toml features failed to parse ({e}); treating as empty");
                Vec::new()
            })
    })
}

/// A feature with the numbers a chooser has to show.
#[derive(Debug, Clone)]
pub struct FeatureState {
    /// The declared description.
    pub feature: Feature,
    /// Total packed size of every artifact it needs.
    pub size_bytes: u64,
    /// Whether every artifact is already in the local store.
    pub downloaded: bool,
    /// Distinct licences across its artifacts, for the chooser's fine print.
    pub licences: Vec<String>,
}

/// Whether every artifact of one tier of one tool is present AND switched on.
///
/// **All-or-nothing, and that is the point.** A tier is a SET, not a file:
/// whisper is four rows and MOSS is one. Three of whisper's four artifacts
/// present is not "nearly ready", it is a tier that cannot run -- and the
/// failure it produces without this check is a model loading and then
/// producing confident nonsense, which is worse than a refusal.
///
/// Enabled as well as downloaded: a user who switched a model off has said
/// something, and reading it back as ready would ignore them.
#[must_use]
pub fn tier_ready(tool: &str, tier: Tier) -> bool {
    feature_rows()
        .iter()
        .filter(|f| f.tool.as_deref() == Some(tool) && f.tier == tier)
        // A tool with no feature at this tier is not ready for it. `all` over
        // an empty iterator is TRUE, which would report every unimplemented
        // tier as available.
        .fold(None::<bool>, |acc, f| {
            let ready = f.models.iter().all(|id| {
                downloaded(id).unwrap_or(false)
                    && rows().iter().any(|r| &r.name == id && effective_enabled(r))
            });
            Some(acc.unwrap_or(false) || ready)
        })
        .unwrap_or(false)
}

/// The best tier of `tool` that could run right now, if any.
///
/// This is what `auto` resolves to, and it reproduces the behaviour that was
/// hard-coded in `tools.rs` before tiers existed: use the better model when it
/// is there, otherwise the small one.
#[must_use]
pub fn best_ready(tool: &str) -> Option<Tier> {
    [Tier::Best, Tier::Better, Tier::Small]
        .into_iter()
        .find(|&tier| tier_ready(tool, tier))
}

/// Which tier of `tool` a run should use, given the user's preference.
///
/// # An explicit choice is never silently downgraded
///
/// `auto` means "the best that is ready" and is the default. An EXPLICIT tier
/// that is not ready returns `None` -- the caller refuses and names what to
/// install, rather than running the small model and writing a receipt nobody
/// reads. Someone who believes they got the 900 MB model and got the 20 MB one
/// has been told something untrue by a program whose whole claim is that it
/// does not do that.
#[must_use]
pub fn tier_for(tool: &str, preference: Option<Tier>) -> Option<Tier> {
    match preference {
        None => best_ready(tool),
        Some(tier) => tier_ready(tool, tier).then_some(tier),
    }
}

/// Every artifact of one tier of one tool, in declaration order.
///
/// Declaration order matters: adapters are handed their weights positionally,
/// and whisper's encoder arriving where its decoder was expected is a load
/// that succeeds and then produces nonsense.
#[must_use]
pub fn tier_models(tool: &str, tier: Tier) -> Vec<String> {
    feature_rows()
        .iter()
        .find(|f| f.tool.as_deref() == Some(tool) && f.tier == tier)
        .map(|f| f.models.clone())
        .unwrap_or_default()
}

/// Every feature, with its size and whether it is already here.
///
/// Sizes are summed from the registry rather than written down twice: a number
/// in a chooser that disagrees with what is actually fetched is the kind of
/// small lie this project spends its gates preventing.
#[must_use]
pub fn features() -> Vec<FeatureState> {
    let rows = rows();
    feature_rows()
        .iter()
        .map(|f| {
            let mut size_bytes = 0;
            let mut licences: Vec<String> = Vec::new();
            let mut all_here = true;
            for id in &f.models {
                if let Some(r) = rows.iter().find(|r| &r.name == id) {
                    size_bytes += r.size_bytes;
                    if !licences.contains(&r.licence) {
                        licences.push(r.licence.clone());
                    }
                }
                if !downloaded(id).unwrap_or(false) {
                    all_here = false;
                }
            }
            FeatureState {
                feature: f.clone(),
                size_bytes,
                downloaded: all_here,
                licences,
            }
        })
        .collect()
}

/// Whether this model's verified artifact is in the local store.
///
/// # Errors
///
/// Underlying I/O other than absence.
pub fn downloaded(id: &str) -> Result<bool, ModelError> {
    let row = find(id)?;
    if row.sha256.eq_ignore_ascii_case(UNPINNED) {
        return Ok(false);
    }
    let path = store_path(&row.sha256);
    match std::fs::metadata(&path) {
        Ok(m) => Ok(row.size_bytes == 0 || m.len() == row.size_bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Download, verify against the pin, and atomically place the artifact.
///
/// The bytes go to `<store>/<sha256>.tmp-<pid>` first (create_new), are
/// hashed IN FULL, and only a matching artifact earns its rename into place.
/// A failed verify leaves nothing behind under the content address.
///
/// Refused outright for unpinned rows — see [`ModelError::NotPinned`] — and
/// for any row whose licence or format fails [`check_row`].
///
/// # Errors
///
/// See [`ModelError`].
pub fn download(id: &str) -> Result<PathBuf, ModelError> {
    download_with_progress(id, &mut |_, _| {})
}

/// [`download`], reporting bytes as they arrive.
///
/// `on_progress` receives `(received, total)`; the total is the server's
/// `Content-Length`, or `0` when it sent none. Verification and placement
/// happen after the last callback, so a caller that wants a "verifying" phase
/// reports it once this returns rather than guessing at a boundary.
///
/// # Errors
///
/// See [`ModelError`].
pub fn download_with_progress(
    id: &str,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, ModelError> {
    let row = find(id)?;
    let problems = check_row(row);
    if problems.iter().any(|p| p.contains("sha256")) {
        return Err(ModelError::NotPinned(row.name.clone()));
    }
    if problems
        .iter()
        .any(|p| p.contains("licensed") || p.contains("format"))
    {
        return Err(ModelError::LicenceRefused {
            model: row.name.clone(),
            licence: row.licence.clone(),
            allow: format!("{LICENCE_ALLOWLIST:?}"),
        });
    }

    let max_bytes = row.size_bytes.max(1).saturating_mul(2).max(1 << 30);
    let body = openconvert_worker::net::fetch_with_progress(&row.source, max_bytes, on_progress)?;

    let actual = hex_sha256(&body);
    if !actual.eq_ignore_ascii_case(&row.sha256) {
        return Err(ModelError::HashMismatch {
            expected: row.sha256.clone(),
            actual,
        });
    }

    std::fs::create_dir_all(store_dir())?;
    let tmp = store_dir().join(format!(
        "{}.tmp-{}",
        &row.sha256[..16.min(row.sha256.len())],
        std::process::id()
    ));
    {
        use std::io::Write;
        let mut f = std::fs::File::create_new(&tmp)?;
        f.write_all(&body)?;
    }
    // The atomic placement of a VERIFIED download: the destination is
    // content-addressed inside this module's own store, and the rename
    // happens only after the full-file sha256 matched the row's pin.
    std::fs::rename(&tmp, store_path(&row.sha256))?; // openconvert-lint: allow -- verified artifact into our own store
    record_install(&row.name, &row.sha256);
    Ok(store_path(&row.sha256))
}

// ---------------------------------------------------------------------------
// Which models have been superseded
// ---------------------------------------------------------------------------
//
// The store is content-addressed: `<store>/<sha256>`. That is the right shape
// for verification and the wrong shape for one question -- "is the copy I have
// of `u2netp` the one this build pins?" -- because a blob under an old hash is
// indistinguishable from any other blob.
//
// So each successful download leaves a marker naming what was installed. It is
// a note about local state, not a second source of truth: the pin still lives
// in models.toml, `downloaded()` still asks the content address, and a missing
// or corrupt marker only costs the user an update prompt they would otherwise
// have got automatically.
//
// DETECTING AN UPDATE COSTS NO NETWORK. A model is out of date when this build
// pins a hash the marker does not name -- which is a comparison between two
// local files. The network is only touched to FETCH the new artifact, which is
// the same one call site as any other download, and only when the user has
// left auto-update on or pressed the button.

/// Where the marker for `id` lives.
fn marker_path(id: &str) -> PathBuf {
    // Ids come from our own table, but this path is joined against a real
    // directory, so a hypothetical `../` in one must not escape the store.
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    store_dir().join(format!("{safe}.installed"))
}

/// Note that `id` was installed at `sha256`.
///
/// Written to a fresh temporary file and renamed into place, exactly as the
/// artifact itself is — `fs::write` truncates, and the destructive-API gate is
/// right to refuse it. A marker is small enough that a torn write seemed
/// unlikely, which is the reasoning that produces half-written files. This way
/// a crash mid-write leaves the previous marker intact.
///
/// Best-effort throughout: a marker that cannot be written costs the user an
/// update prompt, never the download itself.
fn record_install(id: &str, sha256: &str) {
    let final_path = marker_path(id);
    let tmp = final_path.with_extension(format!("installed.tmp-{}", std::process::id()));
    let wrote = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create_new(&tmp)?;
        f.write_all(sha256.as_bytes())?;
        f.flush()
    })();
    if wrote.is_ok() {
        // Our own bookkeeping file, inside our own store, named by an id from
        // our own table.
        let _ = std::fs::rename(&tmp, &final_path); // openconvert-lint: allow -- marker into our own store
    }
    // Cleans up only the temporary this function just created, named after
    // this process. The rename above consumed it on the success path; this is
    // the failure path, where leaving it behind would accumulate one file per
    // interrupted download.
    let _ = std::fs::remove_file(&tmp); // openconvert-lint: allow -- our own temporary, in our own store
}

/// The recorded hash in a marker's contents, or `None` if it is not one.
///
/// **A marker is 64 hex characters or it is not a marker.** The file sits in a
/// directory the user can reach, so half a hash, a log line or an empty file
/// has to read as "nothing recorded" — which costs one update prompt — rather
/// than as a hash that will never match and therefore reports the model as
/// permanently out of date.
///
/// Split out from the file read so it can be tested as what it is: a parser.
fn parse_marker(raw: &str) -> Option<String> {
    let h = raw.trim().to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// The hash of the artifact last installed for `id`, if it was recorded.
fn installed_hash(id: &str) -> Option<String> {
    parse_marker(&std::fs::read_to_string(marker_path(id)).ok()?)
}

/// Record a marker for every artifact that is present without one.
///
/// **A model with no marker is invisible to `outdated()`**, which reads the
/// marker to learn which artifact the user actually installed. Everything
/// downloaded before markers existed — and anything placed in the store by
/// hand — was therefore excluded from update checks permanently: present,
/// working, and never re-fetched when a later build moved the pin. On the
/// machine this was found on, twelve artifacts were in the store and four had
/// markers.
///
/// Nothing is invented. A marker is only written when the artifact for THIS
/// build's pin is the one sitting in the store, so the hash recorded is a hash
/// that was verified — `downloaded()` compares the content address — rather
/// than one assumed.
///
/// Best-effort and idempotent: called at startup, cheap when there is nothing
/// to do, and a failure costs an update prompt rather than a download.
pub fn backfill_markers() -> usize {
    let mut wrote = 0;
    for row in rows() {
        if row.sha256.eq_ignore_ascii_case(UNPINNED) {
            continue;
        }
        if installed_hash(&row.name).is_some() {
            continue;
        }
        if downloaded(&row.name).unwrap_or(false) {
            record_install(&row.name, &row.sha256);
            wrote += 1;
        }
    }
    wrote
}

/// Model ids the user has downloaded that this build now pins differently.
///
/// Empty in the ordinary case. Non-empty after an app update whose models.toml
/// moved a row to a newer artifact, for the rows that user actually has —
/// never for rows they never downloaded, because an "update" to something you
/// do not have is just a download.
#[must_use]
pub fn outdated() -> Vec<String> {
    rows()
        .iter()
        .filter(|r| !r.sha256.eq_ignore_ascii_case(UNPINNED))
        .filter(|r| {
            let Some(had) = installed_hash(&r.name) else {
                return false;
            };
            // Superseded, and the replacement is not already here.
            !had.eq_ignore_ascii_case(&r.sha256) && !downloaded(&r.name).unwrap_or(false)
        })
        .map(|r| r.name.clone())
        .collect()
}

/// Fetch the current artifact for every superseded model.
///
/// Returns the ids brought up to date, then the per-row failures. One failure
/// does not abandon the rest: these are independent artifacts, and a network
/// that drops during the second of four should still leave the first updated.
///
/// The desktop shell drives the same walk itself so it can report progress per
/// row; this is the plain version, for a caller that only wants it done.
#[must_use]
pub fn update_outdated() -> (Vec<String>, Vec<String>) {
    update_outdated_with_progress(&mut |_, _, _| {})
}

/// [`update_outdated`], reporting `(id, received, total)` as bytes arrive.
#[must_use]
pub fn update_outdated_with_progress(
    on_progress: &mut dyn FnMut(&str, u64, u64),
) -> (Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for id in outdated() {
        let result = download_with_progress(&id, &mut |received, total| {
            on_progress(&id, received, total);
        });
        match result {
            Ok(_) => done.push(id),
            Err(e) => failed.push(format!("{id}: {e}")),
        }
    }
    (done, failed)
}

/// Read a downloaded model's bytes, re-checking them against its pin.
///
/// # Why the hash is checked again
///
/// [`download`] already verified it before the artifact was allowed into the
/// store. This checks a second time, at the moment the bytes are about to be
/// handed to a runtime that will execute them — because everything that could
/// go wrong between those two moments is exactly what a pin is for: a store
/// corrupted on disk, an artifact swapped by something else on the machine, a
/// partially written file left by a killed process.
///
/// The cost is one hash of a few tens of megabytes against a model load that
/// is far slower, and it turns "we verified this once" into "this IS what we
/// verified". The content-addressed store makes the check cheap to state: the
/// filename is the expected hash.
///
/// # Errors
///
/// An unknown or unpinned id, a model never downloaded, or bytes that no
/// longer match the pin — the last of which deletes nothing and says so,
/// because silently re-downloading over a mismatch hides a real problem.
pub fn verified_bytes(id: &str) -> Result<Vec<u8>, ModelError> {
    let row = find(id)?;
    if row.sha256.eq_ignore_ascii_case(UNPINNED) {
        return Err(ModelError::NotPinned(row.name.clone()));
    }
    let path = store_path(&row.sha256);
    let bytes = std::fs::read(&path).map_err(|_| ModelError::StoreCorrupt(row.name.clone()))?;
    let actual = hex_sha256(&bytes);
    if !actual.eq_ignore_ascii_case(&row.sha256) {
        return Err(ModelError::HashMismatch {
            expected: row.sha256.clone(),
            actual,
        });
    }
    Ok(bytes)
}

/// Enable or disable a model for THIS machine.
///
/// Rows ship disabled; flipping this is the user's explicit action in
/// Settings. The override persists in the user config alongside every other
/// preference, and a model that is not BOTH pinned-and-present AND enabled
/// reports `enabled: false` and refuses at the worker with that reason.
///
/// Unpinned rows refuse regardless: enabling something that cannot be
/// verified would trade the whole pin story for a checkbox.
///
/// # Errors
///
/// Config persistence failures.
pub fn set_enabled(id: &str, enable: bool) -> Result<(), ModelError> {
    let row = find(id)?;
    if enable && row.sha256.eq_ignore_ascii_case(UNPINNED) {
        return Err(ModelError::NotPinned(row.name.clone()));
    }
    let mut cfg = crate::state::config::UserConfig::load();
    let mut list = cfg.enabled_models.take().unwrap_or_default();
    if enable {
        if !list.contains(&id.to_string()) {
            list.push(id.to_string());
        }
    } else {
        list.retain(|x| x != id);
    }
    cfg.enabled_models = Some(list);
    cfg.save()?;
    Ok(())
}

/// Delete the stored artifact, freeing its disk space. Returns whether a
/// file went.
///
/// # Errors
///
/// I/O errors other than absence.
pub fn delete(id: &str) -> Result<bool, ModelError> {
    let row = find(id)?;
    if row.sha256.eq_ignore_ascii_case(UNPINNED) {
        return Ok(false);
    }
    let path = store_path(&row.sha256);
    // Removes ONLY the content-addressed blob this module placed, named by
    // the row's own pin; the user asked for exactly this reclamation.
    let removed = std::fs::remove_file(&path); // openconvert-lint: allow -- scoped store deletion (see above)
                                               // And the marker, or a model the user deliberately removed comes back as
                                               // "an update is available" — which is the opposite of what they asked for.
    let _ = std::fs::remove_file(marker_path(id)); // openconvert-lint: allow -- scoped store deletion (see above)
    match removed {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// THE pickle gate (SR-8): refuse at the file-type layer, before anything
/// opens the artifact.
///
/// Checks extension AND magic: a `.png`-named pickle with protocol-2 opcode
/// `0x80 0x02` is still a pickle. Called by any code path about to persist or
/// open a model artifact.
///
/// # Errors
///
/// A message naming what was seen, so the refusal explains itself.
pub fn refuse_pickled(file_name: &str, head: &[u8]) -> Result<(), ModelError> {
    let lower = file_name.to_ascii_lowercase();
    const BAD_EXTS: &[&str] = &[".pkl", ".pickle", ".ckpt", ".pt", ".pth", ".bin.pytorch"];
    if BAD_EXTS.iter().any(|e| lower.ends_with(e)) {
        return Err(ModelError::FormatRefused {
            model: file_name.to_string(),
            format: "pickle-family".to_string(),
        });
    }
    // Protocol 2-5 opcodes start 0x80 <proto>. Two-byte prefix is enough to
    // refuse without parsing anything.
    if head.len() >= 2 && head[0] == 0x80 && head[1] <= 5 {
        return Err(ModelError::FormatRefused {
            model: file_name.to_string(),
            format: "pickle".to_string(),
        });
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

// The table is embedded, not read from disk: it is OUR data, same argument
// as the format table in openconvert-core.
const MODELS_TOML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../models.toml"));

#[cfg(test)]
mod feature_coverage {
    use super::{feature_rows, rows, tier_for, tier_models, tier_ready, Tier};

    /// The test-only unpinned specimen: a fixture, not a shipped model.
    use super::TEST_UNPINNED;

    /// Every model a feature names must exist in the registry.
    ///
    /// A typo here is a feature that reports a smaller size than it downloads,
    /// which is exactly the small lie a chooser must not tell.
    #[test]
    fn every_named_model_exists() {
        let known: Vec<&str> = rows().iter().map(|r| r.name.as_str()).collect();
        for f in feature_rows() {
            for m in &f.models {
                assert!(
                    known.contains(&m.as_str()),
                    "feature {:?} names {m:?}, which is not in models.toml",
                    f.id
                );
            }
        }
    }

    /// **Every real model belongs to exactly one feature PER TOOL AND TIER.**
    ///
    /// This is the reachability gate. Three PaddleOCR rows sat in the registry,
    /// downloadable, with nothing in the product able to use them and nothing
    /// anywhere saying so. A model that no feature claims is either dead weight
    /// or a feature somebody forgot to add, and both are worth failing a build
    /// over -- a registry is a promise about what the program can do.
    ///
    /// # Why it is no longer "exactly one feature"
    ///
    /// It was, until a tool could have tiers. `u2netp` is now claimed twice on
    /// purpose: it IS the small tier of background removal, and it is also the
    /// better tier's fallback, because MODNet is a portrait matting network
    /// that returns almost nothing on a subject it was not trained for.
    ///
    /// What must stay unique is the pair. Two features naming the same tool AND
    /// the same tier is an ambiguous chooser -- two rows offering the same
    /// choice, with nothing to tell them apart.
    #[test]
    fn every_model_belongs_to_exactly_one_feature_per_tool_and_tier() {
        for row in rows() {
            if row.name == TEST_UNPINNED {
                continue;
            }
            let owners: Vec<&str> = feature_rows()
                .iter()
                .filter(|f| f.models.contains(&row.name))
                .map(|f| f.id.as_str())
                .collect();
            assert!(
                !owners.is_empty(),
                "{:?} is claimed by no feature; a chooser cannot offer it and a \
                 user cannot find it",
                row.name
            );
        }

        // The pair is the unique thing.
        let mut seen: Vec<(&str, Tier)> = Vec::new();
        for f in feature_rows() {
            let Some(tool) = f.tool.as_deref() else {
                continue;
            };
            let key = (tool, f.tier);
            assert!(
                !seen.contains(&key),
                "two features offer {tool} at the {:?} tier; a chooser showing \
                 both has no way to say which is which",
                f.tier
            );
            seen.push(key);
        }
    }

    /// **A tier is all-or-nothing, and an empty tier is never ready.**
    ///
    /// `all()` over an empty iterator is TRUE, so a naive `tier_ready` reports
    /// every tier no feature declares as available -- and the caller then asks
    /// for a model set that does not exist. The `fold` in `tier_ready` is there
    /// for this, and this is what holds it there.
    #[test]
    fn a_tier_nothing_declares_is_not_ready() {
        // NOT `image-remove-background`, WHICH IS WHY THIS ONCE BROKE.
        //
        // The example used to be that tool's `best` tier, on the premise that
        // nothing declared one. `models.toml` then gained
        // `remove-background-best` (BiRefNet), and the premise expired --
        // silently, because the assertion still passed on any machine where
        // the 224 MB model was not installed. It failed the moment one was.
        //
        // A test whose result depends on what this machine has downloaded is
        // not testing the code. `image-upscale` declares no tiers AT ALL, so
        // every tier of it is the empty case for good, and no download can
        // change that.
        assert!(
            !tier_ready("image-upscale", Tier::Best),
            "upscale declares no tiers, so none of them can be ready"
        );
        assert!(
            !tier_ready("no-such-tool", Tier::Small),
            "a tool with no features has no ready tier"
        );
        assert_eq!(
            tier_for("no-such-tool", None),
            None,
            "auto over a tool with no tiers resolves to nothing, not to a default"
        );
        assert_eq!(
            tier_for("image-upscale", Some(Tier::Best)),
            None,
            "an explicit tier that is not ready refuses; it does not downgrade"
        );
    }

    /// The tiers a tool declares carry the models the adapter expects.
    ///
    /// **THE LADDER MOVED UP A RUNG AND THIS MOVED WITH IT.** It was
    /// [u2netp] / [modnet, u2netp] / [birefnet-lite, u2netp]. The bottom rung
    /// offered one artifact that the rung above already contained, under the
    /// same title, six megabytes apart -- so it was dropped and everything
    /// shifted down, with the full BiRefNet taking the top.
    ///
    /// `u2netp` is in every rung and that is the invariant worth asserting:
    /// it is the fallback the adapter reaches for when the primary model
    /// returns nothing, so a tier that omitted it would be a tier that fails
    /// silently on any subject its main network was not trained for.
    #[test]
    fn each_tier_names_its_own_models() {
        assert_eq!(
            tier_models("image-remove-background", Tier::Small),
            vec!["modnet".to_string(), "u2netp".to_string()]
        );
        assert_eq!(
            tier_models("image-remove-background", Tier::Better),
            vec!["birefnet-lite".to_string(), "u2netp".to_string()]
        );
        assert_eq!(
            tier_models("image-remove-background", Tier::Best),
            vec!["birefnet-general".to_string(), "u2netp".to_string()]
        );
        for tier in [Tier::Small, Tier::Better, Tier::Best] {
            assert!(
                tier_models("image-remove-background", tier)
                    .iter()
                    .any(|m| m == "u2netp"),
                "every matting tier carries the fallback it needs"
            );
        }
    }

    /// Transcription is a ladder now, and both rungs share what they can.
    ///
    /// The tokeniser and the voice-activity model are the same files for both,
    /// so a machine with the base tier fetches only the two large networks.
    /// Naming them in both lists is what makes that true -- `featureSize` sums
    /// a feature's own artifacts, and a rung that omitted them would describe
    /// a download that does not work on its own.
    #[test]
    fn transcription_has_a_base_and_a_large_rung() {
        let base = tier_models("audio-transcribe", Tier::Small);
        let large = tier_models("audio-transcribe", Tier::Best);
        assert!(base.iter().any(|m| m == "whisper-encoder"));
        assert!(large.iter().any(|m| m == "whisper-large-turbo-encoder"));
        for shared in ["whisper-tokenizer", "silero-vad"] {
            assert!(base.iter().any(|m| m == shared), "base is missing {shared}");
            assert!(
                large.iter().any(|m| m == shared),
                "large is missing {shared}"
            );
        }
    }

    /// **Every capability in the registry is reachable, and stays that way.**
    ///
    /// `tool = None` is permitted by the type and by `every_named_tool_exists`
    /// below, on the reasoning that declaring a capability before its interface
    /// exists is more honest than hiding it. That reasoning is sound and it
    /// still produced a real defect: OCR sat declared, pinned and downloadable
    /// for weeks with no button anywhere in the product, so a user could fetch
    /// 15 MB of models and find nothing that used them. It was documented, and
    /// it was still dead weight someone could pay for.
    ///
    /// So the count is pinned at zero rather than the shape being forbidden.
    /// Staging a capability ahead of its tool remains possible; it is now a
    /// visible diff to this number, which is a conversation rather than an
    /// oversight.
    #[test]
    fn every_capability_is_reachable() {
        let orphans: Vec<&str> = feature_rows()
            .iter()
            .filter(|f| f.tool.is_none())
            .map(|f| f.id.as_str())
            .collect();
        assert!(
            orphans.is_empty(),
            "{orphans:?} can be downloaded and nothing in the product can reach them. \
             Either wire a tool, or remove the rows -- and if this is a deliberate \
             staging step, say so here and change the assertion."
        );
    }

    /// A feature whose tool does not exist would offer a download for nothing.
    ///
    /// `None` is allowed and is the honest answer for a capability with no
    /// interface yet; a WRONG tool id is not, because the chooser would then
    /// present it as usable.
    #[test]
    fn every_named_tool_exists() {
        let ids: Vec<String> = crate::tools::list_tools()
            .into_iter()
            .map(|t| t.id)
            .collect();
        for f in feature_rows() {
            if let Some(tool) = &f.tool {
                assert!(
                    ids.iter().any(|i| i == tool),
                    "feature {:?} names tool {tool:?}, which is not in the tools registry",
                    f.id
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_clears_licences_and_formats() {
        for row in rows() {
            assert!(
                licence_allowed(&row.licence),
                "{} claims {}",
                row.name,
                row.licence
            );
            assert!(
                FORMAT_ALLOWLIST.contains(&row.format.as_str()),
                "{} claims format {}",
                row.name,
                row.format
            );
        }
    }

    #[test]
    fn tier_s_wave_rows_exist_and_are_disabled() {
        // Names changed when the adapters landed: OCR and transcription each
        // turned out to be three artifacts, not one, so `paddleocr-mobile` and
        // `whisper-base-int8` became six rows named for what they actually
        // are. The test tracks the artifacts because that is what the store
        // pins and what Settings lists.
        //
        // The three `paddleocr-*` rows were here and were removed on
        // 2026-09-04 with the OCR feature. A row exists in order to be
        // downloaded, and the tool that used them is no longer offered, so
        // keeping them would have meant advertising a download for something
        // nothing reaches -- which is what `every_capability_is_reachable`
        // forbids. The adapter and routes are untouched; see models.toml.
        for id in [
            "whisper-encoder",
            "whisper-decoder",
            "whisper-tokenizer",
            "u2netp",
            "deepfilternet",
            "realesrgan-x4",
            "silero-vad",
            "modnet",
        ] {
            let row = rows()
                .iter()
                .find(|r| r.name == id)
                .unwrap_or_else(|| panic!("{id} missing from models.toml"));
            assert!(!row.enabled, "{id} must ship DISABLED");
            assert!(row.size_bytes > 0, "{id} needs a size for the UI");
        }
    }

    #[test]
    fn unpinned_rows_report_the_problem_by_name() {
        // The specimen is a TEST FIXTURE, not a shipped row -- see
        // `parse_rows`. Using a real broken row for this meant users were
        // shown it.
        let row = rows().iter().find(|r| r.name == TEST_UNPINNED).unwrap();
        let problems = check_row(row);
        assert!(
            problems.iter().any(|p| p.contains("sha256")),
            "the sentinel must be named: {problems:?}"
        );
        assert!(
            matches!(download(TEST_UNPINNED), Err(ModelError::NotPinned(_))),
            "an unpinned row must not be downloadable"
        );
        assert!(!downloaded(TEST_UNPINNED).unwrap());
    }

    /// A marker path is always a file directly inside the store.
    ///
    /// Ids come from our own table today, so this is a guard rather than a
    /// fix — but `marker_path` joins a string against a real directory, and
    /// the day an id arrives from anywhere else, `../../config.toml` must
    /// already be impossible rather than newly considered.
    #[test]
    fn a_marker_path_cannot_escape_the_store() {
        for hostile in ["../../etc/passwd", "..", "a/b", r"C:\windows\system32"] {
            let p = super::marker_path(hostile);
            assert_eq!(
                p.parent(),
                Some(super::store_dir().as_path()),
                "{hostile:?} escaped the store: {p:?}"
            );
        }
    }

    /// A damaged marker reads as "nothing recorded", never as a hash.
    ///
    /// The file is in a directory the user can reach. Anything short of a full
    /// hash must fall back to the update prompt rather than being treated as a
    /// hash that will never match — which would report every model as
    /// permanently out of date, forever.
    ///
    /// This used to write files into a temp directory to prove the same thing.
    /// It never needed to: the rule is about the CONTENTS, and testing a
    /// parser through the filesystem tests the filesystem.
    #[test]
    fn a_damaged_marker_reads_as_nothing_recorded() {
        for bad in [
            "",
            "  ",
            "deadbeef",
            &"z".repeat(64),
            &"a".repeat(63),
            "a".repeat(65).as_str(),
        ] {
            assert!(
                super::parse_marker(bad).is_none(),
                "{bad:?} must not read as a recorded hash"
            );
        }
        let upper = "A".repeat(64);
        assert_eq!(
            super::parse_marker(&upper).as_deref(),
            Some("a".repeat(64).as_str()),
            "an uppercase hash is still a hash, and normalises"
        );
        assert_eq!(
            super::parse_marker(&format!("  {}\n", "b".repeat(64))).as_deref(),
            Some("b".repeat(64).as_str()),
            "a trailing newline is not damage"
        );
    }

    #[test]
    fn set_enabled_refuses_unpinned_rows() {
        assert!(matches!(
            set_enabled(TEST_UNPINNED, true),
            Err(ModelError::NotPinned(_))
        ));
    }

    #[test]
    fn unknown_ids_are_errors_not_panics() {
        assert!(matches!(
            find("no-such-model"),
            Err(ModelError::UnknownModel(_))
        ));
    }

    #[test]
    fn pickle_gate_catches_extensions_and_magic() {
        for name in ["weights.pkl", "ckpt.pickle", "model.pt", "run.pth"] {
            assert!(refuse_pickled(name, b"").is_err(), "{name} passed");
        }
        assert!(refuse_pickled("innocent.onnx", &[0x80, 0x02]).is_err());
        assert!(refuse_pickled("model.onnx", &[0x08, 0x00]).is_ok());
    }

    #[test]
    fn dual_licences_accept_either_side() {
        assert!(licence_allowed("Apache-2.0 OR MIT"));
        assert!(licence_allowed("MIT"));
        // A dual licence genuinely lets us take the permissive side; that is
        // how SPDX "OR" works and how deny.toml treats the Rust tree.
        assert!(licence_allowed("GPL-3.0 OR MIT"));
        assert!(!licence_allowed("AGPL-3.0"), "no alternative side to take");
        assert!(!licence_allowed("GPL-3.0"));
    }
}
