//! Which of tier 1's values are byte-local, and so computed on a block's byte vectors (`Plan`): the analysis every
//! vector backend reads, the CPU's (`native_cpu_lanes`, which lowers it) and Metal's. Nothing here emits code.
//! The classes' packing (`body_pack`), the gated regions of the full body (`Region`), and the values remade where
//! they are read (`remade`) are its parts.

use std::collections::HashMap;

use crate::block::ClassTable;
use crate::block_native::{BlockInst, BlockNative};
/// The run's choice: `native_cpu_codegen::Options::bit_test`, `BIT_TEST_DEFAULT` unless its caller sets it.
/// How far back a byte-local value may look: the bytes before a file (`FRONT`).
pub const MAX_LAG: u8 = 16;

/// Which instructions are computed on the byte vectors, and what is known of them.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Computed on the byte vectors.
    pub lanes: Vec<bool>,
    /// Made a 64-bit mask too: read outside the vectors, an output, or a carried field's value.
    pub mask: Vec<bool>,
    /// May be set on a byte of 0x00 (the zeros before a file), for a shiftable value.
    pub at_zero: Vec<bool>,
    /// Its value on every window of ASCII bytes, when it is the same on all of them.
    pub ascii: Vec<Option<bool>>,
    /// Some value reads the 16 bytes before the block.
    pub before: bool,
    /// Some value is known on ASCII windows: the full body tests for one (`native_cpu_block::ascii_split`).
    pub ascii_branch: bool,
    /// Each value's word in a full block, when it is the same in every one: made of constants
    /// and `within` only (all ones there), `prev`s of such values included.
    pub full: Vec<Option<u64>>,
    /// The gated regions of the full body (`Region`), in the order they were chosen.
    pub regions: Vec<Region>,
    /// Each value's region, if it is in one.
    pub region_of: Vec<Option<usize>>,
    /// The values the full body gets wrong on an edge block (`native_cpu_edge::edge_values`,
    /// 0383), where that is bounded: found when a body first asks (`Plan::edge`).
    edge: std::cell::OnceCell<Option<crate::native_cpu_edge::Edge>>,
}

/// A gated region: values that are zero in a block where the gate is zero and their own carried
/// bits came in zero, and values only they read. The full body computes them behind one branch,
/// so a block with nothing for them (a block without numbers, for a number check) skips them,
/// their class masks and their `prev`s included.
#[derive(Clone, Debug)]
pub struct Region {
    pub gate: usize,
    /// A test before the gate's: the gate's other operand, when the gate's byte class is made a
    /// mask for the gate alone. A block where it has no bit skips the class's mask too, and one
    /// where it has bits makes the class and tests the gate (the class and the gate are then
    /// members). An escape's `u`, say: most blocks have no escape.
    pub pre: Option<usize>,
    /// The values provably zero with the gate (the gate itself not among them).
    pub zero: Vec<usize>,
    /// Every value the region computes, in order: `zero`, and values only the region reads.
    pub members: Vec<usize>,
    /// The values on the vectors only the members read: made on demand inside the branch, and
    /// never in a block it skips.
    pub vectors: Vec<usize>,
}

/// What one operation costs, roughly, in instructions: for choosing the regions worth a branch.
fn weight(inst: &BlockInst, lane: bool, mask: bool) -> usize {
    match inst {
        // On the vectors: one instruction per 16-byte chunk, four a block (a compare, a lookup's
        // test, an `ext` for a `prev`, a logic op); a `not` and a constant are free (a compare
        // gives the complement, a known lane folds); a `pair`, two lookups and an `and`. Made a
        // mask too: four `and`s, three pairwise adds and a move.
        _ if lane => {
            let own = match inst {
                BlockInst::Const(_) | BlockInst::Not(_) => 0,
                BlockInst::Pair { .. } => 8,
                _ => 4,
            };
            own + if mask { 9 } else { 0 }
        }
        BlockInst::Const(_) | BlockInst::Within(_) | BlockInst::Prefix(..) => 0,
        BlockInst::Prev(..) => 6,
        BlockInst::Fill(..) => 9,
        BlockInst::Toggle(_) => 14,
        BlockInst::Escaped(_) => 14,
        _ => 1,
    }
}

/// What may be true of a value over windows of ASCII bytes: whether it can be 0, whether 1.
#[derive(Clone, Copy)]
pub struct Can {
    zero: bool,
    one: bool,
}

pub fn class_byte(t: &ClassTable, b: u8) -> u8 {
    t.lo[usize::from(b & 15)] & t.hi[usize::from(b >> 4)]
}

/// The bytes of a class (`bits` of table `t`) as one run `lo..=hi`, where they are one.
pub fn byte_range(t: &ClassTable, bits: u8) -> Option<(u8, u8)> {
    let mut set = [0u64; 4];
    for b in 0..=255u8 {
        if class_byte(t, b) & bits != 0 {
            set[usize::from(b) / 64] |= 1 << (b % 64);
        }
    }
    set_range(set)
}

/// A 256-bit set of bytes as one run `lo..=hi`, where it is one (not empty).
pub fn set_range(set: [u64; 4]) -> Option<(u8, u8)> {
    let has = |b: usize| set[b / 64] >> (b % 64) & 1 == 1;
    let lo = (0..256).find(|&b| has(b))?;
    let hi = (0..256).rev().find(|&b| has(b))?;
    (lo..=hi).all(has).then_some((lo as u8, hi as u8))
}

/// A body's classes packed into shared tables (`class_pack`), as simdjson's one lookup answers
/// all its classes: each class instruction's `(table, bits)` to its test, and the body's extra
/// sets' tests after them (from `extra`).
#[derive(Clone, Debug, Default)]
pub struct BodyPack {
    pub packing: crate::class_pack::Packing,
    pub of: HashMap<(u16, u8), usize>,
    pub extra: usize,
    /// Each place's tables (`placed_pack`): `(place, tables)`, the body's own part first.
    pub places: Vec<(usize, usize)>,
}

/// The pack of the classes the values `roots` read, with `extras` (sets the body tests besides
/// its program's). A class whose table a `pair` reads keeps that table: its lookup is the
/// pair's. Where the packing would take more lookups than the program's own tables, the
/// classes keep theirs (the extras are packed alone).
pub fn body_pack(native: &BlockNative, roots: &[usize], extras: &[crate::class_pack::Need]) -> BodyPack {
    use crate::class_pack::{Need, pack};
    let mut seen = vec![false; native.insts.len()];
    let mut stack: Vec<usize> = roots.to_vec();
    let (mut classes, mut paired): (Vec<(u16, u8)>, Vec<u16>) = (Vec::new(), Vec::new());
    while let Some(x) = stack.pop() {
        if std::mem::replace(&mut seen[x], true) {
            continue;
        }
        match native.insts[x] {
            BlockInst::Class { table, bits } if !classes.contains(&(table, bits)) => classes.push((table, bits)),
            BlockInst::Pair { before, table, .. } => paired.extend([before, table]),
            _ => {}
        }
        stack.extend(native.insts[x].operands());
    }
    classes.retain(|(t, _)| !paired.contains(t));
    let set_of = |&(t, bits): &(u16, u8)| -> crate::class_pack::ByteSet {
        let mut s = [0u64; 4];
        for b in 0..=255u8 {
            if native.tables[usize::from(t)].member(b, bits) {
                s[usize::from(b >> 6)] |= 1 << (b & 63);
            }
        }
        s
    };
    let needs: Vec<Need> = classes.iter().map(|c| Need::exact(set_of(c))).chain(extras.iter().copied()).collect();
    let packing = pack(&needs);
    // The program's own lookups for these classes: a table each, but a class of one run.
    let mut own: Vec<u16> = classes.iter().filter(|(t, bits)| byte_range(&native.tables[usize::from(*t)], *bits).is_none()).map(|c| c.0).collect();
    own.sort_unstable();
    own.dedup();
    let extra_alone = pack(extras);
    if packing.tables.len() > own.len() + extra_alone.tables.len() {
        return BodyPack { packing: extra_alone, of: HashMap::new(), extra: 0, places: Vec::new() };
    }
    let of = classes.iter().enumerate().map(|(k, &c)| (c, k)).collect();
    BodyPack { packing, of, extra: classes.len(), places: Vec::new() }
}

/// The full body's classes packed by where they are made (0375): `places[x]` is the place of
/// class value `x` (the body's own part, a gated region, the ASCII split's inside), and each
/// place's classes are packed together, as simdjson's one lookup answers the classes of its
/// one body. A place's lookups then serve its own classes only: a region needing one class
/// does not look up a table of others it never reads. A class the body's own part has is not
/// packed again in a place inside it (its lookup dominates there). A class whose table a
/// `pair` reads keeps that table (its lookup is the pair's); a place whose packing would take
/// more lookups than its classes' own tables keeps those.
pub fn placed_pack(native: &BlockNative, places: &[Option<usize>], own_place: usize) -> BodyPack {
    use crate::class_pack::{Need, Packing, Test, pack};
    let paired: Vec<u16> = native.insts.iter().filter_map(|i| match *i {
        BlockInst::Pair { before, table, .. } => Some([before, table]),
        _ => None,
    }).flatten().collect();
    let set_of = |(t, bits): (u16, u8)| -> crate::class_pack::ByteSet {
        let mut s = [0u64; 4];
        for b in 0..=255u8 {
            if native.tables[usize::from(t)].member(b, bits) {
                s[usize::from(b >> 6)] |= 1 << (b & 63);
            }
        }
        s
    };
    let n_places = places.iter().flatten().max().map_or(0, |m| m + 1).max(own_place + 1);
    let mut by_place: Vec<Vec<(u16, u8)>> = vec![Vec::new(); n_places];
    for (x, p) in places.iter().enumerate() {
        if let (Some(p), BlockInst::Class { table, bits }) = (*p, &native.insts[x])
            && !paired.contains(table)
            && !by_place[p].contains(&(*table, *bits))
        {
            by_place[p].push((*table, *bits));
        }
    }
    // A class is its bytes: two classes of one set (of two of the program's tables) are one
    // test, and a place inside the body's part tests none its part has.
    let own: Vec<crate::class_pack::ByteSet> = by_place[own_place].iter().map(|&c| set_of(c)).collect();
    let mut later: Vec<((u16, u8), crate::class_pack::ByteSet)> = Vec::new();
    for (p, cs) in by_place.iter_mut().enumerate() {
        if p != own_place {
            cs.retain(|&c| {
                let keep = !own.contains(&set_of(c));
                if !keep {
                    later.push((c, set_of(c)));
                }
                keep
            });
        }
        let mut sets: Vec<crate::class_pack::ByteSet> = Vec::new();
        cs.retain(|&c| {
            let set = set_of(c);
            let first = !sets.contains(&set);
            if first {
                sets.push(set);
            } else {
                later.push((c, set));
            }
            first
        });
    }
    let mut out = BodyPack::default();
    // The body's own part first: its tables are looked up before any place inside it.
    let order: Vec<usize> = std::iter::once(own_place).chain((0..n_places).filter(|&p| p != own_place)).collect();
    // The body's own part's tables, looked up before any place inside it (`own_tables`).
    let mut own_tables = 0;
    for p in order {
        // A class the own part's products make, its set theirs or its complement's: tested by
        // those bits, its place looks up nothing for it (a region's union of the structural
        // characters, whose products the own part's whitespace and structurals are).
        let mut classes = by_place[p].clone();
        if p != own_place {
            classes.retain(|&c| match by_products(&out.packing.tables[..own_tables], set_of(c)) {
                Some(t) => {
                    out.of.insert(c, out.packing.tests.len());
                    out.packing.tests.push(t);
                    false
                }
                None => true,
            });
        }
        if classes.is_empty() {
            continue;
        }
        let needs: Vec<Need> = classes.iter().map(|&c| Need::exact(set_of(c))).collect();
        let packing: Packing = pack(&needs);
        let mut tables: Vec<u16> = classes.iter().filter(|&&(t, bits)| byte_range(&native.tables[usize::from(t)], bits).is_none()).map(|c| c.0).collect();
        tables.sort_unstable();
        tables.dedup();
        if packing.tables.len() > tables.len() {
            continue;
        }
        let base = out.packing.tables.len() as u16;
        for (&c, t) in classes.iter().zip(packing.tests) {
            out.of.insert(c, out.packing.tests.len());
            out.packing.tests.push(Test { tables: t.tables.iter().map(|&(tb, bits)| (tb + base, bits)).collect(), ..t });
        }
        out.places.push((p, packing.tables.len()));
        out.packing.tables.extend(packing.tables);
        if p == own_place {
            own_tables = out.packing.tables.len();
        }
    }
    // Each class of a set already tested: that set's test, the first made of them (the map's order is not one).
    for (c, set) in later {
        let found = out.of.iter().filter(|&(&d, _)| set_of(d) == set).map(|(_, &k)| k).min();
        if let Some(k) = found {
            out.of.insert(c, k);
        }
    }
    out.extra = out.packing.tests.len();
    out
}

/// The test of `set` by the products of `tables` (each bit's bytes), where the bits inside it
/// make all of it, or the bits outside it all of its complement.
fn by_products(tables: &[crate::block::ClassTable], set: crate::class_pack::ByteSet) -> Option<crate::class_pack::Test> {
    let has = |s: &crate::class_pack::ByteSet, b: u8| s[usize::from(b >> 6)] >> (b & 63) & 1 == 1;
    for negated in [false, true] {
        let want = |b: u8| has(&set, b) != negated;
        let mut got = [0u64; 4];
        let mut parts: Vec<(u16, u8)> = Vec::new();
        for (t, table) in tables.iter().enumerate() {
            let mut bits = 0u8;
            for j in 0..8 {
                let of: Vec<u8> = (0..=255u8).filter(|&b| table.member(b, 1 << j)).collect();
                if !of.is_empty() && of.iter().all(|&b| want(b)) {
                    bits |= 1 << j;
                    for b in of {
                        got[usize::from(b >> 6)] |= 1 << (b & 63);
                    }
                }
            }
            if bits != 0 {
                parts.push((t as u16, bits));
            }
        }
        if (0..=255u8).all(|b| has(&got, b) == want(b)) && !parts.is_empty() {
            return Some(crate::class_pack::Test { range: None, tables: parts, negated });
        }
    }
    None
}

/// The bytes at which value `x` holds, where it is a function of its own byte alone (classes,
/// byte tests, planes and their logic): `None` for any other.
pub fn byte_set(native: &BlockNative, x: usize) -> Option<crate::class_pack::ByteSet> {
    let one = |native: &BlockNative, x: usize, b: u8| -> Option<bool> {
        fn go(native: &BlockNative, x: usize, b: u8) -> Option<bool> {
            Some(match native.insts[x] {
                BlockInst::Const(c) if c == 0 || c == u64::MAX => c != 0,
                BlockInst::EqByte(e) => b == e,
                BlockInst::Class { table, bits } => native.tables[usize::from(table)].member(b, bits),
                BlockInst::Plane(k) => b >> k & 1 == 1,
                BlockInst::Not(a) => !go(native, usize::from(a), b)?,
                BlockInst::And(a, c) => go(native, usize::from(a), b)? && go(native, usize::from(c), b)?,
                BlockInst::Or(a, c) => go(native, usize::from(a), b)? || go(native, usize::from(c), b)?,
                BlockInst::Xor(a, c) => go(native, usize::from(a), b)? != go(native, usize::from(c), b)?,
                BlockInst::AndNot(a, c) => go(native, usize::from(a), b)? && !go(native, usize::from(c), b)?,
                _ => return None,
            })
        }
        go(native, x, b)
    };
    let mut s = [0u64; 4];
    for b in 0..=255u8 {
        if one(native, x, b)? {
            s[usize::from(b >> 6)] |= 1 << (b & 63);
        }
    }
    Some(s)
}

/// Which values are byte-local, on the vectors, and how far back each reads (its leaves' bytes at
/// most that far before its own: a `prev` adds its shift, a `pair` and a `plane_before` one), and
/// what may be true of each on ASCII bytes. `deep`: a `prev` of any byte-local value (within
/// `MAX_LAG`) and `within` are on the vectors too; else only a `prev` of one with no `prev` in
/// it, simdjson's `prev<N>` of the input's classes.
pub fn lane_plan(native: &BlockNative, deep: bool) -> (Vec<bool>, Vec<u8>, Vec<bool>, Vec<Can>) {
    let n = native.insts.len();
    let (mut lanes, mut lag, mut at_zero) = (vec![false; n], vec![0u8; n], vec![false; n]);
    let mut can = vec![Can { zero: true, one: true }; n];
    for (at, inst) in native.insts.iter().enumerate() {
        let o = |x: u16| usize::from(x);
        let (l, s, z, c) = match *inst {
            BlockInst::Const(0) => (true, 0, false, Can { zero: true, one: false }),
            BlockInst::Const(u64::MAX) => (true, 0, true, Can { zero: false, one: true }),
            BlockInst::EqByte(b) => (true, 0, b == 0, Can { zero: true, one: b < 0x80 }),
            BlockInst::Class { table, bits } => {
                let t = &native.tables[usize::from(table)];
                let one = (0..0x80u8).any(|b| class_byte(t, b) & bits != 0);
                let zero = (0..0x80u8).any(|b| class_byte(t, b) & bits == 0);
                (true, 0, class_byte(t, 0) & bits != 0, Can { zero, one })
            }
            BlockInst::Plane(k) => (true, 0, false, Can { zero: true, one: k < 7 }),
            BlockInst::PlaneBefore(k) => (true, 1, false, Can { zero: true, one: k < 7 }),
            BlockInst::Pair { before, table, bits } => {
                let (p, t) = (&native.tables[usize::from(before)], &native.tables[usize::from(table)]);
                let ps = (0..0x80u8).fold(0, |a, b| a | class_byte(p, b));
                let ts = (0..0x80u8).fold(0, |a, b| a | class_byte(t, b));
                (true, 1, false, Can { zero: true, one: ps & ts & bits != 0 })
            }
            BlockInst::Not(a) => (lanes[o(a)], lag[o(a)], !at_zero[o(a)], Can { zero: can[o(a)].one, one: can[o(a)].zero }),
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                let (x, y) = (can[o(a)], can[o(b)]);
                let (zx, zy) = (at_zero[o(a)], at_zero[o(b)]);
                let (z, c) = match *inst {
                    BlockInst::And(..) => (zx && zy, Can { zero: x.zero || y.zero, one: x.one && y.one }),
                    BlockInst::Or(..) => (zx || zy, Can { zero: x.zero && y.zero, one: x.one || y.one }),
                    BlockInst::Xor(..) => (zx != zy, Can { zero: (x.zero && y.zero) || (x.one && y.one), one: (x.one && y.zero) || (x.zero && y.one) }),
                    _ => (zx && !zy, Can { zero: x.zero || y.one, one: x.one && y.zero }),
                };
                (lanes[o(a)] && lanes[o(b)], lag[o(a)].max(lag[o(b)]), z, c)
            }
            // A `prev`: its operand's bytes `k` further back (an `ext` a chunk). Before a file it
            // reads no bits.
            BlockInst::Prev(a, k) => {
                let l = u16::from(lag[o(a)]) + u16::from(k);
                (lanes[o(a)] && (deep || lag[o(a)] == 0) && l <= u16::from(MAX_LAG), l.min(255) as u8, false, can[o(a)])
            }
            // The bytes inside the stream: two compares of the positions, all ones in a full
            // block (past a delay of 48 not in the 16 bytes before one).
            BlockInst::Within(d) => (deep && d <= 48, 0, false, Can { zero: true, one: true }),
            _ => (false, 0, false, Can { zero: true, one: true }),
        };
        (lanes[at], lag[at], at_zero[at], can[at]) = (l, s, z, c);
    }
    (lanes, lag, at_zero, can)
}

/// Whether byte-local value `x` reads its own bytes only, none before them (no `prev`, `pair`
/// or `plane_before` in it): made on chunk 0, the 16 bytes before the block, it stays in the
/// window (`MAX_LAG`).
pub fn reads_here(native: &BlockNative, x: usize, depth: u32) -> bool {
    depth < 32
        && match native.insts[x] {
            BlockInst::Prev(..) | BlockInst::Pair { .. } | BlockInst::PlaneBefore(_) => false,
            ref i => i.operands().into_iter().all(|o| reads_here(native, o, depth + 1)),
        }
}

/// Whether value `x` is false wherever the byte `back` before its position is zero: its bytes
/// say so (an `eq_byte` of a byte not zero, a class without the zero byte, a plane's bit), a
/// `prev` further back, either side of an `and`, an `and_not`'s first, both of an `or`.
pub fn zero_kills(native: &BlockNative, x: usize, back: u32) -> bool {
    fn go(native: &BlockNative, x: usize, back: u32, depth: u32) -> bool {
        if depth > 64 {
            return false;
        }
        let o = |a: u16| usize::from(a);
        match native.insts[x] {
            BlockInst::EqByte(b) => back == 0 && b != 0,
            BlockInst::Class { table, bits } => back == 0 && class_byte(&native.tables[usize::from(table)], 0) & bits == 0,
            BlockInst::Plane(_) => back == 0,
            BlockInst::PlaneBefore(_) => back == 1,
            BlockInst::Pair { before, table, bits } => {
                (back == 0 && class_byte(&native.tables[usize::from(table)], 0) & bits == 0) || (back == 1 && class_byte(&native.tables[usize::from(before)], 0) & bits == 0)
            }
            BlockInst::Prev(a, k) => back >= u32::from(k) && go(native, o(a), back - u32::from(k), depth + 1),
            BlockInst::And(a, b) => go(native, o(a), back, depth + 1) || go(native, o(b), back, depth + 1),
            // `a` and not `b`: false where `a` is (a closer not escaped).
            BlockInst::AndNot(a, _) => go(native, o(a), back, depth + 1),
            BlockInst::Or(a, b) => go(native, o(a), back, depth + 1) && go(native, o(b), back, depth + 1),
            _ => false,
        }
    }
    go(native, x, back, 0)
}

/// Of the `and`s from `at`, the conjuncts that alone are the same on a file of the batch's
/// layout: but each `within(d)` whose byte another conjunct is false on when zero
/// (`zero_kills`). The bytes off the file are zeros, `native_cpu_block::FRONT` of them before
/// it (so `d` at most that). None where every conjunct stays.
pub fn unguarded(native: &BlockNative, at: usize) -> Option<Vec<usize>> {
    let mut leaves = Vec::new();
    let mut work = vec![at];
    while let Some(x) = work.pop() {
        match native.insts[x] {
            BlockInst::And(a, b) => work.extend([usize::from(b), usize::from(a)]),
            _ if !leaves.contains(&x) => leaves.push(x),
            _ => {}
        }
    }
    let guard = |w: usize| matches!(native.insts[w], BlockInst::Within(d) if usize::from(d) <= crate::block_native::FRONT && leaves.iter().any(|&y| y != w && zero_kills(native, y, u32::from(d))));
    let kept: Vec<usize> = leaves.iter().copied().filter(|&w| !guard(w)).collect();
    (kept.len() < leaves.len()).then_some(kept)
}

/// A byte-local value is on the vectors only where that saves a mask or a carried field: a
/// `prev` or logic none of whose readers is on the vectors, and whose operands are masks anyway,
/// is cheaper as a mask's operation; a `within` no value on the vectors reads, too.
fn demote(native: &BlockNative, lanes: &mut [bool], users: &[Vec<usize>], output: &[bool]) {
    let needs_mask = |lanes: &[bool], x: usize| lanes[x] && (output[x] || users[x].iter().any(|&u| !lanes[u]));
    loop {
        let mut changed = false;
        for (at, inst) in native.insts.iter().enumerate() {
            if !lanes[at] || users[at].iter().any(|&u| lanes[u]) {
                continue;
            }
            let demote = match *inst {
                // A `prev` read only as a mask: on the vectors it shifts bytes and needs no
                // carried field, so a gated region can skip it and its operand. Only where the
                // operand is made a mask anyway, for a reader other than `prev`s, is the mask
                // shifted instead.
                BlockInst::Prev(a, _) => {
                    let a = usize::from(a);
                    output[a] || users[a].iter().any(|&u| u != at && !lanes[u] && !matches!(native.insts[u], BlockInst::Prev(..)))
                }
                BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..) => {
                    inst.operands().into_iter().all(|x| needs_mask(lanes, x) || matches!(native.insts[x], BlockInst::Const(_)))
                }
                BlockInst::Within(_) => true,
                _ => false,
            };
            if demote {
                lanes[at] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

impl Plan {
    /// The plan: which values are on the vectors, and the gated regions. On the vectors a
    /// `prev` reads the bytes before the block, so it carries nothing between blocks, and a
    /// region can skip it; as a mask it carries the block's last bits, and a region cannot. But
    /// over a `prev`, logic on the vectors costs four times what it does as masks. So the vectors
    /// go deep (`lane_plan`) only where a gate skips them: the regions are found with them deep,
    /// the values those regions skip keep them, every other value takes the shallow plan, and the
    /// regions are found again.
    pub fn of(native: &BlockNative) -> Plan {
        let n = native.insts.len();
        let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (at, inst) in native.insts.iter().enumerate() {
            for x in inst.operands() {
                users[x].push(at);
            }
        }
        let mut output = vec![false; n];
        for &v in &native.outputs {
            output[usize::from(v)] = true;
        }
        let (mut deep, lag, at_zero, can) = lane_plan(native, true);
        demote(native, &mut deep, &users, &output);
        let (mut lanes, _, _, _) = lane_plan(native, false);
        demote(native, &mut lanes, &users, &output);
        let gated = {
            let (_, regions) = Self::regions_of(native, &deep, &users, &output);
            let mut gated = vec![false; n];
            for r in &regions {
                for &x in r.members.iter().chain(&r.vectors) {
                    gated[x] = true;
                }
            }
            gated
        };
        // Deep where gated, and so are the operands of what is: a value on the vectors reads
        // them there (made on demand, a mask once for a reader outside).
        let mut need = vec![false; n];
        for x in (0..n).rev() {
            if deep[x] && (gated[x] || need[x]) {
                need[x] = true;
                for o in native.insts[x].operands() {
                    need[o] = need[o] || deep[o];
                }
            }
        }
        for x in 0..n {
            lanes[x] = lanes[x] || need[x];
        }
        demote(native, &mut lanes, &users, &output);
        let (mask, regions) = Self::regions_of(native, &lanes, &users, &output);
        let mut region_of = vec![None; n];
        for (r, reg) in regions.iter().enumerate() {
            for &m in &reg.members {
                region_of[m] = Some(r);
            }
        }
        let ascii: Vec<Option<bool>> = can.iter().map(|c| if !c.one { Some(false) } else if !c.zero { Some(true) } else { None }).collect();
        let before = (0..n).any(|at| lanes[at] && lag[at] > 0);
        // Worth a branch: a byte test or class known on ASCII bytes (not a constant).
        let ascii_branch = native.insts.iter().enumerate().any(|(at, i)| lanes[at] && ascii[at].is_some() && matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Pair { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_)));
        let full = full_words(native, &lanes);
        Plan { lanes, mask, at_zero, ascii, before, ascii_branch, full, regions, region_of, edge: Default::default() }
    }

    /// The values made masks and the gated regions, for a choice of values on the vectors.
    fn regions_of(native: &BlockNative, lanes: &[bool], users: &[Vec<usize>], output: &[bool]) -> (Vec<bool>, Vec<Region>) {
        let n = native.insts.len();
        let mask: Vec<bool> = (0..n).map(|x| lanes[x] && (output[x] || users[x].iter().any(|&u| !lanes[u]))).collect();
        let full = full_words(native, lanes);
        // The values with a carried field: a carry of their own, or a `prev` of them on masks.
        let field: Vec<bool> = (0..n)
            .map(|x| matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Escaped(_)) || users[x].iter().any(|&u| matches!(native.insts[u], BlockInst::Prev(..)) && !lanes[u]))
            .collect();
        let (regions, _) = regions(native, lanes, &mask, users, output, &field, &full);
        (mask, regions)
    }

    /// The plan of a window (`Lanes::window`): every byte-local value on the vectors, deep, none
    /// demoted; no regions, no ASCII folding.
    pub fn window(native: &BlockNative) -> Plan {
        let n = native.insts.len();
        let (lanes, _, at_zero, _) = lane_plan(native, true);
        Plan { mask: lanes.clone(), lanes, at_zero, ascii: vec![None; n], before: false, ascii_branch: false, full: vec![None; n], regions: Vec::new(), region_of: vec![None; n], edge: Default::default() }
    }

    /// The values the full body gets wrong on an edge block (0383), where that is bounded: every
    /// other value it makes right there. Found once, when first asked.
    pub fn edge(&self, native: &BlockNative) -> Option<&crate::native_cpu_edge::Edge> {
        self.edge.get_or_init(|| crate::native_cpu_edge::edge_values(native, &self.lanes, &self.full)).as_ref()
    }

    /// Whether `prev` instruction `at` reads a carried word (it is not on the vectors).
    pub fn carries(&self, at: usize) -> bool {
        !self.lanes[at]
    }
}

/// Each value's word in a full block when it is the same in every full block. A full block's
/// `within` is all ones; the block before it is full too, or the file's first, where `within d`
/// lacks its low `d` bits and a `prev` shifts in zeros. So the bits known of each value are
/// followed in both kinds of block, and a `prev` in a full block knows the bits it shifts in when
/// the top bits are known, and the same, in both.
fn full_words(native: &BlockNative, lanes: &[bool]) -> Vec<Option<u64>> {
    /// The bits known of a value, and their values.
    #[derive(Clone, Copy)]
    struct Known {
        known: u64,
        value: u64,
    }
    const NONE: Known = Known { known: 0, value: 0 };
    let all = |c: u64| Known { known: u64::MAX, value: c };
    let not = |x: Known| Known { known: x.known, value: !x.value & x.known };
    let and = |x: Known, y: Known| {
        let ones = x.known & x.value & y.known & y.value;
        let zeros = (x.known & !x.value) | (y.known & !y.value);
        Known { known: ones | zeros, value: ones }
    };
    let or = |x: Known, y: Known| not(and(not(x), not(y)));
    let xor = |x: Known, y: Known| Known { known: x.known & y.known, value: (x.value ^ y.value) & x.known & y.known };
    let n = native.insts.len();
    // (in a full block, in the first block)
    let mut w: Vec<(Known, Known)> = vec![(NONE, NONE); n];
    for (at, inst) in native.insts.iter().enumerate() {
        let get = |x: u16| w[usize::from(x)];
        let both = |f: &dyn Fn(Known, Known) -> Known, a: u16, b: u16| {
            let ((xa, ya), (xb, yb)) = (get(a), get(b));
            (f(xa, xb), f(ya, yb))
        };
        w[at] = match *inst {
            BlockInst::Const(c) => (all(c), all(c)),
            BlockInst::Within(d) => (all(u64::MAX), all(if d == 0 { u64::MAX } else { !((1u64 << d) - 1) })),
            // None in a full block; in the first, what the file's first bytes say.
            BlockInst::Prefix(..) => (all(0), NONE),
            // Bit logic is the same at each byte of a lane as at each bit of a word: a lane of
            // known operands is known (a class the host's stripping cut, `within & ~0`).
            _ if lanes[at] && !matches!(inst, BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..)) => (NONE, NONE),
            BlockInst::Not(a) => {
                let (x, y) = get(a);
                (not(x), not(y))
            }
            BlockInst::And(a, b) => both(&and, a, b),
            BlockInst::Or(a, b) => both(&or, a, b),
            BlockInst::Xor(a, b) => both(&xor, a, b),
            BlockInst::AndNot(a, b) => both(&|x, y| and(x, not(y)), a, b),
            BlockInst::Prev(a, k) => {
                let (x, y) = get(a);
                let low = (1u64 << k) - 1;
                let top = |c: u64| c >> (64 - u32::from(k));
                // Shifted in: the last block's top bits, known where both kinds agree on them.
                let same = top(x.known) & top(y.known) & !(top(x.value) ^ top(y.value));
                let full = Known { known: (x.known << k) | same, value: (x.value << k) | (top(x.value) & same) };
                let first = Known { known: (y.known << k) | low, value: y.value << k };
                (full, first)
            }
            // In the word: the top `k` bits known zero.
            BlockInst::Early(a, k) => {
                let (x, y) = get(a);
                let down = |x: Known| Known { known: (x.known >> k) | !(u64::MAX >> k), value: x.value >> k };
                (down(x), down(y))
            }
            _ => (NONE, NONE),
        };
    }
    w.into_iter().map(|(full, _)| (full.known == u64::MAX).then_some(full.value)).collect()
}

/// The gated regions (`Region`). A gate is an `and` of a mask with a byte class, as a token's
/// start and the bytes a check takes: sparse. A value is zero with it when, in a block where the
/// gate is zero and the carried bits of these values came in zero, it must be: an `and` with
/// one, an `and not` of one, an `or` or `xor` of two, a `prev` of one, a `fill` from one, a
/// `toggle` or `escaped` of one. The regions worth a branch are chosen greedily, the largest
/// first, each without the values of those before it.
fn regions(native: &BlockNative, lanes: &[bool], mask: &[bool], users: &[Vec<usize>], output: &[bool], field: &[bool], full: &[Option<u64>]) -> (Vec<Region>, Vec<Option<usize>>) {
    const WORTH: usize = 24;
    const MOST: usize = 8;
    // The values a region gives the body after it, each held from the region's end to its
    // reader: simdjson's skipped work gives one error bit. More, and the region costs registers
    // the work it skips does not.
    const MOST_OUT: usize = 4;
    let n = native.insts.len();
    // A fixed point's members are made in its loop, never behind a gate.
    let mut grouped = vec![false; n];
    for g in &native.groups {
        for &m in &g.members {
            grouped[m] = true;
        }
    }
    // A gate's conditions: how many tests its bits need at once, and whether one is carried
    // state (an escape, a fill, a value of a pass before). A byte class, or a run of them, is in
    // most blocks of text; a class with an escape before it, or inside a string, is rare.
    // Without measured densities a gate needs two conditions, one of them carried: simdjson's
    // escapes, a `u` after an escaping backslash. A run's first byte counts as carried.
    let mut conditions = vec![0usize; n];
    let mut carried = vec![false; n];
    for (x, inst) in native.insts.iter().enumerate() {
        let c = |o: u16| conditions[usize::from(o)];
        conditions[x] = match *inst {
            BlockInst::Const(_) | BlockInst::Within(_) | BlockInst::Prefix(..) => 0,
            BlockInst::And(a, b) => c(a) + c(b),
            BlockInst::AndNot(a, _) => c(a) + 1,
            BlockInst::Prev(a, _) | BlockInst::Early(a, _) => c(a),
            _ => 1,
        };
        carried[x] = match *inst {
            // A run's first byte, `a & ~prev(a)`: as rare as the runs are, a token's start (a
            // number's, which its check is gated on, as simdjson checks a number at its token).
            BlockInst::AndNot(a, p) if matches!(native.insts[usize::from(p)], BlockInst::Prev(q, 1) if q == a) => true,
            BlockInst::And(a, b) => carried[usize::from(a)] || carried[usize::from(b)],
            BlockInst::AndNot(a, _) | BlockInst::Prev(a, _) | BlockInst::Early(a, _) => carried[usize::from(a)],
            _ => !lanes[x] && inst.operands().is_empty() && !matches!(inst, BlockInst::Const(_) | BlockInst::Within(_) | BlockInst::Prefix(..)) || matches!(inst, BlockInst::Escaped(_) | BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Carry(_) | BlockInst::Walk { .. }) || grouped[x],
        };
    }
    let mut region_of: Vec<Option<usize>> = vec![None; n];
    let mut gates = vec![false; n];
    let mut out: Vec<Region> = Vec::new();
    let zeros = |g: usize, region_of: &[Option<usize>]| -> Vec<bool> {
        let mut z = vec![false; n];
        z[g] = true;
        for (at, inst) in native.insts.iter().enumerate().skip(g + 1) {
            if lanes[at] || region_of[at].is_some() || full[at].is_some() || grouped[at] {
                continue;
            }
            let is = |x: u16| z[usize::from(x)] || matches!(native.insts[usize::from(x)], BlockInst::Const(0));
            z[at] = match *inst {
                BlockInst::And(a, b) => is(a) || is(b),
                BlockInst::AndNot(a, _) | BlockInst::Prev(a, _) | BlockInst::Early(a, _) | BlockInst::Fill(a, _) | BlockInst::Toggle(a) | BlockInst::Escaped(a) => is(a),
                BlockInst::Or(a, b) | BlockInst::Xor(a, b) => is(a) && is(b),
                _ => false,
            };
        }
        z[g] = false;
        z
    };
    for _ in 0..MOST {
        let mut worthy: Vec<(usize, usize, Vec<bool>, Vec<bool>, Vec<bool>)> = Vec::new();
        for (g, inst) in native.insts.iter().enumerate() {
            let candidate = !lanes[g]
                && !grouped[g]
                && region_of[g].is_none()
                && !gates[g]
                // Any mask of two conditions or more, not the same in every full block: what it
                // skips decides. A gate set in most blocks costs its test and a branch predicted
                // taken, against at least `WORTH` of work skipped where it is not.
                && full[g].is_none()
                && conditions[g] >= 2
                && carried[g]
                && !users[g].is_empty()
                && !matches!(inst, BlockInst::Const(_) | BlockInst::Not(_));
            if !candidate {
                continue;
            }
            let z = zeros(g, &region_of);
            // The values only the region reads, with no carried field of their own: made inside it.
            let mut only = vec![false; n];
            for x in (0..n).rev() {
                if z[x] || x == g || gates[x] || region_of[x].is_some() || output[x] || field[x] || users[x].is_empty() || (lanes[x] && !mask[x]) || grouped[x] {
                    continue;
                }
                let inside = |u: usize| z[u] || only[u];
                only[x] = if lanes[x] { users[x].iter().filter(|&&u| !lanes[u]).all(|&u| inside(u)) } else { users[x].iter().all(|&u| inside(u)) };
            }
            // Convex: a value outside that reads the region and is read by it would have to be
            // made both before and after its branch. The region's values after one go. A value on
            // the vectors reading another reads its bytes, not its mask: no order between them.
            let mut member: Vec<bool> = (0..n).map(|x| z[x] || only[x]).collect();
            let word = |o: usize, x: usize| !(lanes[o] && lanes[x]);
            loop {
                let mut reach = vec![false; n];
                let mut cut = false;
                for x in 0..n {
                    let from = native.insts[x].operands().iter().any(|&o| word(o, x) && member[o] || reach[o]);
                    if !member[x] {
                        reach[x] = from;
                    } else if native.insts[x].operands().iter().any(|&o| reach[o]) {
                        member[x] = false;
                        reach[x] = true;
                        cut = true;
                    }
                }
                if !cut {
                    break;
                }
            }
            let (z, only): (Vec<bool>, Vec<bool>) = ((0..n).map(|x| z[x] && member[x]).collect(), (0..n).map(|x| only[x] && member[x]).collect());
            let vectors = exclusive_vectors(lanes, users, output, &member);
            // Read after it: by a value neither a member nor one of its vectors, or an output.
            let out = (0..n).filter(|&x| member[x] && (output[x] || users[x].iter().any(|&u| !member[u] && !vectors[u]))).count();
            if out > MOST_OUT {
                continue;
            }
            let w: usize = (0..n).filter(|&x| member[x] || vectors[x]).map(|x| weight(&native.insts[x], lanes[x], mask[x] && member[x])).sum();
            if w >= WORTH {
                worthy.push((g, w, z, only, vectors));
            }
        }
        let mut best = worthy.iter().max_by_key(|c| (c.1, std::cmp::Reverse(c.0)));
        // A gate inside the region that skips nearly as much is sparser: the number check's, say,
        // a token start with a number's first byte, inside a token start without some other
        // byte.
        while let Some(c) = best {
            // Strictly less, so the walk ends: two gates can each be in the other's region.
            let inner = worthy.iter().filter(|d| d.0 != c.0 && (c.2[d.0] || c.3[d.0]) && d.1 < c.1 && d.1 * 4 >= c.1 * 3).max_by_key(|d| (d.1, std::cmp::Reverse(d.0)));
            match inner {
                Some(d) => best = Some(d),
                None => break,
            }
        }
        let best = best.cloned();
        let Some((g, _, z, only, vectors)) = best else { break };
        let r = out.len();
        // The gate's class, made a mask only for the gate, and carried by no field.
        let pre = match native.insts[g] {
            BlockInst::And(a, b) | BlockInst::AndNot(a, b) => {
                let (a, b) = (usize::from(a), usize::from(b));
                let (word, class) = if lanes[a] { (b, a) } else { (a, b) };
                let alone = lanes[class] && !lanes[word] && !output[class] && !field[class] && region_of[class].is_none() && users[class].iter().all(|&u| lanes[u] || u == g);
                alone.then_some((word, class))
            }
            _ => None,
        };
        let mut members: Vec<usize> = (0..n).filter(|&x| z[x] || only[x]).collect();
        if let Some((_, class)) = pre {
            members.extend([class, g]);
            members.sort_unstable();
        }
        let pre = pre.map(|(word, _)| word);
        for &x in &members {
            region_of[x] = Some(r);
        }
        gates[g] = true;
        let vectors: Vec<usize> = (0..n).filter(|&x| vectors[x] && !members.contains(&x)).collect();
        out.push(Region { gate: g, pre, zero: (0..n).filter(|&x| z[x]).collect(), members, vectors });
    }
    (out, region_of)
}

/// The pure values (`BlockNative::pure`) remade in each pass that reads them, not stored by the
/// first and loaded: simdjson's "recompute, do not keep", where it pays.
/// - Constants and `within`: free in a full block, never worth a slot.
/// - A value inside a gated region of a later pass, when every pure value is remade: made there,
///   behind the gate, it costs nothing in a block the gate skips. A region is worth its branch
///   only with the work it skips inside it, so the regions are found with all of it remade.
///
/// Anything else stays stored once and loaded at its reader: remade, a byte check would be held
/// across the later pass's body, a register cost a load does not have. A remade value's
/// operands are remade too.
pub fn remade(native: &BlockNative) -> Vec<bool> {
    let pure = native.pure();
    let n = native.insts.len();
    let mut remade: Vec<bool> = (0..n).map(|x| pure[x] && matches!(native.insts[x], BlockInst::Const(_) | BlockInst::Within(_) | BlockInst::Prefix(..))).collect();
    let trial = native.passes_with(&pure);
    for (k, pass) in trial.passes.iter().enumerate().skip(1) {
        let crate::block_native::Pass::Forward { body, .. } = pass else { continue };
        let plan = Plan::of(body);
        for r in &plan.regions {
            for &m in r.members.iter().chain(&r.vectors) {
                let o = trial.origins[k][m] as usize;
                if pure[o] {
                    remade[o] = true;
                }
            }
        }
    }
    // A remade value's operands are remade with it: one loaded would be a mask, and the value
    // would leave the vectors, and its region. Made once in the pass, other readers share it.
    for x in (0..n).rev() {
        if remade[x] {
            for o in native.insts[x].operands() {
                remade[o] = remade[o] || pure[o];
            }
        }
    }
    remade
}

/// The values on the vectors that only `member`s read, through other such values: a block that
/// skips the members makes none of them. A value a mask outside reads, or an output, is not.
fn exclusive_vectors(lanes: &[bool], users: &[Vec<usize>], output: &[bool], member: &[bool]) -> Vec<bool> {
    let n = lanes.len();
    let mut inside = vec![false; n];
    for x in (0..n).rev() {
        if !lanes[x] || member[x] || output[x] || users[x].is_empty() {
            continue;
        }
        inside[x] = users[x].iter().all(|&u| member[u] || inside[u]);
    }
    inside
}

/// Whether the body reads the bytes before its block: then they are chunk 0, which the hot
/// loop carries from the block before (simdjson's `prev_input_block`, decision 0405).
pub fn reads_before(native: &BlockNative, plan: &Plan) -> bool {
    plan.before || native.insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)))
}
