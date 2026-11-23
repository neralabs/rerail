//! Tape-scoped kind positions: one container segment per kind code (decision 0045 §5,
//! `content_store::postings::segment_positions`), in preorder.
use crate::code_rule_dsl::space::Builder as SetBuilder;

pub(super) struct Builder {
    positions: Vec<Vec<u32>>,
}

impl Default for Builder {
    fn default() -> Self {
        Self { positions: vec![Vec::new(); 1024] }
    }
}

impl Builder {
    pub fn push(&mut self, kind: u16, pre: u32) {
        self.positions[usize::from(kind)].push(pre);
    }

    /// Kind `code`'s positions as a segment; the positions are released.
    pub fn segment(&mut self, code: usize) -> Vec<u8> {
        let mut set = SetBuilder::default();
        let mut positions = std::mem::take(&mut self.positions[code]);
        positions.sort_unstable();
        positions.dedup();
        positions.into_iter().for_each(|p| set.node(p));
        set.finish().encode()
    }
}
