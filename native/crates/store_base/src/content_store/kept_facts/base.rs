//! A put of a space's kept facts, as the keep's writer reads it (rerail-store-base): its relation's key, digest, reads
//! and column types, and what it is (`KeptWhat`): a derived relation's rows lent by its holder (`KeptRows`), rows read
//! where they lie (`KeptPicked`, `BlockFlat`), a documents' table, a retag, or gone. It reads no `ContentStore`: the
//! observer (rerail-observe) makes its puts with it, and the store (rerail-store, `kept_facts.rs`) writes them and
//! re-exports it.
use rerail_blocks::kept::FactKey;
use rerail_blocks::packed::RowsIn;
use rerail_blocks::relation::Relation;

/// A relation the keep puts, lent by its holder (the engine's run): its rows, dead bits and levels, kept alive while
/// this lives. Nothing writes them meanwhile.
pub trait KeptRows: Send + Sync {
    fn relation(&self) -> &Relation;
    fn dead(&self) -> &[u64];
    fn levels(&self) -> &[u32];
}

/// Rows a put reads where they lie (an input's granule, read from the input in place; a view's rows, read from its
/// source; rows written in a block of their own, `BlockFlat`), which the writer's packer reads by row and column as it
/// writes them (`RowsIn`, decision 0353 step 8): nothing is filled into columns or a scratch first. `order`: the
/// order they are kept in (`packed::Order`): as given, every row (read by position); a relation's, sorted on column
/// 0 alone; or a store table's, on every column, as its readers merge them.
pub trait KeptPicked: RowsIn + Send {
    fn order(&self) -> rerail_blocks::packed::Order {
        rerail_blocks::packed::Order::AsGiven
    }
    /// Each row's level (decision 0090), where its rows have them.
    fn levels(&self) -> Option<std::borrow::Cow<'_, [u32]>> {
        None
    }
    /// The bytes it holds until written (a block of its own), not what it lends.
    fn held(&self) -> usize {
        0
    }
}

/// Rows written in a block of their own, never on the heap (blocks are never copied, decision 0353): `n` rows of
/// `width` 8-byte values, column-major, as the writer reads them (`KeptPicked`). An entry's small rows (a documents'
/// table, statistics' words, a digest's entry) are made here as they are found.
pub struct BlockFlat {
    block: rerail_blocks::block::Block,
    at: usize,
    n: usize,
    width: usize,
    levels: Option<usize>,
    order: rerail_blocks::packed::Order,
}

impl BlockFlat {
    /// `n` rows of `width` columns, every value 0, kept as given (`given`) or as a relation's rows (`packed::Order::Column`:
    /// sorted on column 0, or on another column the rows already lie in order on, decision 0678 step 8).
    pub fn new(n: usize, width: usize, given: bool) -> BlockFlat {
        Self::with_levels(n, width, given, false)
    }

    /// `new`, with each row's level (0 until `put_level`) where `levels`.
    pub fn with_levels(n: usize, width: usize, given: bool, levels: bool) -> BlockFlat {
        let values = n * width * 8;
        let level_bytes = if levels { n * 4 } else { 0 };
        let block = rerail_blocks::block::Block::anon(rerail_blocks::block::kind::FACTS, values + level_bytes + 4096);
        let at = block.alloc(values.max(8), 8);
        let levels = levels.then(|| block.alloc(level_bytes.max(4), 4));
        use rerail_blocks::packed::Order;
        BlockFlat { block, at, n, width, levels, order: if given { Order::AsGiven } else { Order::Column } }
    }

    /// Kept sorted on column 0 whatever order they come in (`packed::Order::Declared`): a table its readers search
    /// on column 0, as a documents' table (`Docs::kept`).
    pub fn on_column_0(mut self) -> BlockFlat {
        self.order = rerail_blocks::packed::Order::Declared;
        self
    }

    /// Kept sorted on every column (`packed::Order::Rows`): a store table its readers merge on leading columns
    /// (`sorted_put`).
    pub fn sorted_on_every_column(mut self) -> BlockFlat {
        self.order = rerail_blocks::packed::Order::Rows;
        self
    }

    /// One row of these values, kept as given.
    pub fn of_row(row: &[i64]) -> BlockFlat {
        Self::of_rows(row, row.len(), true)
    }

    /// `bytes` as one column of words, kept as given: their length, then the bytes, 8 a word, little-endian, the
    /// last word's high bytes zero.
    pub fn of_bytes(bytes: &[u8]) -> BlockFlat {
        let mut f = Self::new(1 + bytes.len().div_ceil(8), 1, true);
        f.put(0, 0, bytes.len() as i64);
        for (i, c) in bytes.chunks(8).enumerate() {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            f.put(1 + i, 0, i64::from_le_bytes(w));
        }
        f
    }

    /// One row of values, as given, row-major: `rows.len() / width` rows.
    pub fn of_rows(rows: &[i64], width: usize, given: bool) -> BlockFlat {
        let n = rows.len().checked_div(width).unwrap_or(0);
        let mut f = Self::new(n, width, given);
        for (r, row) in rows.chunks_exact(width.max(1)).take(n).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                f.put(r, c, v);
            }
        }
        f
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn cols(&self) -> &[i64] {
        // SAFETY: `n * width` values allocated at `at`, written only through `&mut self`.
        unsafe { std::slice::from_raw_parts(self.block.ptr::<i64>(self.at), self.n * self.width) }
    }

    fn cols_mut(&mut self) -> &mut [i64] {
        // SAFETY: as `cols`; `&mut self` is the only writer.
        unsafe { std::slice::from_raw_parts_mut(self.block.ptr::<i64>(self.at), self.n * self.width) }
    }

    fn level_words(&self) -> Option<&[u32]> {
        // SAFETY: `n` levels allocated at `l`.
        self.levels.map(|l| unsafe { std::slice::from_raw_parts(self.block.ptr::<u32>(l), self.n) })
    }

    pub fn put(&mut self, r: usize, c: usize, v: i64) {
        let n = self.n;
        self.cols_mut()[c * n + r] = v;
    }

    pub fn get(&self, r: usize, c: usize) -> i64 {
        self.cols()[c * self.n + r]
    }

    /// A writer of its values row by row, each row's in column order, from row 0 on (`FlatWriter`).
    pub fn writer(&mut self) -> FlatWriter<'_> {
        FlatWriter { flat: self, at: 0 }
    }

    pub fn put_level(&mut self, r: usize, level: u32) {
        if let Some(l) = self.levels {
            // SAFETY: `n` levels allocated at `l`, `r` below `n` (checked).
            assert!(r < self.n);
            unsafe { *self.block.ptr::<u32>(l).add(r) = level };
        }
    }
}

/// Values written into a `BlockFlat` row by row (`BlockFlat::writer`): the `k`-th value pushed is row `k / width`'s
/// column `k % width`.
pub struct FlatWriter<'a> {
    flat: &'a mut BlockFlat,
    at: usize,
}

impl FlatWriter<'_> {
    pub fn push(&mut self, v: i64) {
        let w = self.flat.width;
        self.flat.put(self.at / w, self.at % w, v);
        self.at += 1;
    }

    pub fn extend(&mut self, vs: impl IntoIterator<Item = i64>) {
        for v in vs {
            self.push(v);
        }
    }

    /// The values written: `rows * width` when every row is whole.
    pub fn written(&self) -> usize {
        self.at
    }
}

impl RowsIn for BlockFlat {
    fn n(&self) -> usize {
        self.n
    }
    fn arity(&self) -> usize {
        self.width
    }
    #[inline]
    fn get(&self, r: usize, c: usize) -> i64 {
        BlockFlat::get(self, r, c)
    }
}

impl KeptPicked for BlockFlat {
    fn order(&self) -> rerail_blocks::packed::Order {
        self.order
    }
    fn levels(&self) -> Option<std::borrow::Cow<'_, [u32]>> {
        self.level_words().map(std::borrow::Cow::Borrowed)
    }
    fn held(&self) -> usize {
        self.n * (self.width * 8 + if self.levels.is_some() { 4 } else { 0 })
    }
}

/// What a put is.
pub enum KeptWhat {
    /// A relation the run derived: its live rows, packed.
    Rows(Box<dyn KeptRows>),
    /// Rows read where they lie (`KeptPicked`): an input's granule, a view's rows, rows in a block of their own.
    Picked(Box<dyn KeptPicked>),
    /// A documents' table (`document_places`): its rows as the run gives them (`DOCUMENT_WORDS` each), kept in that
    /// order with each document's places, which the writer finds as it writes them (decision 0494 step 4 part 2).
    /// `tape`: the grammar its tapes are found in (grammar id, grammar version, tape format version).
    Documents { rows: BlockFlat, tape: Option<[i64; 3]> },
    /// The kept entry's rows as they are, with these reads (a frame's retag).
    Reads,
    /// No longer kept.
    Gone,
}

/// One put of a space's next block.
pub struct KeptPut {
    pub rel: String,
    pub ident: [u8; 32],
    pub part: i64,
    pub digest: u64,
    pub reads: Vec<(String, u64)>,
    /// Each column's value type (the engine's codes: `s` an atom).
    pub types: Vec<u8>,
    pub what: KeptWhat,
}

impl KeptPut {
    pub fn key(&self) -> FactKey<'_> {
        FactKey { rel: &self.rel, ident: &self.ident, part: self.part }
    }
}
