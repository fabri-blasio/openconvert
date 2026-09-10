//! Names, parsed once at the boundary.
//!
//! "Parse, don't validate", applied to the thing that actually escapes: an
//! engine's output is not a path, it is a **name**, and a name that survives
//! [`OutputName::parse`] cannot express traversal. Not "is checked for" — cannot
//! express. There is no constructor that takes a `&str` and succeeds.
//!
//! # What this is not
//!
//! It is not the containment guarantee. That comes from the OS — Landlock,
//! AppContainer, App Sandbox — and this is defence in depth on top of it.
//!
//! **Except on Linux, where it is closer to the guarantee.** Measurement found
//! the platforms safe in opposite places (spikes S3, S12c): the Windows kernel
//! refuses `..` through a directory handle outright, while on Linux `..` is an
//! ordinary component to `openat` and **escapes**. So with Landlock absent,
//! this parser is what stands between an archive member and the user's home
//! directory. `03` §9.2.

use core::fmt;

/// Exactly one path component, safe to join under a job root.
///
/// Private field, and the only way in is [`Self::parse`]. That is I6: an engine
/// cannot write outside its job directory, because the type that names its
/// output cannot describe anywhere else.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputName(String);

/// The same rules, applied to the user-facing copy-out.
///
/// A separate type from [`OutputName`] on purpose. The copy-out is the one
/// place **trusted code handles untrusted names**, and it runs the parse again
/// — independently, on the far side of the sandbox. Two types make it a
/// compile error to skip the second pass by reusing the first result.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DestinationName(String);

/// Why a name was refused.
///
/// Carries enough to tell a user *which* rule and *why it exists* — `03` §7.5
/// requires every error to say what to do, and "invalid filename" says nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// Empty, `.`, or `..`.
    NotAComponent,
    /// Contains `/` or `\`.
    ///
    /// Rejected on **every** platform, not only Windows: an archive written on
    /// one is extracted on the other, and a backslash that is an ordinary
    /// character on Linux is a separator the moment the file is opened on
    /// Windows.
    Separator,
    /// Absolute, drive-relative, UNC, or verbatim.
    NotRelative,
    /// Contains `:` — an NTFS alternate data stream.
    ///
    /// `report.txt:run.exe` writes a hidden executable stream onto an
    /// innocuous file. Also rejected on POSIX, where `:` is legal, for the
    /// cross-platform reason above.
    AlternateDataStream,
    /// Contains NUL or a control character.
    ///
    /// NUL truncates across the FFI boundary, so the name Rust validated is
    /// not the name C opens. A newline corrupts any line-oriented log,
    /// manifest or receipt.
    ControlCharacter,
    /// Contains a bidirectional override or an invisible character.
    ///
    /// **Not covered by the control-character rule**, which is how this was
    /// missed on the first attempt: U+202E RIGHT-TO-LEFT OVERRIDE is a *format*
    /// character (category Cf), and `char::is_control` only matches Cc. The
    /// evil corpus caught it; the parser had not thought of it.
    ///
    /// The attack is Trojan Source applied to filenames. A name ending
    /// `<RLO>gnp.exe` renders in every file manager as `…exe.png` — an image,
    /// apparently — while being an executable. Zero-width characters do the
    /// same job more quietly: a name with a ZWSP before `.pdf` is visually
    /// identical to one without and is a different file.
    ///
    /// A user cannot defend against a name they cannot see, so these are
    /// refused rather than displayed with a warning.
    InvisibleOrBidi,
    /// A Windows reserved device name.
    ReservedDeviceName,
    /// Trailing `.` or space.
    ///
    /// Windows silently strips both, so `evil.exe.` collides with `evil.exe`
    /// *after* every containment check has passed on a different string.
    TrailingDotOrSpace,
    /// Longer than 255 bytes.
    TooLong,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAComponent => "not a single filename",
            Self::Separator => "contains a path separator",
            Self::NotRelative => "is an absolute or drive-relative path",
            Self::AlternateDataStream => "contains ':', which names an alternate data stream",
            Self::ControlCharacter => "contains a NUL or control character",
            Self::InvisibleOrBidi => {
                "contains an invisible or text-direction character, so the name shown is \
                 not the name written"
            }
            Self::ReservedDeviceName => "is a reserved device name",
            Self::TrailingDotOrSpace => "ends in a dot or a space",
            Self::TooLong => "is longer than 255 bytes",
        })
    }
}

impl core::error::Error for NameError {}

/// Windows device names. Reserved with or without an extension, any case.
///
/// # These protect the engines, not us
///
/// Of ten reserved names tested, **only `NUL` actually vanished into a device**
/// — `CON`, `PRN`, `AUX`, `COM1`, `LPT1`, `CONIN$` and `CONOUT$` all became
/// ordinary files, because Rust's `std::fs` uses verbatim `\\?\` paths that
/// bypass Win32 device parsing (spike S13).
///
/// The rule stays, because the C engines open ordinary Win32 paths and so does
/// whatever the user opens the output with. But it protects *them*, and the
/// record should say which — the failure it prevents is a write that succeeds,
/// raises no error, and discards the data.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9",
];

/// Characters that change what a name *looks* like without changing what it is.
///
/// Deliberately a small explicit list rather than a Unicode category test: the
/// categories drift between Unicode versions, and a filename rule whose meaning
/// changes when a dependency updates is a rule nobody can reason about.
const fn is_invisible_or_bidi(c: char) -> bool {
    matches!(c,
        // Explicit bidirectional formatting: LRE RLE PDF LRO RLO.
        '\u{202A}'..='\u{202E}'
        // Isolates: LRI RLI FSI PDI.
        | '\u{2066}'..='\u{2069}'
        // Zero-width and invisible marks: ZWSP ZWNJ ZWJ LRM RLM.
        | '\u{200B}'..='\u{200F}'
        // Word joiner and the invisible maths operators.
        | '\u{2060}'..='\u{2064}'
        // Zero-width no-break space, historically the BOM.
        | '\u{FEFF}'
    )
}

fn parse_component(s: &str) -> Result<String, NameError> {
    if s.is_empty() || s == "." || s == ".." {
        return Err(NameError::NotAComponent);
    }
    if s.len() > 255 {
        return Err(NameError::TooLong);
    }
    if s.contains('/') || s.contains('\\') {
        return Err(NameError::Separator);
    }
    if s.starts_with('/') {
        return Err(NameError::NotRelative);
    }
    // `:` covers ADS, `C:`, and drive-relative `C:file.txt` in one rule. The
    // separator check above already caught `\\?\` and UNC.
    if s.contains(':') {
        return Err(NameError::AlternateDataStream);
    }
    if s.chars().any(char::is_control) {
        return Err(NameError::ControlCharacter);
    }
    if s.chars().any(is_invisible_or_bidi) {
        return Err(NameError::InvisibleOrBidi);
    }
    if s.ends_with('.') || s.ends_with(' ') {
        return Err(NameError::TrailingDotOrSpace);
    }

    let stem = s.split('.').next().unwrap_or(s);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(NameError::ReservedDeviceName);
    }

    Ok(s.to_string())
}

impl OutputName {
    /// Parse one component. The only constructor.
    ///
    /// # Errors
    ///
    /// See [`NameError`]. Each variant names a rule and the reason it exists.
    pub fn parse(s: &str) -> Result<Self, NameError> {
        parse_component(s).map(Self)
    }

    /// The name, for joining under a job root.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A key for detecting collisions between two names that **look** the same.
    ///
    /// # Why this is a collision rule and not a rejection rule
    ///
    /// v0.5 rejected any component "that normalises to a different name under
    /// NFC/NFD". Tested against ten ordinary filenames, **that rule rejected
    /// five of them** — `café.jpg`, `Müller-Bericht.pdf`, and the Greek,
    /// Cyrillic and French cases (spike S24). Every archive produced on macOS,
    /// which stores NFD, would have failed extraction with a security error.
    ///
    /// So names are preserved byte-for-byte and this is used only to compare
    /// them. Two entries in one extraction whose keys are equal is a collision,
    /// resolved by `Policy::on_conflict` like any other.
    ///
    /// The underlying premise was also wrong: **NTFS does not fold NFC and
    /// NFD** — both files exist side by side. APFS folds; ext4 does not. Three
    /// filesystems, three outcomes for one archive, which is recorded in
    /// `09` §8 rather than pretended away.
    #[must_use]
    pub fn collision_key(&self) -> String {
        use unicode_normalization::UnicodeNormalization;
        self.0.nfc().collect::<String>().to_lowercase()
    }
}

impl DestinationName {
    /// Parse one component, for the user-facing copy-out.
    ///
    /// # Errors
    ///
    /// See [`NameError`].
    pub fn parse(s: &str) -> Result<Self, NameError> {
        parse_component(s).map(Self)
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DestinationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
