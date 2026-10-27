//! Tape-scoped kind positions: one delta-varint posting segment per kind code, in preorder.
use crate::content_store::append_delta;

pub(super) struct Builder {
    pub segments: Vec<Vec<u8>>,
    previous: [u32; 1024],
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            segments: vec![Vec::new(); 1024],
            previous: [0; 1024],
        }
    }
}

impl Builder {
    pub fn push(&mut self, kind: u16, pre: u32) {
        let code = usize::from(kind);
        let delta = pre - self.previous[code];
        self.previous[code] = pre;
        append_delta(&mut self.segments[code], delta);
    }
}
