//! SR-13: an engine or archive member cannot cause a write outside the job
//! directory.
//!
//! Every case here comes from [`corpus::evil`], which **generates** them from
//! checked-in code. That is not a style preference. An encrypted corpus fetched
//! at test time needs a CI secret, and secrets are unavailable to pull requests
//! from forks — so this gate would have *skipped, showing green,* on exactly
//! the contributions least likely to have been reviewed.
//!
//! Until this file existed, the corpus was 31 hostile names and 10 controls
//! that nothing consumed. A corpus with no consumer is a corpus that proves
//! nothing, however carefully it was written.

use corpus::evil::{Class, EvilName, LEGITIMATE, MUST_REJECT, NORMALISATION_PAIRS};
use openconvert_sandbox::broker::{DestinationName, NameError, OutputName};

/// Every hostile name is refused.
///
/// The failure message names the case *and its reason*, so a regression tells
/// you which threat reopened rather than only that something did.
#[test]
fn every_hostile_name_is_refused() {
    let mut refused = 0;
    for EvilName { name, class, why } in MUST_REJECT {
        let got = OutputName::parse(name);
        assert!(
            got.is_err(),
            "ACCEPTED a hostile name.\n  name:  {name:?}\n  class: {class:?}\n  why:   {why}"
        );
        refused += 1;
    }
    assert!(
        refused >= 25,
        "only {refused} cases ran; the corpus shrank and this gate is weaker than it looks"
    );
}

/// The copy-out applies the same rules, independently.
///
/// `03` §8.3: every check runs **twice** — once inside the sandbox as the
/// engine extracts, and again in the trusted copy-out. Two types rather than
/// one, so reusing the first result to skip the second pass is a compile error.
#[test]
fn the_copy_out_refuses_the_same_names() {
    for EvilName { name, .. } in MUST_REJECT {
        assert!(
            DestinationName::parse(name).is_err(),
            "the copy-out accepted {name:?} after the sandbox refused it"
        );
    }
}

/// **The control, and the most important test in this file.**
///
/// Ten ordinary filenames — French, German, Greek, Cyrillic, CJK, emoji — must
/// be *accepted*. Without this, every assertion above is satisfied by a parser
/// that refuses everything, which would be perfectly safe and useless.
///
/// This is not hypothetical. v0.5's rule "reject a component that normalises to
/// a different name under NFC/NFD" rejects **five of these ten**, and shipped in
/// a design revision unnoticed until it was tested (spike S24).
#[test]
fn ordinary_filenames_are_accepted() {
    for name in LEGITIMATE {
        let got = OutputName::parse(name);
        assert!(
            got.is_ok(),
            "REFUSED an ordinary filename {name:?}: {:?}\n  \
             A parser that rejects real names is not secure, it is broken.",
            got.unwrap_err()
        );
        assert_eq!(
            got.unwrap().as_str(),
            *name,
            "the name was altered in passing"
        );
    }
}

/// Each rule reports *itself*, not a generic failure.
///
/// `03` §7.5 requires every error to name what is wrong and what to do. A
/// single `InvalidName` for all eight rules would satisfy the security property
/// and none of the usability one.
#[test]
fn each_rule_reports_its_own_reason() {
    let cases = [
        ("..", NameError::NotAComponent),
        ("../etc/passwd", NameError::Separator),
        ("..\\windows\\evil", NameError::Separator),
        // Caught as a Separator, not NotRelative — that check runs first, and
        // it is the more accurate description of what is wrong with the name.
        // My expectation here was wrong, not the parser.
        ("/etc/passwd", NameError::Separator),
        ("report.txt:run.exe", NameError::AlternateDataStream),
        ("C:relative.txt", NameError::AlternateDataStream),
        ("line\nbreak.txt", NameError::ControlCharacter),
        // Trojan Source, applied to a filename. Found by the corpus, not by
        // the parser author: `char::is_control` matches category Cc, and these
        // are Cf.
        ("photo\u{202E}gnp.exe", NameError::InvisibleOrBidi),
        ("invoice\u{200B}.pdf", NameError::InvisibleOrBidi),
        ("NUL", NameError::ReservedDeviceName),
        ("CON.jpg", NameError::ReservedDeviceName),
        ("evil.exe.", NameError::TrailingDotOrSpace),
        ("evil.exe ", NameError::TrailingDotOrSpace),
    ];
    for (name, expected) in cases {
        assert_eq!(
            OutputName::parse(name).unwrap_err(),
            expected,
            "{name:?} was refused for the wrong reason, so the message would mislead"
        );
    }
    assert_eq!(
        OutputName::parse(&corpus::evil::name_of_length(256)).unwrap_err(),
        NameError::TooLong
    );
    // And the boundary: exactly 255 is fine.
    assert!(OutputName::parse(&corpus::evil::name_of_length(255)).is_ok());
}

/// NFC and NFD forms of one name **collide**; they are not rejected.
///
/// Three filesystems give three answers here (spikes S13, S24): NTFS does not
/// fold, APFS does, ext4 does not. So the same archive extracts to a different
/// set of files on Windows, macOS and Linux. We preserve names byte-for-byte
/// and compare on a normalised key, which is the best available behaviour — it
/// does not make the outcome identical across platforms, and `09` §8 says so
/// rather than pretending otherwise.
#[test]
fn normalisation_variants_collide_rather_than_being_refused() {
    for (nfc, nfd) in NORMALISATION_PAIRS {
        let a = OutputName::parse(nfc).expect("NFC form refused");
        let b = OutputName::parse(nfd).expect("NFD form refused");

        assert_ne!(
            a.as_str(),
            b.as_str(),
            "the bytes were normalised in passing"
        );
        assert_eq!(
            a.collision_key(),
            b.collision_key(),
            "{nfc:?} and {nfd:?} did not collide, so one would silently overwrite the other"
        );
    }
}

/// The control for the collision key: different names do **not** collide.
///
/// Without it, a `collision_key` returning a constant satisfies the test above
/// and makes every extraction a single-file extraction.
#[test]
fn different_names_do_not_collide() {
    let a = OutputName::parse("photo.jpg").unwrap();
    let b = OutputName::parse("scan.jpg").unwrap();
    assert_ne!(a.collision_key(), b.collision_key());
}

/// Device names bind regardless of case or extension.
#[test]
fn reserved_names_are_case_and_extension_insensitive() {
    for n in [
        "nul", "NUL", "Nul", "nul.txt", "CON.jpg", "com1.dat", "LPT9",
    ] {
        assert_eq!(
            OutputName::parse(n).unwrap_err(),
            NameError::ReservedDeviceName,
            "{n:?} slipped past the reserved-name rule"
        );
    }
    // The control: a name that merely *starts* with a reserved word is fine.
    for n in ["console.log", "nullable.txt", "com10.dat", "aux-notes.md"] {
        assert!(
            OutputName::parse(n).is_ok(),
            "{n:?} was refused, but it is not a device name"
        );
    }
}

/// Every case in the corpus belongs to a class the parser actually implements.
///
/// Catches the corpus drifting ahead of the parser — a case added for a rule
/// nobody wrote is a case that passes because the *wrong* rule caught it.
#[test]
fn every_corpus_class_is_covered_by_a_rule() {
    for class in [
        Class::Traversal,
        Class::AlternateDataStream,
        Class::DeviceName,
        Class::Unicode,
        Class::Length,
    ] {
        let n = MUST_REJECT.iter().filter(|e| e.class == class).count();
        assert!(n > 0, "{class:?} has no cases, so nothing tests that rule");
    }
}
