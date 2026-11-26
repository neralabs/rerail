//! L1's reading of a K0 tape into K-form rows (decision 0686; Bootstrap from one tape, §4): one fixed walk of a
//! `.rerail` tape, with no lowering. It reads the kernel subset (K0): clauses of one head; bodies of atoms, negated
//! atoms and comparisons, in the order written; arguments that are variables, strings, numbers or `_`; facts. It
//! refuses anything else, naming the node, the first by node order. K0 has no arithmetic.
//!
//! Its symbols are read, never made: a leaf's text and a string's value from the document's symbols block (`intern`'s
//! node → symbol facts), and the names it compares (fields, K-form's step kinds) found in the atoms block.
//!
//! The same reading is written as rules in `bootstrap-l0.rerail` (P2's reader). The self-reading check holds the two
//! to the same rows: P2, run over `l0`'s own tape, gives P2.
//!
//! `k0_pass` (*0686: Operations* §10: read tapes, facts · write facts · rw control · emit) is the walk as an
//! operation, a statement an item, in three phases (emit, shape 11: output in order, no atomics): `count` walks each
//! statement and counts its rows by relation, and writes the first refusal by node order into control; `scan` turns
//! the counts into each statement's places, claimed at once in each relation's δ runs block; `place` walks each
//! statement again and writes its rows at its places, with the run's sign. A refused document writes no row.

use crate::intern as syms;
use crate::tape::Reads;
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::atoms::find_in;
use rerail_blocks::gens::{BODY_AT, set_word, word, words, words_mut};
use rerail_blocks::layer::{runs_claim_in, runs_set_in};
use rerail_blocks::maps::hash_bytes;
use rerail_rr_syntax::tape::{FIELD_NAMES, Kind};

/// A refusal: the node, and why.
pub type Refusal = (usize, &'static str);

/// Why a document is not K0, by the code control holds (`REFUSED_WHY`).
pub const WHY: [&str; 14] = [
    "a string that does not unescape",
    "a number past an i64",
    "an argument that is no variable, string, number or `_` (not K0)",
    "no relation",
    "a statement that is no clause (not K0)",
    "a clause of several heads (not K0)",
    "`must` or `may` (not K0)",
    "an edit (not K0)",
    "a body that is no conjunction (not K0)",
    "a negation of nothing",
    "a negation of no atom (not K0)",
    "a comparison with no operator",
    "a comparison with a side missing",
    "a literal that is no atom, negated atom or comparison (not K0)",
];

/// The document's statements, the pass's items, in node order.
pub fn statements<'r>(t: &'r Reads<'_>) -> impl Iterator<Item = usize> + 'r {
    t.children(0).filter(|&c| t.counted(c))
}

/// Walks document `d`'s tape, giving each K-form row to `put` with its relation's place in K-form's eight (`k_order`
/// is not the pass's: a module's stratum is its file's place in the bootstrap, `boot(path, k)`). Every step is at key
/// `(k, -1)`, and the head at `(-1, -1)`. The pass's twin walks it a statement at a time (`statement`).
pub fn walk(t: &Reads<'_>, d: i64, syms: &[u8], atoms: &[u8], put: &mut dyn FnMut(usize, &[i64])) -> Result<(), Refusal> {
    statements(t).try_for_each(|c| statement(t, d, syms, atoms, c, put))
}

/// Walks statement `c` of document `d`'s tape (`walk`).
pub fn statement(t: &Reads<'_>, d: i64, syms: &[u8], atoms: &[u8], c: usize, put: &mut dyn FnMut(usize, &[i64])) -> Result<(), Refusal> {
    let name = |s: &str| find_in(atoms, s, hash_bytes(s.as_bytes())).map_or(0, i64::from);
    let field_id = |f: &str| FIELD_NAMES.iter().position(|n| *n == f).expect("an rr field") as u16;
    let field = |p: usize, f: &str| -> Option<usize> {
        let f = field_id(f);
        t.children(p).find(|&c| t.node(c).1 == f)
    };
    let text = |n: usize| i64::from(syms::text(syms, n));
    let (atom, not, cmp) = (name("atom"), name("not"), name("cmp"));
    // A term: what it is, by its node.
    let term = |n: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refusal> {
        let (k, _, _) = t.node(n);
        if k == Kind::Variable as u16 {
            put(3, &[d, n as i64, text(n)]);
        } else if k == Kind::String as u16 {
            let v = syms::value(syms, n);
            if v == 0 {
                return Err((n, "a string that does not unescape"));
            }
            put(4, &[d, n as i64, i64::from(v)]);
        } else if k == Kind::Number as u16 {
            put(5, &[d, n as i64, t.number(n).ok_or((n, "a number past an i64"))?]);
        } else if k == Kind::Wildcard as u16 {
            put(6, &[d, n as i64]);
        } else {
            return Err((n, "an argument that is no variable, string, number or `_` (not K0)"));
        }
        Ok(())
    };
    // An argument list's arguments, as step `k`'s of clause `c`.
    let args = |c: usize, k: i64, list: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refusal> {
        for (i, a) in t.children(list).filter(|&a| t.counted(a)).enumerate() {
            put(2, &[d, c as i64, k, -1, i as i64, a as i64]);
            term(a, put)?;
        }
        Ok(())
    };
    let relation = |n: usize| -> Result<i64, Refusal> { Ok(text(field(n, "relation").ok_or((n, "no relation"))?)) };
    let head = field_id("head");
    {
        if t.node(c).0 != Kind::Clause as u16 {
            return Err((c, "a statement that is no clause (not K0)"));
        }
        let mut heads = t.children(c).filter(|&x| t.node(x).1 == head);
        let (Some(h), None) = (heads.next(), heads.next()) else {
            return Err((c, "a clause of several heads (not K0)"));
        };
        if field(h, "modal").is_some() {
            return Err((h, "`must` or `may` (not K0)"));
        }
        if field(c, "edit").is_some() {
            return Err((c, "an edit (not K0)"));
        }
        put(0, &[d, c as i64, relation(h)?]);
        if let Some(list) = field(h, "arguments") {
            args(c, -1, list, put)?;
        }
        let Some(b) = field(c, "body") else { return Ok(()) };
        if t.node(b).0 != Kind::Conjunction as u16 {
            return Err((b, "a body that is no conjunction (not K0)"));
        }
        for (i, l) in t.children(b).filter(|&l| t.counted(l)).enumerate() {
            let i = i as i64;
            let lk = t.node(l).0;
            if lk == Kind::Atom as u16 {
                put(1, &[d, c as i64, i, -1, atom, relation(l)?]);
                if let Some(list) = field(l, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Negation as u16 {
                let o = field(l, "operand").ok_or((l, "a negation of nothing"))?;
                if t.node(o).0 != Kind::Atom as u16 {
                    return Err((o, "a negation of no atom (not K0)"));
                }
                put(1, &[d, c as i64, i, -1, not, relation(o)?]);
                if let Some(list) = field(o, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Comparison as u16 {
                let op = field(l, "operator").ok_or((l, "a comparison with no operator"))?;
                put(1, &[d, c as i64, i, -1, cmp, text(op)]);
                for (j, side) in ["left", "right"].into_iter().enumerate() {
                    let a = field(l, side).ok_or((l, "a comparison with a side missing"))?;
                    put(2, &[d, c as i64, i, -1, j as i64, a as i64]);
                    term(a, put)?;
                }
            } else {
                return Err((l, "a literal that is no atom, negated atom or comparison (not K0)"));
            }
        }
    }
    Ok(())
}

// ---- k0_pass ------------------------------------------------------------------------------------------------------

crate::params! {
    /// Where the document's source and tape lie in its tape block, its module, and the run's sign (+1 or -1).
    pub struct K0P {
        pub src_at: u64,
        pub src_len: u64,
        pub tape_at: u64,
        pub tape_len: u64,
        pub module: i64,
        pub diff: i64,
    }
}

/// The slots: the document's tape block and its symbols block, the atoms (read), control (rw), then the δ runs block
/// of each K-form relation (write), in `RELATIONS`' order.
pub const TAPE: usize = 0;
pub const SYMS: usize = 1;
pub const ATOMS: usize = 2;
pub const CONTROL: usize = 3;
pub const RUNS: usize = 4;
const R: usize = 8;

/// Control's words, at `BODY_AT`: the first refused node (`u64::MAX`: none), why (`WHY`'s place), the rows placed;
/// then each statement's places, a relation a word (counts, then places, after `scan`).
pub const REFUSED_NODE: usize = BODY_AT;
pub const REFUSED_WHY: usize = BODY_AT + 8;
pub const PLACED: usize = BODY_AT + 16;
const PLACES: usize = BODY_AT + 24;
/// A relation whose places could not be claimed (its runs block too small: the driver runs again).
const UNCLAIMED: u32 = u32::MAX;

/// Control's body bytes for a document of `n` nodes (at most `n` statements).
pub fn control_bytes(n: usize) -> usize {
    PLACES - BODY_AT + 4 * R * n.max(1)
}

/// Control laid out before a run: no refusal, no row placed.
pub fn init_control(bytes: &mut [u8]) {
    set_word::<u64>(bytes, REFUSED_NODE, u64::MAX);
    set_word::<u64>(bytes, PLACED, 0);
}

/// The run's refusal, if any: the first refused node by node order, and why.
pub fn refused(control: &[u8]) -> Option<Refusal> {
    let n = word::<u64>(control, REFUSED_NODE);
    (n != u64::MAX).then(|| (n as usize, WHY[word::<u32>(control, REFUSED_WHY) as usize]))
}

/// The rows the run placed.
pub fn placed(control: &[u8]) -> u64 {
    word::<u64>(control, PLACED)
}

fn tape_of<'a>(c: &'a Ctx<'_, '_, '_>, p: &K0P) -> Reads<'a> {
    let tape = c.read(TAPE);
    let src = std::str::from_utf8(&tape[p.src_at as usize..(p.src_at + p.src_len) as usize]).expect("a source the file layer checked is UTF-8");
    Reads::in_place(&tape[p.tape_at as usize..(p.tape_at + p.tape_len) as usize], src).expect("a tape the parser wrote")
}

fn statement_items(c: &Ctx<'_, '_, '_>) -> u32 {
    let p = <K0P as crate::Params>::read(c.params);
    statements(&tape_of(c, &p)).count() as u32
}

/// `count`: each statement's rows by relation; a refusal, the first by node order, into control.
fn count(c: &mut Ctx<'_, '_, '_>, tile: Tile) {
    let p = <K0P as crate::Params>::read(c.params);
    let t = tape_of(c, &p);
    let (syms, atoms) = (c.read(SYMS), c.read(ATOMS));
    let mut counts = Vec::with_capacity(tile.items().len()); // copy: not block data (this tile's counts)
    let mut first: Option<Refusal> = None;
    for s in statements(&t).skip(tile.items().start).take(tile.items().len()) {
        let mut n = [0u32; R];
        if let Err(e) = statement(&t, p.module, syms, atoms, s, &mut |r, _| n[r] += 1) {
            first = Some(first.map_or(e, |f| if e.0 < f.0 { e } else { f }));
        }
        counts.push(n);
    }
    let ([], [control]) = c.views([], [CONTROL]);
    if let Some((node, why)) = first
        && (node as u64) < word::<u64>(control, REFUSED_NODE)
    {
        set_word::<u64>(control, REFUSED_NODE, node as u64);
        set_word::<u32>(control, REFUSED_WHY, WHY.iter().position(|w| *w == why).expect("a reason of WHY") as u32);
    }
    for (i, n) in tile.items().zip(counts) {
        words_mut::<u32>(control, PLACES + 4 * R * i, R).copy_from_slice(&n);
    }
}

/// `scan`: one item. The counts' exclusive prefix sums by relation, from the places claimed in each runs block at once;
/// a refused document claims none.
fn scan(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let items = statement_items(c) as usize;
    // Control, then each relation's runs block.
    let ([], mut v) = c.views([], std::array::from_fn::<usize, { R + 1 }, _>(|i| if i == 0 { CONTROL } else { RUNS + i - 1 }));
    let control = &mut *v[0];
    if word::<u64>(control, REFUSED_NODE) != u64::MAX {
        return;
    }
    let mut total = [0u32; R];
    for i in 0..items {
        let n = words_mut::<u32>(control, PLACES + 4 * R * i, R);
        for r in 0..R {
            let k = n[r];
            n[r] = total[r];
            total[r] += k;
        }
    }
    let mut base = [0u32; R];
    for r in 0..R {
        base[r] = if total[r] == 0 { 0 } else { runs_claim_in(&mut *v[1 + r], total[r]).unwrap_or(UNCLAIMED) };
    }
    let control = &mut *v[0];
    for i in 0..items {
        let n = words_mut::<u32>(control, PLACES + 4 * R * i, R);
        for r in 0..R {
            n[r] = if base[r] == UNCLAIMED { UNCLAIMED } else { base[r] + n[r] };
        }
    }
    let placed: u32 = (0..R).filter(|&r| base[r] != UNCLAIMED).map(|r| total[r]).sum();
    set_word::<u64>(control, PLACED, u64::from(placed));
}

/// `place`: each statement walked again, its rows written at its places with the run's sign.
fn place(c: &mut Ctx<'_, '_, '_>, tile: Tile) {
    let p = <K0P as crate::Params>::read(c.params);
    if word::<u64>(c.read(CONTROL), REFUSED_NODE) != u64::MAX {
        return;
    }
    let ([tape, syms, atoms, control], runs) = c.views([TAPE, SYMS, ATOMS, CONTROL], std::array::from_fn::<usize, R, _>(|r| RUNS + r));
    let src = std::str::from_utf8(&tape[p.src_at as usize..(p.src_at + p.src_len) as usize]).expect("a source the file layer checked is UTF-8");
    let t = Reads::in_place(&tape[p.tape_at as usize..(p.tape_at + p.tape_len) as usize], src).expect("a tape the parser wrote");
    let mut runs = runs;
    for (i, s) in tile.items().zip(statements(&t).skip(tile.items().start)) {
        let mut at: [u32; R] = words::<u32>(control, PLACES + 4 * R * i, R).try_into().expect("a statement's places");
        let walked = statement(&t, p.module, syms, atoms, s, &mut |r, row| {
            if at[r] != UNCLAIMED {
                runs_set_in(runs[r], at[r], row, p.diff as i32);
                at[r] += 1;
            }
        });
        debug_assert!(walked.is_ok(), "a statement `count` walked");
    }
}

/// `k0_pass`'s phases: count, scan, place.
pub const K0_PASS: &[TwinPhase] = &[
    TwinPhase { name: "count", items: statement_items, run: count, again: None },
    TwinPhase { name: "scan", items: |_| 1, run: scan, again: None },
    TwinPhase { name: "place", items: statement_items, run: place, again: None },
];
