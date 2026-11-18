//! The values a full block's body gets wrong on an edge block (decision 0383).
//!
//! A full block's body takes every `within` as all ones: the block lies inside the stream, so
//! its values made of constants and `within` fold away (`Plan::full`). The stream's first block
//! and its last ones are edge blocks, where `within d` holds only for `d <= p < len + d`. There
//! the full body is wrong for some values: the program's checks at the stream's ends (a byte
//! order mark, a UTF-8 sequence or a number cut off by the end), and what reads them.
//!
//! Which values, [`edge_values`] finds by running both bodies at once over the bits of a
//! stream, 64 positions a word: the general body's value and the full body's, each 0, 1, or
//! unknown, and whether the two are the same. Outside the stream the bytes are zero (the
//! padding before and after it, `FRONT` and `BACK`), so a value of the byte alone is known
//! there; inside, unknown. A value differs on an edge block where at some position the two may
//! differ. Every other value is the same in both bodies at every position, so the full body
//! makes it right on an edge block too. simdjson's stage 1 is one body and a `finish()` for the
//! stream's end; the values found here are that `finish()`'s.
//!
//! The full body's side is as it is made: a value it folds (`Plan::full`) is that word at every
//! position. Each block starts from the general body's carries, an edge block's leaving by its
//! edge part's join; so a difference ends at its block's end unless it reaches a full block's
//! positions, which is refused.

use crate::block_native::{BlockInst, BlockNative};

/// A bit: 0, 1, or not known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T {
    Z,
    O,
    U,
}

use T::{O, U, Z};

/// A bit of both bodies: one value in both (`Same`), or each its own (`Pair`: the general
/// body's, the full body's), which may differ.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    Same(T),
    Pair(T, T),
}

use V::{Pair, Same};

fn and(a: T, b: T) -> T {
    match (a, b) {
        (Z, _) | (_, Z) => Z,
        (O, x) | (x, O) => x,
        _ => U,
    }
}

fn not(a: T) -> T {
    match a {
        Z => O,
        O => Z,
        U => U,
    }
}

fn xor(a: T, b: T) -> T {
    match (a, b) {
        (U, _) | (_, U) => U,
        _ if a == b => Z,
        _ => O,
    }
}

fn or(a: T, b: T) -> T {
    not(and(not(a), not(b)))
}

fn sides(v: V) -> (T, T) {
    match v {
        Same(x) => (x, x),
        Pair(g, f) => (g, f),
    }
}

fn lift2(op: fn(T, T) -> T, a: V, b: V) -> V {
    if let (Same(x), Same(y)) = (a, b) {
        return Same(op(x, y));
    }
    let ((ag, af), (bg, bf)) = (sides(a), sides(b));
    let (g, f) = (op(ag, bg), op(af, bf));
    if g == f && g != U { Same(g) } else { Pair(g, f) }
}

fn bit(b: bool) -> T {
    if b { O } else { Z }
}

/// A value's bits over a window, 64 positions a word: each body's bit known (`gk`, `fk`) and,
/// where known, its value (`gv`, `fv`); and where the two are one value (`eq`, `Same`), even
/// unknown. Where both are known and equal they are one (`lift2`'s normal form).
#[derive(Clone)]
struct Row {
    gk: Vec<u64>,
    gv: Vec<u64>,
    fk: Vec<u64>,
    fv: Vec<u64>,
    eq: Vec<u64>,
}

impl Row {
    /// `Same(t)` at every position.
    fn same(words: usize, t: T) -> Row {
        let (k, v) = match t {
            Z => (u64::MAX, 0),
            O => (u64::MAX, u64::MAX),
            U => (0, 0),
        };
        Row { gk: vec![k; words], gv: vec![v; words], fk: vec![k; words], fv: vec![v; words], eq: vec![u64::MAX; words] }
    }

    /// `v` at the positions of `mask`.
    fn put(&mut self, mask: &[u64], v: V) {
        let (g, f) = sides(v);
        let bits = |t: T| match t {
            Z => (u64::MAX, 0),
            O => (u64::MAX, u64::MAX),
            U => (0, 0),
        };
        let ((gk, gv), (fk, fv)) = (bits(g), bits(f));
        let eq = if matches!(v, Same(_)) { u64::MAX } else { 0 };
        for (j, &m) in mask.iter().enumerate() {
            let set = |w: &mut u64, x: u64| *w = (*w & !m) | (x & m);
            set(&mut self.gk[j], gk);
            set(&mut self.gv[j], gv);
            set(&mut self.fk[j], fk);
            set(&mut self.fv[j], fv);
            set(&mut self.eq[j], eq);
        }
    }

    fn get(&self, i: usize) -> V {
        let b = |w: &[u64]| w[i / 64] >> (i % 64) & 1 == 1;
        let t = |k: bool, v: bool| if !k { U } else if v { O } else { Z };
        let (g, f) = (t(b(&self.gk), b(&self.gv)), t(b(&self.fk), b(&self.fv)));
        if b(&self.eq) { Same(g) } else { Pair(g, f) }
    }

    fn set(&mut self, i: usize, v: V) {
        let (j, m) = (i / 64, 1u64 << (i % 64));
        let (g, f) = sides(v);
        let put = |k: &mut u64, val: &mut u64, t: T| {
            *k = if t == U { *k & !m } else { *k | m };
            *val = if t == O { *val | m } else { *val & !m };
        };
        put(&mut self.gk[j], &mut self.gv[j], g);
        put(&mut self.fk[j], &mut self.fv[j], f);
        self.eq[j] = if matches!(v, Same(_)) { self.eq[j] | m } else { self.eq[j] & !m };
    }
}

/// Three-valued `and`, on known bits and values: zero where either is a known zero, known
/// where both are.
fn and3(ak: u64, av: u64, bk: u64, bv: u64) -> (u64, u64) {
    let k = (ak & !av) | (bk & !bv) | (ak & bk);
    (k, ak & bk & av & bv)
}

/// Both bodies' `op` on two rows: one value where both were (`Same`, even unknown), or where
/// the two sides come out known and equal.
fn rows2(a: &Row, b: &Row, op: impl Fn(u64, u64, u64, u64) -> (u64, u64)) -> Row {
    let words = a.eq.len();
    let mut r = Row { gk: vec![0; words], gv: vec![0; words], fk: vec![0; words], fv: vec![0; words], eq: vec![0; words] };
    for j in 0..words {
        let (gk, gv) = op(a.gk[j], a.gv[j], b.gk[j], b.gv[j]);
        let (fk, fv) = op(a.fk[j], a.fv[j], b.fk[j], b.fv[j]);
        (r.gk[j], r.gv[j], r.fk[j], r.fv[j]) = (gk, gv, fk, fv);
        r.eq[j] = (a.eq[j] & b.eq[j]) | (gk & fk & !(gv ^ fv));
    }
    r
}

fn not_row(a: &Row) -> Row {
    let mut r = a.clone();
    for j in 0..a.eq.len() {
        r.gv[j] = !a.gv[j] & a.gk[j];
        r.fv[j] = !a.fv[j] & a.fk[j];
    }
    r
}

/// Every plane of `a` shifted `k` positions later (bit `i` from `i - k`), zeros shifted in.
fn later(a: &[u64], k: usize) -> Vec<u64> {
    let (wq, b) = (k / 64, k % 64);
    (0..a.len())
        .map(|j| {
            let at = |x: usize| if j >= x { a[j - x] } else { 0 };
            if b == 0 { at(wq) } else { (at(wq) << b) | (at(wq + 1) >> (64 - b)) }
        })
        .collect()
}

/// Every plane of `a` shifted `k` positions earlier (bit `i` from `i + k`), zeros shifted in.
fn earlier(a: &[u64], k: usize) -> Vec<u64> {
    let (wq, b) = (k / 64, k % 64);
    (0..a.len())
        .map(|j| {
            let at = |x: usize| a.get(j + x).copied().unwrap_or(0);
            if b == 0 { at(wq) } else { (at(wq) >> b) | (at(wq + 1) << (64 - b)) }
        })
        .collect()
}

fn shift_row(a: &Row, by: impl Fn(&[u64]) -> Vec<u64>) -> Row {
    Row { gk: by(&a.gk), gv: by(&a.gv), fk: by(&a.fk), fv: by(&a.fv), eq: by(&a.eq) }
}

/// The window's positions `p` (`lo + i`) with `from <= p < to`, as words.
fn span(words: usize, lo: i64, from: i64, to: i64) -> Vec<u64> {
    (0..words)
        .map(|j| {
            let base = lo + 64 * j as i64;
            let (a, b) = ((from - base).clamp(0, 64), (to - base).clamp(0, 64));
            if b <= a { 0 } else if b - a == 64 { u64::MAX } else { ((1u64 << (b - a)) - 1) << a }
        })
        .collect()
}

/// One run of both bodies over positions `lo..hi` of a stream of `len` bytes, 64 positions a
/// word but for the carried scans (a toggle, a fill, an escape), which go a position at a time.
/// The carried values come in zero at the stream's start, unknown inside it. Each value's row.
fn run(native: &BlockNative, lanes: &[bool], full: &[Option<u64>], len: i64, lo: i64, hi: i64) -> Vec<Row> {
    let w = (hi - lo) as usize;
    let words = w.div_ceil(64);
    let start = if lo <= 0 { Same(Z) } else { Same(U) };
    let inside = span(words, lo, 0, len);
    let outside: Vec<u64> = inside.iter().map(|&m| !m).collect();
    let class = |t: u16, bits: u8, b: u8| -> bool { native.tables[usize::from(t)].member(b, bits) };
    // A value of the byte alone: unknown inside the stream, its value on a zero byte outside.
    let local = |zero: bool| {
        let mut r = Row::same(words, U);
        r.put(&outside, Same(bit(zero)));
        r
    };
    let mut vals: Vec<Row> = Vec::with_capacity(native.insts.len());
    for (x, inst) in native.insts.iter().enumerate() {
        let at = |k: u16| &vals[usize::from(k)];
        let row = match *inst {
            BlockInst::EqByte(e) => local(e == 0),
            BlockInst::Class { table, bits } => local(class(table, bits, 0)),
            BlockInst::Plane(_) => local(false),
            // The byte before: known only where it is outside the stream too.
            BlockInst::PlaneBefore(_) => {
                let mut r = Row::same(words, U);
                r.put(&span(words, lo, i64::MIN / 2, 1), Same(Z));
                r.put(&span(words, lo, len + 1, i64::MAX / 2), Same(Z));
                r
            }
            BlockInst::Pair { before, table, bits } => {
                let (tb, tt) = (&native.tables[usize::from(before)], &native.tables[usize::from(table)]);
                let both = tb.lo[0] & tb.hi[0] & tt.lo[0] & tt.hi[0] & bits != 0;
                let none_before = !(0..8).any(|k| bits >> k & 1 == 1 && class(before, 1 << k, 0));
                let none_here = !(0..8).any(|k| bits >> k & 1 == 1 && class(table, 1 << k, 0));
                let mut r = Row::same(words, U);
                // One outside: the zero byte before the stream's first, or past its last.
                if none_before {
                    r.put(&span(words, lo, 0, 1), Same(Z));
                }
                if none_here {
                    r.put(&span(words, lo, len, len + 1), Same(Z));
                }
                // Both outside: before the stream, past its end (`p - 1 >= len`), or the one
                // position of an empty stream.
                r.put(&span(words, lo, i64::MIN / 2, 0), Same(bit(both)));
                r.put(&span(words, lo, len + 1, i64::MAX / 2), Same(bit(both)));
                if len == 0 {
                    r.put(&span(words, lo, 0, 1), Same(bit(both)));
                }
                r
            }
            BlockInst::Const(c) => {
                let v: Vec<u64> = (0..words).map(|_| c.rotate_right(lo.rem_euclid(64) as u32)).collect();
                Row { gk: vec![u64::MAX; words], gv: v.clone(), fk: vec![u64::MAX; words], fv: v, eq: vec![u64::MAX; words] }
            }
            BlockInst::Within(d) => {
                let d = i64::from(d);
                let mut r = Row::same(words, O);
                let out: Vec<u64> = span(words, lo, d, len + d).iter().map(|&m| !m).collect();
                r.put(&out, Pair(Z, O));
                r
            }
            BlockInst::Not(a) => not_row(at(a)),
            BlockInst::And(a, b) => rows2(at(a), at(b), and3),
            BlockInst::Or(a, b) => not_row(&rows2(&not_row(at(a)), &not_row(at(b)), and3)),
            BlockInst::Xor(a, b) => rows2(at(a), at(b), |ak, av, bk, bv| (ak & bk, ak & bk & (av ^ bv))),
            BlockInst::AndNot(a, b) => rows2(at(a), &not_row(at(b)), and3),
            BlockInst::Prev(a, k) => {
                let k = i64::from(k);
                let mut r = shift_row(at(a), |p| later(p, k as usize));
                // A word's `prev` takes its first `k` bits from the carry, which is the general
                // body's at every block's start (`Emit::edge_part`). On the vectors it reads the
                // bytes before.
                if !lanes[x] {
                    let from_carry = if k >= 64 { u64::MAX } else { (1u64 << k) - 1 }.rotate_right(lo.rem_euclid(64) as u32);
                    for j in 0..words {
                        let m = from_carry;
                        r.fk[j] = (r.fk[j] & !m) | (r.gk[j] & m);
                        r.fv[j] = (r.fv[j] & !m) | (r.gv[j] & m);
                        r.eq[j] |= m;
                    }
                }
                // From before the window, inside the stream and far from both its ends, where
                // the two bodies are one: unknown, or the full body's word of a folded value.
                let head = span(words, lo, lo, (lo + k).min(hi));
                match full[usize::from(a)] {
                    Some(c) => {
                        let v = c.rotate_right((lo - k).rem_euclid(64) as u32);
                        for j in 0..words {
                            let m = head[j];
                            for (plane, val) in [(&mut r.gk[j], u64::MAX), (&mut r.fk[j], u64::MAX), (&mut r.gv[j], v), (&mut r.fv[j], v), (&mut r.eq[j], u64::MAX)] {
                                *plane = (*plane & !m) | (val & m);
                            }
                        }
                    }
                    None => r.put(&head, Same(U)),
                }
                for p in lo..(lo + k).min(hi).min(k) {
                    // Before the stream the general body shifts in the carry, zero, and cuts a
                    // shift on the byte vectors there; the full body, where it makes the shift
                    // on the vectors, keeps the value on the padding's zero bytes before the
                    // stream (`before`).
                    let v = match before(native, a, p - k).filter(|_| lanes[x]) {
                        Some(O) => Pair(Z, O),
                        Some(U) => Pair(Z, U),
                        _ => Same(Z),
                    };
                    r.set((p - lo) as usize, v);
                }
                r
            }
            // In the word only: the bits past the block's are zero.
            BlockInst::Early(a, k) => {
                let k = i64::from(k);
                let mut r = shift_row(at(a), |p| earlier(p, k as usize));
                // From past the window, far past the stream's end: not known here. Only `early`
                // moves a bit back, so these reach back `ahead` positions at most, which no mark
                // reads (`edge_values`).
                r.put(&span(words, lo, hi - k, hi), Same(U));
                let crossing: Vec<u64> = (0..words).map(|_| (u64::MAX << (64 - k.clamp(1, 64))).rotate_right(lo.rem_euclid(64) as u32)).collect();
                if k > 0 {
                    r.put(&crossing, Same(Z));
                }
                r
            }
            BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Escaped(_) => {
                let mut r = Row::same(words, U);
                let mut state = start;
                for i in 0..w {
                    // Each block starts from the general body's carries (`Emit::edge_part`).
                    if (lo + i as i64).rem_euclid(64) == 0 {
                        state = Same(sides(state).0);
                    }
                    let v = match *inst {
                        BlockInst::Toggle(a) => {
                            state = lift2(xor, state, at(a).get(i));
                            state
                        }
                        BlockInst::Fill(s, e) => {
                            let open = lift2(|x, y| and(not(x), y), at(e).get(i), state);
                            state = lift2(or, at(s).get(i), open);
                            state
                        }
                        BlockInst::Escaped(a) => {
                            // Escaped where the run of `a` just before is odd: its parity carried.
                            let escaped = state;
                            state = lift2(|r, odd| and(r, not(odd)), at(a).get(i), state);
                            escaped
                        }
                        _ => unreachable!(),
                    };
                    r.set(i, v);
                }
                r
            }
            // Of the position alone: the same in both bodies.
            BlockInst::Phase(..) | BlockInst::Position(_) => Row::same(words, U),
            // Not in a program this analysis takes (`edge_values`).
            BlockInst::BackFill(..) | BlockInst::Load(_) | BlockInst::Carry(_) | BlockInst::Cmp { .. } | BlockInst::Bit { .. } | BlockInst::Walk { .. } | BlockInst::Args(..) => Row::same(words, U),
        };
        // A value the full body folds (`Plan::full`: of constants and `within`, `prev`s of such
        // values included, and what such a zero masks) is that word there at every position, the
        // stream's ends too: a `prev` of all ones is all ones in the full body, where the general
        // body shifts a zero in at the stream's start. The two are one where the general body's
        // is known and the same.
        let row = match full[x] {
            Some(c) => {
                let mut r = row;
                let fv = c.rotate_right(lo.rem_euclid(64) as u32);
                for j in 0..words {
                    (r.fk[j], r.fv[j]) = (u64::MAX, fv);
                    r.eq[j] = r.gk[j] & !(r.gv[j] ^ fv);
                }
                r
            }
            None => row,
        };
        vals.push(row);
    }
    vals
}

/// Value `x` at position `q < 0`, before the stream, as the byte vectors make it there from the
/// padding's zero bytes: `None` where `x` is not of the bytes alone, and so made on words, from a
/// carry that is zero at the stream's start.
fn before(native: &BlockNative, x: u16, q: i64) -> Option<T> {
    let class = |t: u16, bits: u8| native.tables[usize::from(t)].member(0, bits);
    let at = |y: u16, q: i64| before(native, y, q);
    Some(match native.insts[usize::from(x)] {
        BlockInst::EqByte(e) => bit(e == 0),
        BlockInst::Class { table, bits } => bit(class(table, bits)),
        BlockInst::Plane(_) | BlockInst::PlaneBefore(_) => Z,
        BlockInst::Pair { before, table, bits } => bit(native.tables[usize::from(before)].lo[0] & native.tables[usize::from(before)].hi[0] & native.tables[usize::from(table)].lo[0] & native.tables[usize::from(table)].hi[0] & bits != 0),
        BlockInst::Const(c) => bit((c >> q.rem_euclid(64)) & 1 == 1),
        // Folded to ones in the full body.
        BlockInst::Within(_) => O,
        BlockInst::Not(a) => not(at(a, q)?),
        BlockInst::And(a, b) => and(at(a, q)?, at(b, q)?),
        BlockInst::Or(a, b) => or(at(a, q)?, at(b, q)?),
        BlockInst::Xor(a, b) => xor(at(a, q)?, at(b, q)?),
        BlockInst::AndNot(a, b) => and(at(a, q)?, not(at(b, q)?)),
        BlockInst::Prev(a, k) => at(a, q - i64::from(k)).unwrap_or(Z),
        _ => return None,
    })
}

/// What a full block's body gets wrong on an edge block (`edge_values`).
#[derive(Clone, Debug)]
pub struct Edge {
    /// The values that may differ between the general body and the full body there.
    pub differs: Vec<bool>,
    /// Each output, at its delay: whether it is zero outside the stream (`d <= p < len + d` not
    /// holding) in both bodies, so that the general body's cut of it to the stream (`step`) is
    /// no change, and a full block's body needs none.
    pub cut_free: Vec<bool>,
}

/// What the full body gets wrong on an edge block (the module's doc), or `None` where that is
/// not bounded: a difference at the stream's start carried on (a toggle's parity, say) into a
/// full block, or a
/// program this does not read (an earlier pass's word, a fixed point's group, a back fill, a
/// walk).
pub fn edge_values(native: &BlockNative, lanes: &[bool], full: &[Option<u64>]) -> Option<Edge> {
    if !native.groups.is_empty() || !native.walks.is_empty() || native.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_) | BlockInst::Carry(_) | BlockInst::Cmp { .. } | BlockInst::Bit { .. } | BlockInst::Walk { .. } | BlockInst::Args(..))) {
        return None;
    }
    let n = native.insts.len();
    // How far a value reaches back or ahead of its own position.
    let reach: i64 = native
        .insts
        .iter()
        .map(|i| match *i {
            BlockInst::Prev(_, k) | BlockInst::Early(_, k) | BlockInst::Within(k) => i64::from(k),
            _ => 1,
        })
        .max()
        .unwrap_or(1)
        + 64;
    let delay = i64::from(native.delay);
    // How far back a bit from past a window's end can reach: by `early`s, each its own shift.
    let ahead: i64 = native.insts.iter().map(|i| if let BlockInst::Early(_, k) = *i { i64::from(k) } else { 0 }).sum();
    let mut differs = vec![false; n];
    let mut cut_free = vec![true; native.outputs.len()];
    // An output's bits outside its window, each zero in both bodies.
    let cuts = |vals: &[Row], len: i64, lo: i64, hi: i64, cut_free: &mut [bool]| {
        let words = vals.first().map_or(0, |r| r.eq.len());
        let window = span(words, lo, lo, hi - ahead);
        for (k, &x) in native.outputs.iter().enumerate() {
            let d = i64::from(native.output_delays[k]);
            let r = &vals[usize::from(x)];
            let within = span(words, lo, d, len + d);
            if (0..words).any(|j| window[j] & !within[j] & !(r.eq[j] & r.gk[j] & !r.gv[j]) != 0) {
                cut_free[k] = false;
            }
        }
    };
    // Each value that may differ, and the last window position where one does. A difference
    // on a full block's position (`64 <= p < 64 * (len / 64)`) is refused: the full body runs
    // there with no edge part, and its carries into the next block are its own.
    let mark = |vals: &[Row], len: i64, lo: i64, hi: i64, differs: &mut [bool]| -> Result<Option<i64>, ()> {
        let words = vals.first().map_or(0, |r| r.eq.len());
        let window = span(words, lo, lo, hi - ahead);
        let full = span(words, lo, 64, 64 * (len / 64));
        let mut last: Option<i64> = None;
        for (k, row) in vals.iter().enumerate() {
            for j in 0..words {
                let pairs = !row.eq[j] & window[j];
                if pairs == 0 {
                    continue;
                }
                if pairs & full[j] != 0 {
                    return Err(());
                }
                differs[k] = true;
                let at = 64 * j as i64 + 63 - i64::from(pairs.leading_zeros());
                last = Some(last.map_or(at, |l| l.max(at)));
            }
        }
        Ok(last)
    };
    // The stream's start, the stream long: where its differences end.
    let window = 4 * 64 + 2 * reach + ahead;
    let front = run(native, lanes, full, i64::MAX / 4, 0, window);
    let front_end = mark(&front, i64::MAX / 4, 0, window, &mut differs).ok()?.map_or(0, |l| l + 1);
    cuts(&front, i64::MAX / 4, 0, window, &mut cut_free);
    if front_end + reach >= window {
        return None;
    }
    // The stream's end, at each place in a block: a stream long enough that its start's
    // differences are behind its window, the carries coming in unknown. A window of a stream
    // 64 bytes longer is this one moved by a block, so these cover every longer stream.
    let back_from = front_end + 2 * reach;
    for r in 0..64 {
        let len = 64 * (back_from / 64 + 1) + r;
        let lo = len - 2 * reach;
        let hi = len + delay + 2 * 64 + reach + ahead;
        let vals = run(native, lanes, full, len, lo, hi);
        // A difference at the window's first positions may come from further back.
        let first = span(vals[0].eq.len(), lo, lo, lo + reach);
        if vals.iter().any(|row| row.eq.iter().zip(&first).any(|(&e, &m)| !e & m != 0)) {
            return None;
        }
        mark(&vals, len, lo, hi, &mut differs).ok()?;
        cuts(&vals, len, lo, hi, &mut cut_free);
    }
    // Every shorter stream whole: its two ends near each other.
    for len in 0..64 * (back_from / 64 + 1) {
        let hi = len + delay + 2 * 64 + reach + ahead;
        let vals = run(native, lanes, full, len, 0, hi);
        mark(&vals, len, 0, hi, &mut differs).ok()?;
        cuts(&vals, len, 0, hi, &mut cut_free);
    }
    Some(Edge { differs, cut_free })
}
