//! Programs and arguments, as closed types.
//!
//! I7: **shell injection is unrepresentable.** Not "escaped correctly", not
//! "sanitised" — there is no value of any type in this crate that names a
//! program by string, so there is nothing for an attacker to inject *into*.
//!
//! This closes a bug family rather than a bug. The ImageMagick command-injection
//! CVEs and the filename-format-string class both start from the same place: a
//! filename reaching a program name or a shell.

use crate::broker::OutputName;
use core::fmt;

/// The closed set of programs we may execute.
///
/// # Why a closed enum and not a path
///
/// A closed enum has no `Other(String)`, no `From<&str>`, and `sh` is not a
/// variant. Widening it is a cross-crate diff a reviewer sees, and a source
/// scan catches the alternative route (`Command::new` outside `openconvert-os`).
/// `03` §7.4 classes that combination **Impossible** — the one claim in this
/// crate that earns the word.
///
/// The four workers are ours. We build them, sign them, and confine them; we do
/// not depend on stock upstream CLIs, because that would mean shipping ten
/// third-party executables we do not control and parsing their human-readable
/// output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EngineBin {
    /// libvips · libheif · libavif · libjxl · libraw · lcms2.
    Images,
    /// pdfium · qpdf.
    Pdf,
    /// libarchive.
    Archive,
    /// LAME · libopus · libFLAC. Symphonia decode stays in-process.
    Audio,
    /// ONNX Runtime. The only worker chosen by what the STEP does rather than
    /// by what the file is: a PNG going through background removal is still a
    /// PNG, and every format-based rule would have sent it to `Images`.
    Ai,
}

impl EngineBin {
    /// Every variant, for exhaustive tests over the set.
    pub const ALL: &'static [Self] = &[
        Self::Images,
        Self::Pdf,
        Self::Archive,
        Self::Audio,
        Self::Ai,
    ];

    /// The worker that handles a media kind, if one does.
    ///
    /// `Tabular` has no worker and never will: CSV, TSV and JSON are parsed by
    /// pure Rust, so routing them to a subprocess would buy isolation from a
    /// parser that does not need isolating and cost a process launch per file.
    /// `None` is therefore an answer, not a gap — and `route()` treats it as a
    /// refusal rather than falling back to in-process, because a fallback is
    /// how a sandboxed decision quietly becomes an unsandboxed one.
    #[must_use]
    pub const fn for_media(kind: openconvert_core::format::MediaKind) -> Option<Self> {
        use openconvert_core::format::MediaKind as K;
        match kind {
            K::Image => Some(Self::Images),
            // A WORKBOOK GOES TO THE DOCUMENT WORKER, because a `.xlsx` and a
            // `.ods` are ZIP-of-XML in exactly the shape `office.rs` already
            // scans, and `oc-pdf` is where that scanner lives. The KIND is
            // separate from `Document` for labelling, not for routing: a
            // spreadsheet is not a document, and calling it one in
            // `openconvert formats` would be the small dishonesty this table
            // exists to avoid.
            K::Document | K::Spreadsheet => Some(Self::Pdf),
            // FONTS GO TO oc-archive, and the reason is not "somewhere had to
            // take them".
            //
            // WOFF and WOFF2 are compressed containers wrapping the same SFNT
            // tables -- zlib per table, Brotli over all of them -- so the
            // operation is unpack-and-repack, which is this worker's whole
            // shape. Its SR-5 caps land on exactly the right surface too: a
            // small Brotli stream expanding to gigabytes is the same bomb an
            // archive can be, and `archive_total_bytes` already counts it as
            // the total arrives.
            //
            // The alternatives were a fifth binary to sign, stage and confine
            // for a job measured in kilobytes, or oc-pdf -- which is the
            // typographic worker and would have been the intuitive answer,
            // except that it links pdfium and its conversion path is
            // `#[cfg(windows)]`, so fonts would have been a Windows feature.
            K::Archive | K::Font => Some(Self::Archive),
            K::Audio => Some(Self::Audio),
            // Video's Class A work -- remux, stream copy, lossless trim, audio
            // extraction -- is `02` §3.1 and runs on pure-Rust container
            // crates, in process, so it needs no worker. Class B transcoding
            // needs the Video module, which is a separately downloaded FFmpeg
            // and not one of these four binaries.
            //
            // An earlier version of this comment said video was out of scope
            // for v1. It is not: `01` §154 makes `MKV -> MP4` bit-identical the
            // flagship demo. The routing was right and the reason was wrong,
            // which is the kind of note that becomes a wrong decision later.
            K::Video | K::Tabular => None,
        }
    }

    /// The worker for a whole conversion, which is not always the source's.
    ///
    /// [`Self::for_media`] answers "who parses this kind of file", and that is
    /// the right question for every route where one worker owns both sides.
    /// **Audio extraction is the exception**: `mkv -> mp3` reads a VIDEO
    /// container and writes an AUDIO file, and the worker that can do the
    /// second is the one that must do the first.
    ///
    /// So an audio destination always means `oc-audio`. That is not a special
    /// case bolted on -- it is the same rule stated at conversion granularity:
    /// oc-audio is the only worker that WRITES audio, and symphonia reads
    /// every container we route audio out of (Matroska and WebM among them,
    /// through the `mkv` feature that was enabled for exactly this and then
    /// never reached by a route).
    ///
    /// Everything else still resolves from the source, because for everything
    /// else the source is what decides which parser is exposed.
    #[must_use]
    pub fn for_conversion(
        from: openconvert_core::format::FormatId,
        to: openconvert_core::format::FormatId,
    ) -> Option<Self> {
        use openconvert_core::format::MediaKind as K;
        if to.kind() == Some(K::Audio) {
            return Some(Self::Audio);
        }
        from.kind().and_then(Self::for_media)
    }

    /// The executable name, without extension.
    ///
    /// Resolved against our own installation directory by `openconvert-os`, never
    /// against `PATH` — a `PATH` lookup is a name resolved by the environment,
    /// which is the environment choosing our program for us.
    #[must_use]
    pub const fn file_stem(self) -> &'static str {
        match self {
            Self::Images => "oc-images",
            Self::Pdf => "oc-pdf",
            Self::Archive => "oc-archive",
            Self::Audio => "oc-audio",
            Self::Ai => "oc-ai",
        }
    }
}

impl fmt::Display for EngineBin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_stem())
    }
}

/// One argument. A closed set of shapes, not a string.
///
/// Every variant is something we constructed or something that already survived
/// a parse. There is deliberately no `Raw(String)`: the moment one exists,
/// every other variant becomes decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// A literal flag we wrote, e.g. `--quality`.
    ///
    /// `&'static str` so it cannot come from input. A flag assembled at runtime
    /// is a flag an attacker can influence.
    Flag(&'static str),
    /// A number.
    Number(i64),
    /// An output name that has already been parsed.
    Name(OutputName),
    /// A file descriptor index the child will find already open.
    ///
    /// Not a path. The worker never receives a path — inputs arrive as
    /// pre-opened descriptors, which is what closes the filename-as-attack
    /// vector more completely than renaming ever did.
    Fd(u8),
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag(s) => f.write_str(s),
            Self::Number(n) => write!(f, "{n}"),
            Self::Name(n) => f.write_str(n.as_str()),
            Self::Fd(i) => write!(f, "&{i}"),
        }
    }
}

/// A program and its arguments. Private fields; built only by [`Argv::new`].
///
/// There is no way to obtain an `Argv` that names a shell, and no way to append
/// an unvalidated string to one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argv {
    program: EngineBin,
    args: Vec<Arg>,
}

impl Argv {
    /// Build an argument vector.
    #[must_use]
    pub fn new(program: EngineBin, args: Vec<Arg>) -> Self {
        Self { program, args }
    }

    /// Which program.
    #[must_use]
    pub const fn program(&self) -> EngineBin {
        self.program
    }

    /// The arguments, in order.
    #[must_use]
    pub fn args(&self) -> &[Arg] {
        &self.args
    }

    /// Render for the plan preview and the receipt.
    ///
    /// **Display only.** Nothing parses this back — it exists so a user can see
    /// what will run, and a round-trip through a string is exactly the step
    /// this type exists to remove.
    #[must_use]
    pub fn to_display_string(&self) -> String {
        let mut out = self.program.file_stem().to_string();
        for a in &self.args {
            out.push(' ');
            out.push_str(&a.to_string());
        }
        out
    }
}

impl fmt::Display for Argv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

#[cfg(test)]
mod worker_coverage {
    use super::EngineBin;
    use openconvert_core::format::MediaKind;

    /// `MediaKind::has_worker` and `EngineBin::for_media` must agree.
    ///
    /// The fact is duplicated because it cannot be shared: `route()` needs it
    /// to decide isolation and lives in `openconvert-core`, which this crate
    /// depends on rather than the reverse. Duplication is only safe while
    /// something fails when the two diverge — a new media kind wired into one
    /// and not the other would let a policy force a step into a worker that
    /// does not exist, producing a plan that cannot run.
    #[test]
    fn has_worker_matches_the_engine_map() {
        for kind in MediaKind::ALL {
            assert_eq!(
                EngineBin::for_media(*kind).is_some(),
                kind.has_worker(),
                "{kind:?}: EngineBin::for_media and MediaKind::has_worker disagree"
            );
        }
    }

    /// The predicate the forced-sandbox policy gates on must be the predicate
    /// execution uses to pick an engine — over every route, not most of them.
    ///
    /// `for_conversion` keys on the **input** kind, apart from audio outputs.
    /// A rule gated on the output kind agrees with it on nearly every row of
    /// the table and disagrees exactly where the two kinds differ, which is
    /// where it does damage: forcing a step into a worker that will not be
    /// found, so a route that worked before the setting was switched on fails
    /// with `NoWorker` after it. Checking a couple of representative pairs
    /// would not have caught that; the whole table does.
    #[test]
    fn conversion_worker_predicate_matches_the_engine_choice() {
        use openconvert_core::route::RouteTable;

        for row in RouteTable::v1().all() {
            assert_eq!(
                EngineBin::for_conversion(row.from, row.to).is_some(),
                MediaKind::conversion_has_worker(row.from, row.to),
                "{} -> {}: policy and execution disagree about whether a worker exists",
                row.from,
                row.to
            );
        }
    }
}
