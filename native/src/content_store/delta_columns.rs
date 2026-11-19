//! Columns of integers as one block: each column in turn, each value the zigzag varint of its difference from the
//! value above it. The frames' encoding (`frames.rs`). It was made for the cross-file memo's rows (decision 0076),
//! removed with the memo (decision 0450 §3).

use super::StoreResult;

/// `values` (rows of `width`, flat) as a block: each column in turn, as zigzag varints of the difference from the row
/// above.
pub(super) fn encode(width: usize, values: &[i64]) -> Vec<u8> {
    let rows = values.len() / width.max(1);
    // Many rows: each column's rows in parts at once, each part's first value the difference from the row above it
    // (the part before's last), so the parts' bytes, one after another, are the bytes one thread writes.
    const PART: usize = 1 << 15;
    if width > 0 && rows >= 2 * PART {
        use rayon::prelude::*;
        let parts: Vec<(usize, usize)> = (0..width).flat_map(|c| (0..rows).step_by(PART).map(move |a| (c, a))).collect();
        let bytes: Vec<Vec<u8>> = parts.par_iter().map(|&(c, a)| {
            let mut out = Vec::with_capacity(PART.min(rows - a) * 2);
            let last = if a == 0 { 0 } else { values[(a - 1) * width + c] };
            encode_column(width, c, &values[a * width..(a + PART).min(rows) * width], last, &mut out);
            out
        }).collect();
        let mut out = Vec::with_capacity(bytes.iter().map(Vec::len).sum());
        for b in &bytes {
            out.extend_from_slice(b);
        }
        return out;
    }
    let mut out = Vec::with_capacity(values.len() * 2);
    for c in 0..width {
        encode_column(width, c, values, 0, &mut out);
    }
    out
}

/// `encode`'s bytes of rows given a window at a time (`each` calls its argument with each window, rows of `width`,
/// flat): the rows read once, each column encoded into its own buffer (`cols`), then
/// the columns written one after another into one entry (`write`, given the total). The rows are not copied; the
/// frame's bytes are built once and written into their entry (Q89).
///
/// The encoding is outside `write`: a writer that holds a lock (`FramePlacer::place_rows`) holds it for the copy
/// alone. Encoding under it (reading the rows a second time, to write what a first pass counted) made the jobs that
/// frame a write's inputs take turns: on 50 files of B-now, 0.25 s of CPU became 1.8 s, and its waits more.
pub(super) fn encode_windows<E>(width: usize, each: &dyn Fn(&mut dyn FnMut(&[i64])), cols: &mut Vec<Vec<u8>>, write: impl FnOnce(usize, &mut dyn FnMut(&mut [u8])) -> Result<(), E>) -> Result<(), E> {
    cols.resize_with(width, Vec::new);
    for c in cols.iter_mut() {
        c.clear();
    }
    let mut last = vec![0i64; width];
    each(&mut |rows: &[i64]| {
        for row in rows.chunks_exact(width) {
            for c in 0..width {
                let mut z = ((row[c].wrapping_sub(last[c]) << 1) ^ (row[c].wrapping_sub(last[c]) >> 63)) as u64;
                last[c] = row[c];
                let out = &mut cols[c];
                while z >= 0x80 {
                    out.push((z as u8) | 0x80);
                    z >>= 7;
                }
                out.push(z as u8);
            }
        }
    });
    let total = cols.iter().map(Vec::len).sum();
    write(total, &mut |out: &mut [u8]| {
        let mut at = 0;
        for c in cols.iter() {
            out[at..at + c.len()].copy_from_slice(c);
            at += c.len();
        }
    })
}

/// Column `c` of `values` (rows of `width`) onto `out`, as `encode` writes it: `last` is the value of the row above
/// the first.
fn encode_column(width: usize, c: usize, values: &[i64], mut last: i64, out: &mut Vec<u8>) {
    for row in values.chunks_exact(width) {
        let d = row[c].wrapping_sub(last);
        last = row[c];
        let mut z = ((d << 1) ^ (d >> 63)) as u64;
        while z >= 0x80 {
            out.push((z as u8) | 0x80);
            z >>= 7;
        }
        out.push(z as u8);
    }
}

/// A block's columns: `rows` values each, `width` of them.
pub(super) fn decode(width: usize, rows: usize, data: &[u8], cols: &mut [Vec<i64>]) -> StoreResult<()> {
    let bad = || super::StoreError::Invalid("a block of delta columns ends early".to_owned());
    let mut at = 0;
    for col in cols.iter_mut().take(width) {
        col.reserve(rows);
        let mut last = 0i64;
        for _ in 0..rows {
            let (mut z, mut shift) = (0u64, 0);
            loop {
                let b = *data.get(at).ok_or_else(bad)?;
                at += 1;
                z |= u64::from(b & 0x7f) << shift;
                if b < 0x80 {
                    break;
                }
                shift += 7;
            }
            last = last.wrapping_add(((z >> 1) as i64) ^ -((z & 1) as i64));
            col.push(last);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block gives back its rows, whatever their values (negative, extreme, unsorted), column by column.
    #[test]
    fn a_block_gives_back_its_rows() {
        let values: Vec<i64> = vec![0, i64::MIN, 5 << 32, i64::MAX, -1, 7, 3, 3, (9 << 32) | 4, -12345];
        for width in [1, 2, 5] {
            let values = &values[..values.len() / width * width];
            let rows = values.len() / width;
            let mut cols = vec![Vec::new(); width];
            decode(width, rows, &encode(width, values), &mut cols).unwrap();
            let back: Vec<i64> = (0..rows).flat_map(|r| cols.iter().map(move |c| c[r])).collect();
            assert_eq!(back, values, "width {width}");
        }
        assert!(decode(1, 2, &encode(1, &[1]), &mut [Vec::new()]).is_err());
    }

    /// Many rows are encoded in parts at once, and the bytes are one thread's: each part goes on from the row above.
    #[test]
    fn a_block_encoded_in_parts_is_one_threads() {
        let (width, rows) = (3, 200_003);
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let values: Vec<i64> = (0..width * rows).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x as i64) >> (x % 50) }).collect();
        let mut one = Vec::new();
        for c in 0..width {
            encode_column(width, c, &values, 0, &mut one);
        }
        let parts = encode(width, &values);
        assert_eq!(parts, one);
        let mut cols = vec![Vec::new(); width];
        decode(width, rows, &parts, &mut cols).unwrap();
        assert!((0..rows).all(|r| (0..width).all(|c| cols[c][r] == values[r * width + c])));
    }
}
