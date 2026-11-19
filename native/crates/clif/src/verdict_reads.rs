//! A text test's verdicts in the one rolling cache (decision 0450 step 2d), written once in CLIF for the host (the
//! blocks crate's `generated_reads.rs`), the kernels (`relation_reads.metal`) and the JIT.
//!
//! An entry holds 64 symbols of one test (the user's choice, 2026-11-19): its key is `(test + 1) << 32 | symbol >> 6`,
//! its payload two masks of 64 bits, the symbols known to pass (`YES`) and known to fail (`NO`), each a pair of u32s.
//! A verdict is one atomic or into one of them, so a copy of the entry (into the head, as the cache promotes) may miss
//! a verdict but never gives a wrong one; neither bit set is a verdict not yet made. The cache's dead entry (no room)
//! gives none and keeps none.

use crate::reads::{B, cache_get, cache_load};
use crate::view::cache as c;
use cranelift_codegen::ir::{
    AtomicRmwOp, InstBuilder, MemFlags, Value,
    condcodes::IntCC,
    types::{I32, I64},
};

/// The masks' offsets in an entry: symbols known to pass, then to fail.
pub const YES: i32 = c::PAYLOAD;
pub const NO: i32 = c::PAYLOAD + 8;
/// A verdict: none yet, the symbol fails, it passes.
pub const UNKNOWN: u32 = 0;
pub const FAILS: u32 = 1;
pub const PASSES: u32 = 2;

impl B<'_, '_> {
    /// The entry of `test`'s 64 symbols about `sym` (made, both masks empty, when none is), whether it is the dead
    /// entry, and the offset of `sym`'s word in a mask from the mask, and its bit.
    fn verdict_entry(&mut self, test: Value, sym: Value) -> (Value, Value, Value, Value) {
        let cache = self.tape;
        let t = self.f.ins().uextend(I64, test);
        let t = self.f.ins().iadd_imm(t, 1);
        let t = self.f.ins().ishl_imm(t, 32);
        let group = self.f.ins().ushr_imm(sym, 6);
        let group = self.f.ins().uextend(I64, group);
        let key = self.f.ins().bor(t, group);
        let e = cache_get(self, cache, key, &|b, e| {
            let zero = b.c64(0);
            let rw = MemFlags::new().with_notrap();
            b.f.ins().store(rw, zero, e, YES);
            b.f.ins().store(rw, zero, e, NO);
            b.c32(1)
        });
        let dead = cache_load(self, cache, c::DEAD);
        let dead = self.f.ins().uextend(I64, dead);
        let dead = self.f.ins().iadd(cache, dead);
        let is_dead = self.f.ins().icmp(IntCC::Equal, e, dead);
        let word = self.f.ins().ushr_imm(sym, 5);
        let word = self.f.ins().band_imm(word, 1);
        let word = self.f.ins().ishl_imm(word, 2);
        let word = self.f.ins().uextend(I64, word);
        let at = self.f.ins().iadd(e, word);
        let k = self.f.ins().band_imm(sym, 31);
        let one = self.c32(1);
        let bit = self.f.ins().ishl(one, k);
        (at, is_dead, bit, e)
    }
}

/// `verdict_get(cache, test, sym)`: `PASSES`, `FAILS`, or `UNKNOWN` (no verdict kept: made by `verdict_put`).
pub fn verdict_get(b: &mut B<'_, '_>) -> Value {
    let (test, sym) = (b.arg(0), b.arg(1));
    let (at, dead, bit, _) = b.verdict_entry(test, sym);
    b.either(
        I32,
        dead,
        |b| b.c32(i64::from(UNKNOWN)),
        |b| {
            let p = b.f.ins().iadd_imm(at, i64::from(YES));
            let yes = b.f.ins().atomic_load(I32, MemFlags::trusted(), p);
            let p = b.f.ins().iadd_imm(at, i64::from(NO));
            let no = b.f.ins().atomic_load(I32, MemFlags::trusted(), p);
            let yes = b.f.ins().band(yes, bit);
            let no = b.f.ins().band(no, bit);
            let passes = b.f.ins().icmp_imm(IntCC::NotEqual, yes, 0);
            let fails = b.f.ins().icmp_imm(IntCC::NotEqual, no, 0);
            let p2 = b.c32(i64::from(PASSES));
            let f1 = b.c32(i64::from(FAILS));
            let u0 = b.c32(i64::from(UNKNOWN));
            let other = b.f.ins().select(fails, f1, u0);
            b.f.ins().select(passes, p2, other)
        },
    )
}

/// `verdict_put(cache, test, sym, v)`: keeps verdict `v` (`PASSES` or `FAILS`) of `test` for `sym`; 1 when it is
/// kept, 0 when the cache had no room (the dead entry).
pub fn verdict_put(b: &mut B<'_, '_>) -> Value {
    let (test, sym, v) = (b.arg(0), b.arg(1), b.arg(2));
    let (at, dead, bit, _) = b.verdict_entry(test, sym);
    b.either(
        I32,
        dead,
        |b| b.c32(0),
        |b| {
            let passes = b.f.ins().icmp_imm(IntCC::Equal, v, i64::from(PASSES));
            let yes = b.c64(i64::from(YES));
            let no = b.c64(i64::from(NO));
            let off = b.f.ins().select(passes, yes, no);
            let p = b.f.ins().iadd(at, off);
            b.f.ins().atomic_rmw(I32, MemFlags::trusted(), AtomicRmwOp::Or, p, bit);
            b.c32(1)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{FAILS, PASSES, UNKNOWN};
    use crate::view::cache::Spec;
    use rerail_blocks::block::{Block, kind};
    use rerail_blocks::generated_reads::{verdict_get, verdict_put};

    /// Decision 0450 R3 (its host half): a verdict kept is found, by its test and symbol only; the cache stays within
    /// its generations, and a group read as others come is kept while those not read are let go (LRU).
    #[test]
    fn a_verdict_kept_is_found_and_the_cache_keeps_what_is_read() {
        let spec = Spec::new(3, 4, 32);
        let block = Block::anon(kind::SCRATCH, spec.bytes() + 4096);
        let at = block.alloc(spec.bytes(), 64);
        // SAFETY: the cache's bytes, in the block, read by no one else.
        let buf = unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) };
        spec.init(buf).unwrap();
        let cache = block.ptr::<u8>(at).cast_const();
        // SAFETY: a cache `init` made, of `spec`'s size, in a live block.
        let get = |t: u32, s: u32| unsafe { verdict_get(cache, t, s) };
        let put = |t: u32, s: u32, v: u32| unsafe { verdict_put(cache, t, s, v) };
        assert_eq!(get(7, 5), UNKNOWN);
        assert_eq!(put(7, 5, PASSES), 1);
        assert_eq!(put(7, 6, FAILS), 1);
        assert_eq!(put(7, 40, PASSES), 1);
        assert_eq!((get(7, 5), get(7, 6), get(7, 7), get(7, 40)), (PASSES, FAILS, UNKNOWN, PASSES));
        assert_eq!((get(8, 5), get(7, 69)), (UNKNOWN, UNKNOWN), "another test's, another group's");
        // Many groups pass through: group 0 of test 7, read between them, stays; group 100's, never read, goes.
        assert_eq!(put(7, 6400, PASSES), 1);
        for g in 1..200u32 {
            put(9, g * 64, PASSES);
            assert_eq!(get(7, 5), PASSES, "a group read as others come is kept (after {g})");
        }
        assert_eq!(get(7, 6400), UNKNOWN, "a group not read is let go");
        Spec::check(buf).unwrap();
    }
}
