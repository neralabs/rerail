//! A projection's numbering's reads as CLIF functions, written once (decision 0353 step 2): what the host's `Ranks`
//! and `granules::Table` answer, for the kernels (the MSL backend: `id_reads.metal`) and the JIT. The host's are
//! written directly (`ids::Numbering::is`, …; decision 0517), held equal to these by the facts crate's tests.
//!
//! A read takes the descriptor's address (`ids::Numbering`) and reaches each column through its run, `off` bytes from
//! the run's own address (a relation's reads' family: every load aligned, a word of the block). An item is a numbered
//! node (0), a token (1) or a node under a field (2); a number is an item's rank among its kind.
//!
//! - `num_is`, `num_rank`, `num_select`: the rank index's bit, rank and select.
//! - `num_low_of`, `num_number_of`: an item's id's low 32 bits from its number, and back.
//! - `num_order_of`, `num_number_of_order`: a node's order key from its number, and back.

use crate::ids::{INDEX_BITS, JUMP, ORDER_INDEX_BITS, SLOT_BITS, WORDS, at};
use crate::reads::{B, NONE};
use cranelift_codegen::ir::{
    InstBuilder, Value,
    condcodes::IntCC,
    types::{I32, I64},
};

/// No granule (`num_granule`), no number (`num_number_of`).
const NONE64: i64 = -1;
/// No stretch to check (`num_number_in`).
const ANY: i64 = u32::MAX as i64;

impl B<'_, '_> {
    /// The address of the items of the run at `run` (an address).
    fn items_at(&mut self, run: Value) -> Value {
        let off = self.word_at(I64, run, at::RUN_OFF);
        self.f.ins().iadd(run, off)
    }
    /// The run at descriptor offset `at` plus `k` runs' (`k` a u32): its items' address.
    fn run_k(&mut self, offset: i32, k: Value) -> Value {
        let base = self.f.ins().iadd_imm(self.tape, i64::from(offset));
        let run = self.item(base, k, i64::from(at::RUN));
        self.items_at(run)
    }
    /// The u32 at index `i` of the u32s at `items`.
    fn u32_of(&mut self, items: Value, i: Value) -> Value {
        let p = self.item(items, i, 4);
        self.word_at(I32, p, 0)
    }
    /// Index `i` of the descriptor's column at `offset` (u32s).
    fn column_u32(&mut self, offset: i32, i: Value) -> Value {
        let items = self.follow(self.tape, offset);
        self.u32_of(items, i)
    }
    /// The descriptor's u32 at `offset`.
    fn field(&mut self, offset: i32) -> Value {
        self.word_at(I32, self.tape, offset)
    }
    /// The length of the descriptor's run at `offset`, as a u32.
    fn run_len(&mut self, offset: i32) -> Value {
        let len = self.word_at(I64, self.tape, offset + at::RUN_LEN);
        self.f.ins().ireduce(I32, len)
    }
    /// Word `w` of rank group `g` (both u32s).
    fn group_word(&mut self, g: Value, w: Value) -> Value {
        let groups = self.follow(self.tape, at::GROUPS);
        let group = self.item(groups, g, 8 * i64::from(WORDS));
        let word = self.item(group, w, 8);
        self.word_at(I64, word, 0)
    }
    /// `item`'s count before group `g`: word 3's low or high half (nodes, tokens), word 4's low (fielded).
    fn before(&mut self, g: Value, item: Value) -> Value {
        let half = self.f.ins().ushr_imm(item, 1);
        let w = self.f.ins().iadd_imm(half, i64::from(super::ids::ITEMS));
        let word = self.group_word(g, w);
        let odd = self.f.ins().band_imm(item, 1);
        let shift = self.f.ins().imul_imm(odd, 32);
        let shift = self.f.ins().uextend(I64, shift);
        let v = self.f.ins().ushr(word, shift);
        self.f.ins().ireduce(I32, v)
    }
    /// `item`'s count.
    fn count_of(&mut self, item: Value) -> Value {
        let p = self.item(self.tape, item, 4);
        self.word_at(I32, p, at::COUNT)
    }
    /// The length of the run at descriptor offset `offset` plus `k` runs' (`k` a u32), as a u32.
    fn run_k_len(&mut self, offset: i32, k: Value) -> Value {
        let base = self.f.ins().iadd_imm(self.tape, i64::from(offset));
        let run = self.item(base, k, i64::from(at::RUN));
        let len = self.word_at(I64, run, at::RUN_LEN);
        self.f.ins().ireduce(I32, len)
    }
    /// The first `i` in `0..n` (u32s) for which `below(i)` is false, `below` true before it and false from it.
    pub(crate) fn bisect(&mut self, n: Value, below: &mut dyn FnMut(&mut Self, Value) -> Value) -> Value {
        let zero = self.c32(0);
        self.bisect_in(zero, n, below)
    }
    /// `bisect` within `lo..hi`: the first `i` there for which `below(i)` is false (`hi` where none is).
    fn bisect_in(&mut self, lo: Value, hi: Value, below: &mut dyn FnMut(&mut Self, Value) -> Value) -> Value {
        let (head, body, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(head, I32);
        self.f.append_block_param(done, I32);
        self.f.ins().jump(head, &[lo.into(), hi.into()]);
        self.f.switch_to_block(head);
        let (lo, hi) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
        self.f.ins().brif(more, body, &[], done, &[lo.into()]);
        self.f.switch_to_block(body);
        let sum = self.f.ins().iadd(lo, hi);
        let middle = self.f.ins().ushr_imm(sum, 1);
        let yes = below(self, middle);
        let next = self.f.ins().iadd_imm(middle, 1);
        let lo = self.f.ins().select(yes, next, lo);
        let hi = self.f.ins().select(yes, hi, middle);
        self.f.ins().jump(head, &[lo.into(), hi.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// `item` number `k`'s index in granule `g` (`j` the last granule starting at or before it).
    fn index_in(&mut self, item: Value, k: Value, g: Value, j: Value) -> Value {
        let starts = self.run_k(at::RANK_STARTS, item);
        let start = self.u32_of(starts, g);
        let own = self.f.ins().isub(k, start);
        let (g1, j1) = (self.f.ins().iadd_imm(g, 1), self.f.ins().iadd_imm(j, 1));
        let ng = self.column_u32(at::NODES, g);
        let ng1 = self.column_u32(at::NODES, g1);
        let nj1 = self.column_u32(at::NODES, j1);
        let nested_nodes = self.f.ins().isub(nj1, ng1);
        let as_node = self.f.ins().isub(own, nested_nodes);
        let tg1 = self.column_u32(at::TOKENS, g1);
        let tj1 = self.column_u32(at::TOKENS, j1);
        let nested_tokens = self.f.ins().isub(tj1, tg1);
        let own_nodes = self.f.ins().isub(ng1, ng);
        let as_token = self.f.ins().iadd(own_nodes, own);
        let as_token = self.f.ins().isub(as_token, nested_tokens);
        let node = self.f.ins().icmp_imm(IntCC::Equal, item, 0);
        self.f.ins().select(node, as_node, as_token)
    }
    /// Granule `g`'s item `index`'s low bits: its slot (`slots[at[g] + index >> INDEX_BITS]`), then its index's low
    /// bits.
    fn low_in(&mut self, g: Value, index: Value) -> Value {
        let first = self.column_u32(at::AT, g);
        let continuation = self.f.ins().ushr_imm(index, i64::from(INDEX_BITS));
        let k = self.f.ins().iadd(first, continuation);
        let slot = self.column_u32(at::SLOTS, k);
        let high = self.f.ins().ishl_imm(slot, i64::from(INDEX_BITS));
        let low = self.f.ins().band_imm(index, (1i64 << INDEX_BITS) - 1);
        self.f.ins().bor(high, low)
    }
    /// The slot of entry `i` of the `[slot, granule, continuation]`s at `by_slot`.
    fn slot_at(&mut self, by_slot: Value, i: Value) -> Value {
        let p = self.item(by_slot, i, 12);
        self.word_at(I32, p, 0)
    }
    /// Word `w` (0: the granule; 1, 2: nodes, tokens before it) of child granule entry `k` (u32s).
    fn kid(&mut self, k: Value, w: Value) -> Value {
        let kids = self.follow(self.tape, at::KIDS);
        let kid = self.item(kids, k, 12);
        let p = self.item(kid, w, 4);
        self.word_at(I32, p, 0)
    }
}

/// The position of `m`'s set bit `k` (a u64 and a u32; `k` below its popcount): the byte by the bytes' running
/// popcounts, then the bit within it (`ranks::select64`).
fn select64(b: &mut B<'_, '_>, m: Value, k: Value) -> Value {
    const ONES: i64 = 0x0101_0101_0101_0101;
    let half = b.f.ins().ushr_imm(m, 1);
    let half = b.f.ins().band_imm(half, 0x5555_5555_5555_5555);
    let x = b.f.ins().isub(m, half);
    let lo = b.f.ins().band_imm(x, 0x3333_3333_3333_3333);
    let hi = b.f.ins().ushr_imm(x, 2);
    let hi = b.f.ins().band_imm(hi, 0x3333_3333_3333_3333);
    let x = b.f.ins().iadd(lo, hi);
    let shifted = b.f.ins().ushr_imm(x, 4);
    let x = b.f.ins().iadd(x, shifted);
    let x = b.f.ins().band_imm(x, 0x0f0f_0f0f_0f0f_0f0f);
    // Byte b of `sums`: the set bits of bytes 0..=b.
    let sums = b.f.ins().imul_imm(x, ONES);
    // The bytes whose running count is at most `k` come before the bit's byte: one high bit for each.
    let k64 = b.f.ins().uextend(I64, k);
    let k8 = b.f.ins().imul_imm(k64, ONES);
    let high = b.f.ins().bor_imm(k8, 0x8080_8080_8080_8080_u64 as i64);
    let le = b.f.ins().isub(high, sums);
    let le = b.f.ins().band_imm(le, 0x8080_8080_8080_8080_u64 as i64);
    let byte = b.f.ins().popcnt(le);
    let first = b.f.ins().icmp_imm(IntCC::Equal, byte, 0);
    let prev = b.f.ins().iadd_imm(byte, -1);
    let prev = b.f.ins().imul_imm(prev, 8);
    let before = b.f.ins().ushr(sums, prev);
    let before = b.f.ins().band_imm(before, 0xff);
    let zero = b.c64(0);
    let before = b.f.ins().select(first, zero, before);
    let at = b.f.ins().imul_imm(byte, 8);
    let bits = b.f.ins().ushr(m, at);
    let bits = b.f.ins().band_imm(bits, 0xff);
    // Clear the byte's lower set bits: at most 7.
    let left = b.f.ins().isub(k64, before);
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I64);
    b.f.append_block_param(head, I64);
    b.f.append_block_param(done, I64);
    b.f.ins().jump(head, &[bits.into(), left.into()]);
    b.f.switch_to_block(head);
    let (bits, left) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp_imm(IntCC::NotEqual, left, 0);
    b.f.ins().brif(more, body, &[], done, &[bits.into()]);
    b.f.switch_to_block(body);
    let less = b.f.ins().iadd_imm(bits, -1);
    let bits = b.f.ins().band(bits, less);
    let left = b.f.ins().iadd_imm(left, -1);
    b.f.ins().jump(head, &[bits.into(), left.into()]);
    b.f.switch_to_block(done);
    let bits = b.f.block_params(done)[0];
    let bit = b.f.ins().ctz(bits);
    let p = b.f.ins().iadd(at, bit);
    b.f.ins().ireduce(I32, p)
}

/// `num_is(item, x)`: whether tape node `x` is a numbered `item` (1 or 0).
pub fn is(b: &mut B<'_, '_>) -> Value {
    let (item, x) = (b.arg(0), b.arg(1));
    let gx = b.f.ins().ushr_imm(x, 6);
    let first = b.field(at::FIRST);
    let len = b.run_len(at::GROUPS);
    let g = b.f.ins().isub(gx, first);
    let after = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, gx, first);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, g, len);
    let ok = b.f.ins().band(after, inside);
    b.either(
        I32,
        ok,
        |b| {
            let bits = b.group_word(g, item);
            let bit = b.f.ins().band_imm(x, 63);
            let bit = b.f.ins().uextend(I64, bit);
            let v = b.f.ins().ushr(bits, bit);
            let v = b.f.ins().band_imm(v, 1);
            b.f.ins().ireduce(I32, v)
        },
        |b| b.c32(0),
    )
}

/// `num_rank(item, x)`: the numbered `item`s before tape node `x`.
pub fn rank(b: &mut B<'_, '_>) -> Value {
    let (item, x) = (b.arg(0), b.arg(1));
    let gx = b.f.ins().ushr_imm(x, 6);
    let first = b.field(at::FIRST);
    let below = b.f.ins().icmp(IntCC::UnsignedLessThan, gx, first);
    b.either(
        I32,
        below,
        |b| b.c32(0),
        |b| {
            let g = b.f.ins().isub(gx, first);
            let len = b.run_len(at::GROUPS);
            let past = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, g, len);
            b.either(
                I32,
                past,
                |b| b.count_of(item),
                |b| {
                    let before = b.before(g, item);
                    let bits = b.group_word(g, item);
                    let bit = b.f.ins().band_imm(x, 63);
                    let bit = b.f.ins().uextend(I64, bit);
                    let one = b.c64(1);
                    let mask = b.f.ins().ishl(one, bit);
                    let mask = b.f.ins().iadd_imm(mask, -1);
                    let lower = b.f.ins().band(bits, mask);
                    let n = b.f.ins().popcnt(lower);
                    let n = b.f.ins().ireduce(I32, n);
                    b.f.ins().iadd(before, n)
                },
            )
        },
    )
}

/// `num_select(item, r)`: the tape node of `item` number `r` (`NONE` past the last): its sample's group, forward.
pub fn select(b: &mut B<'_, '_>) -> Value {
    let (item, r) = (b.arg(0), b.arg(1));
    let count = b.count_of(item);
    let past = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, r, count);
    b.either(
        I32,
        past,
        |b| b.c32(NONE),
        |b| {
            let samples = b.run_k(at::SAMPLES, item);
            let s = b.f.ins().ushr_imm(r, 6);
            let g0 = b.u32_of(samples, s);
            let (head, done) = (b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I32);
            b.f.append_block_param(done, I32);
            b.f.append_block_param(done, I32);
            b.f.append_block_param(done, I64);
            b.f.ins().jump(head, &[g0.into()]);
            b.f.switch_to_block(head);
            let g = b.f.block_params(head)[0];
            let m = b.group_word(g, item);
            let before = b.before(g, item);
            let k = b.f.ins().isub(r, before);
            let n = b.f.ins().popcnt(m);
            let n = b.f.ins().ireduce(I32, n);
            let here = b.f.ins().icmp(IntCC::UnsignedLessThan, k, n);
            let next = b.f.ins().iadd_imm(g, 1);
            let on = b.f.create_block();
            b.f.ins().brif(here, done, &[g.into(), k.into(), m.into()], on, &[]);
            b.f.switch_to_block(on);
            b.f.ins().jump(head, &[next.into()]);
            b.f.switch_to_block(done);
            let (g, k, m) = (b.f.block_params(done)[0], b.f.block_params(done)[1], b.f.block_params(done)[2]);
            let first = b.field(at::FIRST);
            let group = b.f.ins().iadd(first, g);
            let base = b.f.ins().imul_imm(group, 64);
            let bit = select64(b, m, k);
            b.f.ins().iadd(base, bit)
        },
    )
}

/// `num_granule(item, k)`: the innermost granule holding `item` number `k` (0 or 1), and the last granule starting at
/// or before it, as `g << 32 | j` (`NONE64` for none): a bisection of the starts' ranks between the jumps either side
/// of `k` (`Numbering::jumps`), then up the parents.
pub fn granule(b: &mut B<'_, '_>) -> Value {
    const _: () = assert!(JUMP.is_power_of_two());
    let (item, k) = (b.arg(0), b.arg(1));
    let starts = b.run_k(at::RANK_STARTS, item);
    let n = b.field(at::GRANULES);
    let jumps = b.run_k(at::JUMPS, item);
    let held = b.run_k_len(at::JUMPS, item);
    let q = b.f.ins().ushr_imm(k, i64::from(JUMP.trailing_zeros()));
    let q1 = b.f.ins().iadd_imm(q, 1);
    let has_lo = b.f.ins().icmp(IntCC::UnsignedLessThan, q, held);
    let lo = b.either(I32, has_lo, |b| b.u32_of(jumps, q), |b| b.c32(0));
    let has_hi = b.f.ins().icmp(IntCC::UnsignedLessThan, q1, held);
    let hi = b.either(
        I32,
        has_hi,
        |b| {
            let j = b.u32_of(jumps, q1);
            let j = b.f.ins().iadd_imm(j, 1);
            b.f.ins().umin(j, n)
        },
        |_| n,
    );
    let after = b.bisect_in(lo, hi, &mut |b, i| {
        let s = b.u32_of(starts, i);
        b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, s, k)
    });
    let none = b.f.ins().icmp_imm(IntCC::Equal, after, 0);
    b.either(
        I64,
        none,
        |b| b.c64(NONE64),
        |b| {
            let j = b.f.ins().iadd_imm(after, -1);
            let ends = b.run_k(at::RANK_ENDS, item);
            let (head, up, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I32);
            b.f.append_block_param(up, I32);
            b.f.append_block_param(done, I64);
            b.f.ins().jump(head, &[j.into()]);
            b.f.switch_to_block(head);
            let g = b.f.block_params(head)[0];
            let end = b.u32_of(ends, g);
            let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, k, end);
            let g64 = b.f.ins().uextend(I64, g);
            let g64 = b.f.ins().ishl_imm(g64, 32);
            let j64 = b.f.ins().uextend(I64, j);
            let found = b.f.ins().bor(g64, j64);
            b.f.ins().brif(inside, done, &[found.into()], up, &[g.into()]);
            b.f.switch_to_block(up);
            let g = b.f.block_params(up)[0];
            let q = b.column_u32(at::PARENTS, g);
            let root = b.f.ins().icmp_imm(IntCC::Equal, q, NONE);
            let missing = b.c64(NONE64);
            b.f.ins().brif(root, done, &[missing.into()], head, &[q.into()]);
            b.f.switch_to_block(done);
            b.f.block_params(done)[0]
        },
    )
}

/// `num_low_of(item, k)`: the low 32 bits of `item` number `k`'s id (a node, 0, or a token, 1), `NONE` for none.
pub fn low_of(b: &mut B<'_, '_>) -> Value {
    let (item, k) = (b.arg(0), b.arg(1));
    let gj = b.call("num_granule", &[item, k]);
    let none = b.f.ins().icmp_imm(IntCC::Equal, gj, NONE64);
    b.either(
        I32,
        none,
        |b| b.c32(NONE),
        |b| {
            let g = b.f.ins().ushr_imm(gj, 32);
            let g = b.f.ins().ireduce(I32, g);
            let j = b.f.ins().ireduce(I32, gj);
            let index = b.index_in(item, k, g, j);
            b.low_in(g, index)
        },
    )
}

/// `num_number_in(g, item, k, stretch)`: the number of granule `g`'s own `item` `k` (its `k`-th, in order), `NONE`
/// when `stretch` is not `u32::MAX` and the item does not lie in that stretch of `g` (after `stretch` of its child
/// granules).
pub fn number_in(b: &mut B<'_, '_>) -> Value {
    let (g, item, k, stretch) = (b.arg(0), b.arg(1), b.arg(2), b.arg(3));
    let a = b.column_u32(at::FIRST_KID, g);
    let g1 = b.f.ins().iadd_imm(g, 1);
    let e = b.column_u32(at::FIRST_KID, g1);
    let n = b.f.ins().isub(e, a);
    let w = b.f.ins().iadd_imm(item, 1);
    let i = b.bisect(n, &mut |b, t| {
        let entry = b.f.ins().iadd(a, t);
        let before = b.kid(entry, w);
        b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, before, k)
    });
    let any = b.f.ins().icmp_imm(IntCC::Equal, stretch, ANY);
    let same = b.f.ins().icmp(IntCC::Equal, stretch, i);
    let fits = b.f.ins().bor(any, same);
    b.either(
        I32,
        fits,
        |b| {
            let first = b.f.ins().icmp_imm(IntCC::Equal, i, 0);
            b.either(
                I32,
                first,
                |b| {
                    let starts = b.run_k(at::RANK_STARTS, item);
                    let from = b.u32_of(starts, g);
                    b.f.ins().iadd(from, k)
                },
                |b| {
                    let entry = b.f.ins().iadd(a, i);
                    let entry = b.f.ins().iadd_imm(entry, -1);
                    let zero = b.c32(0);
                    let kid = b.kid(entry, zero);
                    let before = b.kid(entry, w);
                    let ends = b.run_k(at::RANK_ENDS, item);
                    let from = b.u32_of(ends, kid);
                    let at = b.f.ins().iadd(from, k);
                    b.f.ins().isub(at, before)
                },
            )
        },
        |b| b.c32(NONE),
    )
}

/// `num_number_of(low)`: the item whose id's low 32 bits are `low`, as `item << 32 | number` (`NONE64` for none):
/// the slot's entry, then its index among the granule's nodes or tokens. The slots are hashes, about evenly spread:
/// the entry is near `slot`'s share of them, a few steps from a guess there; a run of clashes past `STEPS` is
/// bisected.
pub fn number_of(b: &mut B<'_, '_>) -> Value {
    const STEPS: i64 = 8;
    let low = b.arg(0);
    let slot = b.f.ins().ushr_imm(low, i64::from(INDEX_BITS));
    let by_slot = b.follow(b.tape, at::BY_SLOT);
    let n = b.run_len(at::BY_SLOT);
    // The guess: `slot`'s share of the entries, the last one at most.
    let (s64, n64) = (b.f.ins().uextend(I64, slot), b.f.ins().uextend(I64, n));
    let share = b.f.ins().imul(s64, n64);
    let share = b.f.ins().ushr_imm(share, i64::from(SLOT_BITS));
    let share = b.f.ins().ireduce(I32, share);
    let empty = b.f.ins().icmp_imm(IntCC::Equal, n, 0);
    let last = b.f.ins().iadd_imm(n, -1);
    let zero = b.c32(0);
    let last = b.f.ins().select(empty, zero, last);
    let guess = b.f.ins().umin(share, last);
    // Forward while the entry is below the slot, then back while the one before is not, `STEPS` in all.
    let (fwd, fwd_load, back, back_load, walked) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    for block in [fwd, back, walked] {
        b.f.append_block_param(block, I32);
        b.f.append_block_param(block, I32);
    }
    b.f.ins().jump(fwd, &[guess.into(), zero.into()]);
    b.f.switch_to_block(fwd);
    let (at_f, steps_f) = (b.f.block_params(fwd)[0], b.f.block_params(fwd)[1]);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, at_f, n);
    let budget = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, steps_f, STEPS);
    let go = b.f.ins().band(inside, budget);
    b.f.ins().brif(go, fwd_load, &[], back, &[at_f.into(), steps_f.into()]);
    b.f.switch_to_block(fwd_load);
    let s = b.slot_at(by_slot, at_f);
    let below = b.f.ins().icmp(IntCC::UnsignedLessThan, s, slot);
    let (at_n, steps_n) = (b.f.ins().iadd_imm(at_f, 1), b.f.ins().iadd_imm(steps_f, 1));
    b.f.ins().brif(below, fwd, &[at_n.into(), steps_n.into()], back, &[at_f.into(), steps_f.into()]);
    b.f.switch_to_block(back);
    let (at_b, steps_b) = (b.f.block_params(back)[0], b.f.block_params(back)[1]);
    let after_first = b.f.ins().icmp_imm(IntCC::NotEqual, at_b, 0);
    let budget = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, steps_b, STEPS);
    let go = b.f.ins().band(after_first, budget);
    b.f.ins().brif(go, back_load, &[], walked, &[at_b.into(), steps_b.into()]);
    b.f.switch_to_block(back_load);
    let before = b.f.ins().iadd_imm(at_b, -1);
    let s = b.slot_at(by_slot, before);
    let not_below = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, slot);
    let steps_n = b.f.ins().iadd_imm(steps_b, 1);
    b.f.ins().brif(not_below, back, &[before.into(), steps_n.into()], walked, &[at_b.into(), steps_b.into()]);
    b.f.switch_to_block(walked);
    let (at_w, steps_w) = (b.f.block_params(walked)[0], b.f.block_params(walked)[1]);
    let spent = b.f.ins().icmp_imm(IntCC::Equal, steps_w, STEPS);
    let at_ = b.either(
        I32,
        spent,
        |b| {
            b.bisect(n, &mut |b, i| {
                let s = b.slot_at(by_slot, i);
                b.f.ins().icmp(IntCC::UnsignedLessThan, s, slot)
            })
        },
        |_| at_w,
    );
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, at_, n);
    b.either(
        I64,
        inside,
        |b| {
            let p = b.item(by_slot, at_, 12);
            let s = b.word_at(I32, p, 0);
            let same = b.f.ins().icmp(IntCC::Equal, s, slot);
            b.either(
                I64,
                same,
                |b| {
                    let g = b.word_at(I32, p, 4);
                    let c = b.word_at(I32, p, 8);
                    let high = b.f.ins().ishl_imm(c, i64::from(INDEX_BITS));
                    let own_index = b.f.ins().band_imm(low, (1i64 << INDEX_BITS) - 1);
                    let index = b.f.ins().bor(high, own_index);
                    let g1 = b.f.ins().iadd_imm(g, 1);
                    let ng = b.column_u32(at::NODES, g);
                    let ng1 = b.column_u32(at::NODES, g1);
                    let own = b.f.ins().isub(ng1, ng);
                    let tg = b.column_u32(at::TOKENS, g);
                    let tg1 = b.column_u32(at::TOKENS, g1);
                    let tokens = b.f.ins().isub(tg1, tg);
                    let node = b.f.ins().icmp(IntCC::UnsignedLessThan, index, own);
                    let (zero, one) = (b.c32(0), b.c32(1));
                    let item = b.f.ins().select(node, zero, one);
                    let as_token = b.f.ins().isub(index, own);
                    let k = b.f.ins().select(node, index, as_token);
                    let all = b.f.ins().select(node, own, tokens);
                    let fits = b.f.ins().icmp(IntCC::UnsignedLessThan, k, all);
                    b.either(
                        I64,
                        fits,
                        |b| {
                            let any = b.c32(ANY);
                            let number = b.call("num_number_in", &[g, item, k, any]);
                            let item = b.f.ins().uextend(I64, item);
                            let item = b.f.ins().ishl_imm(item, 32);
                            let number = b.f.ins().uextend(I64, number);
                            b.f.ins().bor(item, number)
                        },
                        |b| b.c64(NONE64),
                    )
                },
                |b| b.c64(NONE64),
            )
        },
        |b| b.c64(NONE64),
    )
}

/// `num_order_of(l)`: node number `l`'s order key: its stretch's run rank, then its index in its granule (its number
/// itself where the keys are preorder indexes); `NONE` for none.
pub fn order_of(b: &mut B<'_, '_>) -> Value {
    let l = b.arg(0);
    let preorder = b.field(at::PREORDER_KEYS);
    b.either(
        I32,
        preorder,
        |_| l,
        |b| {
            let zero = b.c32(0);
            let gj = b.call("num_granule", &[zero, l]);
            let none = b.f.ins().icmp_imm(IntCC::Equal, gj, NONE64);
            b.either(
                I32,
                none,
                |b| b.c32(NONE),
                |b| {
                    let g = b.f.ins().ushr_imm(gj, 32);
                    let g = b.f.ins().ireduce(I32, g);
                    let j = b.f.ins().ireduce(I32, gj);
                    let zero = b.c32(0);
                    let index = b.index_in(zero, l, g, j);
                    let a = b.column_u32(at::FIRST_KID, g);
                    let g1 = b.f.ins().iadd_imm(g, 1);
                    let e = b.column_u32(at::FIRST_KID, g1);
                    let n = b.f.ins().isub(e, a);
                    let starts = b.run_k(at::RANK_STARTS, zero);
                    let s = b.bisect(n, &mut |b, t| {
                        let entry = b.f.ins().iadd(a, t);
                        let zero = b.c32(0);
                        let kid = b.kid(entry, zero);
                        let start = b.u32_of(starts, kid);
                        b.f.ins().icmp(IntCC::UnsignedLessThan, start, l)
                    });
                    let r = b.f.ins().iadd(a, g);
                    let r = b.f.ins().iadd(r, s);
                    let run = b.column_u32(at::RUNS, r);
                    let high = b.f.ins().ishl_imm(run, i64::from(ORDER_INDEX_BITS));
                    b.f.ins().bor(high, index)
                },
            )
        },
    )
}

/// `num_number_of_order(key)`: the node number whose order key is `key` (`num_order_of`'s inverse), `NONE` for none.
pub fn number_of_order(b: &mut B<'_, '_>) -> Value {
    let key = b.arg(0);
    let preorder = b.field(at::PREORDER_KEYS);
    b.either(
        I32,
        preorder,
        |b| {
            let zero = b.c32(0);
            let nodes = b.count_of(zero);
            let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, key, nodes);
            let none = b.c32(NONE);
            b.f.ins().select(inside, key, none)
        },
        |b| {
            let r = b.f.ins().ushr_imm(key, i64::from(ORDER_INDEX_BITS));
            let n = b.run_len(at::RUN_AT);
            let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, r, n);
            b.either(
                I32,
                inside,
                |b| {
                    let run_at = b.follow(b.tape, at::RUN_AT);
                    let p = b.item(run_at, r, 8);
                    let g = b.word_at(I32, p, 0);
                    let s = b.word_at(I32, p, 4);
                    let k = b.f.ins().band_imm(key, (1i64 << ORDER_INDEX_BITS) - 1);
                    let g1 = b.f.ins().iadd_imm(g, 1);
                    let ng = b.column_u32(at::NODES, g);
                    let ng1 = b.column_u32(at::NODES, g1);
                    let own = b.f.ins().isub(ng1, ng);
                    let fits = b.f.ins().icmp(IntCC::UnsignedLessThan, k, own);
                    b.either(
                        I32,
                        fits,
                        |b| {
                            let zero = b.c32(0);
                            b.call("num_number_in", &[g, zero, k, s])
                        },
                        |b| b.c32(NONE),
                    )
                },
                |b| b.c32(NONE),
            )
        },
    )
}
