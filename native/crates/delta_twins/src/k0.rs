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
//! operation over every document of a run at once (one launch): its documents are a docs block's, which refers to
//! each one's source, tape and symbols blocks (and to the next docs block, past `DOCS_PER_BLOCK`), so the launch reads
//! them all where they lie. A statement of any document is an item, in three phases (emit, shape 11: output in order,
//! no atomics): `count` walks each statement and counts its rows by relation, and writes the first refusal by
//! document and node order into control; `scan` turns the counts into each statement's places, claimed at once in
//! each relation's δ runs block; `place` walks each statement again and writes its rows at its places, with the run's
//! sign. A refusal writes no row.

use crate::intern as syms;
use crate::tape::Reads;
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::atoms::find_in;
use rerail_blocks::gens::{BODY_AT, MAX_REFS, Table, set_word, word, words, words_mut};
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
    /// The first docs block (its id in the descriptor table) and the run's sign (+1 or -1).
    pub struct K0P {
        pub docs: u64,
        pub diff: i64,
    }
}

/// The slots: the first docs block, the atoms (read), control (rw), then the δ runs block of each K-form relation
/// (write), in `RELATIONS`' order.
pub const DOCS: usize = 0;
pub const ATOMS: usize = 1;
pub const CONTROL: usize = 2;
pub const RUNS: usize = 3;
const R: usize = 8;
/// L1's cache (read-write): each document's sizes, the rows it was given by relation (`cache::keep_sizes`).
pub const CACHE: usize = RUNS + R;

/// A docs block's words, at `BODY_AT`: its body's length, its documents' count, the next docs block (`NO_NEXT`:
/// none); then each document's: its module, its source's, its tape's and its symbols' blocks, where its source and
/// its tape lie in theirs.
const DOCS_LEN_AT: usize = BODY_AT;
const DOCS_N_AT: usize = BODY_AT + 8;
const DOCS_NEXT_AT: usize = BODY_AT + 16;
const DOCS_AT: usize = BODY_AT + 24;
const DOC_WORDS: usize = 8;
pub const NO_NEXT: u32 = u32::MAX;
/// The documents a docs block refers to: three references each (its source's, its tape's and its symbols' blocks),
/// and one to the next docs block.
pub const DOCS_PER_BLOCK: usize = (MAX_REFS - 1) / 3;

/// One document of a docs block.
#[derive(Clone, Copy, Debug)]
pub struct DocAt {
    pub module: i64,
    pub source: u32,
    pub tape: u32,
    pub syms: u32,
    pub src: (u64, u64),
    pub at: (u64, u64),
}

/// A docs block's body bytes for `n` documents.
pub fn docs_bytes(n: usize) -> usize {
    DOCS_AT - BODY_AT + 8 * DOC_WORDS * n
}

/// A docs block, written by the host: its documents and the next docs block.
pub fn init_docs(bytes: &mut [u8], docs: &[DocAt], next: u32) {
    assert!(docs.len() <= DOCS_PER_BLOCK, "a docs block's references");
    set_word::<u64>(bytes, DOCS_LEN_AT, docs_bytes(docs.len()) as u64);
    set_word::<u64>(bytes, DOCS_N_AT, docs.len() as u64);
    set_word::<u64>(bytes, DOCS_NEXT_AT, u64::from(next));
    for (i, d) in docs.iter().enumerate() {
        let w = [d.module as u64, u64::from(d.source), u64::from(d.tape), u64::from(d.syms), d.src.0, d.src.1, d.at.0, d.at.1];
        for (k, x) in w.into_iter().enumerate() {
            set_word::<u64>(bytes, DOCS_AT + 8 * (DOC_WORDS * i + k), x);
        }
    }
}

/// One document of the launch, read where it lies: its module, its tape's reads, its symbols, its statements' count.
struct Doc<'t> {
    module: i64,
    t: Reads<'t>,
    syms: &'t [u8],
    n: usize,
}

/// The launch's documents, in the docs blocks' order, each read through the descriptor table where it lies, one at a
/// time.
fn docs_of<'c, 't>(table: &'c Table<'t>, first: u32) -> impl Iterator<Item = Doc<'t>> + 'c {
    let (mut at, mut i) = (Some(first), 0usize);
    core::iter::from_fn(move || {
        loop {
            let b = table.block(at?).expect("a docs block, reached by reference");
            if i < word::<u64>(b, DOCS_N_AT) as usize {
                let w = |k: usize| word::<u64>(b, DOCS_AT + 8 * (DOC_WORDS * i + k));
                let source = table.block(w(1) as u32).expect("a source's block, reached by reference");
                let tape = table.block(w(2) as u32).expect("a tape's block, reached by reference");
                let src = core::str::from_utf8(&source[w(4) as usize..(w(4) + w(5)) as usize]).expect("a source the file layer checked is UTF-8");
                let t = Reads::in_place(&tape[w(6) as usize..(w(6) + w(7)) as usize], src).expect("a tape the parser wrote");
                let syms = table.block(w(3) as u32).expect("a symbols block, reached by reference");
                let (n, module) = (statements(&t).count(), w(0) as i64);
                i += 1;
                return Some(Doc { module, t, syms, n });
            }
            let next = word::<u64>(b, DOCS_NEXT_AT) as u32;
            (at, i) = ((next != NO_NEXT).then_some(next), 0);
        }
    })
}

/// `f` of each statement whose item (its place among every document's statements, in order) is in `items`: its
/// document's place, the document, its node and its item.
fn each<'t>(docs: impl Iterator<Item = Doc<'t>>, items: core::ops::Range<usize>, f: &mut dyn FnMut(usize, &Doc<'t>, usize, usize)) {
    let mut first = 0;
    for (k, d) in docs.enumerate() {
        if first >= items.end {
            break;
        }
        if first + d.n > items.start {
            let skip = items.start.saturating_sub(first);
            for (j, s) in statements(&d.t).enumerate().skip(skip).take(items.end - first - skip) {
                f(k, &d, s, first + j);
            }
        }
        first += d.n;
    }
}

/// Control's words, at `BODY_AT`: the first refused node (`u64::MAX`: none), why (`WHY`'s place) and its document's
/// place, the rows placed; then each statement's places, a relation a word (counts, then places, after `scan`).
pub const REFUSED_NODE: usize = BODY_AT;
pub const REFUSED_WHY: usize = BODY_AT + 8;
pub const REFUSED_DOC: usize = BODY_AT + 12;
pub const PLACED: usize = BODY_AT + 16;
/// The documents gone from L1's tape port (their cache entries let go): their count, then their modules.
const GONE_N: usize = BODY_AT + 24;
/// Each relation's end in its runs block after `scan` (its first place past the run's rows; `UNCLAIMED`: none).
const ENDS: usize = BODY_AT + 32;
const GONE: usize = ENDS + 4 * R;

/// Where control's statement places start, past its gone documents.
fn places_at(control: &[u8]) -> usize {
    GONE + (4 * word::<u64>(control, GONE_N) as usize).next_multiple_of(8)
}
/// A relation whose places could not be claimed (its runs block too small: the driver runs again).
const UNCLAIMED: u32 = u32::MAX;

/// Control's body bytes for documents of `n` nodes in all (at most `n` statements), and `gone` documents gone.
pub fn control_bytes(n: usize, gone: usize) -> usize {
    GONE - BODY_AT + (4 * gone).next_multiple_of(8) + 4 * R * n.max(1)
}

/// Control laid out before a run: no refusal, no row placed, and the documents gone.
pub fn init_control(bytes: &mut [u8], gone: &[u32]) {
    set_word::<u64>(bytes, REFUSED_NODE, u64::MAX);
    set_word::<u64>(bytes, PLACED, 0);
    set_word::<u64>(bytes, GONE_N, gone.len() as u64);
    words_mut::<u32>(bytes, GONE, gone.len()).copy_from_slice(gone);
}

/// The run's refusal, if any, the first by document and node order: its document's place, its node, and why.
pub fn refused(control: &[u8]) -> Option<(usize, usize, &'static str)> {
    let n = word::<u64>(control, REFUSED_NODE);
    (n != u64::MAX).then(|| (word::<u32>(control, REFUSED_DOC) as usize, n as usize, WHY[word::<u32>(control, REFUSED_WHY) as usize]))
}

/// The rows the run placed.
pub fn placed(control: &[u8]) -> u64 {
    word::<u64>(control, PLACED)
}

/// The launch's params, and its descriptor table its documents are read through.
fn launch<'c, 't>(c: &Ctx<'c, '_, 't>) -> (K0P, &'c Table<'t>) {
    (<K0P as crate::Params>::read(c.params), c.table)
}

fn statement_items(c: &Ctx<'_, '_, '_>) -> u32 {
    let (p, table) = launch(c);
    docs_of(table, p.docs as u32).map(|d| d.n).sum::<usize>() as u32
}

/// `count`: each statement's rows by relation; a refusal, the first by document and node order, into control.
fn count(c: &mut Ctx<'_, '_, '_>, tile: Tile) {
    let (p, table) = launch(c);
    let ([atoms], [control]) = c.views([ATOMS], [CONTROL]);
    // Each statement's counts written to its places' words as it is walked.
    let at = places_at(control);
    let mut first: Option<(usize, usize, &'static str)> = None;
    each(docs_of(table, p.docs as u32), tile.items(), &mut |k, d, s, i| {
        let mut n = [0u32; R];
        if let Err((node, why)) = statement(&d.t, d.module, d.syms, atoms, s, &mut |r, _| n[r] += 1) {
            let e = (k, node, why);
            first = Some(first.map_or(e, |f| if (e.0, e.1) < (f.0, f.1) { e } else { f }));
        }
        words_mut::<u32>(control, at + 4 * R * i, R).copy_from_slice(&n);
    });
    if let Some((k, node, why)) = first {
        let was = (word::<u64>(control, REFUSED_NODE) != u64::MAX).then(|| (word::<u32>(control, REFUSED_DOC) as usize, word::<u64>(control, REFUSED_NODE) as usize));
        if was.is_none_or(|w| (k, node) < w) {
            set_word::<u64>(control, REFUSED_NODE, node as u64);
            set_word::<u32>(control, REFUSED_WHY, WHY.iter().position(|w| *w == why).expect("a reason of WHY") as u32);
            set_word::<u32>(control, REFUSED_DOC, k as u32);
        }
    }
}

/// `scan`: one item. The counts' exclusive prefix sums by relation, from the places claimed in each runs block at once;
/// a refused run claims none.
fn scan(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let items = statement_items(c) as usize;
    // Control, then each relation's runs block.
    let ([], v) = c.views([], core::array::from_fn::<usize, { R + 1 }, _>(|i| if i == 0 { CONTROL } else { RUNS + i - 1 }));
    let control = &mut *v[0];
    if word::<u64>(control, REFUSED_NODE) != u64::MAX {
        return;
    }
    let at = places_at(control);
    let mut total = [0u32; R];
    for i in 0..items {
        let n = words_mut::<u32>(control, at + 4 * R * i, R);
        // An exclusive scan: each count's place is the total before it.
        for (x, t) in n.iter_mut().zip(total.iter_mut()) {
            let k = *x;
            *x = *t;
            *t += k;
        }
    }
    let mut base = [0u32; R];
    for r in 0..R {
        base[r] = if total[r] == 0 { 0 } else { runs_claim_in(&mut *v[1 + r], total[r]).unwrap_or(UNCLAIMED) };
    }
    let control = &mut *v[0];
    for i in 0..items {
        let n = words_mut::<u32>(control, at + 4 * R * i, R);
        for r in 0..R {
            n[r] = if base[r] == UNCLAIMED { UNCLAIMED } else { base[r] + n[r] };
        }
    }
    let ends: [u32; R] = core::array::from_fn(|r| if base[r] == UNCLAIMED { UNCLAIMED } else { base[r] + total[r] });
    words_mut::<u32>(control, ENDS, R).copy_from_slice(&ends);
    let placed: u32 = (0..R).filter(|&r| base[r] != UNCLAIMED).map(|r| total[r]).sum();
    set_word::<u64>(control, PLACED, u64::from(placed));
}

/// `place`: each statement walked again, its rows written at its places with the run's sign.
fn place(c: &mut Ctx<'_, '_, '_>, tile: Tile) {
    let (p, table) = launch(c);
    if word::<u64>(c.read(CONTROL), REFUSED_NODE) != u64::MAX {
        return;
    }
    let ([atoms, control], mut runs) = c.views([ATOMS, CONTROL], core::array::from_fn::<usize, R, _>(|r| RUNS + r));
    let places = places_at(control);
    each(docs_of(table, p.docs as u32), tile.items(), &mut |_, d, s, i| {
        let mut at: [u32; R] = words::<u32>(control, places + 4 * R * i, R).try_into().expect("a statement's places");
        let walked = statement(&d.t, d.module, d.syms, atoms, s, &mut |r, row| {
            if at[r] != UNCLAIMED {
                runs_set_in(runs[r], at[r], row, p.diff as i32);
                at[r] += 1;
            }
        });
        debug_assert!(walked.is_ok(), "a statement `count` walked");
    });
}

/// `keep`: one item. L1's cache: the documents gone let go of their entries, and each document's sizes, the rows it
/// was given by relation (its statements' places, end to start), kept for L1's next run, which opens its runs at the
/// room they name (Operations §6.1: what an operator reads to go fast and could make again). A run refused, or one
/// whose runs were too small (the driver runs it again from a cache read as ∅), keeps no size.
fn keep(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let (p, table) = launch(c);
    if word::<u64>(c.read(CONTROL), REFUSED_NODE) != u64::MAX {
        return;
    }
    // The documents gone, read where control holds them, let go of their entries.
    {
        let ([control], [cache]) = c.views([CONTROL], [CACHE]);
        for &g in words::<u32>(control, GONE, word::<u64>(control, GONE_N) as usize) {
            crate::cache::forget(cache, g);
        }
    }
    let control = c.read(CONTROL);
    let ends: [u32; R] = words::<u32>(control, ENDS, R).try_into().expect("the relations' ends");
    if ends.contains(&UNCLAIMED) {
        return;
    }
    let items = docs_of(table, p.docs as u32).map(|d| d.n).sum::<usize>();
    let start = |control: &[u8], i: usize, r: usize| if i == items { ends[r] } else { word::<u32>(control, places_at(control) + 4 * (R * i + r)) };
    // Each document's sizes, from its statements' places, kept as it is read.
    let mut first = 0;
    for d in docs_of(table, p.docs as u32) {
        let n: [u32; R] = {
            let control = c.read(CONTROL);
            core::array::from_fn(|r| start(control, first + d.n, r) - start(control, first, r))
        };
        if !crate::cache::keep_sizes(c, CACHE, d.module as u32, &n) {
            return;
        }
        first += d.n;
    }
}

/// `k0_pass`'s phases: count, scan, place, keep.
pub const K0_PASS: &[TwinPhase] = &[
    TwinPhase { name: "count", items: statement_items, run: count, again: None },
    TwinPhase { name: "scan", items: |_| 1, run: scan, again: None },
    TwinPhase { name: "place", items: statement_items, run: place, again: None },
    TwinPhase { name: "keep", items: |_| 1, run: keep, again: None },
];
