//! Hostile names, **generated from this file rather than fetched**.
//!
//! ## Why this is code and not a downloaded archive
//!
//! An encrypted corpus fetched at test time needs a CI secret, and secrets are
//! unavailable to pull requests from forks. So the SR-13 and SR-15 gates would
//! have **skipped, showing green, on exactly the contributions least likely to
//! have been reviewed** — an outside contributor's first PR.
//!
//! Generating the cases from checked-in code costs nothing and cannot skip.
//! Real exploit samples still never enter this repository; they run in a
//! separate nightly job that is allowed to be secret-dependent precisely
//! because it is not the gate. See `09` §9.
//!
//! ## What is here
//!
//! Every case is annotated with the class it belongs to and, where one exists,
//! the measurement that put it here. Several entries look like paranoia and are
//! not: they are names that behaved differently from expectation when tested.

/// One hostile name, and why it is hostile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvilName {
    /// The name an archive entry or engine output might carry.
    pub name: &'static str,
    /// What class of attack or hazard it represents.
    pub class: Class,
    /// Why this specific string is here. Shown on failure, so a future reader
    /// knows what broke rather than only that something did.
    pub why: &'static str,
}

/// Attack classes, matching `09` §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A12 — escaping the job directory.
    Traversal,
    /// A12 — Windows alternate data streams.
    AlternateDataStream,
    /// A12 — reserved device names.
    DeviceName,
    /// A13 — a name that is legal but destroys something on arrival.
    Collision,
    /// Unicode handling: normalisation, direction overrides, homoglyphs.
    Unicode,
    /// Length and truncation.
    Length,
}

/// Names `OutputName::parse` must **reject**.
///
/// Note what is *not* here: ordinary non-English filenames. An earlier version
/// of the design rejected "a component that normalises to a different name
/// under NFC/NFD", and tested against ten ordinary names **that rule rejected
/// five of them** — `café.jpg`, `Müller-Bericht.pdf`, and the Greek, Cyrillic
/// and French cases (spike S24). Those now live in [`LEGITIMATE`] and must be
/// accepted. Normalisation is a *collision* rule, not a rejection rule.
pub const MUST_REJECT: &[EvilName] = &[
    // ---- A12: traversal -------------------------------------------------
    EvilName {
        name: "..",
        class: Class::Traversal,
        why: "the parent directory — the single component that escapes, and the one \n              Linux openat() accepts as ordinary while the NT object manager refuses it",
    },
    EvilName {
        name: ".",
        class: Class::Traversal,
        why: "the current directory — same hazard as the empty name, and it survives \n              a naive filter that only looks for '..'",
    },
    EvilName {
        name: "",
        class: Class::Traversal,
        why: "empty name — joins to the job root itself, so a create would target \n              the directory rather than a file inside it",
    },
    EvilName {
        name: "../etc/passwd",
        class: Class::Traversal,
        why: "classic Zip Slip; a member path is a SEQUENCE of names, parsed one at a time",
    },
    EvilName {
        name: "..\\windows\\system32\\drivers\\etc\\hosts",
        class: Class::Traversal,
        why: "backslash separator — rejected on every platform, not only Windows, \
              because an archive written on Windows is extracted on Linux",
    },
    EvilName {
        name: "/etc/passwd",
        class: Class::Traversal,
        why: "absolute POSIX path — ignores the job root entirely when joined naively",
    },
    EvilName {
        name: "C:\\Windows\\System32\\calc.exe",
        class: Class::Traversal,
        why: "absolute Windows path — ignores the job root entirely when joined naively",
    },
    EvilName {
        name: "C:relative.txt",
        class: Class::Traversal,
        why: "drive-RELATIVE form — resolves against the drive's current directory, \
              which is not the job directory and is not even predictable",
    },
    EvilName {
        name: "\\\\?\\C:\\Windows\\evil.exe",
        class: Class::Traversal,
        why: "verbatim path — bypasses Win32 path parsing entirely, which is \
              exactly why our own std::fs uses it (spike S13)",
    },
    EvilName {
        name: "\\\\server\\share\\evil.exe",
        class: Class::Traversal,
        why: "UNC path — a write that leaves the machine altogether, which no \n              filesystem-level containment check would even see",
    },
    EvilName {
        name: "....//....//etc/passwd",
        class: Class::Traversal,
        why: "doubled dots, defeating a naive strip-'../' filter that runs once",
    },
    // ---- A12: alternate data streams ------------------------------------
    EvilName {
        name: "report.txt:run.exe",
        class: Class::AlternateDataStream,
        why: "NTFS ADS — writes a hidden executable stream onto an innocuous file",
    },
    EvilName {
        name: "photo.jpg:Zone.Identifier",
        class: Class::AlternateDataStream,
        why: "overwrites the mark-of-the-web stream — forging provenance rather than \
              hiding a payload, which is the subtler use",
    },
    // ---- A12: reserved device names -------------------------------------
    // Only NUL actually vanished into a device when tested; CON, PRN, AUX,
    // COM1, LPT1, CONIN$ and CONOUT$ all became ordinary files, because
    // std::fs uses verbatim paths (spike S13). The rule stays because it
    // protects the C ENGINES, which open ordinary Win32 paths, and whatever
    // the user opens the output with. It protects them, not us.
    EvilName {
        name: "NUL",
        class: Class::DeviceName,
        why: "the one that genuinely vanishes — write succeeds, data discarded, no error",
    },
    EvilName {
        name: "CON",
        class: Class::DeviceName,
        why: "console device — harmless on our own verbatim paths, hazardous the \n              moment a C engine opens the name as an ordinary Win32 path",
    },
    EvilName {
        name: "PRN",
        class: Class::DeviceName,
        why: "printer device — output routed to hardware instead of a file",
    },
    EvilName {
        name: "AUX",
        class: Class::DeviceName,
        why: "auxiliary device — legacy alias, still resolved by Win32 path parsing",
    },
    EvilName {
        name: "COM1",
        class: Class::DeviceName,
        why: "serial port — COM1..COM9 all bind; a loop over the range is not paranoia",
    },
    EvilName {
        name: "LPT1",
        class: Class::DeviceName,
        why: "parallel port — LPT1..LPT9 all bind, same as the COM range",
    },
    EvilName {
        name: "CONIN$",
        class: Class::DeviceName,
        why: "console input — absent from most reserved-name lists",
    },
    EvilName {
        name: "CONOUT$",
        class: Class::DeviceName,
        why: "console output — absent from most reserved-name lists",
    },
    EvilName {
        name: "nul.txt",
        class: Class::DeviceName,
        why: "device names bind regardless of extension, and case does not matter",
    },
    EvilName {
        name: "CON.jpg",
        class: Class::DeviceName,
        why: "device name with a plausible media extension — the form most likely to \n              survive a human glance at an archive listing",
    },
    // ---- Windows silent-strip collisions --------------------------------
    EvilName {
        name: "evil.exe.",
        class: Class::Collision,
        why: "Windows silently strips the trailing dot, so this collides with evil.exe \
              AFTER every containment check has passed on a different string",
    },
    EvilName {
        name: "evil.exe ",
        class: Class::Collision,
        why: "trailing space — stripped the same way, and invisible in every UI a \n              reviewer would use to inspect the archive",
    },
    // ---- Unicode --------------------------------------------------------
    EvilName {
        name: "photo\u{202E}gnp.exe",
        class: Class::Unicode,
        why: "right-to-left override — renders as 'photoexe.png' in a file manager \
              while being an executable",
    },
    EvilName {
        name: "invoice\u{200B}.pdf",
        class: Class::Unicode,
        why: "zero-width space — visually identical to invoice.pdf",
    },
    EvilName {
        name: "sched\u{0000}ule.pdf",
        class: Class::Length,
        why: "embedded NUL — truncates to 'sched' across an FFI boundary, so the \
              name Rust validated is not the name C opens",
    },
    EvilName {
        name: "line\nbreak.txt",
        class: Class::Length,
        why: "newline — corrupts any line-oriented log, manifest or receipt",
    },
];

/// Names that are hostile as a **pair**, not individually.
///
/// Each of these is a perfectly legal single path component. The traversal
/// checks pass them, correctly, because they are not traversal — and that is
/// the entire point. This is where an earlier revision of the design lost the
/// user's existing files: an archive whose members are named after files
/// already in the destination.
pub const MUST_COLLIDE_NOT_DESTROY: &[EvilName] = &[
    EvilName {
        name: ".gitconfig",
        class: Class::Collision,
        why: "a dotfile the user already has — legal, non-traversing, and destructive \n              on arrival if anything truncates rather than refusing",
    },
    EvilName {
        name: ".bashrc",
        class: Class::Collision,
        why: "executes on next shell login — overwriting it turns a file conversion \n              into code execution, with no memory-safety bug anywhere",
    },
    EvilName {
        name: "Brand-Guidelines.pdf",
        class: Class::Collision,
        why: "plausible archive content, irreplaceable to its owner — this is the case \n              an earlier revision lost, because every traversal check passes it",
    },
    EvilName {
        name: "logo.ai",
        class: Class::Collision,
        why: "irreplaceable to its owner, and the sort of name a design brief archive \n              genuinely contains",
    },
    EvilName {
        name: "id_rsa",
        class: Class::Collision,
        why: "if the destination is ~/.ssh and the user chose it, the conversion has \n              replaced a private key — we refuse to overwrite; we do not refuse to write",
    },
];

/// Pairs that must be treated as a **collision**, never silently merged and
/// never rejected outright.
///
/// NFC and NFD forms of the same visible name. Three filesystems give three
/// different answers here (spikes S13, S24):
///
/// | Filesystem | Behaviour |
/// |---|---|
/// | NTFS | does **not** fold — both files exist side by side |
/// | APFS | folds |
/// | ext4 | does not fold |
///
/// So the same archive extracts to a different set of files on Windows, macOS
/// and Linux. We preserve names byte-for-byte and use NFC only as a comparison
/// key, which is the best available behaviour; it does not make the outcome
/// identical across platforms, and `09` §8 says so.
pub const NORMALISATION_PAIRS: &[(&str, &str)] = &[
    ("caf\u{00E9}.jpg", "cafe\u{0301}.jpg"),
    ("\u{00C5}ngstr\u{00F6}m.txt", "A\u{030A}ngstro\u{0308}m.txt"),
];

/// Names that **must be accepted**.
///
/// This list is the control, and it is the most important list in the file.
/// Without it every property here is satisfied by a parser that rejects
/// everything — and a rule that rejected five of these ten shipped in a design
/// revision, unnoticed, until it was tested (spike S24).
pub const LEGITIMATE: &[&str] = &[
    "photo.jpg",
    "Annual Report 2026.pdf",
    "caf\u{00E9}.jpg",
    "M\u{00FC}ller-Bericht.pdf",
    "\u{03A3}\u{03C7}\u{03AD}\u{03B4}\u{03B9}\u{03BF}.pdf",
    "\u{041E}\u{0442}\u{0447}\u{0451}\u{0442}.docx",
    "r\u{00E9}sum\u{00E9}-fran\u{00E7}ais.odt",
    "\u{6587}\u{4EF6}.txt",
    "emoji-\u{1F4C1}-folder.zip",
    "file.with.many.dots.tar.gz",
];

/// A name of exactly `n` bytes, for length-boundary tests.
///
/// 255 is the filesystem limit for a single component. The *path* is bounded
/// separately, at the job root: an 829-character path with a 255-character leaf
/// was created without complaint by our own Rust, which uses verbatim paths —
/// but `MAX_PATH` still binds the C engines (spike S28).
#[must_use]
pub fn name_of_length(n: usize) -> String {
    "a".repeat(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus is non-trivial and its two halves do not overlap.
    ///
    /// A corpus that is accidentally empty passes every test that consumes it.
    /// That is not hypothetical here: mutation testing found a test in this
    /// project that passed vacuously when its own input was emptied, after 7.4
    /// million fuzz cases had failed to notice (spikes S29, S30).
    #[test]
    fn corpus_is_populated_and_disjoint() {
        assert!(
            MUST_REJECT.len() >= 25,
            "reject corpus shrank: {}",
            MUST_REJECT.len()
        );
        assert!(
            LEGITIMATE.len() >= 10,
            "control corpus shrank: {}",
            LEGITIMATE.len()
        );
        assert!(!MUST_COLLIDE_NOT_DESTROY.is_empty());
        assert!(!NORMALISATION_PAIRS.is_empty());

        for evil in MUST_REJECT {
            assert!(
                !LEGITIMATE.contains(&evil.name),
                "{:?} is in both MUST_REJECT and LEGITIMATE — one of them is wrong",
                evil.name
            );
        }
    }

    /// Every entry explains itself. A corpus case with no stated reason becomes
    /// a case nobody dares delete and nobody understands.
    #[test]
    fn every_case_states_why() {
        for e in MUST_REJECT.iter().chain(MUST_COLLIDE_NOT_DESTROY) {
            assert!(e.why.len() > 15, "{:?} has no real explanation", e.name);
        }
    }

    /// The normalisation pairs really are distinct byte sequences that mean the
    /// same thing — the control for the collision rule.
    #[test]
    fn normalisation_pairs_differ_in_bytes() {
        for (nfc, nfd) in NORMALISATION_PAIRS {
            assert_ne!(nfc, nfd, "pair is byte-identical, so it tests nothing");
            assert!(nfd.len() > nfc.len(), "the NFD form should be longer");
        }
    }
}
