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
//! operation over every document of a run at once (one launch): its documents are a docs block's entries, the blocks
//! they name bound to the launch by reference, so the launch reads them all where they lie. `docs_of` writes those
//! entries: the selection's rows joined with the blobs' rows, read where they lie. A statement of any document is an item, in three phases (emit, shape 11: output in order,
//! no atomics): `count` walks each statement and counts its rows by relation, and writes the first refusal by
//! document and node order into control; `scan` turns the counts into each statement's places, claimed at once in
//! each relation's δ runs block; `place` walks each statement again and writes its rows at its places, with the run's
//! sign. A refusal writes no row.

use crate::intern as syms;
use crate::tape::Reads;
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::atoms::find_in;
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::gens::{BODY_AT, Table, set_word, word, words, words_mut};
use rerail_blocks::layer::Runs;
use rerail_blocks::relation::View;
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

/// The slots: the first docs block, the atoms (read), control (rw), the documents gone (read: a parts block,
/// `drop::parts`, the one `drop_parts` reads), then the δ runs block of each K-form relation (write), in `RELATIONS`'
/// order.
pub const DOCS: usize = 0;
pub const ATOMS: usize = 1;
pub const CONTROL: usize = 2;
pub const GONE: usize = 3;
pub const RUNS: usize = 4;
pub const R: usize = 8;
/// L1's cache (read-write): each document's sizes, the rows it was given by relation (`cache::keep_sizes`).
pub const CACHE: usize = RUNS + R;
/// The `.rerail` language's table (read; `intern::write_language`): what a kernel reads a node's kind and field by.
pub const LANG: usize = CACHE + 1;

/// A docs block's words, at `BODY_AT`: its body's length, its documents' count, the first document a blob of which no
/// blobs' row names (`NO_DOC`: none; `docs_of`'s); then each document's: its module, its source's, its tape's and its
/// symbols' blocks, where its source and its tape lie in theirs. The blocks they name are bound to the launch by
/// reference, not by the docs block (it refers to at most `MAX_REFS`).
pub const DOCS_LEN_AT: usize = BODY_AT;
pub const DOCS_N_AT: usize = BODY_AT + 8;
pub const DOCS_MISSING_AT: usize = BODY_AT + 16;
pub const DOCS_AT: usize = BODY_AT + 24;
pub const DOC_WORDS: usize = 8;
pub const NO_DOC: i64 = i64::MIN;

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

/// A docs block opened (its room `docs_bytes(n)`): no document yet, none missing. Its documents are written where they
/// lie as they are found (`put_doc`), then it is sealed (`seal_docs`).
pub fn open_docs(bytes: &mut [u8]) {
    set_word::<u64>(bytes, DOCS_LEN_AT, docs_bytes(0) as u64);
    set_word::<u64>(bytes, DOCS_N_AT, 0);
    set_word::<i64>(bytes, DOCS_MISSING_AT, NO_DOC);
}

/// Document `d` at place `i` of an open docs block.
pub fn put_doc(bytes: &mut [u8], i: usize, d: &DocAt) {
    let w = [d.module as u64, u64::from(d.source), u64::from(d.tape), u64::from(d.syms), d.src.0, d.src.1, d.at.0, d.at.1];
    for (k, x) in w.into_iter().enumerate() {
        set_word::<u64>(bytes, DOCS_AT + 8 * (DOC_WORDS * i + k), x);
    }
}

/// The first document a blob of which no blobs' row names (`docs_of`'s), if any.
pub fn docs_missing(bytes: &[u8]) -> Option<i64> {
    let d = word::<i64>(bytes, DOCS_MISSING_AT);
    (d != NO_DOC).then_some(d)
}

/// A sealed docs block's documents' count, and its module at place `i`.
pub fn docs_n(bytes: &[u8]) -> usize {
    word::<u64>(bytes, DOCS_N_AT) as usize
}

pub fn docs_module(bytes: &[u8], i: usize) -> i64 {
    word::<u64>(bytes, DOCS_AT + 8 * DOC_WORDS * i) as i64
}

/// The open docs block sealed, its first `n` documents listed.
pub fn seal_docs(bytes: &mut [u8], n: usize) {
    set_word::<u64>(bytes, DOCS_LEN_AT, docs_bytes(n) as u64);
    set_word::<u64>(bytes, DOCS_N_AT, n as u64);
}

/// One document of the launch, read where it lies: its module, its tape's reads, its symbols, its statements' count.
struct Doc<'t> {
    module: i64,
    t: Reads<'t>,
    syms: &'t [u8],
    n: usize,
}

/// The launch's documents, in the docs block's order, each read through the descriptor table where it lies, one at a
/// time.
fn docs_of<'c, 't>(table: &'c Table<'t>, at: u32) -> impl Iterator<Item = Doc<'t>> + 'c {
    let b = table.block(at).expect("a docs block, a slot");
    (0..word::<u64>(b, DOCS_N_AT) as usize).map(move |i| {
        let w = |k: usize| word::<u64>(b, DOCS_AT + 8 * (DOC_WORDS * i + k));
        let source = table.block(w(1) as u32).expect("a source's block, reached by reference");
        let tape = table.block(w(2) as u32).expect("a tape's block, reached by reference");
        let src = core::str::from_utf8(&source[w(4) as usize..(w(4) + w(5)) as usize]).expect("a source the file layer checked is UTF-8");
        let t = Reads::in_place(&tape[w(6) as usize..(w(6) + w(7)) as usize], src).expect("a tape the parser wrote");
        let syms = table.block(w(3) as u32).expect("a symbols block, reached by reference");
        Doc { module: w(0) as i64, n: statements(&t).count(), t, syms }
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

/// Control's words, at `BODY_AT`: the first refused node (`u64::MAX`: none), why (`WHY`'s place), its document's
/// place and module, the rows placed; then each statement's places, a relation a word (counts, then places, after
/// `scan`).
pub const REFUSED_NODE: usize = BODY_AT;
pub const REFUSED_WHY: usize = BODY_AT + 8;
pub const REFUSED_DOC: usize = BODY_AT + 12;
pub const PLACED: usize = BODY_AT + 16;
pub const REFUSED_MODULE: usize = BODY_AT + 24;
/// Each relation's end in its runs block after `scan` (its first place past the run's rows; `UNCLAIMED`: none).
pub const ENDS: usize = BODY_AT + 32;
/// Where control's statement places start.
pub const PLACES: usize = ENDS + 4 * R;
/// A relation whose places could not be claimed (its runs block too small: the driver runs again).
pub const UNCLAIMED: u32 = u32::MAX;

/// Control's body bytes for `n` statements' places.
pub fn control_bytes(n: usize) -> usize {
    PLACES - BODY_AT + 4 * R * n.max(1)
}

/// Where control's places end for `n` statements (`count` grows control to it).
fn control_end(n: usize) -> usize {
    PLACES + 4 * R * n.max(1)
}

/// Control laid out before a run: no refusal, no row placed.
pub fn init_control(bytes: &mut [u8]) {
    set_word::<u64>(bytes, REFUSED_NODE, u64::MAX);
    set_word::<u64>(bytes, PLACED, 0);
}

/// The run's refusal, if any, the first by document and node order: its document's module, its node, and why.
pub fn refused(control: &[u8]) -> Option<(i64, usize, &'static str)> {
    let n = word::<u64>(control, REFUSED_NODE);
    (n != u64::MAX).then(|| (word::<i64>(control, REFUSED_MODULE), n as usize, WHY[word::<u32>(control, REFUSED_WHY) as usize]))
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
    // Its statements' places, past control's room: the room they need recorded, and the phase run again grown.
    if !c.reach(CONTROL, control_end(tile.items().end)) {
        return;
    }
    let ([atoms], [control]) = c.views([ATOMS], [CONTROL]);
    // Each statement's counts written to its places' words as it is walked.
    let at = PLACES;
    let mut first: Option<(usize, usize, &'static str, i64)> = None;
    each(docs_of(table, p.docs as u32), tile.items(), &mut |k, d, s, i| {
        let mut n = [0u32; R];
        if let Err((node, why)) = statement(&d.t, d.module, d.syms, atoms, s, &mut |r, _| n[r] += 1) {
            let e = (k, node, why, d.module);
            first = Some(first.map_or(e, |f| if (e.0, e.1) < (f.0, f.1) { e } else { f }));
        }
        words_mut::<u32>(control, at + 4 * R * i, R).copy_from_slice(&n);
    });
    if let Some((k, node, why, module)) = first {
        let was = (word::<u64>(control, REFUSED_NODE) != u64::MAX).then(|| (word::<u32>(control, REFUSED_DOC) as usize, word::<u64>(control, REFUSED_NODE) as usize));
        if was.is_none_or(|w| (k, node) < w) {
            set_word::<u64>(control, REFUSED_NODE, node as u64);
            set_word::<u32>(control, REFUSED_WHY, WHY.iter().position(|w| *w == why).expect("a reason of WHY") as u32);
            set_word::<u32>(control, REFUSED_DOC, k as u32);
            set_word::<i64>(control, REFUSED_MODULE, module);
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
    let at = PLACES;
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
    let places = PLACES;
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
    // The documents gone, read where their parts block lists them, let go of their entries.
    {
        let ([gone], [cache]) = c.views([GONE], [CACHE]);
        for g in crate::drop::parts(gone) {
            crate::cache::forget(cache, g as u32);
        }
    }
    let control = c.read(CONTROL);
    let ends: [u32; R] = words::<u32>(control, ENDS, R).try_into().expect("the relations' ends");
    if ends.contains(&UNCLAIMED) {
        return;
    }
    let items = docs_of(table, p.docs as u32).map(|d| d.n).sum::<usize>();
    let start = |control: &[u8], i: usize, r: usize| if i == items { ends[r] } else { word::<u32>(control, PLACES + 4 * (R * i + r)) };
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

crate::params! {
    /// `docs_of`: the blobs' rows' head, and where a selection row's module and its source's, tape's and symbols'
    /// blob ids lie (four words each).
    pub struct DocsOfP {
        pub blobs: u64,
        pub module: u32,
        pub source: u32,
        pub tape: u32,
        pub syms: u32,
    }
}

/// `docs_of`'s slots: the selection's rows (read: a runs block, its +1 rows the documents read), the blobs' rows' head
/// (read; its layers below by reference), the docs block (write) and the blocks its documents name (write: a parts
/// block, `drop::parts`, each `(block << 32) | the furthest offset referred to`, sorted, each once).
pub const D_ROWS: usize = 0;
pub const D_BLOBS: usize = 1;
pub const D_DOCS: usize = 2;
pub const D_BLOCKS: usize = 3;

/// The blobs' rows' arity (`files::blobs_shape`: the name's four words, block, at, len).
const BLOB_ARITY: usize = 7;

/// Blob `w`'s place, as the blobs' rows at `s` name it: (block, at, len).
fn blob_at(s: &View<'_>, w: [i64; 4]) -> Option<(u32, u64, u64)> {
    let cols = [0, 1, 2, 3].map(ColumnIndex::new);
    let id = Read::matching(s, &cols, &w).next()?;
    let c = |k: u16| Read::get(s, id, ColumnIndex::new(k));
    Some((c(4) as u32, c(5) as u64, c(6) as u64))
}

/// `docs_of`'s one item: each +1 row of the selection a document, its blobs found by their names where the blobs' rows
/// lie, written into the docs block in the selection's order, and the blocks it names into the blocks' list.
fn docs_of_rows(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <DocsOfP as crate::Params>::read(c.params);
    let blobs = View::chain(c.table, Some(p.blobs as u32), BLOB_ARITY).expect("the blobs' rows, their layers reached by reference");
    let ([rows], [docs, blocks]) = c.views([D_ROWS], [D_DOCS, D_BLOCKS]);
    let rows = Runs::new(rows);
    let words = |i: usize, at: u32| core::array::from_fn::<i64, 4, _>(|k| rows.get(i, at as usize + k));
    let (mut n, mut nb) = (0usize, 0usize);
    for i in (0..rows.len()).filter(|&i| rows.diff(i) > 0) {
        let module = rows.get(i, p.module as usize);
        let (Some(s), Some(t), Some(y)) = (blob_at(&blobs, words(i, p.source)), blob_at(&blobs, words(i, p.tape)), blob_at(&blobs, words(i, p.syms))) else {
            if word::<i64>(docs, DOCS_MISSING_AT) == NO_DOC {
                set_word::<i64>(docs, DOCS_MISSING_AT, module);
            }
            continue;
        };
        put_doc(docs, n, &DocAt { module, source: s.0, tape: t.0, syms: y.0, src: (s.1, s.2), at: (t.1, t.2) });
        n += 1;
        for (b, at) in [(s.0, s.1), (t.0, t.1), (y.0, BODY_AT as u64)] {
            crate::drop::put_part(blocks, nb, (i64::from(b) << 32) | at as i64);
            nb += 1;
        }
    }
    seal_docs(docs, n);
    crate::drop::seal_parts(blocks, nb);
}

/// `docs_of`: one item.
pub const DOCS_OF: &[TwinPhase] = &[TwinPhase { name: "docs_of", items: |_| 1, run: docs_of_rows, again: None }];

/// `room_of`'s slots: L1's cache (read, holding the run's before), the documents gone (read: a parts block), the
/// documents read (read: a docs block) and the room (write: control, a word a K-form relation at `BODY_AT`).
pub const O_CACHE: usize = 0;
pub const O_GONE: usize = 1;
pub const O_DOCS: usize = 2;
pub const O_ROOM: usize = 3;

/// The room a run's runs need, by relation: each gone document's kept rows, from L1's cache (the sizes `keep` kept;
/// none for a document it holds no sizes of), and each read one's nodes: a K-form row names a node of its own in its
/// relation (a clause, a literal, an argument, a term), so a document gives each relation no more rows than it has
/// nodes, whatever the cache holds.
pub fn room(bytes: &[u8]) -> [u64; R] {
    core::array::from_fn(|r| word::<u64>(bytes, BODY_AT + 8 * r))
}

/// Document `i`'s nodes, as its symbols block counts them (read where it lies, by reference).
fn doc_nodes(table: &Table<'_>, docs: &[u8], i: usize) -> u64 {
    let syms = table.block(word::<u64>(docs, DOCS_AT + 8 * (DOC_WORDS * i + 3)) as u32).expect("a symbols block, reached by reference");
    crate::intern::count(syms) as u64
}

/// `room_of`'s one item.
fn room_of(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([cache, gone, docs], [room]) = c.views([O_CACHE, O_GONE, O_DOCS], [O_ROOM]);
    let mut need = [0u64; R];
    for d in crate::drop::parts(gone) {
        if let Some(s) = crate::cache::sizes(cache, d as u32) {
            for (n, &k) in need.iter_mut().zip(s) {
                *n += u64::from(k);
            }
        }
    }
    let nodes: u64 = (0..docs_n(docs)).map(|i| doc_nodes(table, docs, i)).sum();
    for (r, n) in need.into_iter().enumerate() {
        set_word::<u64>(room, BODY_AT + 8 * r, n + nodes);
    }
}

/// `room_of`: one item.
pub const ROOM_OF: &[TwinPhase] = &[TwinPhase { name: "room_of", items: |_| 1, run: room_of, again: None }];

crate::params! {
    /// `named_blocks`: the document set's head and arity, the blobs' rows' head, and where a set row's document and
    /// its source's, tape's and symbols' blob ids lie.
    pub struct NamedP {
        pub set: u64,
        pub blobs: u64,
        pub arity: u32,
        pub module: u32,
        pub source: u32,
        pub tape: u32,
        pub syms: u32,
        pub pad: u32,
    }
}

/// `named_blocks`' slots: the document set's head and the blobs' rows' head (read; each one's layers below by
/// reference), the blocks its documents' blobs lie in (write: a parts block of block ids, each once) and the
/// documents whose source or tape is in no blobs' row (write: a parts block of documents, each once: to load).
pub const N_SET: usize = 0;
pub const N_BLOBS: usize = 1;
pub const N_BLOCKS: usize = 2;
pub const N_MISSING: usize = 3;

/// `named_blocks`' one item: every live row of the set, its blobs found by their names where the blobs' rows lie.
fn named_blocks(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <NamedP as crate::Params>::read(c.params);
    let set = View::chain(c.table, Some(p.set as u32), p.arity as usize).expect("the document set, its layers reached by reference");
    let blobs = View::chain(c.table, Some(p.blobs as u32), BLOB_ARITY).expect("the blobs' rows, their layers reached by reference");
    let ([], [blocks, missing]) = c.views([], [N_BLOCKS, N_MISSING]);
    let words = |id, at: u32| core::array::from_fn::<i64, 4, _>(|k| Read::get(&set, id, ColumnIndex::new((at as usize + k) as u16)));
    let (mut nb, mut nm) = (0usize, 0usize);
    for id in Read::live_ids(&set) {
        let found = [p.source, p.tape, p.syms].map(|at| blob_at(&blobs, words(id, at)));
        for (b, _, _) in found.iter().flatten() {
            crate::drop::put_part(blocks, nb, i64::from(*b));
            nb += 1;
        }
        if found[0].is_none() || found[1].is_none() {
            crate::drop::put_part(missing, nm, Read::get(&set, id, ColumnIndex::new(p.module as u16)));
            nm += 1;
        }
    }
    crate::drop::seal_parts(blocks, nb);
    crate::drop::seal_parts(missing, nm);
}

/// `named_blocks`: one item.
pub const NAMED_BLOCKS: &[TwinPhase] = &[TwinPhase { name: "named_blocks", items: |_| 1, run: named_blocks, again: None }];
