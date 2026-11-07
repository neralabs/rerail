//! Candidate transport only. SQL executes every rule plan; matchers consume prepared ordinals.
use std::sync::Arc;

#[derive(Clone)]
pub(super) enum Roots {
    All,
    Sparse(Arc<[u32]>),
    Bitmap(Arc<[u64]>),
}

impl Roots {
    pub(super) fn pack(mut roots: Vec<u32>, nodes: usize) -> Self {
        roots.sort_unstable();
        roots.dedup();
        if roots.len() == nodes {
            return Self::All;
        }
        if roots.len() * 4 <= nodes.div_ceil(64) * 8 {
            return Self::Sparse(roots.into());
        }
        let mut words = vec![0; nodes.div_ceil(64)];
        for root in roots {
            words[root as usize / 64] |= 1 << (root % 64);
        }
        Self::Bitmap(words.into())
    }
    pub(super) fn bytes(&self) -> usize {
        match self {
            Self::All => 0,
            Self::Sparse(roots) => roots.len() * 4,
            Self::Bitmap(words) => words.len() * 8,
        }
    }
    pub(super) fn ordinals(&self) -> Option<Arc<[u32]>> {
        match self {
            Self::All => None,
            Self::Sparse(roots) => Some(Arc::clone(roots)),
            Self::Bitmap(words) => {
                let mut roots = Vec::new();
                for (index, &word) in words.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        roots.push((index * 64) as u32 + word.trailing_zeros());
                        word &= word - 1;
                    }
                }
                Some(roots.into())
            }
        }
    }
}

/// Unordered SQL transport stays bounded by one file's compact candidate representation.
/// Promotion avoids retaining repository-sized uncompressed ordinal vectors.
pub(super) struct Builder {
    nodes: usize,
    sparse: Vec<u32>,
    words: Option<Vec<u64>>,
    count: usize,
}
impl Builder {
    pub(super) fn new(nodes: usize) -> Self {
        Self {
            nodes,
            sparse: Vec::new(),
            words: None,
            count: 0,
        }
    }
    pub(super) fn push(&mut self, pre: u32) {
        assert!(
            (pre as usize) < self.nodes,
            "candidate outside source version"
        );
        if self.count == self.nodes {
            return;
        }
        if let Some(words) = &mut self.words {
            let word = &mut words[pre as usize / 64];
            let bit = 1 << (pre % 64);
            if *word & bit == 0 {
                self.count += 1;
            }
            *word |= bit;
            if self.count == self.nodes {
                self.words = None;
            }
            return;
        }
        self.sparse.push(pre);
        if self.sparse.len() * 4 > self.nodes.div_ceil(64) * 8 {
            self.words = Some(vec![0; self.nodes.div_ceil(64)]);
            for pre in std::mem::take(&mut self.sparse) {
                self.push(pre);
            }
        }
    }
    pub(super) fn finish(self) -> Roots {
        if self.count == self.nodes {
            return Roots::All;
        }
        match self.words {
            Some(words) => Roots::Bitmap(words.into()),
            None => Roots::pack(self.sparse, self.nodes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shuffled_transport_promotes_with_a_file_sized_memory_bound() {
        for nodes in [1, 63, 64, 65, 4096, 16385] {
            let mut builder = Builder::new(nodes);
            for pre in (0..nodes).rev().filter(|p| p % 3 == 0) {
                builder.push(pre as u32);
                builder.push(pre as u32);
                let bytes = builder.sparse.capacity() * 4
                    + builder.words.as_ref().map_or(0, |w| w.capacity() * 8);
                assert!(
                    bytes <= 2 * nodes.div_ceil(64) * 8 + 16,
                    "transport grew beyond the file bitmap: {bytes}"
                );
            }
            let expected = (0..nodes)
                .filter(|p| p % 3 == 0)
                .map(|p| p as u32)
                .collect::<Vec<_>>();
            let roots = builder.finish();
            if nodes == 1 {
                assert!(roots.ordinals().is_none());
            } else {
                assert_eq!(roots.ordinals().as_deref(), Some(expected.as_slice()));
            }
            let mut full = Builder::new(nodes);
            for pre in (0..nodes).rev() {
                full.push(pre as u32);
            }
            assert!(full.finish().ordinals().is_none());
        }
    }
    #[test]
    fn unordered_sparse_positions_are_canonical_and_duplicates_do_not_imply_all() {
        let sparse = Roots::pack(vec![91, 3, 91, 7], 4096);
        assert_eq!(sparse.ordinals().as_deref(), Some([3, 7, 91].as_slice()));
        let repeated = Roots::pack(vec![1; 8], 8);
        assert_eq!(repeated.ordinals().as_deref(), Some([1].as_slice()));
    }
}
