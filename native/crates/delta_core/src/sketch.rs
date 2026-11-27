//! A column's kept sketch read where it lies (decision 0678 step 10; the format is `rerail_blocks::sketch`'s, whose
//! `Sketch` adds, takes and encodes): its reading (rows, distinct values, the sum of each value's rows squared), taken
//! from the encoded bytes as they stream, so a trace's statistics read nothing into a structure of their own.
//!
//! The bytes, as `Sketch::encode` writes them: the format, the form (0 few values, 1 a page, 2 a label set, 3 rank
//! counts), the rows, `REGISTERS` registers, `BUCKETS` join counters, then how many entries follow and the entries.
//! Varints throughout, the signed ones zigzagged.

use crate::refs::Span;
use crate::relation::Relation;

/// What a sketch's state reads: its rows, distinct values and sum of squared rows a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
}

/// The join counters a column keeps.
pub const BUCKETS: usize = 256;
/// The values a column keeps its few exact counts of, at most.
pub const FEW: usize = 64;
pub const REGISTERS: usize = 256;
/// The ranks a register's rows are counted at.
pub const RANKS: usize = 32;
pub const DENSE_WIDTH: usize = 4096;
pub const LABEL_LIMIT: i64 = 1_i64 << 32;
pub const SPARSE_LIMIT: usize = 1024;
/// The kept form's format.
pub const SKETCH_FORMAT: u8 = 1;

pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// A varint from `next`: `None` past the bytes or past 64 bits.
pub fn read_varint(next: &mut impl FnMut() -> Option<u8>) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = next()?;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Column `c`'s kept sketch in a layer's relation (its sketches' directory, `Span<u8>`s in its block): `None` when it
/// keeps none.
pub fn kept(rel: &Relation, c: usize) -> Option<&[u8]> {
    let b = rel.sketches.as_slice();
    if b.is_empty() {
        return None;
    }
    // SAFETY: its writer laid the directory out as `Span<u8>`s, aligned to 8, in the relation's block, frozen since.
    let dir = unsafe { core::slice::from_raw_parts(b.as_ptr().cast::<Span<u8>>(), b.len() / size_of::<Span<u8>>()) };
    dir.get(c).filter(|s| !s.is_empty()).map(Span::as_slice)
}

/// `2^-rank`, exactly (a power of two's bits).
fn pow2_neg(rank: u8) -> f64 {
    f64::from_bits((1023 - u64::from(rank)) << 52)
}

/// `ln x` for `x >= 1`: its power of two's, then `2 atanh((f - 1) / (f + 1))` of the rest (`f` in `[1, 2)`), within an
/// ulp or two of the platform's. (`core` has no `ln`; the floor links no math library.)
fn ln(x: f64) -> f64 {
    debug_assert!(x >= 1.0 && x.is_finite());
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let f = f64::from_bits((bits & ((1u64 << 52) - 1)) | (1023u64 << 52));
    let s = (f - 1.0) / (f + 1.0);
    let (s2, mut term, mut sum) = (s * s, s, 0.0);
    for k in 0..30 {
        sum += term / f64::from(2 * k + 1);
        term *= s2;
    }
    e as f64 * core::f64::consts::LN_2 + 2.0 * sum
}

/// `x` rounded half away from zero, for `0 <= x < 2^63`.
fn round_pos(x: f64) -> u64 {
    let t = x as u64;
    if x - t as f64 >= 0.5 { t + 1 } else { t }
}

/// Distinct values from the registers (HyperLogLog, with its small-range correction), at least 1 and at most `rows`.
fn estimate(registers: &[u8; REGISTERS], rows: u64) -> u64 {
    if rows == 0 {
        return 0;
    }
    let m = REGISTERS as f64;
    let (mut sum, mut zeros) = (0.0, 0u32);
    for &rank in registers {
        sum += pow2_neg(rank);
        zeros += u32::from(rank == 0);
    }
    let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
    let est = if raw <= 2.5 * m && zeros > 0 { m * ln(m / f64::from(zeros)) } else { raw };
    round_pos(est).clamp(1, rows)
}

/// The sum of squares from join counters, at least a row a value and at most every row one value.
fn sq_of(counters: &[i32; BUCKETS], rows: u64) -> u64 {
    let est: f64 = counters.iter().map(|&c| f64::from(c) * f64::from(c)).sum();
    round_pos(est).clamp(rows, rows.saturating_mul(rows))
}

/// The reading of a kept sketch's bytes (`Sketch::encode`'s), as its decoded sketch reads: `None` when they are not a
/// sketch's of this format, by `Sketch::decode`'s checks.
pub fn reading(bytes: &[u8]) -> Option<Reading> {
    let mut at = 0usize;
    let mut next = || -> Option<u8> {
        let b = *bytes.get(at)?;
        at += 1;
        Some(b)
    };
    if next()? != SKETCH_FORMAT {
        return None;
    }
    let form = next()?;
    let rows = read_varint(&mut next)?;
    let mut registers = [0u8; REGISTERS];
    for r in registers.iter_mut() {
        *r = next()?;
    }
    let mut counters = [0i32; BUCKETS];
    for c in counters.iter_mut() {
        *c = unzigzag(read_varint(&mut next)?) as i32;
    }
    let count = read_varint(&mut next)? as usize;
    let (ndv, sq) = match form {
        0 => {
            if count > FEW {
                return None;
            }
            let (mut ndv, mut sq) = (0u64, 0u64);
            for _ in 0..count {
                unzigzag(read_varint(&mut next)?);
                let k = read_varint(&mut next)? as i64;
                ndv += u64::from(k > 0);
                sq = sq.wrapping_add((k as u64).wrapping_mul(k as u64));
            }
            (ndv, sq)
        }
        1 => {
            unzigzag(read_varint(&mut next)?);
            let (mut i, mut sq) = (0usize, 0u64);
            for e in 0..count {
                let step = read_varint(&mut next)? as usize;
                i = i.checked_add(step)?;
                if e > 0 && step == 0 || i >= DENSE_WIDTH {
                    return None;
                }
                // The page keeps it as a u32; its sum of squares, as the decoder adds it, the count read.
                let k = read_varint(&mut next)?;
                sq += k * k;
            }
            (count as u64, sq)
        }
        2 => {
            let (mut ndv, mut sq) = (0u64, 0u64);
            for _ in 0..count {
                let v = read_varint(&mut next)? as i64;
                let n = read_varint(&mut next)?;
                if !(0..LABEL_LIMIT).contains(&v) {
                    return None;
                }
                if n > 0 {
                    ndv += 1;
                    sq += n * n;
                }
            }
            if count > SPARSE_LIMIT {
                return None;
            }
            (ndv, sq)
        }
        3 => {
            let mut i = 0usize;
            for e in 0..count {
                let step = read_varint(&mut next)? as usize;
                i = i.checked_add(step)?;
                if e > 0 && step == 0 || i >= REGISTERS * RANKS {
                    return None;
                }
                read_varint(&mut next)?;
            }
            (estimate(&registers, rows), sq_of(&counters, rows))
        }
        _ => return None,
    };
    (at == bytes.len()).then_some(Reading { rows, ndv, sq })
}
