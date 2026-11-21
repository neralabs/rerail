//! A text test's verdicts in the one rolling cache (decision 0478 step 2d), in CLIF for the kernels (the UKL backend:
//! `<ukl/reads/relation.h>`) and the JIT. The host's are written directly (`crate::cache`, decision 0517), held equal
//! to these by the tests (`the_rust_cache_is_the_jits`).
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

/// The masks' offsets in an entry, and the verdicts: the Rust's (`crate::cache`).
pub use crate::cache::{FAILS, NO, PASSES, UNKNOWN, YES};

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
        // An entry of its header and the two masks.
        let bytes = self.c32(i64::from(c::PAYLOAD) + 16);
        let e = cache_get(self, cache, key, bytes, &|b, e| {
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
    let cache = b.tape;
    let none = b.c32(0);
    crate::reads::cache_lookup(b, cache, key, none, None)
}

/// `text_find`, or on a miss an entry taken for a text of `len` bytes (`view::cache::TAKEN`): its caller writes the text
/// at `PAYLOAD`, then `cache_done`. A text past the cache's most is not kept (`NOT_KEPT`).
pub fn text_open(b: &mut B<'_, '_>) -> Value {
    let (fns, sym, len) = (b.arg(0), b.arg(1), b.arg(2));
    let key = text_key(b, fns, sym);
    let cache = b.tape;
    let size = b.f.ins().iadd_imm(len, i64::from(c::PAYLOAD));
    crate::reads::cache_lookup(b, cache, key, size, None)
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
    use crate::cache::{verdict_get, verdict_put};
    use rerail_blocks::block::{Block, kind};

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
        use crate::cache::{done as cache_done, text_find, text_open};
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

    /// Decision 0517 for the rolling cache: its reads written directly in Rust (`crate::cache`) do what its reads in
    /// CLIF do, by the JIT. Two caches of one shape, each in its own block at the same alignment, one read by the Rust
    /// and one by the JIT, are given the same steps (verdicts kept and read, texts found, taken, written and
    /// completed, one taken and left open while the generations turn, one past the most, finds of a size of 0) and
    /// answer the same each step, an entry by its place in its cache, and hold the same bytes after each.
    #[test]
    fn the_rust_cache_is_the_jits() {
        use crate::cache::{done as cache_done, text_find, text_open};
        use crate::view::cache::PAYLOAD;
        let jit = crate::jit::Reads::new();
        type Get = extern "C" fn(*const u8, u32, u32) -> u32;
        type Put = extern "C" fn(*const u8, u32, u32, u32) -> u32;
        type Find = extern "C" fn(*const u8, u32, u32) -> u64;
        type Open = extern "C" fn(*const u8, u32, u32, u32) -> u64;
        type Done = extern "C" fn(*const u8, u64) -> u32;
        // SAFETY: each read's signature: the cache, then its arguments.
        let (jget, jput): (Get, Put) = unsafe { (std::mem::transmute(jit.function("verdict_get")), std::mem::transmute(jit.function("verdict_put"))) };
        // SAFETY: as above.
        let (jfind, jopen, jdone): (Find, Open, Done) =
            unsafe { (std::mem::transmute(jit.function("text_find")), std::mem::transmute(jit.function("text_open")), std::mem::transmute(jit.function("cache_done"))) };
        let spec = Spec::arena(3, 16, 1024, 256);
        let make = || {
            let block = Block::anon(kind::SCRATCH, spec.bytes() + 4096);
            let at = block.alloc(spec.bytes(), 256);
            // SAFETY: the cache's bytes, in the block, read by no one else.
            spec.init(unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) }).unwrap();
            (block, at)
        };
        let ((rb, ra), (jb, ja)) = (make(), make());
        let (rust, jits) = (rb.ptr::<u8>(ra).cast_const(), jb.ptr::<u8>(ja).cast_const());
        // SAFETY: each cache's bytes, in its live block.
        let bytes = |c: *const u8| unsafe { std::slice::from_raw_parts(c, spec.bytes()) }.to_vec();
        // An answer by its place: an entry's offset in its cache, with what was found in its low bits.
        let place = |c: *const u8, r: u64| if r == 0 { 0 } else { r - c as u64 };
        let mut steps = 0;
        let mut same = |what: String, r: u64, j: u64| {
            assert_eq!(r, j, "{what}: the Rust's answer, the JIT's");
            assert!(bytes(rust) == bytes(jits), "{what}: the caches' bytes differ after it");
            steps += 1;
        };
        // SAFETY (each step): the reads of a cache `init` made, in a live block; an entry taken is written in its own
        // bytes alone, then completed.
        unsafe {
            for (t, s, v) in [(7, 5, PASSES), (7, 6, FAILS), (7, 40, PASSES), (8, 5, FAILS), (7, 6400, PASSES)] {
                same(format!("put {t} {s}"), u64::from(verdict_put(rust, t, s, v)), u64::from(jput(jits, t, s, v)));
            }
            for (t, s) in [(7, 5), (7, 6), (7, 7), (7, 40), (8, 5), (7, 69), (9, 1)] {
                same(format!("get {t} {s}"), u64::from(verdict_get(rust, t, s)), u64::from(jget(jits, t, s)));
            }
            let put = |r: u64, n: usize, done: &dyn Fn(u64)| {
                if r & 3 == crate::view::cache::TAKEN as u64 {
                    std::ptr::write_bytes(((r & !3) as *mut u8).add(PAYLOAD as usize), b'a' + (n % 26) as u8, n);
                    done(r);
                }
            };
            for (k, n) in [0u32, 5, 40, 100, 200, 300, 0, 17].into_iter().enumerate() {
                let k = k as u32;
                same(format!("find {k}"), place(rust, text_find(rust, 1, k)), place(jits, jfind(jits, 1, k)));
                let (r, j) = (text_open(rust, 1, k, n), jopen(jits, 1, k, n));
                same(format!("open {k} of {n}"), place(rust, r), place(jits, j));
                put(r, n as usize, &|r| cache_done(rust, r));
                put(j, n as usize, &|j| {
                    jdone(jits, j);
                });
                same(format!("done {k}"), 0, 0);
                same(format!("found {k}"), place(rust, text_find(rust, 1, k)), place(jits, jfind(jits, 1, k)));
            }
            // One left open while texts of 200 bytes turn the generations, a verdict and a text read between them.
            let (w, wj) = (text_open(rust, 6, 1, 200), jopen(jits, 6, 1, 200));
            same("open, held".into(), place(rust, w), place(jits, wj));
            for s in 0..60u32 {
                let (r, j) = (text_open(rust, 5, s, 200), jopen(jits, 5, s, 200));
                same(format!("long {s}"), place(rust, r), place(jits, j));
                put(r, 200, &|r| cache_done(rust, r));
                put(j, 200, &|j| {
                    jdone(jits, j);
                });
                same(format!("get between {s}"), u64::from(verdict_get(rust, 7, 5)), u64::from(jget(jits, 7, 5)));
                same(format!("found between {s}"), place(rust, text_find(rust, 1, 1)), place(jits, jfind(jits, 1, 1)));
                if s == 30 {
                    put(w, 200, &|r| cache_done(rust, r));
                    put(wj, 200, &|j| {
                        jdone(jits, j);
                    });
                    same("the held one done".into(), 0, 0);
                }
            }
        }
        // 5 puts, 7 gets, 4 steps for each of 8 texts, the held one's open, 3 steps for each of 60 long ones, its done.
        assert_eq!(steps, 5 + 7 + 8 * 4 + 1 + 60 * 3 + 1, "every step compared");
        Spec::check(&mut bytes(rust)).unwrap();
    }
}
