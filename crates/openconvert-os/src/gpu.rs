//! Which graphics card this machine has, by name.
//!
//! # Why the app needs to say it out loud
//!
//! The GPU setting was a switch and two sentences of prose about what a
//! graphics card does. Every word was true in general and none of it was about
//! *this* machine, so the one question a user actually has -- "is the card I
//! paid for being used" -- was the one thing the screen did not answer. A name
//! answers it in three words.
//!
//! # Why the registry and not DXGI
//!
//! `EnumAdapters1` is the API for this and it is a COM interface.
//! `windows-sys` carries no COM (0.61 dropped what little it had), so using it
//! would mean adding the full `windows` crate to a tree that has deliberately
//! kept to raw bindings -- a large dependency, a new licence row, and a
//! vtable-walking `unsafe` block, all to read a string.
//!
//! Windows publishes the same string under the display-adapter setup class,
//! `{4d36e968-e325-11ce-bfc1-08002be10318}`: one numbered subkey per adapter,
//! `DriverDesc` holding the vendor's own name and
//! `HardwareInformation.qwMemorySize` holding its dedicated memory. That is
//! four registry calls against an API already in the manifest.
//!
//! What it costs: the registry describes what is INSTALLED, and DXGI describes
//! what is currently enumerable. A card that is disabled in Device Manager
//! still has its key. That distinction does not matter for a line of text
//! naming the hardware, and it would matter a great deal for anything that
//! decided where work runs -- which is why this decides nothing.
//!
//! # What this is not
//!
//! **It is not a capability check.** Whether ONNX Runtime's DirectML provider
//! will load, and whether a given model fits in a card's memory, are separate
//! questions answered elsewhere and later. A name here means "this machine has
//! this card installed", and the caller must not imply more.
//!
//! **It is not a fingerprint to send anywhere.** It is rendered in one line of
//! one settings panel on the machine it was read from.
//!
//! Everywhere but Windows this returns nothing, which the interface renders as
//! the switch with no name beside it -- the state the screen was already in.

/// A display adapter, as the operating system describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// The vendor's own string, e.g. `NVIDIA GeForce RTX 4050 Laptop GPU`.
    ///
    /// It comes from a driver, not from this program: render it as text.
    pub name: String,
    /// Dedicated video memory in bytes, or zero when the adapter reports none.
    /// Integrated graphics normally report none.
    pub video_memory_bytes: u64,
}

/// The adapters this machine has, most video memory first.
///
/// Empty rather than an error when there are none, when the registry cannot be
/// read, or when this is not Windows. All three lead to the same line on
/// screen, and a `Result` would push a decision with one answer onto every
/// caller.
#[must_use]
pub fn adapters() -> Vec<Adapter> {
    #[cfg(windows)]
    {
        windows_adapters()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// The adapter a GPU workload would most likely land on: the one with the most
/// dedicated memory, or the first listed if none report any.
#[must_use]
pub fn primary() -> Option<Adapter> {
    adapters().into_iter().next()
}

#[cfg(windows)]
mod imp {
    use super::Adapter;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ, RRF_RT_REG_QWORD, RRF_RT_REG_SZ,
    };

    /// The setup class every display adapter is registered under.
    const DISPLAY_CLASS: &str = concat!(
        r"SYSTEM\CurrentControlSet\Control\Class\",
        "{4d36e968-e325-11ce-bfc1-08002be10318}"
    );

    /// Adapters that are not graphics cards.
    ///
    /// Windows' own fallback driver enumerates here exactly like hardware
    /// does. Naming it would answer "which card do I have" with the name of
    /// the thing that runs when there is no card.
    const NOT_A_CARD: &[&str] = &[
        "Microsoft Basic Display Adapter",
        "Microsoft Basic Render Driver",
        "Microsoft Remote Display Adapter",
    ];

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A NUL-terminated UTF-16 buffer as a `String`, padding dropped.
    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end]).trim().to_string()
    }

    /// A string value under `key\subkey`.
    fn string_value(key: HKEY, subkey: &str, name: &str) -> Option<String> {
        let (sub, val) = (wide(subkey), wide(name));
        // 512 wide chars: driver descriptions are short, and a name longer
        // than this is one nobody would want rendered whole anyway. The call
        // reports the size it wanted, which is how truncation is detected.
        let mut buf = [0_u16; 512];
        let mut bytes = u32::try_from(std::mem::size_of_val(&buf)).ok()?;
        // SAFETY: `key` is an open key owned by the caller; `sub` and `val`
        // are NUL-terminated for the whole call; `buf` and `bytes` are live
        // locals, and `bytes` is initialised to the buffer's true size, which
        // is what bounds the write.
        let rc = unsafe {
            RegGetValueW(
                key,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let s = from_wide(&buf);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    /// A 64-bit value under `key\subkey`, or zero.
    fn qword_value(key: HKEY, subkey: &str, name: &str) -> u64 {
        let (sub, val) = (wide(subkey), wide(name));
        let mut out = 0_u64;
        let mut bytes = 8_u32;
        // SAFETY: as above; `out` is a live 8-byte local and `bytes` says so.
        let rc = unsafe {
            RegGetValueW(
                key,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_QWORD,
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(out).cast(),
                &mut bytes,
            )
        };
        if rc == ERROR_SUCCESS {
            out
        } else {
            0
        }
    }

    pub(super) fn adapters() -> Vec<Adapter> {
        let path = wide(DISPLAY_CLASS);
        let mut key: HKEY = std::ptr::null_mut::<std::ffi::c_void>() as HANDLE;
        // SAFETY: `path` is NUL-terminated and lives across the call; `key` is
        // a live local written only on success.
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                path.as_ptr(),
                0,
                KEY_READ,
                std::ptr::addr_of_mut!(key),
            )
        };
        if rc != ERROR_SUCCESS {
            return Vec::new();
        }

        let mut found = Vec::new();
        let mut index = 0_u32;
        loop {
            let mut name = [0_u16; 256];
            let mut len = u32::try_from(name.len()).unwrap_or(0);
            // SAFETY: `key` is open; `name`/`len` are live and `len` is the
            // buffer's length in characters, which is what bounds the write.
            // Anything other than success ends the enumeration.
            let rc = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            index += 1;

            // ONLY THE NUMBERED SUBKEYS ARE ADAPTERS. The class key also holds
            // `Configuration`, `Properties` and per-vendor keys, none of which
            // are a card.
            let sub = from_wide(&name[..len as usize]);
            if sub.len() != 4 || !sub.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }

            let Some(desc) = string_value(key, &sub, "DriverDesc") else {
                continue;
            };
            if NOT_A_CARD.iter().any(|n| desc.eq_ignore_ascii_case(n)) {
                continue;
            }
            found.push(Adapter {
                video_memory_bytes: qword_value(key, &sub, "HardwareInformation.qwMemorySize"),
                name: desc,
            });
        }

        // SAFETY: closed exactly once, after the last use of `key`.
        unsafe { RegCloseKey(key) };

        // Most dedicated memory first: on a laptop with both, that is the
        // discrete card -- the one a GPU workload wants, and the one the user
        // means by "my graphics card".
        found.sort_by_key(|a| std::cmp::Reverse(a.video_memory_bytes));
        found.dedup();
        found
    }
}

#[cfg(windows)]
use imp::adapters as windows_adapters;

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever it finds, the answer has to be SHOWABLE.
    ///
    /// This cannot assert a name: a CI machine has no card, and the machine
    /// that has one has a different card from the next. What it can assert is
    /// that nothing here produces a row the interface would render badly -- an
    /// empty name, or one still carrying the NUL padding out of the fixed
    /// buffer. Both were real possibilities in the parsing above.
    #[test]
    fn every_adapter_named_is_one_that_can_be_shown() {
        for a in adapters() {
            assert!(!a.name.is_empty(), "an adapter was listed with no name");
            assert!(
                !a.name.contains('\0'),
                "the fixed-width buffer's padding survived into {:?}",
                a.name
            );
            assert_eq!(a.name.trim(), a.name, "untrimmed name {:?}", a.name);
        }
    }

    /// `primary` is the head of the same list, not a second enumeration.
    #[test]
    fn the_primary_adapter_is_the_first_listed() {
        assert_eq!(primary(), adapters().into_iter().next());
    }

    /// Sorted by dedicated memory, descending.
    #[test]
    fn the_discrete_card_comes_first() {
        for pair in adapters().windows(2) {
            assert!(
                pair[0].video_memory_bytes >= pair[1].video_memory_bytes,
                "{pair:?} is out of order"
            );
        }
    }

    /// Reading it twice gives the same answer.
    ///
    /// Enumeration walks an index that the OS is free to renumber; the sort
    /// is what makes the result stable, and without it the settings screen
    /// could name a different card on each visit.
    #[test]
    fn the_answer_does_not_move() {
        assert_eq!(adapters(), adapters());
    }
}
