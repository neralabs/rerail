//! A text test's verdicts in the one rolling cache (decision 0478 step 2d), written once in CLIF for the host (the
//! blocks crate's `generated_reads.rs`), the kernels (`<ukl/reads/relation.h>`) and the JIT.
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

/// `verdict_entry(cache, test, sym)`: the entry of `test`'s 64 symbols about `sym`, made (both masks empty) when none
/// is; the dead entry when the cache has no room. A read of its own, which `verdict_get` and `verdict_put` call: the
/// cache's lookup is written once, not into each of them.
pub fn verdict_entry(b: &mut B<'_, '_>) -> Value {
    let (test, sym) = (b.arg(0), b.arg(1));
    let cache = b.tape;
    let t = b.f.ins().uextend(I64, test);
    let t = b.f.ins().iadd_imm(t, 1);
    let t = b.f.ins().ishl_imm(t, 32);
    let group = b.f.ins().ushr_imm(sym, 6);
    let group = b.f.ins().uextend(I64, group);
    let key = b.f.ins().bor(t, group);
    // An entry of its header and the two masks.
    let bytes = b.c32(i64::from(c::PAYLOAD) + 16);
    cache_get(b, cache, key, bytes, &|b, e| {
        let zero = b.c64(0);
        let rw = MemFlags::new().with_notrap();
        b.f.ins().store(rw, zero, e, YES);
        b.f.ins().store(rw, zero, e, NO);
        b.c32(1)
    })
}

/// `cache_open(cache, key, size)`: the cache's open lookup of `key` (`reads::cache_lookup` with no fill), taking an
/// entry of `size` bytes on a miss (none for 0); the entry and what was found in its low bits. A read of its own,
/// which `text_find` and `text_open` call.
pub fn cache_open(b: &mut B<'_, '_>) -> Value {
    let (key, size) = (b.arg(0), b.arg(1));
    let cache = b.tape;
    crate::reads::cache_lookup(b, cache, key, size, None)
}

impl B<'_, '_> {
    /// The entry of `test`'s 64 symbols about `sym` (`verdict_entry`), whether it is the dead entry, and the offset of
    /// `sym`'s word in a mask from the mask, and its bit.
    fn verdict_entry(&mut self, test: Value, sym: Value) -> (Value, Value, Value, Value) {
        let cache = self.tape;
        let e = self.call("verdict_entry", &[test, sym]);
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

/// A text's key: bit 63 set (no verdict's key has it), the functions' id plus one, the symbol.
fn text_key(b: &mut B<'_, '_>, fns: Value, sym: Value) -> Value {
    let f = b.f.ins().uextend(I64, fns);
    let f = b.f.ins().iadd_imm(f, 1);
    let f = b.f.ins().ishl_imm(f, 32);
    let f = b.f.ins().bor_imm(f, i64::MIN);
    let s = b.f.ins().uextend(I64, sym);
    b.f.ins().bor(f, s)
}

/// The text of functions `fns` of symbol `sym` kept in the cache (the descriptor): its entry (its bytes `LEN` less
/// `PAYLOAD`, at `PAYLOAD`), with what was found in the low bits (`view::cache::FOUND`, `NOT_KEPT`, `PENDING`).
pub fn text_find(b: &mut B<'_, '_>) -> Value {
    let (fns, sym) = (b.arg(0), b.arg(1));
    let key = text_key(b, fns, sym);
    let none = b.c32(0);
    b.call("cache_open", &[key, none])
}

/// `text_find`, or on a miss an entry taken for a text of `len` bytes (`view::cache::TAKEN`): its caller writes the text
/// at `PAYLOAD`, then `cache_done`. A text past the cache's most is not kept (`NOT_KEPT`).
pub fn text_open(b: &mut B<'_, '_>) -> Value {
    let (fns, sym, len) = (b.arg(0), b.arg(1), b.arg(2));
    let key = text_key(b, fns, sym);
    let size = b.f.ins().iadd_imm(len, i64::from(c::PAYLOAD));
    b.call("cache_open", &[key, size])
}

/// An entry `text_open` took (`TAKEN`'s bit cleared or not), written: completed.
pub fn cache_done(b: &mut B<'_, '_>) -> Value {
    let e = b.arg(0);
    let e = b.f.ins().band_imm(e, !3);
    let cache = b.tape;
    crate::reads::cache_done(b, cache, e);
    b.c32(1)
}

#[cfg(test)]
mod tests {
    use super::{FAILS, PASSES, UNKNOWN};
    use crate::view::cache::Spec;
    use rerail_blocks::block::{Block, kind};
    use rerail_blocks::generated_reads::{verdict_get, verdict_put};

    /// Decision 0478 R3 (its host half): a verdict kept is found, by its test and symbol only; the cache stays within
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

    /// Decision 0478 step 2e: entries of any size up to the cache's most, from each generation's bump arena. A text
    /// is found, or taken for its length, written and completed; texts of different lengths lie apart; one past the
    /// most is not kept and turns no generation; a text being written is pending to others; a generation full by
    /// its bytes turns, and a text read as others come is kept while one not read goes; a generation with a writer
    /// still in it is not cleared under it, and is cleared once its writer is done.
    #[test]
    fn texts_of_any_length_are_kept_in_the_arenas_by_bytes() {
        use crate::view::cache::{FOUND, NOT_KEPT, PENDING, TAKEN};
        use crate::view::cache::{Counts, LEN, PAYLOAD};
        use rerail_blocks::generated_reads::{cache_done, text_find, text_open};
        let spec = Spec::arena(3, 16, 1024, 256);
        let block = Block::anon(kind::SCRATCH, spec.bytes() + 4096);
        let at = block.alloc(spec.bytes(), 256);
        // SAFETY: the cache's bytes, in the block, read by no one else.
        let buf = unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) };
        spec.init(buf).unwrap();
        let cache = block.ptr::<u8>(at).cast_const();
        let state = |r: u64| (r & 3) as i64;
        let entry = |r: u64| (r & !3) as *mut u8;
        // SAFETY (each closure): reads of a cache `init` made, in a live block; an entry taken is written in its
        // own bytes only.
        let find = |f: u32, s: u32| unsafe { text_find(cache, f, s) };
        let open = |f: u32, s: u32, n: u32| unsafe { text_open(cache, f, s, n) };
        let put = |f: u32, s: u32, text: &[u8]| {
            let r = open(f, s, text.len() as u32);
            assert_eq!(state(r), TAKEN, "{f}:{s} taken");
            unsafe {
                std::ptr::copy_nonoverlapping(text.as_ptr(), entry(r).add(PAYLOAD as usize), text.len());
                cache_done(cache, r);
            }
            entry(r)
        };
        let read = |f: u32, s: u32| -> Option<Vec<u8>> {
            let r = find(f, s);
            (state(r) == FOUND).then(|| unsafe {
                let e = entry(r);
                let len = e.add(LEN as usize).cast::<u32>().read() as usize - PAYLOAD as usize;
                std::slice::from_raw_parts(e.add(PAYLOAD as usize), len).to_vec()
            })
        };
        let counts = || unsafe { Counts::read(cache) };
        assert_eq!(state(find(1, 10)), NOT_KEPT, "a miss of a find takes nothing");
        put(1, 10, b"hello");
        assert_eq!(read(1, 10).as_deref(), Some(&b"hello"[..]));
        // Texts of different lengths, the empty one among them, lie apart, aligned.
        let texts: Vec<Vec<u8>> = [0usize, 40, 100, 200].iter().map(|&n| (0..n).map(|k| b'a' + (k % 26) as u8).collect()).collect();
        let mut at: Vec<(usize, usize)> = texts.iter().enumerate().map(|(k, t)| (put(2, k as u32, t) as usize, PAYLOAD as usize + t.len())).collect();
        for (k, t) in texts.iter().enumerate() {
            assert_eq!(read(2, k as u32).as_deref(), Some(&t[..]), "text {k}");
        }
        at.sort_unstable();
        assert!(at.iter().all(|(a, _)| a % 64 == 0) && at.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0), "apart and aligned: {at:?}");
        // Past the most: not kept, nothing turned.
        let rotated = counts().rotated;
        assert_eq!(state(open(3, 1, 300)), NOT_KEPT);
        assert_eq!(counts().rotated, rotated, "a text past the most turns no generation");
        // Being written: pending to a find and to another open, found once completed.
        let r = open(4, 1, 8);
        assert_eq!(state(r), TAKEN);
        assert_eq!((state(find(4, 1)), state(open(4, 1, 8))), (PENDING, PENDING));
        unsafe { cache_done(cache, r) };
        assert_eq!(state(find(4, 1)), FOUND);
        // Full by bytes: texts of 200 bytes (four to an arena) turn the generations; text (1, 10), read between
        // them, stays; text (2, 3), never read again, goes.
        let long = vec![b'z'; 200];
        for s in 0..40u32 {
            put(5, s, &long);
            assert_eq!(read(1, 10).as_deref(), Some(&b"hello"[..]), "a text read as others come is kept (after {s})");
        }
        assert!(counts().rotated >= rotated + 5, "{:?}", counts());
        assert_eq!(read(2, 3), None, "a text not read is let go");
        // A writer still in a generation: its generation is not cleared under it. Taken, not completed, it holds
        // its generation when the head comes round to it; the head then finds no room until it completes.
        let w = open(6, 1, 200);
        assert_eq!(state(w), TAKEN);
        let mut s = 0u32;
        let failed = counts().failed;
        while counts().failed == failed && s < 100 {
            let r = open(7, s, 200);
            if state(r) == TAKEN {
                unsafe { cache_done(cache, r) };
            }
            s += 1;
        }
        assert!(counts().failed > failed && s < 100, "no room while the writer's generation is held: {:?}", counts());
        // SAFETY: the taken entry's own bytes: the generation was not cleared, so they are still its alone.
        unsafe {
            std::ptr::write_bytes(entry(w).add(PAYLOAD as usize), b'w', 200);
            cache_done(cache, w);
        }
        assert_eq!(state(open(8, 1, 200)), TAKEN, "its writer done, its generation cleared and the head turned");
        Spec::check(buf).unwrap();
    }
}
