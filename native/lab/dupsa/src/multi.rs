//! MLDB's suffix array adapters (`plugins/tabular/suffix_array.h` on mldbai/mldb branch `tabular-string-analysis`):
//! `MultiSuffixArray` over many documents, and the walk of `countPrefixes`, ported over symbols instead of bytes.
//!
//! One departure from MLDB: MLDB concatenates the strings into one (`str.append(s)` with a NUL after each). Here the
//! concatenation is virtual: position `i` is mapped to (document, offset) by the document starts (MLDB's `offsets`,
//! searched as its `offsetToStringIndex` does), and each document's symbol is read from the document where it lies.
//! Nothing holds the documents' symbols a second time.

use crate::sais::{self, Text};

/// Documents of symbols, each read where it lies.
pub trait Docs {
    fn count(&self) -> usize;
    fn doc_len(&self, d: usize) -> usize;
    /// Symbol `j` of document `d`, in `0..=upper()`.
    fn at(&self, d: usize, j: usize) -> u32;
    fn upper(&self) -> u32;
}

/// The documents as one text: each followed by a separator, symbol 0 (the documents' symbols are shifted up by one).
struct Joined<'a, D: Docs + ?Sized> {
    docs: &'a D,
    /// `offsets[d]`: where document `d` starts in the joined text; `offsets[count]` its length.
    offsets: &'a [u32],
}

impl<D: Docs + ?Sized> Joined<'_, D> {
    #[inline(always)]
    fn locate(&self, i: usize) -> (usize, usize) {
        // MLDB's `offsetToStringIndex`: the last start at or before `i`.
        let d = self.offsets.partition_point(|&o| o as usize <= i) - 1;
        (d, i - self.offsets[d] as usize)
    }
}

impl<D: Docs + ?Sized> Text for Joined<'_, D> {
    fn len(&self) -> usize {
        *self.offsets.last().unwrap() as usize
    }
    #[inline(always)]
    fn at(&self, i: usize) -> u32 {
        let (d, j) = self.locate(i);
        if j == self.docs.doc_len(d) { 0 } else { self.docs.at(d, j) + 1 }
    }
}

/// A suffix array over many documents (MLDB's `MultiSuffixArray`): every suffix that starts in a document (those that
/// start at a separator are skipped, as MLDB skips its NULs), in sorted order, with the common prefix of each and the
/// one before it, never past either's document end.
pub struct MultiSuffixArray {
    pub offsets: Vec<u32>,
    /// (document, offset in it), in suffix order: MLDB's `entries`.
    pub entries: Vec<(u32, u32)>,
    /// `lcp[r]`: symbols entries `r - 1` and `r` share, within their documents (`lcp[0]` = 0).
    pub lcp: Vec<u32>,
}

impl MultiSuffixArray {
    pub fn new<D: Docs + ?Sized>(docs: &D) -> Self {
        let mut offsets = Vec::with_capacity(docs.count() + 1);
        let mut at = 0u64;
        offsets.push(0);
        for d in 0..docs.count() {
            at += docs.doc_len(d) as u64 + 1;
            offsets.push(u32::try_from(at).expect("documents of 2^32 symbols or more"));
        }
        let joined = Joined { docs, offsets: &offsets };
        let sa = sais::suffix_array(&joined, docs.upper() + 1);
        let raw = sais::lcp(&joined, &sa);
        let mut entries = Vec::with_capacity(sa.len() - docs.count());
        let mut lcp = Vec::with_capacity(sa.len() - docs.count());
        // The common prefix of two kept entries is the least of the raw LCPs between them (the separators' suffixes
        // skipped between them sort first in their bucket, symbol 0, so they are never between two kept entries of a
        // shared prefix longer than 0).
        let mut run = u32::MAX;
        for (r, &p) in sa.iter().enumerate() {
            run = run.min(raw[r]);
            let (d, j) = joined.locate(p as usize);
            if j == docs.doc_len(d) {
                continue;
            }
            // Never past the document's end: a shared prefix that reaches a separator reaches it in both suffixes at
            // the same offset, so the end of either bounds it.
            let left = (docs.doc_len(d) - j) as u32;
            lcp.push(if entries.is_empty() { 0 } else { run.min(left) });
            entries.push((d as u32, j as u32));
            run = u32::MAX;
        }
        Self { offsets, entries, lcp }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Each entry's place in `entries`, by document and offset: `rank[offsets[d] + j]` (separators' slots unused).
    pub fn ranks(&self) -> Vec<u32> {
        let mut rank = vec![u32::MAX; *self.offsets.last().unwrap() as usize];
        for (r, &(d, j)) in self.entries.iter().enumerate() {
            rank[self.offsets[d as usize] as usize + j as usize] = r as u32;
        }
        rank
    }

    /// MLDB's `countPrefixes`: every repeated prefix of at least `min` symbols that is a maximal LCP interval, as
    /// (length, first entry, entries). A stack of the open intervals' starts, closed as the common prefix shrinks.
    pub fn repeats(&self, min: u32, mut f: impl FnMut(u32, usize, usize)) {
        // (length, start entry) of each open interval, lengths increasing.
        let mut open: Vec<(u32, usize)> = Vec::new();
        for r in 1..=self.entries.len() {
            let h = if r < self.entries.len() { self.lcp[r] } else { 0 };
            let mut start = r - 1;
            while let Some(&(len, s)) = open.last() {
                if len <= h {
                    break;
                }
                open.pop();
                if len >= min {
                    f(len, s, r - s);
                }
                start = s;
            }
            if h > 0 && open.last().is_none_or(|&(len, _)| len < h) {
                open.push((h, start));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Strs(Vec<Vec<u32>>);

    impl Docs for Strs {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn doc_len(&self, d: usize) -> usize {
            self.0[d].len()
        }
        fn at(&self, d: usize, j: usize) -> u32 {
            self.0[d][j]
        }
        fn upper(&self) -> u32 {
            self.0.iter().flatten().copied().max().unwrap_or(0)
        }
    }

    fn of(words: &[&str]) -> Strs {
        Strs(words.iter().map(|w| w.bytes().map(u32::from).collect()).collect())
    }

    /// MLDB's `validateMultiSuffixArray`: every suffix no smaller than the one before; and each LCP as compared.
    fn validate(docs: &Strs) {
        let a = MultiSuffixArray::new(docs);
        assert_eq!(a.len(), docs.0.iter().map(Vec::len).sum::<usize>());
        let suffix = |r: usize| {
            let (d, j) = a.entries[r];
            &docs.0[d as usize][j as usize..]
        };
        for r in 1..a.len() {
            let (x, y) = (suffix(r - 1), suffix(r));
            assert!(x <= y, "{x:?} > {y:?}");
            let h = x.iter().zip(y).take_while(|(p, q)| p == q).count();
            assert_eq!(a.lcp[r] as usize, h);
        }
    }

    #[test]
    fn null_one_and_many() {
        validate(&of(&[]));
        validate(&of(&["1"]));
        validate(&of(&["", "", "a"]));
        validate(&of(&["banana", "ananas", "bandana", "nab", "ana"]));
        validate(&of(&["aaaa", "aaa", "aaaaa", "a"]));
    }

    #[test]
    fn repeats_are_the_lcp_intervals() {
        let docs = of(&["xabcdy", "zabcdw", "abcq"]);
        let a = MultiSuffixArray::new(&docs);
        let mut got = Vec::new();
        a.repeats(3, |len, _, n| got.push((len, n)));
        got.sort();
        // "abcd" twice, "abc" three times, "bcd" twice.
        assert_eq!(got, vec![(3, 2), (3, 3), (4, 2)]);
    }
}
