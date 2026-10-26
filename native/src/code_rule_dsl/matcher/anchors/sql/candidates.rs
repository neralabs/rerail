//! Candidate transport only. SQL executes every rule plan; matchers consume prepared ordinals.
use std::sync::Arc;

#[derive(Clone)]
pub(super) enum Roots {
    All,
    Sparse(Arc<[u32]>),
    Bitmap(Arc<[u64]>),
}

impl Roots {
    pub(super) fn pack(roots: Vec<u32>, nodes: usize) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unordered_sparse_positions_are_canonical_and_duplicates_do_not_imply_all() {
        let sparse = Roots::pack(vec![91,3,91,7], 4096);
        assert_eq!(sparse.ordinals().as_deref(), Some([3,7,91].as_slice()));
        let repeated = Roots::pack(vec![1;8], 8);
        assert_eq!(repeated.ordinals().as_deref(), Some([1].as_slice()));
    }
}
