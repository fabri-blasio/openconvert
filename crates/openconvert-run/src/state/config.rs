//! The user configuration layer.
//!
//! Three layers, lowest wins first: built-in defaults < `config.toml` < CLI
//! flags. The config can only *express preferences* — a default format, an
//! output directory — and **cannot lower the isolation floor**, because the
//! floor has no config key at all (`09` §8: "network: Denied — NOT
//! LOWERABLE. There is no config key."). A hostile or corrupt file can make
//! conversions refuse to run; it cannot make them run less confined.
//!
//! Like everything in the state directory, the file is untrusted on read:
//! corrupt means defaults plus a warning, never a crash and never silence.

use serde::{Deserialize, Serialize};

/// What the user may set, all optional.
///
/// The GUI-facing keys (`theme` onward) are written by the desktop shell's
/// settings screen (INTERFACES.md §11). Every one of them is a *preference*:
/// none can widen what a conversion is permitted to do, and the isolation
/// floor still has no key at all. Absent means default, so a config file
/// written by an older build loads unchanged.
/// # One network key, and it is read
///
/// `network_posture`, `update_checks` and `diagnostics` were here and are gone.
/// None was ever read by anything: there is no telemetry and no diagnostics in
/// this build. A key that configures a request that never happens is worse
/// than no key -- it tells the reader the app phones home and offers them a
/// switch over it.
///
/// `model_auto_update` is the one that stayed, because there is something for
/// it to gate: `models::update_outdated`, which re-fetches artifacts this
/// build pins differently from the copy on disk. Detecting that costs no
/// network at all -- it compares two local files -- so switching this off
/// leaves a program that reaches the network only when a person presses a
/// button.
///
/// # Five more keys removed on 2026-08-27, for the same reason
///
/// `output_dir`, `quality`, `max_parallel`, `confirm_overwrite` and
/// `metadata_policy` were all stored, all serialised, and all read by nothing.
/// Four were invisible; `metadata_policy` was worse than invisible, because it
/// was a **switch on the settings screen labelled "Strip GPS, serial and
/// author"** that no conversion consulted. A privacy control that does nothing
/// is the worst kind of dead setting: the user believes they turned something
/// off.
///
/// Removing it required making the replacement claim TRUE first. "Metadata
/// does not survive a conversion" held for images and audio, which re-encode,
/// and did not hold for the PDF tools, which move objects between documents
/// without rasterising: a test found `reorder` keeping the author's name as
/// live `/Info` metadata and `merge` leaving it in the output bytes. See
/// `pages::strip_document_metadata`.
///
/// `output_dir` was superseded by `output_destination`. `confirm_overwrite`
/// was superseded by the acknowledgement gate the settings screen puts in
/// front of "Replace originals". `quality` and `max_parallel` never had an
/// implementation at all.
///
/// There is no `deny_unknown_fields` on this struct, so a config written by an
/// older build still parses; the removed keys are simply ignored.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct UserConfig {
    /// Default target format when no `-t` is given (name or extension).
    pub default_format: Option<String>,

    /// `"balanced" | "isolated"` — how far worker reuse goes.
    ///
    /// `Policy::raise_worker_reuse` is a **ratchet**: a later layer may raise
    /// this to `isolated` and may never lower it, so a managed deployment can
    /// pin the strict value and this key cannot undo it. Reading it here and
    /// raising with it preserves that; there is deliberately no path that
    /// lowers.
    pub worker_reuse: Option<String>,

    /// Whether conversions may use a GPU. Absent means yes.
    ///
    /// ON by default: the GPU is used only where it measurably pays — a 7x
    /// speedup on upscaling — and the adapter falls back to the CPU on its own
    /// where it does not, or where the card has too little memory. Defaulting
    /// to off would leave that switched off for everyone who never opens
    /// Settings.
    ///
    /// Reaching the worker is `Policy::without_gpu` -> `Limits::use_gpu` ->
    /// `RunLimits::use_gpu`, which is a ratchet the whole way: this key can
    /// turn the GPU off and nothing downstream can turn it back on.
    pub use_gpu: Option<bool>,

    /// Memory one confined conversion may use, in MB. Absent means 1024.
    ///
    /// Unlike [`Self::worker_reuse`] and [`Self::force_sandbox`], which are
    /// ratchets a managed deployment can pin, this one lowers as well as
    /// raises: it exists so someone can run a model that does not fit in the
    /// default -- BiRefNet-lite peaks at 6,465 MB -- and a value that could
    /// only ever go up would be a trap. `Policy::with_worker_memory` bounds it
    /// at both ends, so a hand-edited file cannot set it below the default or
    /// above the hard maximum.
    pub worker_memory_mb: Option<u64>,

    /// Whether the AI chooser has been answered.
    ///
    /// Asked once, on first run, and never again -- including when the answer
    /// was "none of them". A prompt that returns because the user declined is a
    /// prompt that teaches people to dismiss it without reading.
    pub ai_setup_done: Option<bool>,

    /// Confine every step that has a worker, including the pure-Rust ones.
    ///
    /// `Policy::force_sandbox_everywhere` is a ratchet with no way back, so
    /// this key can only turn confinement ON. It cannot reach the routes with
    /// no worker — CSV/JSON and Matroska stream copy — and the interface says
    /// so rather than implying a completeness the mechanism does not have.
    pub force_sandbox: Option<bool>,

    /// Re-fetch model artifacts this build pins differently, without asking.
    ///
    /// Defaults to ON when absent. Off does not disable downloading — pressing
    /// Download still works — it disables the app deciding to fetch on its own.
    pub model_auto_update: Option<bool>,

    /// `"system" | "light" | "dark"`.
    pub theme: Option<String>,
    /// Whether desktop conversions are recorded in the app-data receipt
    /// database and shown on the done screen. CLI receipt flags remain an
    /// independent beside-output sidecar contract.
    pub write_receipts: Option<bool>,
    /// `"same_folder" | "downloads" | "desktop" | "replace_source"`.
    pub output_destination: Option<String>,
    /// Output name template over `{name}` `{ext}` `{date}` `{index}`.
    pub naming_template: Option<String>,
    /// Last window geometry, persisted on close by the desktop shell.
    pub window_x: Option<i32>,
    /// Last window geometry, persisted on close by the desktop shell.
    pub window_y: Option<i32>,
    /// Last window geometry, persisted on close by the desktop shell.
    pub window_w: Option<u32>,
    /// Last window geometry, persisted on close by the desktop shell.
    pub window_h: Option<u32>,
    /// Whether the window was MAXIMIZED when it was last closed.
    ///
    /// Separate from the geometry, and not derivable from it. A maximized
    /// Windows window reports a negative position and a size larger than the
    /// work area -- its borders sit off-screen on purpose. Restoring those
    /// numbers as an ordinary window produces something that covers the whole
    /// display and whose resize edges cannot be reached, which is neither
    /// maximized nor resizable. Persisting the STATE is what lets the window
    /// be re-maximized properly, and the geometry beneath it stays the
    /// restore-down size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_maximized: Option<bool>,

    /// Which tier of each tool's models to run, keyed by tool id.
    ///
    /// `"auto"` -- and an absent entry -- mean "the best tier that is ready",
    /// which is what the app did before the user could choose. An explicit
    /// tier is honoured or REFUSED, never quietly downgraded: someone who
    /// believes they got the 900 MB model and got the 20 MB one has been told
    /// something untrue by a program whose whole claim is that it does not do
    /// that.
    ///
    /// Keyed by tool rather than by capability because the tool is what the
    /// tiers are alternatives FOR, and it is the id both the registry and the
    /// run path already carry.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub model_tier: std::collections::BTreeMap<String, String>,

    /// Model ids the user enabled in Settings (A10's model registry reads
    /// this). A row ships disabled; nothing here can enable an UNPINNED row
    /// because the registry refuses that flip before persisting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_models: Option<Vec<String>>,
}

impl UserConfig {
    /// Write the configuration back as TOML, preserving every field that is
    /// set. `None` fields are omitted rather than written as empty, so the
    /// file stays hand-editable and older builds read it unchanged.
    ///
    /// # Errors
    ///
    /// Serialisation failure or any I/O failure writing the file.
    pub fn save(&self) -> std::io::Result<()> {
        let body = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let dir = super::paths::config_path();
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // The one deliberate overwrite in the state layer: the user asked for
        // this exact file to change, at this audited path, by saving settings.
        // Config.toml is shell-owned preference state under state_dir,
        // written by the settings screen; I12/SR-15 govern CONVERSION
        // OUTPUTS, which never take this path.
        std::fs::write(&dir, body) // openconvert-lint: allow -- state-dir preference file, not a conversion output (see above) // openconvert-lint: allow -- config.toml is the file being edited; path fixed above
    }

    /// Load `config.toml` from the state directory.
    ///
    /// Missing file = defaults. Corrupt file = defaults + stderr warning:
    /// a broken preference must not look like a working one, but must also
    /// never stop the machine converting.
    #[must_use]
    pub fn load() -> Self {
        Self::from_path(&super::paths::config_path())
    }

    /// Load from an explicit path. Same rules as [`Self::load`]; separate so
    /// tests can point at a temp directory instead of the user's real state.
    fn from_path(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("warning: corrupt config.toml ({e}); using defaults");
                    Self::default()
                }
            },
            // Missing is the ordinary first-run case; stay quiet about it.
            Err(_) => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path under a fresh temp directory, deleted first for repeatability.
    fn config_file(name: &str) -> std::path::PathBuf {
        let tmp =
            std::env::temp_dir().join(format!("tx-config-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp); // openconvert-lint: allow -- test scratch under temp_dir()
        std::fs::create_dir_all(&tmp).unwrap();
        tmp.join("config.toml")
    }

    /// No file is the ordinary first run: silent defaults.
    #[test]
    fn missing_file_is_defaults() {
        let c = UserConfig::from_path(&config_file("missing"));
        assert_eq!(c.default_format, None);
        assert_eq!(c.theme, None);
    }

    /// A well-formed file parses into every field.
    #[test]
    fn valid_config_parses() {
        let path = config_file("valid");
        // Includes two keys this build removed. An older config that still
        // carries `quality` and `max_parallel` must LOAD, not fail -- there is
        // no `deny_unknown_fields`, and someone's file on disk still has them.
        let contents = "default_format = \"jpeg\"
theme = \"dark\"
quality = 85
max_parallel = 4
";
        std::fs::write(&path, contents).unwrap(); // openconvert-lint: allow -- test scratch
        let c = UserConfig::from_path(&path);
        assert_eq!(c.default_format.as_deref(), Some("jpeg"));
        assert_eq!(c.theme.as_deref(), Some("dark"));
    }

    /// **Corrupt state degrades, it does not kill** (SR-19). Garbage falls
    /// back to defaults rather than failing the command that read it.
    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = config_file("corrupt");
        std::fs::write(&path, "this is ][ not toml\n===\n").unwrap(); // openconvert-lint: allow -- test scratch
        let c = UserConfig::from_path(&path);
        assert_eq!(c.default_format, None);
        assert_eq!(c.theme, None);
    }

    /// An empty file is a valid config that sets nothing.
    #[test]
    fn empty_file_is_defaults() {
        let path = config_file("empty");
        std::fs::write(&path, "").unwrap(); // openconvert-lint: allow -- test scratch
        let c = UserConfig::from_path(&path);
        assert_eq!(c.default_format, None);
    }

    /// A save → load round trip keeps every field that was set, and omits
    /// everything that was not — so an older build reading a newer file sees
    /// only the keys it knows.
    #[test]
    fn save_then_load_round_trips() {
        let path = config_file("round-trip");
        let mut c = UserConfig {
            default_format: Some("jpeg".into()),
            ..UserConfig::default()
        };
        c.theme = Some("dark".into());
        c.write_receipts = Some(false);
        c.window_x = Some(-8);

        // Point the saver at the test file by writing through it directly:
        // `save` targets the real user state, which a test must never touch.
        let body = toml::to_string_pretty(&c).unwrap(); // openconvert-lint: allow -- test scratch
        std::fs::write(&path, &body).unwrap(); // openconvert-lint: allow -- test scratch

        let loaded = UserConfig::from_path(&path);
        assert_eq!(loaded.default_format.as_deref(), Some("jpeg"));
        assert_eq!(loaded.theme.as_deref(), Some("dark"));
        assert_eq!(loaded.write_receipts, Some(false));
        assert_eq!(loaded.window_x, Some(-8));
        assert_eq!(
            loaded.window_x,
            Some(-8),
            "a key that was set has to survive the round trip"
        );

        // The serialised form carries no None keys at all.
        assert!(!body.contains("output_destination"), "got: {body}");
    }
}
