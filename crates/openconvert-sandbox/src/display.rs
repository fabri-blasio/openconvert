//! `DisplayName` — the only way an untrusted string reaches a screen.
//!
//! # The attack, concretely
//!
//! An archive contains a member named `invoice\u{202E}fdp.exe`. U+202E is RIGHT
//! TO LEFT OVERRIDE, and every terminal and every webview honours it, so the
//! user reads **`invoiceexe.pdf`** and approves extracting a document. They get
//! an executable. Nothing was forged: the name really is what it says, and the
//! renderer really is doing what the standard requires.
//!
//! `09` §3 names *"a crafted filename rendered into the UI"* as a concrete A15
//! example. Until this module existed it had **no control at all** — the CLI
//! printed member names, engine messages and detected formats straight into a
//! terminal that interprets escape sequences.
//!
//! # Why this is not the broker's job
//!
//! [`crate::broker::OutputName`] **refuses** a hostile name, and refusing is
//! right when the name is going to become a file. It is wrong here. A name only
//! being *shown* must be shown — telling the user "this archive contains a file
//! I will not describe" is worse than describing it safely, because the whole
//! point is to let them decide. So this transforms rather than validates, and
//! it is infallible by construction: there is no input for which the right
//! answer is silence.
//!
//! # What is escaped
//!
//! Anything that moves the cursor, ends a line, starts a control sequence,
//! reorders text, or occupies no space:
//!
//! | Range | Why |
//! |---|---|
//! | C0 (`U+0000`–`U+001F`), DEL | `ESC` starts CSI/OSC; `\r` rewrites the line just printed; `\n` fabricates output the program never produced |
//! | C1 (`U+0080`–`U+009F`) | single-byte CSI (`U+009B`) and OSC (`U+009D`) — the same attack without an `ESC` to grep for |
//! | `U+200E`, `U+200F`, `U+202A`–`U+202E`, `U+2066`–`U+2069` | bidi marks, embeddings, overrides and isolates: the reordering class |
//! | `U+200B`, `U+2060`, `U+FEFF` | zero-width: two names that render identically are two names the user cannot tell apart |
//!
//! # The one ambiguity, and why it is the harmless direction
//!
//! A name containing the literal seven characters `\u{202E}` renders the same
//! as an escaped RLO. The usual fix is to escape the escape — `\` becomes `\\`
//! — and it is **not** applied here, because these strings are frequently
//! Windows paths and doubling every separator makes the common case unreadable
//! to protect against a spoof that hides nothing. A name pretending to be an
//! escape sequence looks *more* suspicious, not less. The dangerous direction
//! is a control character rendering as nothing, and that one is closed.

use core::fmt;

/// How many characters of output a single name may occupy.
///
/// A 4000-character member name is not a rendering problem, it is a denial of
/// the surrounding output — it scrolls the warning above it off the screen.
///
/// # Why the middle is elided rather than the end
///
/// The first version cut the tail, and running the CLI against a real hostile
/// filename in a real temp directory produced this:
///
/// ```text
/// C:/Users/blasi/AppData/Local/Temp/claude/C--Users-blasi-Desktop-Proj…
/// ```
///
/// The path was long enough on its own that **the filename never appeared** —
/// so the single most important part of the string, and the part carrying the
/// attack, was the part thrown away. A control that hides what it was built to
/// show is worse than none, because it looks like it is working.
///
/// Characters are escaped into atomic pieces before any of this, so a cut can
/// never land inside an escape and produce a misleading fragment.
pub const MAX_DISPLAY_CHARS: usize = 120;

/// Characters kept from the start when a name is elided.
///
/// The remainder goes to the tail, because on a path the tail is the filename.
const HEAD_CHARS: usize = 60;

/// How much of an untrusted SENTENCE is kept. See [`DisplayName::sentence`].
///
/// Larger than [`MAX_DISPLAY_CHARS`] because a sentence carries its meaning
/// across its whole length, where a path carries most of it in the last
/// component. Still bounded: the string is untrusted, and an engine that
/// returns a megabyte of text must not put a megabyte of text on screen.
pub const MAX_SENTENCE_CHARS: usize = 400;

/// An untrusted string, safe to render.
///
/// Constructed from anything; there is no failing path. The type exists so that
/// "was this string sanitised?" is answered by its type rather than by whether
/// someone remembered — the same reason `OutputName` exists one module over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName {
    rendered: String,
    /// Whether anything was escaped or cut.
    ///
    /// Surfaced so a caller can say *why* a name looks odd. A user shown
    /// `invoice\u{202e}fdp.exe` with no explanation concludes the tool is
    /// broken; one told the name contains a text-direction override concludes
    /// the file is.
    altered: bool,
}

impl DisplayName {
    /// Render an untrusted **sentence** safely: escaped, and cut at the END.
    ///
    /// # Why a second constructor rather than one shape for everything
    ///
    /// [`new`](Self::new) elides the MIDDLE, which is right for a path — the
    /// tail is the filename, and it is the part worth keeping. Applied to
    /// prose it destroys the sentence:
    ///
    /// ```text
    /// This recording does not sound like any of the 99 languages t…ich is
    /// not close enough to transcribe. Nothing was written.
    /// ```
    ///
    /// That is a real message this produced. Both halves are readable and the
    /// join is nonsense, so the user is told something they cannot act on by
    /// a control whose whole purpose is being readable.
    ///
    /// A sentence is read front to back, so what is kept is the front. The
    /// escaping is identical and non-negotiable: a worker's message is
    /// untrusted text, and it is the terminal-escape and bidi-override
    /// handling that makes it safe to print at all.
    #[must_use]
    pub fn sentence(raw: &str) -> Self {
        let (pieces, altered) = Self::escape(raw);
        let total: usize = pieces.iter().map(|p| p.chars().count()).sum();
        if total <= MAX_SENTENCE_CHARS {
            return Self {
                rendered: pieces.concat(),
                altered,
            };
        }
        let mut rendered = String::new();
        let mut used = 0usize;
        for p in &pieces {
            let w = p.chars().count();
            // The ellipsis has to fit too, or the result is one char over.
            if used + w > MAX_SENTENCE_CHARS - 1 {
                break;
            }
            rendered.push_str(p);
            used += w;
        }
        rendered.push('…');
        Self {
            rendered,
            altered: true,
        }
    }

    /// Every source character as one atomic piece — itself, or its escape.
    ///
    /// Shared so that no length decision anywhere can land inside an escape
    /// and produce a misleading fragment.
    fn escape(raw: &str) -> (Vec<String>, bool) {
        let mut altered = false;
        let pieces: Vec<String> = raw
            .chars()
            .map(|c| {
                if needs_escape(c) {
                    altered = true;
                    format!("\\u{{{:04x}}}", c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect();
        (pieces, altered)
    }

    /// Render an untrusted string safely.
    #[must_use]
    pub fn new(raw: &str) -> Self {
        // Each source character becomes one atomic piece -- itself, or its
        // escape. Every length decision below is made over pieces, which is why
        // no cut can land inside an escape.
        let (pieces, altered) = Self::escape(raw);

        let total: usize = pieces.iter().map(|p| p.chars().count()).sum();
        if total <= MAX_DISPLAY_CHARS {
            return Self {
                rendered: pieces.concat(),
                altered,
            };
        }

        // Head, ellipsis, tail. Taken from both ends so a long path keeps its
        // filename -- see MAX_DISPLAY_CHARS.
        let mut head = String::new();
        let mut used = 0usize;
        let mut i = 0usize;
        while i < pieces.len() {
            let w = pieces[i].chars().count();
            if used + w > HEAD_CHARS {
                break;
            }
            head.push_str(&pieces[i]);
            used += w;
            i += 1;
        }

        let tail_budget = MAX_DISPLAY_CHARS - used - 1; // the ellipsis
        let mut tail_rev: Vec<&str> = Vec::new();
        let mut used_tail = 0usize;
        for p in pieces[i..].iter().rev() {
            let w = p.chars().count();
            if used_tail + w > tail_budget {
                break;
            }
            tail_rev.push(p);
            used_tail += w;
        }
        tail_rev.reverse();

        let mut rendered = head;
        rendered.push('…');
        rendered.extend(tail_rev);

        Self {
            rendered,
            altered: true,
        }
    }

    /// Whether anything was escaped or truncated.
    #[must_use]
    pub const fn altered(&self) -> bool {
        self.altered
    }

    /// The safe rendering.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.rendered
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rendered)
    }
}

impl From<&str> for DisplayName {
    fn from(raw: &str) -> Self {
        Self::new(raw)
    }
}

/// Whether a character must not reach a renderer as itself.
const fn needs_escape(c: char) -> bool {
    matches!(c,
        // C0 and DEL. ESC (0x1B) is the one that starts CSI and OSC, but the
        // whole block goes: \r rewrites the line just printed, \n fabricates
        // output, \x08 backspaces over it.
        '\u{0}'..='\u{1f}' | '\u{7f}'
        // C1. U+009B is CSI and U+009D is OSC as single characters -- the same
        // attack with no ESC in the string to notice.
        | '\u{80}'..='\u{9f}'
        // The reordering class.
        | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        // Zero-width: two names that render identically are two names the user
        // cannot distinguish, which is the whole trick.
        | '\u{200b}' | '\u{2060}' | '\u{feff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The attack from the module header, defused.
    #[test]
    fn a_right_to_left_override_becomes_visible() {
        let d = DisplayName::new("invoice\u{202e}fdp.exe");
        assert_eq!(d.as_str(), "invoice\\u{202e}fdp.exe");
        assert!(d.altered());
        assert!(
            !d.as_str().contains('\u{202e}'),
            "the override survived into the output"
        );
    }

    /// An OSC sequence cannot retitle the user's terminal.
    ///
    /// `ESC ] 0 ; ... BEL` sets the window title. A name carrying one turns
    /// printing a filename into an action.
    #[test]
    fn an_osc_sequence_cannot_reach_the_terminal() {
        let d = DisplayName::new("\u{1b}]0;pwned\u{7}report.pdf");
        assert!(!d.as_str().contains('\u{1b}'));
        assert!(!d.as_str().contains('\u{7}'));
        assert!(d.as_str().contains("report.pdf"));
    }

    /// The single-character C1 forms are closed too.
    ///
    /// Escaping only `ESC` catches the sequence everyone writes and misses the
    /// one an attacker writes.
    #[test]
    fn the_c1_control_forms_are_escaped_as_well() {
        for c in ['\u{9b}', '\u{9d}', '\u{84}'] {
            let d = DisplayName::new(&format!("a{c}b"));
            assert!(!d.as_str().contains(c), "U+{:04X} survived", c as u32);
        }
    }

    /// **The control.** An ordinary name passes through byte for byte.
    ///
    /// Without this, every assertion above is satisfied by a function that
    /// returns the empty string.
    #[test]
    fn ordinary_names_are_untouched() {
        for name in [
            "report.pdf",
            "Ünicode is fine.txt",
            "日本語のファイル.png",
            "emoji 🎉 ok.jpg",
            "C:\\Users\\me\\Documents\\a b.docx",
            "hyphen-and_underscore.tar.gz",
        ] {
            let d = DisplayName::new(name);
            assert_eq!(d.as_str(), name, "{name} was altered");
            assert!(!d.altered(), "{name} was reported as altered");
        }
    }

    /// **`{}` prints the rendering.**
    ///
    /// Every caller uses `Display`, not `as_str` — `println!("{}", show(path))`
    /// in the CLI, `DisplayName::new(&msg).to_string()` at the worker boundary.
    /// A `fmt` that returned `Ok(())` without writing would make every path and
    /// every engine message print as **nothing**, and mutation testing found
    /// that no test here would have noticed: the assertions all read `as_str`,
    /// and the one caller-side test that goes through `Display` lives in
    /// another crate and skips when `oc-images` is not built.
    ///
    /// A test that can skip is not coverage for the thing it skips over.
    #[test]
    fn display_writes_the_rendering() {
        let d = DisplayName::new("invoice\u{202e}fdp.exe");
        assert_eq!(format!("{d}"), "invoice\\u{202e}fdp.exe");
        assert_eq!(d.to_string(), d.as_str());
        assert!(!format!("{d}").is_empty());
        assert_eq!(format!("[{}]", DisplayName::new("a.txt")), "[a.txt]");
    }

    /// A long name is cut, and the cut is reported.
    #[test]
    fn a_long_name_is_elided_and_says_so() {
        let d = DisplayName::new(&"a".repeat(1000));
        assert!(d.altered());
        assert!(d.as_str().chars().count() <= MAX_DISPLAY_CHARS);
        assert!(d.as_str().contains('…'));
    }

    /// **A long path keeps its filename.**
    ///
    /// The defect the first version had, found by running the CLI rather than
    /// by reading it: a deep temp directory pushed the name off the end, so the
    /// part carrying the attack was the part discarded. `09` §3's control was
    /// present, ran, and showed the user nothing.
    #[test]
    fn eliding_a_long_path_keeps_the_filename() {
        let deep = format!(
            "C:\\Users\\someone\\AppData\\Local\\Temp\\{}\\invoice\u{202e}fdp.exe",
            "nested\\".repeat(12)
        );
        let d = DisplayName::new(&deep);

        assert!(d.as_str().chars().count() <= MAX_DISPLAY_CHARS);
        assert!(d.as_str().contains('…'), "should be elided: {d}");
        assert!(
            d.as_str().ends_with("invoice\\u{202e}fdp.exe"),
            "the filename must survive: {d}"
        );
        assert!(
            d.as_str().starts_with("C:\\Users\\someone"),
            "the start should still orient the user: {d}"
        );
    }

    /// Eliding never splits an escape, from either end.
    #[test]
    fn eliding_never_cuts_an_escape_from_either_end() {
        for pad in 0..80 {
            // Overrides at both ends, so a naive head or tail cut lands inside
            // one of them.
            let raw = format!(
                "\u{202e}{}\u{202e}{}\u{202e}",
                "a".repeat(pad),
                "b".repeat(200)
            );
            let s = DisplayName::new(&raw).rendered;
            assert_eq!(
                s.matches("\\u{").count(),
                s.matches('}').count(),
                "unbalanced escape at pad {pad}: {s}"
            );
            assert!(
                !s.contains('\u{202e}'),
                "a live override survived at pad {pad}"
            );
        }
    }

    /// **A SENTENCE IS NOT ELIDED IN THE MIDDLE.**
    ///
    /// The real one, from `oc-ai` refusing a music file. Through `new` it came
    /// back as "...the 99 languages t...ich is not close enough..." -- two
    /// readable halves and a nonsense join, shown to someone who then had no
    /// idea why nothing had been written.
    #[test]
    fn a_message_keeps_its_beginning_and_reads_as_english() {
        let real = "This recording does not sound like any of the 99 languages this model \
                    knows. The closest was English at 28%, which is not close enough to \
                    transcribe. Nothing was written.";
        let d = DisplayName::sentence(real);
        assert_eq!(d.as_str(), real, "a message this length is not cut at all");
        assert!(!d.altered());

        // Long enough to be cut, and the cut is at the END.
        let long = "a".repeat(MAX_SENTENCE_CHARS + 50);
        let d = DisplayName::sentence(&long);
        assert!(d.as_str().starts_with("aaaa"));
        assert!(d.as_str().ends_with('\u{2026}'));
        assert_eq!(
            d.as_str().matches('\u{2026}').count(),
            1,
            "nothing is removed from the middle of a sentence"
        );
        assert!(d.as_str().chars().count() <= MAX_SENTENCE_CHARS);
    }

    /// The escaping is the same, and it is why this type exists at all.
    #[test]
    fn a_message_is_still_escaped() {
        let d = DisplayName::sentence("before\u{202e}after\u{1b}[31m");
        assert!(d.altered());
        assert!(!d.as_str().contains('\u{202e}'), "a bidi override survived");
        assert!(
            !d.as_str().contains('\u{1b}'),
            "an escape sequence survived"
        );
    }

    /// Truncation never splits an escape sequence.
    ///
    /// A name cut mid-escape would render `\u{20` — which is not a control
    /// character and not the truth either. The budget is counted before the
    /// escape is appended precisely so this cannot happen.
    #[test]
    fn truncation_never_cuts_an_escape_in_half() {
        // Padding chosen so the budget runs out partway through an escape if
        // the length were checked afterwards rather than before.
        for pad in 100..MAX_DISPLAY_CHARS {
            let raw = format!("{}\u{202e}tail", "a".repeat(pad));
            let out = DisplayName::new(&raw);
            let s = out.as_str();
            let opens = s.matches("\\u{").count();
            let closes = s.matches('}').count();
            assert_eq!(opens, closes, "unbalanced escape at pad {pad}: {s}");
        }
    }

    /// Every hostile name in the corpus renders without a live control
    /// character.
    ///
    /// The corpus is shared with the broker's tests, so this is the same 31
    /// inputs judged by a different rule: the broker asks *may this become a
    /// file*, and this asks *may this reach a screen*. They are different
    /// questions and both have to be answered.
    #[test]
    fn no_corpus_name_survives_with_a_live_control_character() {
        let mut escaped_something = 0usize;
        for case in corpus::evil::MUST_REJECT
            .iter()
            .chain(corpus::evil::MUST_COLLIDE_NOT_DESTROY)
        {
            let d = DisplayName::new(case.name);
            for c in d.as_str().chars() {
                assert!(
                    !needs_escape(c),
                    "U+{:04X} reached the output from {:?} ({})",
                    c as u32,
                    case.name,
                    case.why
                );
            }
            if d.altered() {
                escaped_something += 1;
            }
        }
        // The corpus is mostly path and device-name tricks, which are the
        // broker's problem and print harmlessly. Asserting a floor rather than
        // a total keeps this test honest about what it actually proves.
        assert!(
            escaped_something >= 3,
            "only {escaped_something} corpus names needed escaping -- \
             the corpus may have lost its invisible-character cases"
        );
    }

    /// The legitimate controls pass through untouched.
    #[test]
    fn no_legitimate_corpus_name_is_altered() {
        for name in corpus::evil::LEGITIMATE {
            let d = DisplayName::new(name);
            assert!(!d.altered(), "{name} should render as itself");
            assert_eq!(d.as_str(), *name);
        }
    }
}
