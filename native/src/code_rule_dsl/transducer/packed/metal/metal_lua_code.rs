//! One immutable Lua descriptor with checked frames and exact byte masks.
use super::*;
use crate::policy_dsl::transducer::value_packing::lua_matcher;

pub(super) fn append(
    batch: &PackedBatch,
    value: ValueRecord,
    words: &mut Vec<u32>,
) -> Result<[u32; 4], Error> {
    let captures = value.operand >> 2;
    if captures > 32 || value.auxiliary != 0 {
        return Err(Error::InvalidAnchor);
    }
    let end = value
        .state_start
        .checked_add(value.state_count)
        .ok_or(Error::InvalidAnchor)?;
    let instructions = batch
        .lua_instructions
        .get(value.state_start as usize..end as usize)
        .ok_or(Error::InvalidAnchor)?;
    let stride = 8u32
        .checked_add(captures.checked_mul(3).ok_or(Error::BackendCapacity)?)
        .ok_or(Error::BackendCapacity)?;
    let slots = value
        .state_count
        .checked_add(1)
        .and_then(|n| n.checked_mul(stride))
        .and_then(|n| n.checked_add(8))
        .ok_or(Error::BackendCapacity)?;
    let width = slots
        .checked_next_power_of_two()
        .ok_or(Error::BackendCapacity)?;
    let table = size(words.len())?;
    words.extend([
        captures,
        value.operand & 3,
        stride,
        slots,
        value.state_count,
        0,
        0,
        0,
    ]);
    for &i in instructions {
        let mut mask = [0u32; 8];
        match i.kind {
            0 if i.first <= 4 && i.second == 0 => {}
            1 if i.first == 0 && i.second == 0 => {}
            2 if i.first <= 255 && i.second <= 255 => {}
            3..=6 if i.first < captures && i.second == 0 => {}
            _ => return Err(Error::InvalidAnchor),
        }
        if i.kind <= 1 {
            let matcher = lua_matcher(batch, i)?;
            for byte in 0u32..256 {
                if matcher.matches(byte as u8) {
                    mask[byte as usize / 32] |= 1 << (byte & 31);
                }
            }
        } else if [
            i.matcher_kind,
            i.matcher_first,
            i.matcher_second,
            i.data_start,
            i.data_count,
        ] != [0; 5]
        {
            return Err(Error::InvalidAnchor);
        }
        words.extend([i.kind, i.first, i.second, 0]);
        words.extend(mask);
    }
    size(words.len())?;
    Ok([value.state_count, width, table, 1])
}
