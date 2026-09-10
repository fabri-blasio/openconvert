//! The browser executor — **detect and plan, in the page**.
//!
//! # What this is for
//!
//! `openconvert.dev` says the plan is shown before the conversion runs, and that
//! the decision is made by a pure function. Both claims are checkable only if
//! the visitor can watch it happen on their own file. So the homepage drops the
//! first few hundred bytes of a dropped file into *this*, which is
//! `openconvert-core` compiled to `wasm32` and nothing else, and renders what it
//! answers.
//!
//! The page and the desktop app therefore route from **one implementation**. A
//! marketing site with its own reimplementation of the route table would drift
//! within a release, and nobody would notice until someone compared them.
//!
//! # What it deliberately does not do
//!
//! It does not convert. `openconvert-run` is subprocesses and OS sandboxes and
//! does not compile to `wasm32` — by design, not by accident. Decoding in the
//! page needs a thin in-page executor over the pure-Rust engines, which is a
//! separate artifact with a separate size budget. This one answers *what would
//! happen*, which is the claim that needed proving.
//!
//! # The profile it reports
//!
//! `BrowserOrigin` filesystem, `Csp` network, no privilege drop, no syscall
//! filter. `SandboxProfile::strength()` caps that at **`Reduced`**, never
//! `Full`, because the page origin confines the page and not one library from
//! the rest of the page's memory. The site prints `Reduced`. A marketing
//! surface that flattered its own isolation would undermine the one claim the
//! whole site exists to make.
//!
//! # ABI
//!
//! Four exports, a linear-memory buffer, and no `wasm-bindgen`:
//!
//! ```text
//! alloc(len)              -> ptr        caller writes `len` bytes there
//! plan(ptr, len, target)  -> ptr        NUL-terminated JSON in linear memory
//! last_len()              -> len        length of that JSON, without the NUL
//! dealloc(ptr, len)                     hand the buffer back
//! ```

#![allow(clippy::missing_safety_doc)]

use core::fmt::Write as _;
use openconvert_core::environment::{EngineEntry, Environment};
use openconvert_core::facts::Properties;
use openconvert_core::format::{sniff_window, FormatId, TABLE};
use openconvert_core::isolation::{
    FsConfinement, Isolation, NetConfinement, PrivDrop, ResourceEnforcement, SandboxProfile,
    SyscallFilter,
};
use openconvert_core::plan::{Class, PlanRequest, StepKind};
use openconvert_core::policy::Policy;
use openconvert_core::route::{route, RouteTable};
use openconvert_core::sniff::sniff_bytes;
use openconvert_core::target::Target;

// `panic = "abort"` in the release profile is what handles a panic here: the
// instance traps and the page reports a trap rather than reading a silently
// wrong answer out of linear memory. No `#[panic_handler]` of our own —
// this links std, which already provides one, and a second is a duplicate lang
// item.
//
// Nothing here should panic in any case: `sniff_bytes` is total over `&[u8]`
// and `route` is total over its inputs. The abort is the backstop, not the
// plan.

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Reserve `len` bytes for the caller to write a file head into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// Give a buffer back. The page calls this after reading the JSON out.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

/// Length of the JSON from the last [`plan`] call, excluding its NUL.
///
/// A second export rather than a length prefix: the page reads the bytes with
/// a `Uint8Array` view and decoding a prefix in JS is more glue than one call.
static mut LAST_LEN: usize = 0;

#[no_mangle]
pub extern "C" fn last_len() -> usize {
    unsafe { LAST_LEN }
}

/// How many bytes of a file the page needs to send.
///
/// Derived from the table rather than guessed, so a format with a deeper
/// signature cannot leave the page reading too little. `tar`'s `ustar` at
/// offset 257 is why this is not the 8 or 16 bytes one might assume, and
/// separating a DNG from the TIFF it is built on -- an IFD0 walk for the
/// `DNGVersion` tag -- is why it is now larger still.
///
/// This is `sniff_window`, the same number `openconvert-run::detect` reads, so
/// the page and the desktop app cannot disagree about what a file is. That
/// divergence would be very easy to ship and very hard to notice, which is
/// the whole reason this crate shares the core's sniffer instead of its own.
#[no_mangle]
pub extern "C" fn head_bytes() -> usize {
    sniff_window()
}

/// What a FILENAME claims, as a format id — for **mismatch detection only**.
///
/// Separate from [`plan`] on purpose. SR-4 keeps the extension's claim and the
/// content's verdict apart, and a single call returning both would invite a
/// caller to treat them as one answer. The page asks this second, compares, and
/// says so when they disagree.
///
/// Returns the empty string when the extension names no known format, which is
/// the ordinary case for a file with no extension at all.
///
/// # Safety
///
/// `ptr` must point at `len` initialised UTF-8 bytes from [`alloc`].
#[no_mangle]
pub unsafe extern "C" fn declared(ptr: *const u8, len: usize) -> *const u8 {
    let name = core::str::from_utf8(core::slice::from_raw_parts(ptr, len)).unwrap_or("");
    let id = openconvert_core::sniff::from_extension(name);

    let mut bytes = id
        .map_or_else(String::new, |f| id_str(f).to_string())
        .into_bytes();
    bytes.push(0);
    LAST_LEN = bytes.len() - 1;
    let out = bytes.as_ptr();
    core::mem::forget(bytes);
    out
}

// ---------------------------------------------------------------------------
// The one interesting export
// ---------------------------------------------------------------------------

/// Identify the bytes at `ptr[..len]`, plan a conversion to `target`, and
/// return the answer as JSON.
///
/// `target` is a [`FormatId`] discriminant, or `u32::MAX` to mean "identify
/// only, and list what this could convert to".
///
/// # Safety
///
/// `ptr` must point at `len` initialised bytes from [`alloc`].
#[no_mangle]
pub unsafe extern "C" fn plan(ptr: *const u8, len: usize, target: u32) -> *const u8 {
    let head = core::slice::from_raw_parts(ptr, len);
    let json = plan_json(head, target);

    let mut bytes = json.into_bytes();
    bytes.push(0);
    LAST_LEN = bytes.len() - 1;
    let out = bytes.as_ptr();
    core::mem::forget(bytes);
    out
}

fn plan_json(head: &[u8], target: u32) -> String {
    let sniff = sniff_bytes(head);
    let detected = sniff.detected;

    let mut s = String::with_capacity(512);
    s.push('{');
    let _ = write!(s, "\"detected\":\"{}\"", id_str(detected));
    let _ = write!(s, ",\"polyglot\":{}", sniff.polyglot);
    let _ = write!(s, ",\"head_bytes\":{}", head.len());

    // Every target this input can reach, from the route table itself. The page
    // offers these and nothing else, so it cannot advertise a conversion the
    // binary lacks any more than the format pages can.
    s.push_str(",\"targets\":[");
    let mut first = true;
    for (i, f) in TABLE.iter().enumerate() {
        if f.id == detected {
            continue;
        }
        let p = build(detected, f.id);
        if p.is_executable() {
            if !first {
                s.push(',');
            }
            first = false;
            // The TABLE index rides along so the page can ask for a full plan
            // on this target without JS holding its own copy of the enum. One
            // less place for the page and the core to disagree.
            let _ = write!(s, "{{\"i\":{i},\"id\":\"{}\"", id_str(f.id));
            let _ = write!(s, ",\"class\":\"{}\"}}", p.class().map_or("?", class_str));
        }
    }
    s.push(']');

    if target != u32::MAX {
        if let Some(to) = from_index(target) {
            let p = build(detected, to);
            s.push_str(",\"plan\":{");
            let _ = write!(s, "\"target\":\"{}\"", id_str(to));
            let _ = write!(s, ",\"executable\":{}", p.is_executable());
            let _ = write!(s, ",\"class\":\"{}\"", p.class().map_or("-", class_str));
            s.push_str(",\"steps\":[");
            for (i, st) in p.steps().iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let _ = write!(s, "{{\"kind\":{}", quote(&kind_str(st.kind)));
                let _ = write!(s, ",\"class\":\"{}\"", class_str(st.class));
                let _ = write!(s, ",\"isolation\":{}", quote(&iso_str(st.isolation)));
                let _ = write!(
                    s,
                    ",\"limits\":{{\"memory_bytes\":{},\"decode_pixels\":{},\"wall_time_secs\":{}}}}}",
                    st.limits.memory_bytes,
                    st.limits.decode_pixels,
                    st.limits.wall_time.as_secs()
                );
            }
            s.push_str("]}");
        }
    }

    s.push('}');
    s
}

// ---------------------------------------------------------------------------
// The environment the page routes in
// ---------------------------------------------------------------------------

/// Route one pair under the browser's own profile.
fn build(from: FormatId, to: FormatId) -> openconvert_core::plan::Plan {
    route(
        PlanRequest {
            input: from,
            target: Target::Format(to),
            polyglot: false,
        },
        Properties::None,
        &Policy::default(),
        &browser_environment(),
    )
}

/// What a browser actually offers, stated without flattery.
///
/// This is the `attest` call the crate exists to make, and every argument is a
/// read-back rather than a request:
///
/// * `BrowserOrigin` — the page origin confines the page. It does not confine
///   `image-rs` from the rest of the page's memory, so it is not `Landlock`.
/// * `Csp` — `connect-src 'none'` is a real network denial and the site ships
///   it on every page. It is enforced by the browser, not by us.
/// * no `SyscallFilter`, no `ResourceEnforcement`, no `PrivDrop` — a page has
///   none of them. `Rlimit` in a browser is a memory ceiling, not a wall clock.
///
/// `strength()` reads that as `Reduced`. It is never `Full`, and the page says
/// so.
fn browser_environment() -> Environment {
    let profile = SandboxProfile::attest(
        FsConfinement::BrowserOrigin,
        NetConfinement::Csp,
        SyscallFilter::None,
        ResourceEnforcement::None,
        PrivDrop::None,
    );

    // ONLY `image-rs`. The four worker binaries are subprocesses under an OS
    // sandbox and have no counterpart in a page, so they are absent rather than
    // listed-and-unavailable — and `route()` refuses every route that requires
    // one, on its own, from the same table the CLI uses.
    //
    // That is why the page offers fewer targets than the desktop app, and why
    // the shorter list is the correct one: `available` has to mean "a plan
    // routed here finishes", and a page cannot finish a HEIC decode.
    let engines = vec![EngineEntry {
        name: "image-rs",
        memory_safe: true,
        available: true,
    }];

    Environment::new(profile, engines, RouteTable::v1())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// A JSON string literal. Escapes the two characters that matter plus the
/// control range; the inputs are `&'static str` from our own tables, and it is
/// still written correctly because a serialiser that is only safe for trusted
/// input stops being safe the day someone points it elsewhere.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The stable identifier, the one the CLI prints and the site's routes use.
///  IS that identifier — a short lowercase word, not a display
/// title — so the page's route ids match /formats/<id> without a mapping table.
fn id_str(f: FormatId) -> &'static str {
    f.row().map_or("unknown", |r| r.name)
}

fn from_index(i: u32) -> Option<FormatId> {
    TABLE.get(i as usize).map(|f| f.id)
}

const fn class_str(c: Class) -> &'static str {
    match c {
        Class::A => "A",
        Class::B => "B",
        Class::C => "C",
        Class::D => "D",
    }
}

fn kind_str(k: StepKind) -> String {
    match k {
        StepKind::StreamCopy => "StreamCopy".into(),
        StepKind::Transcode { from, to } => {
            format!("Decode {} → Encode {}", id_str(from), id_str(to))
        }
        StepKind::StripMetadata => "StripMetadata".into(),
        StepKind::Extract { .. } => "Extract".into(),
        StepKind::Trim { start_ms, end_ms } => format!("Trim {start_ms}..{end_ms} ms"),
        StepKind::RenderPage { page } => format!("RenderPage {page}"),
        StepKind::Pixel { op } => format!("Pixel {op}"),
        // The model is named, not summarised as "AI". A step the user cannot
        // identify is a step they cannot check.
        StepKind::Infer { models, task, to } => {
            format!("{task} via {} → {}", models.join(" + "), id_str(to))
        }
    }
}

fn iso_str(i: Isolation) -> String {
    match i {
        Isolation::InProcess => "in-page (pure Rust)".into(),
        Isolation::Sandboxed(p) => format!(
            "sandboxed · {} — browser origin, network denied by CSP",
            p.strength()
        ),
    }
}
