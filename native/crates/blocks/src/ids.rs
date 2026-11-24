//! Branded indexes (decision 0678): a row's id, a column's index and a word of liveness bits are each their own
//! type, so one is never passed where another is meant. Each is its integer underneath (`repr(transparent)`), so a
//! read costs what the integer's did.

/// A row's id in a relation: its place among the rows a state reads, `0 .. Read::end()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RowId(u32);

impl RowId {
    pub const ZERO: RowId = RowId(0);

    #[inline]
    pub const fn new(id: u32) -> RowId {
        RowId(id)
    }

    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The word of liveness bits this row's bit is in.
    #[inline]
    pub const fn word(self) -> WordIndex {
        WordIndex(self.0 >> 6)
    }

    /// Its bit in its word.
    #[inline]
    pub const fn bit(self) -> u32 {
        self.0 & 63
    }
}

impl std::fmt::Display for RowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A column of a relation's rows, `0 .. arity`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ColumnIndex(u16);

impl ColumnIndex {
    #[inline]
    pub const fn new(c: u16) -> ColumnIndex {
        ColumnIndex(c)
    }

    /// Its place, to index a row's values by.
    #[inline]
    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// Every column of `arity`, in order.
    pub fn all(arity: usize) -> impl Iterator<Item = ColumnIndex> {
        let arity = u16::try_from(arity).expect("a relation's columns fit a u16");
        (0..arity).map(ColumnIndex)
    }
}

/// A word of liveness bits: the rows `64 * w .. 64 * w + 64`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct WordIndex(u32);

impl WordIndex {
    #[inline]
    pub const fn new(w: u32) -> WordIndex {
        WordIndex(w)
    }

    /// Its place, to index a slice of words by.
    #[inline]
    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// Its first row.
    #[inline]
    pub const fn first(self) -> RowId {
        RowId(self.0 << 6)
    }

    /// Every word holding a bit of rows `0 .. end`, in order.
    pub fn upto(end: RowId) -> impl Iterator<Item = WordIndex> {
        (0..end.0.div_ceil(64)).map(WordIndex)
    }
}
