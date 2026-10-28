//! The spike's rec6, span checkpoint, and overflow columns, without a row per node.
use super::*;

pub(in crate::code_rule_dsl) struct IndexImage {
    pub rec: Vec<u8>,
    pub spans: Vec<u8>,
    pub checkpoints: Vec<u8>,
    pub parb: Vec<u8>,
    pub sizb: Vec<u8>,
    pub flags: Vec<u8>,
}

impl TapeDoc {
    pub(in crate::code_rule_dsl) fn index_image(
        &self,
        interrupted: &impl Fn() -> bool,
    ) -> Result<IndexImage, String> {
        let mut image = IndexImage {
            rec: Vec::with_capacity(self.data.len() * WIDTH),
            spans: Vec::new(),
            checkpoints: Vec::new(),
            parb: Vec::new(),
            sizb: Vec::new(),
            flags: Vec::with_capacity(self.data.len()),
        };
        let mut previous = 0i64;
        for (i, node) in self.data.iter().enumerate() {
            if i.is_multiple_of(256) {
                checked(interrupted)?;
            }
            let pre = i as u32;
            let kind = if node.error { ERROR_CODE } else { node.kind };
            image
                .rec
                .extend_from_slice(&(kind | (node.field << 10)).to_le_bytes());
            for (value, overflow) in [
                (
                    if node.parent == NONE {
                        pre + 1
                    } else {
                        pre - node.parent
                    },
                    &mut image.parb,
                ),
                (node.size, &mut image.sizb),
            ] {
                if value >= u32::from(SENTINEL) {
                    image.rec.extend_from_slice(&SENTINEL.to_le_bytes());
                    overflow.extend_from_slice(&pre.to_le_bytes());
                    overflow.extend_from_slice(&value.to_le_bytes());
                } else {
                    image.rec.extend_from_slice(&(value as u16).to_le_bytes());
                }
            }
            if i.is_multiple_of(CHECKPOINT_STRIDE) {
                checkpoint(image.spans.len(), &mut image.checkpoints)?;
                checkpoint(node.start as usize, &mut image.checkpoints)?;
            }
            let delta = i32::try_from(i64::from(node.start) - previous)
                .map_err(|_| "span delta overflow")?;
            varint(((delta << 1) ^ (delta >> 31)) as u32, &mut image.spans);
            varint(node.end - node.start, &mut image.spans);
            previous = i64::from(node.start);
            image
                .flags
                .push(self.transducer_flags(pre) | (u8::from(node.first_field) << 4));
        }
        Ok(image)
    }
}
