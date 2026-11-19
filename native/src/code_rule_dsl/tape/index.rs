//! The spike's rec6, span checkpoint, and overflow columns, without a row per node.
use super::*;

pub struct IndexImage {
    pub rec: Vec<u8>,
    pub spans: Vec<u8>,
    pub checkpoints: Vec<u8>,
    pub parb: Vec<u8>,
    pub sizb: Vec<u8>,
    pub flags: Vec<u8>,
}

impl TapeDoc {
    pub fn index_image(
        &self,
        interrupted: &impl Fn() -> bool,
    ) -> Result<IndexImage, String> {
        rerail_tape_format::with_tape!(self.tape(), |t| self.index_of(&t, interrupted))
    }

    /// `index_image` over the tape, read in place: one pass in preorder, each node's parent the subtree open around
    /// it, and whether it is the first child in its field from the fields its parent's earlier children filled.
    fn index_of<T: rerail_tape_format::tape::Tape>(&self, t: &T, interrupted: &impl Fn() -> bool) -> Result<IndexImage, String> {
        let n = t.len();
        let mut image = IndexImage {
            rec: Vec::with_capacity(n * WIDTH),
            spans: Vec::new(),
            checkpoints: Vec::new(),
            parb: Vec::new(),
            sizb: Vec::new(),
            flags: Vec::with_capacity(n),
        };
        let mut previous = 0i64;
        // The subtrees open around a node: (node, subtree end, the fields its children so far filled).
        let mut open: Vec<(u32, usize, u64)> = Vec::new();
        let mut s = 0u64;
        for i in 0..n {
            if i.is_multiple_of(256) {
                checked(interrupted)?;
            }
            while open.last().is_some_and(|&(_, end, _)| end <= i) {
                open.pop();
            }
            let pre = i as u32;
            let e = t.table().entry(t.code(i));
            let kind = if e.flags & CODE_ERROR != 0 { ERROR_CODE } else { e.kind };
            let field = e.field;
            let mut first_field = false;
            let parent = match open.last_mut() {
                None => NONE,
                Some((p, _, fields)) => {
                    if field != 0 {
                        let bit = 1_u64 << field;
                        first_field = *fields & bit == 0;
                        *fields |= bit;
                    }
                    *p
                }
            };
            let end = t.subtree_end(i);
            let size = (end - i) as u32;
            open.push((pre, end, 0));
            image
                .rec
                .extend_from_slice(&(kind | (field << 10)).to_le_bytes());
            for (value, overflow) in [
                (
                    if parent == NONE {
                        pre + 1
                    } else {
                        pre - parent
                    },
                    &mut image.parb,
                ),
                (size, &mut image.sizb),
            ] {
                if value >= u32::from(SENTINEL) {
                    image.rec.extend_from_slice(&SENTINEL.to_le_bytes());
                    overflow.extend_from_slice(&pre.to_le_bytes());
                    overflow.extend_from_slice(&value.to_le_bytes());
                } else {
                    image.rec.extend_from_slice(&(value as u16).to_le_bytes());
                }
            }
            s = t.start_next(i, s);
            let (start, end) = (s as u32, t.end_with_start(i, s) as u32);
            if i.is_multiple_of(CHECKPOINT_STRIDE) {
                checkpoint(image.spans.len(), &mut image.checkpoints)?;
                checkpoint(start as usize, &mut image.checkpoints)?;
            }
            let delta = i32::try_from(i64::from(start) - previous)
                .map_err(|_| "span delta overflow")?;
            varint(((delta << 1) ^ (delta >> 31)) as u32, &mut image.spans);
            varint(end - start, &mut image.spans);
            previous = i64::from(start);
            image
                .flags
                .push(self.transducer_flags(pre) | (u8::from(first_field) << 4));
        }
        Ok(image)
    }
}
