//! Progress and summary rendering for multi-file runs.
//!
//! # Why the glyphs are chosen at runtime
//!
//! `07-DESIGN-SYSTEM` §"the glyphs are a dependency" is explicit that the GUI
//! solves the glyph problem by subsetting a font it ships, and that the CLI
//! **cannot**: we control no font in someone else's terminal. The rule it sets
//! for us is to detect capability and fall back to ASCII "rather than emitting
//! a box".
//!
//! So this is conservative in one direction on purpose: ASCII unless there is
//! positive evidence of a UTF-8 terminal. A box glyph is worse than `~`,
//! because `~` is legible and a box is a bug report. `⛨` (U+26E8) in
//! particular is absent from most terminal fonts on every platform.
//!
//! The label always follows the glyph (`≈ lossy`, not a bare `≈`), which is
//! the same rule the design system gives the GUI: the meaning survives the
//! glyph failing.

use openconvert_core::plan::Class;

/// Which vocabulary this terminal gets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyphs {
    /// The design-system glyphs, for a terminal that showed evidence of UTF-8.
    Unicode,
    /// `07-DESIGN-SYSTEM`'s stated ASCII fallback.
    Ascii,
}

impl Glyphs {
    /// Detect once, from the environment only.
    ///
    /// Never probes by writing: a capability test that prints is visible to
    /// the user when it is wrong, and this runs before every batch.
    #[must_use]
    pub fn detect() -> Self {
        // An explicit answer wins over any guess. Someone piping our output
        // into a file knows better than we do what will read it.
        match std::env::var("OPENCONVERT_GLYPHS").as_deref() {
            Ok("unicode") => return Self::Unicode,
            Ok("ascii") => return Self::Ascii,
            _ => {}
        }
        if cfg!(windows) {
            // Windows Terminal and VS Code's terminal are UTF-8 and ship
            // fonts with these ranges. The legacy conhost console is neither,
            // and it is still the default for a double-clicked .exe.
            let wt = std::env::var_os("WT_SESSION").is_some();
            let vscode = std::env::var("TERM_PROGRAM").is_ok_and(|v| v == "vscode");
            if wt || vscode {
                return Self::Unicode;
            }
            return Self::Ascii;
        }
        // Elsewhere the locale is the conventional answer, and an unset
        // locale means the C locale, which is ASCII.
        let utf8 = ["LC_ALL", "LC_CTYPE", "LANG"].iter().any(|k| {
            std::env::var(k).is_ok_and(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("utf-8") || v.contains("utf8")
            })
        });
        if utf8 {
            Self::Unicode
        } else {
            Self::Ascii
        }
    }

    /// The class glyph and its label, always together.
    #[must_use]
    pub fn class(self, class: Class) -> &'static str {
        match (self, class) {
            (Self::Unicode, Class::A) => "= lossless",
            (Self::Unicode, Class::B) => "≈ lossy",
            (Self::Unicode, Class::C) => "⌇ AI-read",
            (Self::Unicode, Class::D) => "✦ AI-generated",
            (Self::Ascii, Class::A) => "= lossless",
            (Self::Ascii, Class::B) => "~ lossy",
            (Self::Ascii, Class::C) => "? AI-read",
            (Self::Ascii, Class::D) => "* AI-generated",
        }
    }

    /// The sandbox marker. Quiet by design — `07` §"don't decorate it".
    #[must_use]
    pub fn sandboxed(self) -> &'static str {
        match self {
            Self::Unicode => "⛨",
            Self::Ascii => "[#]",
        }
    }

    /// Per-file status in a progress list.
    #[must_use]
    pub fn status(self, s: Status) -> &'static str {
        match (self, s) {
            (Self::Unicode, Status::Ok) => "✓",
            (Self::Unicode, Status::Failed) => "✗",
            (Self::Unicode, Status::Skipped) => "·",
            (Self::Ascii, Status::Ok) => "+",
            (Self::Ascii, Status::Failed) => "x",
            (Self::Ascii, Status::Skipped) => ".",
        }
    }

    /// The rule above and below a summary block.
    #[must_use]
    pub fn rule(self) -> &'static str {
        match self {
            Self::Unicode => "─────────────────────────────────────────────────",
            Self::Ascii => "-------------------------------------------------",
        }
    }

    /// Between two facts on one line.
    #[must_use]
    pub fn sep(self) -> &'static str {
        match self {
            Self::Unicode => " · ",
            Self::Ascii => " | ",
        }
    }

    /// Input to output.
    #[must_use]
    pub fn arrow(self) -> &'static str {
        match self {
            Self::Unicode => "→",
            Self::Ascii => "->",
        }
    }
}

/// How one file ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok,
    Failed,
    Skipped,
}

/// A byte count, in the units a person reads.
#[must_use]
pub fn bytes(n: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let x = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", x / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", x / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", x / (1024.0 * 1024.0 * 1024.0))
    }
}

/// A duration, in the units a person reads.
#[must_use]
pub fn secs(d: std::time::Duration) -> String {
    let s = d.as_secs_f64();
    if s < 10.0 {
        format!("{s:.1}s")
    } else if s < 600.0 {
        format!("{s:.0}s")
    } else {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback is the point of the module: every glyph must differ
    /// between the vocabularies, or a terminal that cannot render one is
    /// being handed it anyway.
    #[test]
    fn ascii_vocabulary_shares_no_multibyte_glyph() {
        for g in [
            Glyphs::Ascii.class(Class::A),
            Glyphs::Ascii.class(Class::B),
            Glyphs::Ascii.class(Class::C),
            Glyphs::Ascii.class(Class::D),
            Glyphs::Ascii.sandboxed(),
            Glyphs::Ascii.status(Status::Ok),
            Glyphs::Ascii.status(Status::Failed),
            Glyphs::Ascii.status(Status::Skipped),
            Glyphs::Ascii.rule(),
            Glyphs::Ascii.sep(),
            Glyphs::Ascii.arrow(),
        ] {
            assert!(g.is_ascii(), "{g:?} is not ASCII in the ASCII vocabulary");
        }
    }

    /// The label survives the glyph. A terminal that renders `≈` as a box
    /// still shows the word "lossy" beside it.
    #[test]
    fn every_class_carries_its_word() {
        for v in [Glyphs::Unicode, Glyphs::Ascii] {
            assert!(v.class(Class::A).contains("lossless"));
            assert!(v.class(Class::B).contains("lossy"));
            assert!(v.class(Class::C).contains("AI-read"));
            assert!(v.class(Class::D).contains("AI-generated"));
        }
    }

    #[test]
    fn units_read_the_way_a_person_reads_them() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(12_300_000), "11.7 MB");
        assert_eq!(secs(std::time::Duration::from_millis(300)), "0.3s");
        assert_eq!(secs(std::time::Duration::from_secs(14)), "14s");
        assert_eq!(secs(std::time::Duration::from_secs(754)), "12m34s");
    }
}
