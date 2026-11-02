//! A wave's node space and its node sets (decision 0024 §1).
//!
//! The wave's files are concatenated: file `f`'s node `pre` is the space's node `base(f) + pre`.
//! Every node set of the wave (a prefilter plan's candidates, a label, a kind mask) is one value
//! over the space, so set algebra is word-parallel over every file at once, on the CPU or on
//! Metal, and a file's whole set or its complement is a few words, never a list of its nodes.
//!
//! Sets are dense bitmaps of `u64` words. Sparse sets (sorted indices, where `bound·32 < n`) and
//! per-file ranges are representations to come (0024 §1); the operations here are their contract.

/// Each file's first node in the space and its node count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct NodeSpace {
    bases: Vec<u32>,
    counts: Vec<u32>,
    len: u32,
}

impl NodeSpace {
    /// The space of files with `counts[f]` nodes each, in order.
    pub(in crate::code_rule_dsl) fn new(counts: Vec<u32>) -> Self {
        let mut bases = Vec::with_capacity(counts.len());
        let mut len = 0_u32;
        for &count in &counts {
            bases.push(len);
            len = len.checked_add(count).expect("a wave's nodes fit in u32");
        }
        Self { bases, counts, len }
    }

    pub(in crate::code_rule_dsl) fn files(&self) -> usize {
        self.counts.len()
    }

    pub(in crate::code_rule_dsl) fn len(&self) -> u32 {
        self.len
    }

    pub(in crate::code_rule_dsl) fn words(&self) -> usize {
        (self.len as usize).div_ceil(64)
    }

    pub(in crate::code_rule_dsl) fn base(&self, file: usize) -> u32 {
        self.bases[file]
    }

    pub(in crate::code_rule_dsl) fn count(&self, file: usize) -> u32 {
        self.counts[file]
    }

    /// File `file`'s nodes as a range of the space.
    pub(in crate::code_rule_dsl) fn range(&self, file: usize) -> std::ops::Range<u32> {
        self.bases[file]..self.bases[file] + self.counts[file]
    }
}

/// A set of a space's nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct NodeSet {
    words: Vec<u64>,
}

impl NodeSet {
    pub(in crate::code_rule_dsl) fn empty(space: &NodeSpace) -> Self {
        Self { words: vec![0; space.words()] }
    }

    /// Every node of the space.
    pub(in crate::code_rule_dsl) fn all(space: &NodeSpace) -> Self {
        let mut set = Self::empty(space);
        set.fill(0..space.len());
        set
    }

    /// The set of `words`, a slot of the space's words (Metal's `set_steps`).
    pub(in crate::code_rule_dsl) fn from_words(space: &NodeSpace, words: Vec<u64>) -> Self {
        assert_eq!(words.len().max(1), space.words().max(1), "a slot of the space");
        let mut words = words;
        words.truncate(space.words());
        Self { words }
    }

    pub(in crate::code_rule_dsl) fn words(&self) -> &[u64] {
        &self.words
    }

    pub(in crate::code_rule_dsl) fn insert(&mut self, node: u32) {
        self.words[node as usize / 64] |= 1 << (node % 64);
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn contains(&self, node: u32) -> bool {
        self.words[node as usize / 64] >> (node % 64) & 1 != 0
    }

    /// Adds the nodes in `range`.
    pub(in crate::code_rule_dsl) fn fill(&mut self, range: std::ops::Range<u32>) {
        let (start, end) = (range.start as usize, range.end as usize);
        if start >= end {
            return;
        }
        let (first, last) = (start / 64, (end - 1) / 64);
        let low = u64::MAX << (start % 64);
        let high = u64::MAX >> (63 - (end - 1) % 64);
        if first == last {
            self.words[first] |= low & high;
            return;
        }
        self.words[first] |= low;
        self.words[first + 1..last].iter_mut().for_each(|word| *word = u64::MAX);
        self.words[last] |= high;
    }

    /// Whether any node in `range` is in the set.
    pub(in crate::code_rule_dsl) fn any_in(&self, range: std::ops::Range<u32>) -> bool {
        let (start, end) = (range.start as usize, range.end as usize);
        if start >= end {
            return false;
        }
        let (first, last) = (start / 64, (end - 1) / 64);
        let low = u64::MAX << (start % 64);
        let high = u64::MAX >> (63 - (end - 1) % 64);
        if first == last {
            return self.words[first] & low & high != 0;
        }
        self.words[first] & low != 0
            || self.words[first + 1..last].iter().any(|&word| word != 0)
            || self.words[last] & high != 0
    }

    pub(in crate::code_rule_dsl) fn union_with(&mut self, other: &Self) {
        self.words.iter_mut().zip(&other.words).for_each(|(a, b)| *a |= b);
    }

    pub(in crate::code_rule_dsl) fn intersect_with(&mut self, other: &Self) {
        self.words.iter_mut().zip(&other.words).for_each(|(a, b)| *a &= b);
    }

    pub(in crate::code_rule_dsl) fn difference_with(&mut self, other: &Self) {
        self.words.iter_mut().zip(&other.words).for_each(|(a, b)| *a &= !b);
    }

    /// Keeps the set's nodes in files where each of `tests` has a node.
    pub(in crate::code_rule_dsl) fn within_files(&mut self, space: &NodeSpace, tests: &[&Self]) {
        for file in 0..space.files() {
            let range = space.range(file);
            if !tests.iter().all(|test| test.any_in(range.clone())) {
                self.clear(range);
            }
        }
    }

    /// Removes the nodes in `range`.
    pub(in crate::code_rule_dsl) fn clear(&mut self, range: std::ops::Range<u32>) {
        let mut mask = Self { words: vec![0; self.words.len()] };
        mask.fill(range);
        self.difference_with(&mask);
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.words.iter().map(|word| word.count_ones() as usize).sum()
    }

    /// The set's nodes in ascending order.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(at, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                (word != 0).then(|| {
                    let bit = word.trailing_zeros();
                    word &= word - 1;
                    at as u32 * 64 + bit
                })
            })
        })
    }

    /// The set's nodes of file `file`, as positions in the file, ascending.
    pub(in crate::code_rule_dsl) fn file_nodes<'a>(&'a self, space: &'a NodeSpace, file: usize) -> impl Iterator<Item = u32> + 'a {
        let range = space.range(file);
        let base = range.start;
        let words = if range.is_empty() { 0..0 } else { range.start as usize / 64..(range.end as usize - 1) / 64 + 1 };
        words.flat_map(move |at| {
            let mut word = self.words[at];
            std::iter::from_fn(move || {
                (word != 0).then(|| {
                    let bit = word.trailing_zeros();
                    word &= word - 1;
                    at as u32 * 64 + bit
                })
            })
        })
        .filter(move |node| range.contains(node))
        .map(move |node| node - base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_over_a_space_keep_files_apart() {
        let space = NodeSpace::new(vec![3, 0, 70, 5]);
        assert_eq!((space.len(), space.words(), space.base(2), space.base(3)), (78, 2, 3, 73));
        let mut a = NodeSet::empty(&space);
        a.fill(space.range(2));
        assert_eq!(a.len(), 70);
        assert!(a.any_in(space.range(2)) && !a.any_in(space.range(0)) && !a.any_in(space.range(1)));
        let mut b = NodeSet::empty(&space);
        b.insert(1);
        b.insert(74);
        let mut within = NodeSet::all(&space);
        within.within_files(&space, &[&b]);
        assert_eq!(within.iter().collect::<Vec<_>>(), [0, 1, 2, 73, 74, 75, 76, 77]);
        let mut union = a.clone();
        union.union_with(&b);
        assert_eq!(union.len(), 72);
        union.difference_with(&a);
        assert_eq!(union.iter().collect::<Vec<_>>(), [1, 74]);
        assert_eq!(b.file_nodes(&space, 3).collect::<Vec<_>>(), [1]);
        assert_eq!(b.file_nodes(&space, 1).count(), 0);
    }
}
