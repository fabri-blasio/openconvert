//! The handle table -- where the one open handle per input actually lives.
//!
//! `openconvert-core` carries an `InputToken`; this resolves it. I13's guarantee
//! ("the bytes routed on are the bytes converted") comes from the pairing, not
//! from the field's location: one open, and every later read -- header check,
//! decode, hash -- goes through it. That defeats replace-by-rename, which is
//! the realistic attack.

use openconvert_core::facts::InputToken;
use std::collections::HashMap;
use std::fs::File;

/// Owns every open input handle for a job.
#[derive(Debug, Default)]
pub struct HandleTable {
    next: u64,
    open: HashMap<u64, File>,
}

impl HandleTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Take ownership of an opened file and mint its token.
    pub fn insert(&mut self, file: File) -> InputToken {
        let id = self.next;
        self.next += 1;
        self.open.insert(id, file);
        InputToken::new(id)
    }

    /// Resolve a token. Returns `None` for a token this table did not mint,
    /// which is what makes a forged token useless rather than dangerous.
    #[must_use]
    pub fn get(&self, token: InputToken) -> Option<&File> {
        self.open.get(&token.get())
    }

    /// Resolve a token for reading.
    ///
    /// `&mut` because reading moves the file cursor. Handing out a shared
    /// reference would let two callers interleave reads on one descriptor and
    /// each see half a file.
    pub fn get_mut(&mut self, token: InputToken) -> Option<&mut File> {
        self.open.get_mut(&token.get())
    }

    /// How many handles are open. For the batch budget, and for tests
    /// asserting that a finished job released what it held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// Whether the table holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}
