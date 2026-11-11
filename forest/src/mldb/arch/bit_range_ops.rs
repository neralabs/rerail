//! bit_range_ops.h, in Rust.
//!
//! Jeremy Barnes, 23 March 2009. Copyright (c) 2009 Jeremy Barnes. This file is part of MLDB. Copyright 2015
//! mldb.ai inc. Apache-2.0 (see `../LICENSE`).
//!
//! Operations for operating over a range of bits.
//!
//! In Rust the memory a buffer walks is a slice and a word index, not a pointer. The two memory buffers
//! (`Simple_Mem_Buffer`, `Buffered_Mem_Buffer`) read the same words; the slice cursor is both. Prefetching is the
//! hardware's (`prefetch` is kept, as a no-op, for the callers). The `std::atomic` overload of
//! `extract_bit_range` is `extract_bit_range_atomic`.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub type ShiftT = u32;

/// An unsigned machine word (`Data` in the C++ templates).
pub trait Word: Copy + Default + Eq + std::fmt::Debug {
    const BITS: u32;
    const ZERO: Self;
    const ONES: Self;
    fn shr(self, n: u32) -> Self;
    fn shl(self, n: u32) -> Self;
    fn or(self, o: Self) -> Self;
    fn and(self, o: Self) -> Self;
    fn not(self) -> Self;
    fn from_u64(v: u64) -> Self;
    fn to_u64(self) -> u64;
}

macro_rules! word {
    ($($t:ty),*) => {$(
        impl Word for $t {
            const BITS: u32 = <$t>::BITS;
            const ZERO: Self = 0;
            const ONES: Self = <$t>::MAX;
            #[inline(always)] fn shr(self, n: u32) -> Self { self >> n }
            #[inline(always)] fn shl(self, n: u32) -> Self { self << n }
            #[inline(always)] fn or(self, o: Self) -> Self { self | o }
            #[inline(always)] fn and(self, o: Self) -> Self { self & o }
            #[inline(always)] fn not(self) -> Self { !self }
            #[inline(always)] fn from_u64(v: u64) -> Self { v as $t }
            #[inline(always)] fn to_u64(self) -> u64 { self as u64 }
        }
    )*};
}
word!(u8, u16, u32, u64);

/// Performs the same as the shrd instruction in x86 land: shifts a low and a high value together and returns a
/// middle set of bits.
///
/// ```text
/// 2n                n                    0
/// +-----------------+--------------------+
/// |      high       |       low          |
/// +--------+--------+-----------+--------+
///          |     result         |<-bits---
///          +--------------------+
/// ```
///
/// `bits` must be less than the number of bits in the word; 0 returns `low`.
#[inline(always)]
pub fn shrd_emulated<T: Word>(low: T, high: T, bits: ShiftT) -> T {
    if bits == 0 {
        return low;
    }
    low.shr(bits).or(high.shl(T::BITS - bits))
}

/// shrd, for `0 < bits < T::BITS` only.
#[inline(always)]
pub fn shrd_emulated_unsafe<T: Word>(low: T, high: T, bits: ShiftT) -> T {
    debug_assert!(bits > 0 && bits < T::BITS);
    low.shr(bits).or(high.shl(T::BITS - bits))
}

/// The compiler makes the instruction from the emulated form on every target.
#[inline(always)]
pub fn shrd<T: Word>(low: T, high: T, bits: ShiftT) -> T {
    shrd_emulated(low, high, bits)
}

#[inline(always)]
pub fn shrd_unsafe<T: Word>(low: T, high: T, bits: ShiftT) -> T {
    shrd_emulated_unsafe(low, high, bits)
}

#[inline(always)]
pub fn mask_lower<T: Word>(val: T, bits: ShiftT) -> T {
    let mask = if bits == 0 {
        T::ZERO
    } else if bits >= T::BITS {
        T::ONES
    } else {
        T::ONES.shr(T::BITS - bits)
    };
    val.and(mask)
}

/// Extract the bits from bit to bit+numBits starting at `p[0]`. There can be a maximum of one word extracted in
/// this way. No adjustment is done on the address; `bit` is expected to already be there.
///
/// 1. If it fits in a single word: copy it, shift it right.
/// 2. Otherwise, do a double shift right across the two memory locations.
#[inline(always)]
pub fn extract_bit_range<T: Word>(p: &[T], bit: usize, bits: ShiftT) -> T {
    if bits == 0 {
        return T::ZERO;
    }
    let mut result = p[0];
    if bit + bits as usize > T::BITS as usize {
        result = shrd(result, p[1], bit as u32);
    } else {
        result = result.shr(bit as u32);
    }
    if bits == T::BITS {
        return result;
    }
    mask_lower(result, bits)
}

/// `extract_bit_range` over atomic words.
pub fn extract_bit_range_atomic64(p: &[AtomicU64], bit: usize, bits: ShiftT, order: Ordering) -> u64 {
    if bits == 0 {
        return 0;
    }
    let mut result = p[0].load(order);
    if bit + bits as usize > 64 {
        result = shrd(result, p[1].load(order), bit as u32);
    } else {
        result >>= bit;
    }
    if bits == 64 {
        return result;
    }
    mask_lower(result, bits)
}

/// `extract_bit_range` over atomic 32-bit words.
pub fn extract_bit_range_atomic32(p: &[AtomicU32], bit: usize, bits: ShiftT, order: Ordering) -> u32 {
    if bits == 0 {
        return 0;
    }
    let mut result = p[0].load(order);
    if bit + bits as usize > 32 {
        result = shrd(result, p[1].load(order), bit as u32);
    } else {
        result >>= bit;
    }
    if bits == 32 {
        return result;
    }
    mask_lower(result, bits)
}

/// Same, but the low and high values are passed in, making it pure.
#[inline(always)]
pub fn extract_bit_range_pure<T: Word>(p0: T, p1: T, bit: usize, bits: ShiftT) -> T {
    mask_lower(shrd(p0, p1, bit as u32), bits)
}

#[inline(always)]
pub fn extract_bit_range_unsafe<T: Word>(p0: T, p1: T, bit: usize, bits: ShiftT) -> T {
    mask_lower(shrd_unsafe(p0, p1, bit as u32), bits)
}

/// Same, but high bits are not filtered out.
#[inline(always)]
pub fn extract_bit_range_unmasked<T: Word>(p0: T, p1: T, bit: usize, _bits: ShiftT) -> T {
    shrd(p0, p1, bit as u32)
}

#[inline(always)]
pub fn extract_bit_range_unmasked_unsafe<T: Word>(p0: T, p1: T, bit: usize, _bits: ShiftT) -> T {
    shrd_unsafe(p0, p1, bit as u32)
}

/// Set the given range of bits in `in_` to the given value. `val` mustn't have bits set outside `bits`, and it
/// must entirely fit within the value.
#[inline(always)]
pub fn set_bits<T: Word>(in_: T, val: T, bit: ShiftT, bits: ShiftT) -> T {
    let mask = mask_lower(T::ONES, bits);
    let (mask, val) = if bit >= T::BITS { (T::ZERO, T::ZERO) } else { (mask.shl(bit), val.shl(bit)) };
    in_.and(mask.not()).or(val.and(mask))
}

#[inline(always)]
pub fn set_bit_range<T: Word>(p0: &mut T, p1: &mut T, val: T, bit: ShiftT, bits: ShiftT) {
    if bits == 0 {
        return;
    }
    // There's some part of the first and some part of the second value (both zero or more bits) that need to be
    // replaced by the low and high bits respectively.
    let bits0 = bits.min(T::BITS - bit);
    let bits1 = bits - bits0;
    *p0 = set_bits(*p0, val, bit, bits0);
    if bits1 != 0 {
        *p1 = set_bits(*p1, val.shr(bits0), 0, bits1);
    }
}

/// Sign extend the sign from the given bit into the rest of the sign bits.
pub fn sign_extend_i64(raw: i64, sign_bit: u32) -> i64 {
    let u = raw as u64;
    let sign = (u >> sign_bit) & 1;
    let new_bits = (sign.wrapping_neg()) << sign_bit;
    (u | new_bits) as i64
}

/// `fixup_extract`: unsigned values are as extracted; signed ones are sign extended (`sign_extend_i64`).
pub fn fixup_extract<T>(extracted: T, _bits: ShiftT) -> T {
    extracted
}

/// A cursor over words, reading bits from the least-significant end (`Bit_Buffer` over `Simple_Mem_Buffer`).
#[derive(Clone, Copy, Debug)]
pub struct BitBuffer<'a, T: Word> {
    data: &'a [T],
    /// The current word.
    word: usize,
    /// Number of bits from the start of the current word.
    bit_ofs: usize,
}

impl<'a, T: Word> BitBuffer<'a, T> {
    pub fn new(data: &'a [T]) -> Self {
        BitBuffer { data, word: 0, bit_ofs: 0 }
    }

    #[inline(always)]
    fn curr(&self) -> T {
        self.data[self.word]
    }

    #[inline(always)]
    fn next(&self) -> T {
        self.data[self.word + 1]
    }

    /// Extracts bits starting from the least-significant bits of the buffer.
    pub fn extract(&mut self, bits: ShiftT) -> T {
        if bits == 0 {
            return T::ZERO;
        }
        let result = if self.bit_ofs + bits as usize <= T::BITS as usize {
            extract_bit_range_pure(self.curr(), T::ZERO, self.bit_ofs, bits)
        } else {
            extract_bit_range_pure(self.curr(), self.next(), self.bit_ofs, bits)
        };
        self.advance(bits as isize);
        result
    }

    /// Extracts bits starting from the least-significant bits of the buffer. The caller guarantees that the word
    /// after the last word in the buffer can be read, and that `bits` is not zero.
    #[inline(always)]
    pub fn extract_fast(&mut self, bits: ShiftT) -> T {
        let result = extract_bit_range_pure(self.curr(), self.next(), self.bit_ofs, bits);
        self.advance(bits as isize);
        result
    }

    #[inline(always)]
    pub fn extract_fast_unsafe(&mut self, bits: ShiftT) -> T {
        let result = extract_bit_range_unsafe(self.curr(), self.next(), self.bit_ofs, bits);
        self.advance(bits as isize);
        result
    }

    /// Like `extract_fast`, but doesn't filter the top bits out.
    #[inline(always)]
    pub fn extract_fast_unmasked(&mut self, bits: ShiftT) -> T {
        let result = extract_bit_range_unmasked(self.curr(), self.next(), self.bit_ofs, bits);
        self.advance(bits as isize);
        result
    }

    #[inline(always)]
    pub fn extract_fast_unmasked_unsafe(&mut self, bits: ShiftT) -> T {
        let result = extract_bit_range_unmasked_unsafe(self.curr(), self.next(), self.bit_ofs, bits);
        self.advance(bits as isize);
        result
    }

    /// Extracts bits starting from the most-significant bits of the buffer.
    pub fn rextract(&mut self, bits: ShiftT) -> T {
        if bits == 0 {
            return T::ZERO;
        }
        let dbits = T::BITS as usize;
        let result = if self.bit_ofs + bits as usize <= dbits {
            let shift = dbits - (self.bit_ofs + bits as usize);
            extract_bit_range_pure(self.curr(), T::ZERO, shift, bits)
        } else {
            let shift = dbits * 2 - (self.bit_ofs + bits as usize);
            extract_bit_range_pure(self.next(), self.curr(), shift, bits)
        };
        self.advance(bits as isize);
        result
    }

    /// Increase the cursor. No bound checking is performed until the next read.
    #[inline(always)]
    pub fn advance(&mut self, bits: isize) {
        let total = (self.word * T::BITS as usize + self.bit_ofs) as isize + bits;
        debug_assert!(total >= 0);
        self.word = total as usize / T::BITS as usize;
        self.bit_ofs = total as usize % T::BITS as usize;
    }

    /// Bits from the start of the buffer.
    pub fn current_offset(&self) -> usize {
        self.word * T::BITS as usize + self.bit_ofs
    }

    #[inline(always)]
    pub fn prefetch(&self, _bytes_ahead: usize) {}
}

/// A class to perform extraction of bits from a range of memory (`Bit_Extractor`). The goal is to support the
/// streaming case efficiently (where we stream across a lot of memory, extracting different numbers of bits all the
/// time). For lots of random access, use `set_bit_range` and `extract_bit_range` instead.
#[derive(Clone, Copy, Debug)]
pub struct BitExtractor<'a, T: Word> {
    buf: BitBuffer<'a, T>,
}

impl<'a, T: Word> BitExtractor<'a, T> {
    pub fn new(data: &'a [T]) -> Self {
        BitExtractor { buf: BitBuffer::new(data) }
    }
    pub fn extract(&mut self, num_bits: ShiftT) -> T {
        self.buf.extract(num_bits)
    }
    pub fn extract_fast(&mut self, num_bits: ShiftT) -> T {
        self.buf.extract_fast(num_bits)
    }
    pub fn extract_fast_unmasked(&mut self, num_bits: ShiftT) -> T {
        self.buf.extract_fast_unmasked(num_bits)
    }
    pub fn extract_fast_unsafe(&mut self, num_bits: ShiftT) -> T {
        self.buf.extract_fast_unsafe(num_bits)
    }
    pub fn extract_fast_unmasked_unsafe(&mut self, num_bits: ShiftT) -> T {
        self.buf.extract_fast_unmasked_unsafe(num_bits)
    }
    pub fn rextract(&mut self, num_bits: ShiftT) -> T {
        self.buf.rextract(num_bits)
    }
    pub fn advance(&mut self, bits: isize) {
        self.buf.advance(bits)
    }
    pub fn current_offset(&self) -> usize {
        self.buf.current_offset()
    }
    pub fn prefetch(&self, bytes_ahead: usize) {
        self.buf.prefetch(bytes_ahead)
    }
}

/// Writes bits into a range of memory (`Bit_Writer`).
#[derive(Debug)]
pub struct BitWriter<'a, T: Word> {
    data: &'a mut [T],
    word: usize,
    bit_ofs: usize,
}

impl<'a, T: Word> BitWriter<'a, T> {
    pub fn new(data: &'a mut [T]) -> Self {
        BitWriter { data, word: 0, bit_ofs: 0 }
    }

    pub fn reset(&mut self) {
        self.word = 0;
        self.bit_ofs = 0;
    }

    /// The two words at the cursor (the second only when the write crosses into it).
    fn pair(&mut self, crosses: bool) -> (&mut T, Option<&mut T>) {
        let (a, b) = self.data[self.word..].split_at_mut(1);
        (&mut a[0], if crosses { b.first_mut() } else { None })
    }

    /// Writes bits starting from the least-significant bits of the buffer.
    pub fn write(&mut self, val: T, bits: ShiftT) {
        if bits == 0 {
            return;
        }
        let bit = self.bit_ofs as u32;
        let crosses = bit + bits > T::BITS;
        let (p0, p1) = self.pair(crosses);
        let mut spare = T::ZERO;
        let p1 = match p1 {
            Some(p1) => p1,
            None => {
                assert!(!crosses, "bit writer: write past the end of the buffer");
                &mut spare
            }
        };
        set_bit_range(p0, p1, val, bit, bits);
        self.skip(bits);
    }

    /// Writes bits starting from the most-significant bits of the buffer.
    pub fn rwrite(&mut self, val: T, bits: ShiftT) {
        if bits == 0 {
            return;
        }
        let dbits = T::BITS as usize;
        if bits as usize + self.bit_ofs <= dbits {
            let shift = (dbits - (self.bit_ofs + bits as usize)) as u32;
            let w = &mut self.data[self.word];
            *w = set_bits(*w, val, shift, bits);
        } else {
            let shift = (dbits * 2 - (self.bit_ofs + bits as usize)) as u32;
            let (a, b) = self.data[self.word..].split_at_mut(1);
            set_bit_range(&mut b[0], &mut a[0], val, shift, bits);
        }
        self.skip(bits);
    }

    pub fn skip(&mut self, bits: ShiftT) {
        self.bit_ofs += bits as usize;
        self.word += self.bit_ofs / T::BITS as usize;
        self.bit_ofs %= T::BITS as usize;
    }

    /// Bits from the start of the buffer.
    pub fn current_offset(&self) -> usize {
        self.word * T::BITS as usize + self.bit_ofs
    }

    /// Fill any unwritten bits in the current word with zeros, for determinism in the produced data.
    pub fn zero_extra_bits(&mut self) {
        let num_extra_bits = T::BITS as usize - self.bit_ofs;
        if num_extra_bits == T::BITS as usize {
            return;
        }
        self.write(T::ZERO, num_extra_bits as u32);
    }
}

/// Random access to an array of `num_bits`-wide values (`BitArrayIterator`, as an index rather than an iterator).
#[derive(Clone, Copy, Debug)]
pub struct BitArray<'a, T: Word> {
    pub data: &'a [T],
    pub num_bits: u32,
}

impl<'a, T: Word> BitArray<'a, T> {
    pub fn get(&self, index: usize) -> T {
        let mut extractor = BitExtractor::new(self.data);
        extractor.advance((index * self.num_bits as usize) as isize);
        extractor.extract(self.num_bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_range_write_then_extract() {
        let widths = [1u32, 3, 7, 13, 31, 32, 33, 47, 64];
        let mut data = vec![0u64; 64];
        let vals: Vec<u64> = (0..200u64).map(|i| i.wrapping_mul(0x9e37_79b9_7f4a_7c15)).collect();
        {
            let mut w = BitWriter::new(&mut data);
            for (i, v) in vals.iter().enumerate() {
                let b = widths[i % widths.len()];
                w.write(mask_lower(*v, b), b);
                if w.current_offset() > 60 * 64 {
                    break;
                }
            }
            w.zero_extra_bits();
        }
        let mut e = BitExtractor::new(&data);
        for (i, v) in vals.iter().enumerate() {
            if e.current_offset() > 60 * 64 {
                break;
            }
            let b = widths[i % widths.len()];
            let off = e.current_offset();
            assert_eq!(e.extract(b), mask_lower(*v, b), "value {i}, {b} bits");
            let mut f = BitExtractor::new(&data);
            f.advance(off as isize);
            assert_eq!(f.extract_fast(b), mask_lower(*v, b), "fast, value {i}");
        }
    }

    #[test]
    fn bit_range_shrd_and_sign() {
        assert_eq!(shrd(0xf0u8, 0x0fu8, 4), 0xff);
        assert_eq!(shrd(5u32, 9u32, 0), 5);
        assert_eq!(sign_extend_i64(0b100, 2), -4);
        assert_eq!(set_bits(0u32, 0b11, 30, 2), 0xc000_0000);
        let mut words = [0u32; 3];
        {
            let mut w = BitWriter::new(&mut words);
            w.rwrite(0b101, 3);
        }
        let mut e = BitExtractor::new(&words);
        assert_eq!(e.rextract(3), 0b101);
        assert_eq!(BitArray { data: &[0x4321u32, 0], num_bits: 4 }.get(2), 3);
    }
}
