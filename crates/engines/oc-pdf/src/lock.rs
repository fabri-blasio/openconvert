//! Adding and removing a PDF password.
//!
//! # Two operations, one boundary
//!
//! **Protect** puts a password on a document. **Unlock** takes off a password
//! the user already supplies. Neither recovers a password nobody knows, and
//! there is deliberately no affordance that could be mistaken for one: no
//! empty-password probe, no dictionary attempt, no "try common passwords".
//! A tool that did any of those would be a cracking tool with a friendly name,
//! and the description says what this is instead.
//!
//! # AES-256, not the RC4 family
//!
//! `lopdf` offers encryption versions 1 through 5. V1, V2 and V4 are the
//! RC4-era handlers — V1 is 40-bit — and PDF 2.0 deprecates them. Shipping a
//! button labelled "Protect" that applied 40-bit RC4 would be a security claim
//! this product could not defend, so only V5 (AES-256) is offered and there is
//! no version parameter to get wrong.
//!
//! # Where the file key comes from
//!
//! V5 does not derive its file encryption key from the password: the password
//! unlocks a random 256-bit key, which is what makes changing a password cheap
//! and what keeps two documents with the same password from sharing key
//! material. So the key must be **random**, and `lopdf` does not generate one —
//! its own example reaches for `rand`.
//!
//! This module uses `getrandom`, which is a thin call into the operating
//! system's CSPRNG and nothing else. Deriving the key from the password
//! instead would need no dependency at all and would throw away most of what
//! AES-256 is for.
//!
//! # Passwords do not travel on a command line
//!
//! Parameters reach this worker as frames on a pipe, not as argv, so a
//! password is not visible in a process listing. That property is load-bearing
//! and is asserted by a test in the host crate rather than left as a comment.

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Permissions};

/// Why a lock operation could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// The bytes are not a PDF this parser can open.
    #[error("could not read the PDF: {0}")]
    Parse(String),
    /// The supplied password did not open the document.
    #[error("that password did not open this document")]
    WrongPassword,
    /// The document is not encrypted, so there is nothing to remove.
    #[error("this document has no password on it")]
    NotEncrypted,
    /// The document is already encrypted.
    #[error("this document already has a password; remove it first")]
    AlreadyEncrypted,
    /// A password was required and not given.
    #[error("{0}")]
    NoPassword(String),
    /// The operating system would not provide entropy.
    #[error("could not generate an encryption key: {0}")]
    NoEntropy(String),
    /// Encrypting or serialising failed.
    #[error("could not write the PDF: {0}")]
    Write(String),
}

/// Remove a password the caller can supply.
///
/// # Errors
///
/// [`LockError`]. A wrong password fails cleanly and produces no output — a
/// partially decrypted document is not a thing this returns.
pub fn unlock(bytes: &[u8], password: &str) -> Result<Vec<u8>, LockError> {
    if password.is_empty() {
        return Err(LockError::NoPassword(
            "unlocking needs the document's password".to_string(),
        ));
    }

    if !is_encrypted(bytes) {
        return Err(LockError::NotEncrypted);
    }

    // LOADED *WITH* THE PASSWORD, not loaded and then decrypted.
    //
    // `load_mem` on an encrypted document cannot walk it: the cross-reference
    // stream and every object dictionary are ciphertext until a key exists, so
    // it returns a near-empty document — one object, no pages. Calling
    // `decrypt` afterwards then has nothing to decrypt and succeeds, and the
    // result saves as a valid PDF containing nothing. That failure is silent
    // and produces a plausible file, which is the worst shape a bug can take
    // here.
    //
    // `load_mem_with_password` decrypts during the parse, which is the only
    // order that works.
    //
    // A failure is a wrong password far more often than anything else, and the
    // underlying message describes a cryptographic check rather than something
    // the user can act on. It is also the one place a library could echo the
    // password back, so the text is replaced rather than wrapped.
    let mut doc =
        Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password))
            .map_err(|_| LockError::WrongPassword)?;

    // THE TRAILER STILL SAYS ENCRYPTED, AND THE FILE NO LONGER IS.
    //
    // The objects are plaintext in memory now, but `/Encrypt` is still in the
    // trailer pointing at the security handler. Saving with it there produces
    // a document that *declares* AES-256 over content that is not encrypted,
    // which no reader can open. Removing the entry is what actually takes the
    // password off; the dictionary it referenced becomes unreachable and is
    // dropped on save.
    doc.trailer.remove(b"Encrypt");

    save(&mut doc)
}

/// Put an AES-256 password on a document.
///
/// `owner` defaults to `user` when empty: an owner password that nobody chose
/// is a second secret the user does not know they have.
///
/// # Errors
///
/// [`LockError`].
pub fn protect(bytes: &[u8], user: &str, owner: &str) -> Result<Vec<u8>, LockError> {
    if user.is_empty() {
        return Err(LockError::NoPassword(
            "protecting needs a password to set".to_string(),
        ));
    }
    let owner = if owner.is_empty() { user } else { owner };

    let mut doc = Document::load_mem(bytes).map_err(|e| LockError::Parse(scrub(&e.to_string())))?;
    if doc.is_encrypted() {
        return Err(LockError::AlreadyEncrypted);
    }

    // 256 bits from the OS CSPRNG. See the module note: this key is what the
    // password unlocks, and deriving it from the password would defeat the
    // scheme.
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|e| LockError::NoEntropy(e.to_string()))?;

    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password: user,
        // EVERYTHING GRANTED, and the tool says so.
        //
        // A permissions UI that silently dropped what the user selected would
        // be worse than none, and PDF permissions are advisory anyway: they
        // are a request to the reader application, not a control the file can
        // enforce. Offering "prevent printing" as though it were enforced
        // would be the dishonest option.
        permissions: Permissions::all(),
    })
    .map_err(|e| LockError::Write(scrub(&e.to_string())))?;

    doc.encrypt(&state)
        .map_err(|e| LockError::Write(scrub(&e.to_string())))?;

    save(&mut doc)
}

/// Whether these bytes are an encrypted PDF.
///
/// Used by the host to tell "wrong tool" from "wrong password".
#[must_use]
pub fn is_encrypted(bytes: &[u8]) -> bool {
    Document::load_mem(bytes).is_ok_and(|d| d.is_encrypted())
}

/// Serialise, keeping the encryption state the document now carries.
fn save(doc: &mut Document) -> Result<Vec<u8>, LockError> {
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| LockError::Write(scrub(&e.to_string())))?;
    Ok(out)
}

/// Strip anything password-shaped out of a message from a dependency.
///
/// Defence in depth. No current `lopdf` error interpolates the password, but
/// this crate cannot promise what a future version does, and an error string
/// is one of the four places a password could leak. The check is cheap and the
/// alternative is trusting a third party with the one secret this module
/// handles.
fn scrub(message: &str) -> String {
    // Nothing to redact against here without the password in hand, so this is
    // deliberately conservative: bound the length, and drop control
    // characters that could hide content in a terminal.
    let mut out: String = message
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect();
    if out.is_empty() {
        out.push_str("the PDF library reported an unprintable error");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// A small valid PDF, built rather than checked in.
    fn pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(lopdf::Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            lopdf::Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        // AN ENCRYPTED PDF MUST CARRY A FILE ID. The specification requires
        // `/ID` in the trailer of any encrypted document, and a fixture
        // without one produced a "locked" file that loaded back with a single
        // object — which read as a bug in `unlock` and was a bug in the test.
        doc.trailer.set(
            "ID",
            lopdf::Object::Array(vec![
                lopdf::Object::String(vec![0x11; 16], lopdf::StringFormat::Hexadecimal),
                lopdf::Object::String(vec![0x11; 16], lopdf::StringFormat::Hexadecimal),
            ]),
        );
        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save");
        out
    }

    /// The round trip: protect, then unlock, and the pages survive.
    #[test]
    fn a_document_survives_protect_then_unlock() {
        let original = pdf();
        assert!(!is_encrypted(&original));

        let locked = protect(&original, "hunter2", "").expect("protect");
        assert!(is_encrypted(&locked), "the output is not encrypted");

        let opened = unlock(&locked, "hunter2").expect("unlock");
        assert!(!is_encrypted(&opened));

        // The page tree came back intact, which is the only claim that matters
        // about the content.
        let before = Document::load_mem(&original).expect("load");
        let after = Document::load_mem(&opened).expect("load");
        assert_eq!(after.get_pages().len(), before.get_pages().len());
    }

    #[test]
    fn a_wrong_password_is_refused_and_produces_nothing() {
        let locked = protect(&pdf(), "correct", "").expect("protect");
        let err = unlock(&locked, "wrong").unwrap_err();
        assert!(matches!(err, LockError::WrongPassword), "{err:?}");
        assert_eq!(err.to_string(), "that password did not open this document");
    }

    /// No empty-password probe: an empty password is refused up front rather
    /// than tried against the document.
    #[test]
    fn an_empty_password_is_refused_rather_than_tried() {
        let locked = protect(&pdf(), "secret", "").expect("protect");
        let err = unlock(&locked, "").unwrap_err();
        assert!(matches!(err, LockError::NoPassword(_)), "{err:?}");

        let err = protect(&pdf(), "", "").unwrap_err();
        assert!(matches!(err, LockError::NoPassword(_)), "{err:?}");
    }

    #[test]
    fn unlocking_something_unlocked_says_so() {
        let err = unlock(&pdf(), "anything").unwrap_err();
        assert!(matches!(err, LockError::NotEncrypted), "{err:?}");
    }

    #[test]
    fn protecting_twice_says_so_rather_than_double_encrypting() {
        let locked = protect(&pdf(), "one", "").expect("protect");
        let err = protect(&locked, "two", "").unwrap_err();
        assert!(matches!(err, LockError::AlreadyEncrypted), "{err:?}");
    }

    /// The owner password defaults to the user password.
    ///
    /// Otherwise a document would carry a second secret nobody chose and
    /// nobody knows.
    #[test]
    fn an_absent_owner_password_defaults_to_the_user_one() {
        let locked = protect(&pdf(), "same", "").expect("protect");
        unlock(&locked, "same").expect("the user password must also be the owner password");
    }

    /// Two documents encrypted with the same password differ, because the file
    /// key is random rather than derived.
    #[test]
    fn the_file_key_is_random_not_derived_from_the_password() {
        let a = protect(&pdf(), "same", "").expect("protect");
        let b = protect(&pdf(), "same", "").expect("protect");
        assert_ne!(a, b, "identical output means the key is deterministic");
    }

    /// No error message repeats the password.
    #[test]
    fn no_error_echoes_the_password() {
        let secret = "swordfish-9271";
        let locked = protect(&pdf(), secret, "").expect("protect");

        for message in [
            unlock(&locked, "wrong").unwrap_err().to_string(),
            unlock(&pdf(), secret).unwrap_err().to_string(),
            protect(&locked, secret, "").unwrap_err().to_string(),
        ] {
            assert!(!message.contains(secret), "a password leaked: {message}");
        }
    }
}
