//! Immutable column batches of interim facts (C18; docs/final-shape/03-architecture.md §4, §10.2),
//! adapted from the data layer's projection batches (`violations/projection.rs`): a builder fills
//! one column per field and seals into boxed columns that never grow, and memory is counted from
//! the sealed columns. Here every cell is a u32 (an atom, a count, or one lane of a digest), a run
//! holds one relation, and sealing sorts the rows by their key columns, keeping the last row
//! pushed for a key, so a run is looked up by binary search and merged into SQLite in key order.
//! A run encodes to a flat little-endian file when the overlay spills it.

use std::cmp::Ordering;
use std::io::{self, Read, Write};

const MAGIC: &[u8; 8] = b"rrlrun01";

/// One relation's rows: `arity` columns of u32, the first `key` of them the key, sorted, unique.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Run {
    relation: u32,
    key: usize,
    rows: usize,
    columns: Box<[Box<[u32]>]>,
}

/// Rows pushed in any order; `seal` makes the run.
pub(crate) struct Builder {
    relation: u32,
    key: usize,
    columns: Vec<Vec<u32>>,
}

impl Builder {
    pub(crate) fn new(relation: u32, arity: usize, key: usize) -> Self {
        assert!(key >= 1 && key <= arity, "a run's key is a non-empty prefix of its columns");
        Self { relation, key, columns: (0..arity).map(|_| Vec::new()).collect() }
    }

    pub(crate) fn push(&mut self, row: &[u32]) {
        assert_eq!(row.len(), self.columns.len(), "row arity");
        for (column, value) in self.columns.iter_mut().zip(row) {
            column.push(*value);
        }
    }

    pub(crate) fn rows(&self) -> usize {
        self.columns[0].len()
    }

    /// Sorts by key (stable, so the last row pushed for a key is the last of its equal run) and
    /// keeps one row per key.
    pub(crate) fn seal(self) -> Run {
        let rows = self.rows();
        let mut order = (0..rows).collect::<Vec<_>>();
        let key = |row: usize| self.columns[..self.key].iter().map(move |c| c[row]);
        order.sort_by(|&a, &b| key(a).cmp(key(b)));
        let mut kept = Vec::with_capacity(rows);
        for (index, &row) in order.iter().enumerate() {
            if order.get(index + 1).is_some_and(|&next| key(next).eq(key(row))) {
                continue;
            }
            kept.push(row);
        }
        // Published columns never grow: each is boxed at exactly its length.
        let columns = self
            .columns
            .iter()
            .map(|column| kept.iter().map(|&row| column[row]).collect::<Box<[u32]>>())
            .collect();
        Run { relation: self.relation, key: self.key, rows: kept.len(), columns }
    }
}

impl Run {
    pub(crate) fn relation(&self) -> u32 {
        self.relation
    }

    pub(crate) fn len(&self) -> usize {
        self.rows
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub(crate) fn cell(&self, row: usize, column: usize) -> u32 {
        self.columns[column][row]
    }

    /// The row's cells.
    pub(crate) fn row(&self, row: usize) -> impl Iterator<Item = u32> + '_ {
        self.columns.iter().map(move |column| column[row])
    }

    /// The row whose key starts with `prefix` (shorter than or equal to the key), first of them.
    pub(crate) fn lower_bound(&self, prefix: &[u32]) -> usize {
        let (mut low, mut high) = (0, self.rows);
        while low < high {
            let middle = (low + high) / 2;
            if self.compare(middle, prefix) == Ordering::Less {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    /// The rows whose key starts with `prefix`.
    pub(crate) fn range(&self, prefix: &[u32]) -> std::ops::Range<usize> {
        let start = self.lower_bound(prefix);
        let mut end = start;
        while end < self.rows && self.compare(end, prefix) == Ordering::Equal {
            end += 1;
        }
        start..end
    }

    /// The row whose whole key is `key`.
    pub(crate) fn find(&self, key: &[u32]) -> Option<usize> {
        assert_eq!(key.len(), self.key, "a whole key");
        let row = self.lower_bound(key);
        (row < self.rows && self.compare(row, key) == Ordering::Equal).then_some(row)
    }

    fn compare(&self, row: usize, prefix: &[u32]) -> Ordering {
        self.columns[..prefix.len()].iter().map(|c| c[row]).cmp(prefix.iter().copied())
    }

    /// Heap bytes the run holds.
    pub(crate) fn bytes(&self) -> usize {
        self.columns.len() * std::mem::size_of::<Box<[u32]>>() + self.columns.len() * self.rows * 4
    }

    /// Flat encoding: magic, relation, key, arity, rows (u32 each), then each column.
    pub(crate) fn encode(&self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(MAGIC)?;
        for value in [self.relation, self.key as u32, self.columns.len() as u32, self.rows as u32] {
            out.write_all(&value.to_le_bytes())?;
        }
        for column in self.columns.iter() {
            let bytes = column.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
            out.write_all(&bytes)?;
        }
        Ok(())
    }

    pub(crate) fn decode(input: &mut impl Read) -> io::Result<Self> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("run file: {what}"));
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("bad magic"));
        }
        let mut word = || -> io::Result<u32> {
            let mut bytes = [0; 4];
            input.read_exact(&mut bytes)?;
            Ok(u32::from_le_bytes(bytes))
        };
        let (relation, key, arity, rows) = (word()?, word()? as usize, word()? as usize, word()? as usize);
        if key == 0 || key > arity {
            return Err(invalid("bad key"));
        }
        let mut columns = Vec::with_capacity(arity);
        for _ in 0..arity {
            let mut bytes = vec![0; rows * 4];
            input.read_exact(&mut bytes)?;
            columns.push(bytes.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect());
        }
        Ok(Run { relation, key, rows, columns: columns.into() })
    }
}

/// A digest as eight u32 cells.
pub(crate) fn lanes(digest: &[u8; 32]) -> [u32; 8] {
    let mut lanes = [0; 8];
    for (lane, chunk) in lanes.iter_mut().zip(digest.chunks_exact(4)) {
        *lane = u32::from_le_bytes(chunk.try_into().expect("four bytes"));
    }
    lanes
}

/// The digest of eight u32 cells.
pub(crate) fn digest(lanes: impl IntoIterator<Item = u32>) -> [u8; 32] {
    let mut digest = [0; 32];
    for (chunk, lane) in digest.chunks_exact_mut(4).zip(lanes) {
        chunk.copy_from_slice(&lane.to_le_bytes());
    }
    digest
}

#[cfg(test)]
mod tests;
