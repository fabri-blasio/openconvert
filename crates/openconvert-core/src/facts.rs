//! What we know about an input file, and nothing more.
//!
//! `FileFacts` carries an [`InputToken`], never a file handle. v0.5 put an
//! `InputHandle` here; that is a live OS resource inside the crate this design
//! declares has no I/O and tests with no files (spike S1). `openconvert-run` owns
//! the `HandleTable` that resolves a token to the one open handle.
//!
//! I13 survives the move: the guarantee comes from the pairing of token and
//! table, not from where the field sits.

/// Opaque handle to an input the shell has opened. Resolves only in
/// `openconvert-run`'s `HandleTable`.
///
/// Deliberately not `Deserialize`: a token that arrived over the wire would
/// address a handle the sender does not own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputToken(u64);

impl InputToken {
    /// Mint a token. Only the shell should call this; it is `pub` because the
    /// shell is another crate, and Rust cannot express "only that one".
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// The raw id, for table lookup.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Where a file came from, and therefore how much we trust it.
///
/// Captured once at `detect()` time and **never re-derived from a copy** --
/// the mandated `create_new` copy path strips `Zone.Identifier` (spike S26),
/// so a re-read after copying would report a downloaded file as local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provenance {
    /// Local, no mark-of-the-web, no quarantine attribute, fixed volume.
    Trusted,
    /// Mark-of-the-web, quarantine xattr, network volume, or removable volume.
    ///
    /// Detection **fails open** -- a file that reached the disk by a route that
    /// strips the zone identifier looks `Trusted`. Stated in `09` section 8
    /// rather than implied away.
    Untrusted,
}

impl Provenance {
    /// Whether a file with this provenance may share a worker process with
    /// another file. SR-20.
    #[must_use]
    pub const fn may_share_worker(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// What phase 1 detection concluded, from bytes alone.
///
/// Phase 1 is pure Rust, in-process and fuzzed. It reads the first
/// [`crate::format::max_magic_reach`] bytes and decides identity — which is
/// enough to choose an isolation level, and that is the only thing it needs to
/// be enough for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniff {
    /// What the content says it is.
    pub detected: crate::format::FormatId,
    /// What the filename claimed, if there was one.
    pub declared: Option<crate::format::FormatId>,
    /// More than one format's signature matched.
    ///
    /// A polyglot is quarantined and the user is asked. It is **not** resolved
    /// by preference order: a file that is validly two things is a file whose
    /// author wanted two different programs to disagree about it.
    pub polyglot: bool,
}

impl Sniff {
    /// Whether the extension lied.
    ///
    /// Surfaced always — in the receipt, in the plan preview, and in the
    /// file-type icon, which is drawn from the *detected* type. SR-4.
    #[must_use]
    pub fn mismatched(&self) -> bool {
        self.declared.is_some_and(|d| d != self.detected)
    }
}

/// Format-specific facts, extracted in phase 2.
///
/// An enum per [`crate::format::MediaKind`] rather than one flat struct: adding
/// a format inside a kind touches no shared type, and adding a kind touches
/// exactly one.
///
/// **Everything in here arrives from a confined, possibly compromised engine**,
/// which is why it may only ever narrow a limit — see [`crate::wire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Properties {
    /// Raster or vector image.
    Image {
        /// Pixel width.
        width: u32,
        /// Pixel height.
        height: u32,
        /// Whether an alpha channel is present.
        has_alpha: bool,
        /// Frame count; > 1 for animation.
        frames: u32,
    },
    /// Audio stream.
    Audio {
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Channel count.
        channels: u8,
        /// Sample rate in Hz.
        sample_rate: u32,
    },
    /// Video container.
    Video {
        /// Duration in milliseconds, or **0 for "not determined"**.
        ///
        /// The container header reader stops at the track list. A duration
        /// needs timescale arithmetic each container spells differently, it is
        /// easy to get subtly wrong, and nothing consumes it yet -- so it is
        /// reported as undetermined rather than guessed, the same convention
        /// `width` and `frames` use.
        duration_ms: u64,
        /// Pixel width.
        width: u32,
        /// Pixel height.
        height: u32,
        /// The video codec, as read from the container header.
        ///
        /// **This replaced `stream_copyable: bool`.** That field was documented
        /// as "the single most consequential boolean an engine reports", and it
        /// could not be reported: a stream is copyable *into a particular
        /// container*, and phase-2 detection runs before a destination is
        /// chosen. `probe()` takes a file, `route()` takes a target — the
        /// boolean was asked of the half of the system that cannot know.
        ///
        /// An engine now reports what it read, and
        /// [`crate::codec::streams_carryable`] decides, beside the route table
        /// that already knows the destination.
        video: Option<crate::codec::VideoCodec>,
        /// The audio codec, as read from the container header.
        audio: Option<crate::codec::AudioCodec>,
    },
    /// Paged or flowed document.
    Document {
        /// Page count where the format has pages.
        pages: u32,
        /// Whether the document is encrypted.
        encrypted: bool,
    },
    /// Container of other files.
    Archive {
        /// Entry count.
        entries: u32,
        /// Deepest nesting observed.
        depth: u8,
        /// Total uncompressed size, if the container declares one.
        declared_total_bytes: Option<u64>,
    },
    /// Row-and-column data.
    Tabular {
        /// Row count.
        rows: u64,
        /// Column count.
        columns: u32,
    },
    /// Nothing format-specific was extracted.
    ///
    /// Not an error. A format with no phase-2 parser, or an input we could not
    /// identify, both land here and both must still route.
    None,
}

/// Everything known about one input.
///
/// Private fields, and only `detect()` — in `openconvert-run` — constructs one.
/// That is I1: nothing can invent facts about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    token: InputToken,
    content_id: [u8; 32],
    sniff: Sniff,
    props: Properties,
    provenance: Provenance,
}

impl FileFacts {
    /// Construct. Called by `detect()` and by tests.
    ///
    /// **This is `pub` and I1 says it should not be.** Rust cannot express
    /// "only `openconvert-run::detect` may call this" across a crate boundary, so
    /// the honest position is that I1 is enforced by private *fields* — nothing
    /// can be modified after construction, and nothing can be built from parts
    /// picked up elsewhere — plus a review rule on this one function. `03` §7.4
    /// classes that kind of claim **Hard**, not Impossible, and says so.
    #[must_use]
    pub const fn new(
        token: InputToken,
        content_id: [u8; 32],
        sniff: Sniff,
        props: Properties,
        provenance: Provenance,
    ) -> Self {
        Self {
            token,
            content_id,
            sniff,
            props,
            provenance,
        }
    }

    /// The token addressing this input's one open handle.
    #[must_use]
    pub const fn token(&self) -> InputToken {
        self.token
    }

    /// Hash of the bytes actually read, computed once through that handle.
    ///
    /// The receipt's subject. Not a fourth read of the path.
    #[must_use]
    pub const fn content_id(&self) -> &[u8; 32] {
        &self.content_id
    }

    /// Phase 1 detection result.
    #[must_use]
    pub const fn sniff(&self) -> Sniff {
        self.sniff
    }

    /// Phase 2 format-specific facts.
    #[must_use]
    pub const fn properties(&self) -> Properties {
        self.props
    }

    /// Where the file came from, captured before any copy existed.
    #[must_use]
    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }
}
