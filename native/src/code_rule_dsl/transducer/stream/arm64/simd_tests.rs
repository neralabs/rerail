//! G1's tests (`docs/parser-generators/09-compile-and-lower.md` §12.4), written before the layer.
//!
//! Each primitive of `simd.rs`, `bitmask.rs` and `bitmanipulation.rs` is compared with a scalar
//! oracle. The oracles live here and nowhere else: they say what simdjson's intrinsic does, one
//! byte at a time, which the layer must never do. Inputs are edge values, every value where a
//! primitive has 256 or fewer, and 10,000 seeded random registers.

use super::bitmanipulation::*;
use super::bitmask::prefix_xor;
use super::simd::*;
use crate::code_rule_dsl::transducer::stream::internal::simdprune_tables::*;

const RANDOM: usize = 10_000;
const EDGES: [u8; 6] = [0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Bytes biased to the edges a quarter of the time.
    fn bytes16(&mut self) -> [u8; 16] {
        let mut out = [0u8; 16];
        for o in &mut out {
            let r = self.next();
            *o = if r & 3 == 0 { EDGES[(r >> 2) as usize % EDGES.len()] } else { (r >> 8) as u8 };
        }
        out
    }
    fn bools16(&mut self) -> [u8; 16] {
        let r = self.next();
        std::array::from_fn(|i| if r >> i & 1 == 1 { 0xff } else { 0 })
    }
    fn bytes64(&mut self) -> [u8; 64] {
        let mut out = [0u8; 64];
        for k in 0..4 {
            out[16 * k..16 * k + 16].copy_from_slice(&self.bytes16());
        }
        out
    }
}

fn rng() -> Rng {
    Rng(0x9e37_79b9_7f4a_7c15)
}

fn u8s(v: Simd8<u8>) -> [u8; 16] {
    let mut out = [0u8; 16];
    v.store(&mut out);
    out
}

fn i8s(v: Simd8<i8>) -> [i8; 16] {
    let mut out = [0i8; 16];
    v.store(&mut out);
    out
}

/// A `Simd8<bool>` holds 0x00 or 0xff per byte; read it through its `u8` view.
fn bools(v: Simd8<bool>) -> [u8; 16] {
    u8s(Simd8::<u8>::from(v.value))
}

fn as_bool(b: [u8; 16]) -> Simd8<bool> {
    Simd8::<bool>::from(Simd8::<u8>::load(&b).value)
}

fn mask_of(f: impl Fn(usize) -> bool) -> [u8; 16] {
    std::array::from_fn(|i| if f(i) { 0xff } else { 0 })
}

fn map2(a: [u8; 16], b: [u8; 16], f: impl Fn(u8, u8) -> u8) -> [u8; 16] {
    std::array::from_fn(|i| f(a[i], b[i]))
}

/// Pairs of random registers, plus every pair of edge splats.
fn pairs() -> Vec<([u8; 16], [u8; 16])> {
    let mut r = rng();
    let mut out: Vec<_> = (0..RANDOM).map(|_| (r.bytes16(), r.bytes16())).collect();
    for &a in &EDGES {
        for &b in &EDGES {
            out.push(([a; 16], [b; 16]));
        }
    }
    out
}

// ---- simd8<bool> ----

#[test]
fn bool_splat_and_constructors() {
    assert_eq!(bools(Simd8::<bool>::splat(true)), [0xff; 16]);
    assert_eq!(bools(Simd8::<bool>::splat(false)), [0; 16]);
    assert_eq!(bools(Simd8::<bool>::default()), [0; 16]);
}

#[test]
fn bool_to_bitmask() {
    let mut r = rng();
    for _ in 0..RANDOM {
        let b = r.bools16();
        let want = (0..16).fold(0u32, |w, i| w | ((b[i] == 0xff) as u32) << i);
        assert_eq!(as_bool(b).to_bitmask(), want, "{b:02x?}");
    }
}

/// `vshrn_n_u16(…, 4)`: byte `i` of the result is bits 4..12 of the 16-bit lane `i`.
fn bitmask64_oracle(b: [u8; 16]) -> u64 {
    (0..8).fold(0u64, |w, i| {
        let lane = b[2 * i] as u16 | (b[2 * i + 1] as u16) << 8;
        w | ((lane >> 4) as u8 as u64) << (8 * i)
    })
}

#[test]
fn to_bitmask64_is_four_bits_per_byte() {
    let mut r = rng();
    for _ in 0..RANDOM {
        let b = r.bools16();
        let want = (0..16).fold(0u64, |w, i| w | if b[i] == 0xff { 0xf << (4 * i) } else { 0 });
        assert_eq!(as_bool(b).to_bitmask64(), want);
        assert_eq!(bitmask64_oracle(b), want);
        let u = r.bytes16();
        assert_eq!(Simd8::<u8>::load(&u).to_bitmask64(), bitmask64_oracle(u), "{u:02x?}");
    }
}

#[test]
fn bool_any() {
    let mut r = rng();
    assert!(!Simd8::<bool>::splat(false).any());
    for _ in 0..RANDOM {
        let b = r.bools16();
        assert_eq!(as_bool(b).any(), b.iter().any(|&x| x != 0));
    }
    for i in 0..16 {
        let mut b = [0u8; 16];
        b[i] = 0xff;
        assert!(as_bool(b).any(), "{i}");
    }
}

// ---- base_u8: on simd8<bool> and simd8<uint8_t> ----

#[test]
fn base_logic_on_u8_and_bool() {
    for (a, b) in pairs() {
        let (x, y) = (Simd8::<u8>::load(&a), Simd8::<u8>::load(&b));
        assert_eq!(u8s(x | y), map2(a, b, |p, q| p | q));
        assert_eq!(u8s(x & y), map2(a, b, |p, q| p & q));
        assert_eq!(u8s(x ^ y), map2(a, b, |p, q| p ^ q));
        assert_eq!(u8s(x.bit_andnot(y)), map2(a, b, |p, q| p & !q));
        assert_eq!(u8s(!x), a.map(|p| !p));
        let (mut o, mut n, mut e) = (x, x, x);
        o |= y;
        n &= y;
        e ^= y;
        assert_eq!((u8s(o), u8s(n), u8s(e)), (u8s(x | y), u8s(x & y), u8s(x ^ y)));
        assert_eq!(bools(x.eq(y)), mask_of(|i| a[i] == b[i]));
        assert_eq!(bools(x.eq(x)), [0xff; 16]);
    }
    let mut r = rng();
    for _ in 0..RANDOM {
        let (a, b) = (r.bools16(), r.bools16());
        let (x, y) = (as_bool(a), as_bool(b));
        assert_eq!(bools(x | y), map2(a, b, |p, q| p | q));
        assert_eq!(bools(x & y), map2(a, b, |p, q| p & q));
        assert_eq!(bools(x ^ y), map2(a, b, |p, q| p ^ q));
        assert_eq!(bools(x.bit_andnot(y)), map2(a, b, |p, q| p & !q));
        assert_eq!(bools(!x), a.map(|p| !p));
        assert_eq!(bools(x.eq(y)), mask_of(|i| a[i] == b[i]));
    }
}

/// `prev::<N>(before)`: byte `i` is byte `i - N`, reaching into the end of `before`.
fn prev_oracle(this: [u8; 16], before: [u8; 16], n: usize) -> [u8; 16] {
    std::array::from_fn(|i| if i >= n { this[i - n] } else { before[16 - n + i] })
}

macro_rules! check_prev {
    ($x:expr, $b:expr, $a:expr, $bb:expr, $($n:literal)*) => {
        $(assert_eq!(u8s($x.prev::<$n>($b)), prev_oracle($a, $bb, $n), "N = {}", $n);)*
    };
}

#[test]
fn base_prev_every_n() {
    for (a, b) in pairs().into_iter().take(500) {
        let (x, y) = (Simd8::<u8>::load(&a), Simd8::<u8>::load(&b));
        check_prev!(x, y, a, b, 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);
        let (p, q) = (as_bool(a.map(|v| if v & 1 == 1 { 0xff } else { 0 })), as_bool(b.map(|v| if v & 2 == 2 { 0xff } else { 0 })));
        assert_eq!(bools(p.prev::<1>(q)), prev_oracle(bools(p), bools(q), 1));
        assert_eq!(bools(p.prev::<3>(q)), prev_oracle(bools(p), bools(q), 3));
    }
}

// ---- simd8<uint8_t> ----

#[test]
fn u8_constructors_load_store() {
    let mut r = rng();
    assert_eq!(u8s(Simd8::<u8>::zero()), [0; 16]);
    assert_eq!(u8s(Simd8::<u8>::default()), [0; 16]);
    for v in 0..=255u8 {
        assert_eq!(u8s(Simd8::<u8>::splat(v)), [v; 16]);
    }
    for _ in 0..RANDOM {
        let a = r.bytes16();
        assert_eq!(u8s(Simd8::<u8>::load(&a)), a);
        assert_eq!(u8s(Simd8::<u8>::new(a)), a);
        assert_eq!(u8s(Simd8::<u8>::repeat_16(a)), a);
    }
}

#[test]
fn u8_arithmetic() {
    for (a, b) in pairs() {
        let (x, y) = (Simd8::<u8>::load(&a), Simd8::<u8>::load(&b));
        assert_eq!(u8s(x.saturating_add(y)), map2(a, b, u8::saturating_add));
        assert_eq!(u8s(x.saturating_sub(y)), map2(a, b, u8::saturating_sub));
        assert_eq!(u8s(x + y), map2(a, b, u8::wrapping_add));
        assert_eq!(u8s(x - y), map2(a, b, u8::wrapping_sub));
        let (mut p, mut m) = (x, x);
        p += y;
        m -= y;
        assert_eq!((u8s(p), u8s(m)), (u8s(x + y), u8s(x - y)));
        assert_eq!(x.max_val(), *a.iter().max().unwrap());
        assert_eq!(x.min_val(), *a.iter().min().unwrap());
        assert_eq!(u8s(x.max_val_with(y)), map2(a, b, u8::max));
        assert_eq!(u8s(x.min_val_with(y)), map2(a, b, u8::min));
    }
}

#[test]
fn u8_comparisons() {
    for (a, b) in pairs() {
        let (x, y) = (Simd8::<u8>::load(&a), Simd8::<u8>::load(&b));
        assert_eq!(bools(x.le(y)), mask_of(|i| a[i] <= b[i]));
        assert_eq!(bools(x.ge(y)), mask_of(|i| a[i] >= b[i]));
        assert_eq!(bools(x.lt(y)), mask_of(|i| a[i] < b[i]));
        assert_eq!(bools(x.gt(y)), mask_of(|i| a[i] > b[i]));
        assert_eq!(u8s(x.gt_bits(y)), mask_of(|i| a[i] > b[i]));
        assert_eq!(u8s(x.lt_bits(y)), mask_of(|i| a[i] < b[i]));
        assert_eq!(bools(x.any_bits_set(y)), mask_of(|i| a[i] & b[i] != 0));
        assert_eq!(x.any_bits_set_anywhere(), a.iter().any(|&v| v != 0));
        assert_eq!(x.any_bits_set_anywhere_in(y), (0..16).any(|i| a[i] & b[i] != 0));
    }
}

macro_rules! check_shifts {
    ($x:expr, $a:expr, $($n:literal)*) => {
        $(
            assert_eq!(u8s($x.shr::<$n>()), $a.map(|v| v >> $n), "shr {}", $n);
            assert_eq!(u8s($x.shl::<$n>()), $a.map(|v| v << $n), "shl {}", $n);
        )*
    };
}

#[test]
fn u8_shifts() {
    for (a, _) in pairs().into_iter().take(2000) {
        let x = Simd8::<u8>::load(&a);
        check_shifts!(x, a, 1 2 3 4 5 6 7);
    }
    let x = Simd8::<u8>::splat(0xff);
    assert_eq!(u8s(x.shr::<8>()), [0; 16]);
}

/// `vqtbl1q_u8(table, idx)`: `table[idx]`, or 0 when `idx >= 16`.
fn lookup_oracle(table: [u8; 16], idx: [u8; 16]) -> [u8; 16] {
    idx.map(|i| if i < 16 { table[i as usize] } else { 0 })
}

#[test]
fn u8_lookup_16_every_index() {
    let mut r = rng();
    for _ in 0..1000 {
        let table = r.bytes16();
        let t = Simd8::<u8>::load(&table);
        for base in (0..=255u8).step_by(16) {
            let idx: [u8; 16] = std::array::from_fn(|i| base + i as u8);
            let x = Simd8::<u8>::load(&idx);
            assert_eq!(u8s(x.lookup_16(t)), lookup_oracle(table, idx));
            assert_eq!(u8s(t.apply_lookup_16_to(x)), lookup_oracle(table, idx));
            assert_eq!(u8s(x.lookup_16_values(table)), lookup_oracle(table, idx));
        }
        let idx = r.bytes16();
        let x = Simd8::<u8>::load(&idx);
        assert_eq!(u8s(x.lookup_16(t)), lookup_oracle(table, idx));
        // A table of i8 read by u8 indexes.
        let ti = Simd8::<i8>::load(&table.map(|v| v as i8));
        assert_eq!(i8s(x.lookup_16(ti)), lookup_oracle(table, idx).map(|v| v as i8));
    }
}

/// simdjson's `compress`: the bytes whose mask bit is clear, in order.
fn kept(a: &[u8], mask: u64) -> Vec<u8> {
    a.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 0).map(|(_, &v)| v).collect()
}

#[test]
fn u8_compress_and_halves() {
    let mut r = rng();
    let mut masks: Vec<u16> = (0..=255u16).flat_map(|lo| [lo, lo << 8, lo | 0xff00, lo * 0x0101]).collect();
    masks.extend((0..RANDOM).map(|_| r.next() as u16));
    for m in masks {
        let a = r.bytes16();
        let x = Simd8::<u8>::load(&a);
        let want = kept(&a, m as u64);
        let mut out = [0xaau8; 16];
        x.compress(m, &mut out);
        assert_eq!(&out[..want.len()], &want[..], "mask {m:016b}");
        let mut buf = [0xaau8; 24];
        let lo = 8 - (m as u8).count_ones() as usize;
        unsafe { x.compress_halves(m, buf.as_mut_ptr(), buf.as_mut_ptr().add(lo)) };
        assert_eq!(&buf[..want.len()], &want[..], "halves, mask {m:016b}");
    }
}

// ---- simd8<int8_t> ----

#[test]
fn i8_everything() {
    let mut r = rng();
    assert_eq!(i8s(Simd8::<i8>::zero()), [0; 16]);
    assert_eq!(i8s(Simd8::<i8>::default()), [0; 16]);
    for v in -128..=127i8 {
        assert_eq!(i8s(Simd8::<i8>::splat(v)), [v; 16]);
    }
    for (a, b) in pairs() {
        let (ai, bi) = (a.map(|v| v as i8), b.map(|v| v as i8));
        let (x, y) = (Simd8::<i8>::load(&ai), Simd8::<i8>::load(&bi));
        assert_eq!(i8s(Simd8::<i8>::new(ai)), ai);
        assert_eq!(i8s(Simd8::<i8>::repeat_16(ai)), ai);
        assert_eq!(u8s(x.to_u8()), a);
        assert_eq!(i8s(Simd8::<i8>::from_u8(Simd8::<u8>::load(&a))), ai);
        let f = |g: fn(i8, i8) -> i8| -> [i8; 16] { std::array::from_fn(|i| g(ai[i], bi[i])) };
        assert_eq!(i8s(x + y), f(i8::wrapping_add));
        assert_eq!(i8s(x - y), f(i8::wrapping_sub));
        let (mut p, mut m) = (x, x);
        p += y;
        m -= y;
        assert_eq!((i8s(p), i8s(m)), (i8s(x + y), i8s(x - y)));
        assert_eq!(i8s(x.max_val(y)), f(i8::max));
        assert_eq!(i8s(x.min_val(y)), f(i8::min));
        assert_eq!(bools(x.gt(y)), mask_of(|i| ai[i] > bi[i]));
        assert_eq!(bools(x.lt(y)), mask_of(|i| ai[i] < bi[i]));
        assert_eq!(bools(x.eq(y)), mask_of(|i| ai[i] == bi[i]));
        assert_eq!(i8s(x.prev::<1>(y)), prev_oracle(a, b, 1).map(|v| v as i8));
        assert_eq!(i8s(x.prev::<2>(y)), prev_oracle(a, b, 2).map(|v| v as i8));
        assert_eq!(i8s(x.prev::<15>(y)), prev_oracle(a, b, 15).map(|v| v as i8));
        // i8 indexes: negative ones are 128 or more, so they read 0.
        let table = r.bytes16();
        let t = Simd8::<u8>::load(&table);
        assert_eq!(u8s(x.lookup_16(t)), lookup_oracle(table, a));
        let ti = table.map(|v| v as i8);
        assert_eq!(i8s(x.lookup_16_values(ti)), lookup_oracle(table, a).map(|v| v as i8));
        assert_eq!(i8s(Simd8::<i8>::load(&ti).apply_lookup_16_to(x)), lookup_oracle(table, a).map(|v| v as i8));
    }
}

// ---- simd8x64 ----

#[test]
fn x64_shape() {
    assert_eq!(Simd8x64::<u8>::NUM_CHUNKS, 4);
    assert_eq!(Simd8x64::<bool>::NUM_CHUNKS, 4);
}

#[test]
fn x64_load_store_get_new_reduce_or() {
    let mut r = rng();
    for _ in 0..RANDOM / 4 {
        let a = r.bytes64();
        let x = Simd8x64::<u8>::load(&a);
        let mut out = [0u8; 64];
        x.store(&mut out);
        assert_eq!(out, a);
        assert_eq!(u8s(x.get::<0>()), a[0..16]);
        assert_eq!(u8s(x.get::<1>()), a[16..32]);
        assert_eq!(u8s(x.get::<2>()), a[32..48]);
        assert_eq!(u8s(x.get::<3>()), a[48..64]);
        let y = Simd8x64::<u8>::new(x.get::<3>(), x.get::<2>(), x.get::<1>(), x.get::<0>());
        assert_eq!(u8s(y.get::<0>()), a[48..64]);
        let want: [u8; 16] = std::array::from_fn(|i| a[i] | a[16 + i] | a[32 + i] | a[48 + i]);
        assert_eq!(u8s(x.reduce_or()), want);
    }
}

#[test]
fn x64_to_bitmask_eq_lteq() {
    let mut r = rng();
    for _ in 0..RANDOM / 4 {
        let a = r.bytes64();
        let x = Simd8x64::<u8>::load(&a);
        // simdjson ANDs each byte with its bit position: bit i is bit (i % 8) of byte i.
        let want = (0..64).fold(0u64, |w, i| w | ((a[i] >> (i % 8) & 1) as u64) << i);
        assert_eq!(x.to_bitmask(), want);
        for m in [a[r.next() as usize % 64], 0, 0x7f, 0x80, 0xff] {
            assert_eq!(x.eq(m), (0..64).fold(0u64, |w, i| w | ((a[i] == m) as u64) << i), "eq {m}");
            assert_eq!(x.lteq(m), (0..64).fold(0u64, |w, i| w | ((a[i] <= m) as u64) << i), "lteq {m}");
        }
        let b: [Simd8<bool>; 4] = std::array::from_fn(|k| as_bool(std::array::from_fn(|i| if a[16 * k + i] & 1 == 1 { 0xff } else { 0 })));
        let xb = Simd8x64::<bool>::new(b[0], b[1], b[2], b[3]);
        assert_eq!(xb.to_bitmask(), (0..64).fold(0u64, |w, i| w | ((a[i] & 1) as u64) << i));
    }
}

#[test]
fn x64_compress() {
    let mut r = rng();
    let mut masks: Vec<u64> = vec![0, u64::MAX, 1, 1 << 63, 0x5555_5555_5555_5555, 0x00ff_00ff_00ff_00ff];
    masks.extend((0..RANDOM).map(|_| r.next() & r.next()));
    masks.extend((0..RANDOM).map(|_| r.next()));
    for m in masks {
        let a = r.bytes64();
        let want = kept(&a, m);
        let mut out = [0xaau8; 64];
        let n = Simd8x64::<u8>::load(&a).compress(m, &mut out);
        assert_eq!(n as usize, want.len(), "mask {m:064b}");
        assert_eq!(&out[..want.len()], &want[..], "mask {m:064b}");
    }
}

// ---- bitmask.h, bitmanipulation.h ----

#[test]
fn bitmask_prefix_xor() {
    let mut r = rng();
    let mut words = vec![0, 1, u64::MAX, 1 << 63, 0b0010_0100];
    words.extend((0..RANDOM).map(|_| r.next()));
    for w in words {
        let want = (0..64).fold((0u64, 0u64), |(acc, p), i| {
            let p = p ^ (w >> i & 1);
            (acc | p << i, p)
        });
        assert_eq!(prefix_xor(w), want.0, "{w:064b}");
    }
    assert_eq!(prefix_xor(0b0010_0100), 0b0001_1100);
}

#[test]
fn bitmanipulation_matches_its_definitions() {
    let mut r = rng();
    let mut words: Vec<u64> = (0..64).map(|i| 1u64 << i).collect();
    words.extend([u64::MAX, 3, 0x8000_0000_0000_0001]);
    words.extend((0..RANDOM).map(|_| r.next() | 1 << (r.next() % 64)));
    for w in words {
        let tz = (0..64).find(|&i| w >> i & 1 == 1).unwrap();
        let lz = (0..64).find(|&i| w >> (63 - i) & 1 == 1).unwrap();
        assert_eq!(trailing_zeroes(w), tz);
        assert_eq!(leading_zeroes(w), lz);
        assert_eq!(clear_lowest_bit(w), w & !(1 << tz));
        assert_eq!(count_ones(w), (0..64).filter(|&i| w >> i & 1 == 1).count() as i32);
        let rev = (0..64).fold(0u64, |a, i| a | (w >> i & 1) << (63 - i));
        assert_eq!(reverse_bits(w), rev);
        // zero_leading_bit(rev, lz) clears the bit `lz` places below the top.
        assert_eq!(zero_leading_bit(rev, leading_zeroes(rev)), rev & !(1 << (63 - leading_zeroes(rev))));
    }
}

// ---- simdprune_tables ----

#[test]
fn prune_tables_match_simdjson() {
    // Entries copied from simdjson's src/internal/simdprune_tables.cpp.
    assert_eq!(&THINTABLE_EPI8[..8], &[
        0x0706050403020100, 0x0007060504030201, 0x0007060504030200, 0x0000070605040302,
        0x0007060504030100, 0x0000070605040301, 0x0000070605040300, 0x0000000706050403,
    ]);
    assert_eq!(THINTABLE_EPI8[0xf0], 0x0000000003020100);
    assert_eq!(THINTABLE_EPI8[0xfe], 0);
    assert_eq!(THINTABLE_EPI8[0xff], 0);
    assert_eq!(&BITS_SET_TABLE_256_MUL2[..16], &[0, 2, 2, 4, 2, 4, 4, 6, 2, 4, 4, 6, 4, 6, 6, 8]);
    assert_eq!(&PSHUFB_COMBINE_TABLE[..32], &[
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0xff,
    ]);
    assert_eq!(&PSHUFB_COMBINE_TABLE[128..144], &[0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    assert!(PSHUFB_COMBINE_TABLE[144..].iter().all(|&v| v == 0));
    assert_eq!(PSHUFB_COMBINE_TABLE.len(), 272);
    // Their definitions, entry by entry.
    for m in 0..256usize {
        let idx: Vec<u8> = (0..8u8).filter(|&i| m >> i & 1 == 0).collect();
        let want = idx.iter().enumerate().fold(0u64, |w, (k, &i)| w | (i as u64) << (8 * k));
        assert_eq!(THINTABLE_EPI8[m], want, "thintable {m}");
        assert_eq!(BITS_SET_TABLE_256_MUL2[m] as u32, 2 * (m as u32).count_ones(), "mul2 {m}");
    }
    for pc in 0..=8usize {
        let row: Vec<u8> = (0..(8 - pc) as u8).chain(8..16).chain(std::iter::repeat_n(0xff, pc)).collect();
        assert_eq!(&PSHUFB_COMBINE_TABLE[16 * pc..16 * pc + 16], &row[..], "combine row {pc}");
    }
}
