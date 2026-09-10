//! `OutputName::parse` over arbitrary text.
//!
//! This parser is the last thing standing between an archive member and the
//! user's home directory when Landlock is absent -- measurement found `..`
//! escapes an `openat` on Linux while the Windows kernel refuses it, so the
//! platforms are safe in opposite places and this is load-bearing on one of
//! them (spikes S3, S12c).
//!
//! The assertions matter as much as the absence of panics. A parser that
//! *accepts* something containing a separator has not crashed; it has failed.
#![no_main]
use libfuzzer_sys::fuzz_target;
use openconvert_sandbox::broker::OutputName;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(name) = OutputName::parse(text) else {
        return;
    };
    let s = name.as_str();

    // Anything that parsed is exactly one component, and cannot escape.
    assert!(!s.is_empty());
    assert_ne!(s, ".");
    assert_ne!(s, "..");
    assert!(!s.contains('/'), "accepted a POSIX separator: {s:?}");
    assert!(!s.contains('\\'), "accepted a Windows separator: {s:?}");
    assert!(!s.contains(':'), "accepted an ADS separator: {s:?}");
    assert!(!s.chars().any(char::is_control));
    assert!(!s.ends_with('.') && !s.ends_with(' '));
    assert!(s.len() <= 255);

    // The bytes survive unchanged. A parser that silently normalises is a
    // parser whose output nobody can predict from its input.
    assert_eq!(s, text, "the name was altered in passing");

    // Idempotent: parsing the result again gives the same thing.
    assert_eq!(OutputName::parse(s).unwrap().as_str(), s);
});
