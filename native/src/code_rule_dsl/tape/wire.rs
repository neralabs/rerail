//! Shared packed-span codec for the engine and the captured-SQL laboratory.
use std::ops::Range;

pub(super) const WIDTH: usize = 6;
pub(super) const CHECKPOINT_STRIDE: usize = 16;

pub(super) fn read_varint(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut result = 0u32;
    for shift in (0..=28).step_by(7) {
        let byte = *bytes.get(*at).ok_or("truncated tape span")?;
        *at += 1;
        if shift == 28 && byte > 0x0f {
            return Err("span varint overflow".into());
        }
        result |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(result);
        }
    }
    Err("span varint overflow".into())
}

pub(crate) fn index_span(
    spans: &[u8],
    checkpoints: &[u8],
    pre: usize,
) -> Result<Range<usize>, String> {
    let block = pre / CHECKPOINT_STRIDE;
    let cp = checkpoints
        .get(block.checked_mul(WIDTH).ok_or("checkpoint overflow")?..)
        .and_then(|bytes| bytes.get(..WIDTH))
        .ok_or("missing span checkpoint")?;
    let mut offset = u32::from_le_bytes([cp[0], cp[1], cp[2], 0]) as usize;
    let mut start = i64::from(u32::from_le_bytes([cp[3], cp[4], cp[5], 0]));
    let mut length = 0;
    for i in block * CHECKPOINT_STRIDE..=pre {
        let encoded = read_varint(spans, &mut offset)?;
        let delta = ((encoded >> 1) as i32) ^ -((encoded & 1) as i32);
        length = read_varint(spans, &mut offset)? as usize;
        if i != block * CHECKPOINT_STRIDE {
            start += i64::from(delta);
        }
    }
    let start = usize::try_from(start).map_err(|_| "invalid span start")?;
    Ok(start..start.checked_add(length).ok_or("span end overflow")?)
}
