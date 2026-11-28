//! L1, the boot operation (decision 0686, B6; B§4, B§5, B§9; the rebuild plan's target): a delta operator of one
//! tape, no program, one production (K-form's eight relations) and facts in that are program globals (the symbols).
//!
//! Its ports, B§4's exactly:
//!
//! - **tape** (B4): a row a document, `(d, source, tape, symbols)`, each a blob by its id (P§11.4's document set,
//!   path → tape id): its source, its tape, and the node → symbol facts `intern` wrote for it; a changed file is its
//!   old row gone and its new row added, an unchanged one the identity;
//! - **no program**: L1 reads no K-form;
//! - **facts in** (B5): the program globals, a global port each (`contract::Global`): the atoms, the one every level
//!   shares, read in place;
//! - **output** (B2): P2's bistate in the program trace, a trace a K-form relation (`kform::shape`).
//!
//! Its strategy is file grain (B§4: "an edited l0 is read again whole and diffed against its kept rows"): the
//! documents added are read whole by one `k0_pass` launch into the outputs' δ runs; a document gone has its kept rows retracted, read in
//! its output traces where they lie (`drop_parts`, P§3: "the file's last result is its output trace's before, not
//! cache"), never its old tape; and `consolidate` nets the two, so the δ is the diff against the rows it kept. An
//! unchanged tape is L4's identity: no operation runs, and the cache is as it was.
//!
//! Its cache (Bootstrap, Figure 2: `k0_pass` rw L1's cache; Operations §6.1) holds what it reads to go fast and could
//! make again, partitioned by document: each document's sizes, the rows `k0_pass` gave it by relation. A run reads
//! them, before it writes the cache, for the room its outputs' runs need: the gone documents' kept rows, and the
//! added ones' as they were. Its runs opened at that room, it does not outgrow them and run again. `k0_pass` keeps
//! the sizes of the documents it reads and lets go of the gone ones'. The kept rows are not in it: they are the
//! output traces' before (P§3). Dropped, the cache costs the next run an attempt; its rows are the same (L5).
//! Its Rust beyond `k0_pass` and `drop_parts` is the wiring of these ports.

use crate::cache::Sizes;
use crate::contract::{Asks, At, CacheW, Cx, DeltaOp, In, Out, Port, PortGlobal, PortIn, PortOut, PortSet, Ports, Work, outs};
use crate::dir::{BlockId, Directory};
use crate::drop::{Parts, drop_parts};
use crate::semi::{self, Side};
use crate::trace::open_runs;
use rerail_blocks::layer::Runs;
use crate::files::{BlobId, DocIds};
use crate::k0::{DocsIn, RowCols, k0_pass, room_of};
use crate::kform::{self, RELATIONS};
use rerail_blocks::bistate::{Bistate, Read};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;

/// L1's port set (B§4's ports): the tape, the atoms, and K-form's eight relations.
pub struct L1Ports;

impl L1Ports {
    /// The tape port.
    pub const TAPE: PortIn<L1Ports> = PortIn::new(0);
    /// The atoms, the one program global.
    pub const ATOMS: PortGlobal<L1Ports> = PortGlobal::new(0);

    /// The output port of K-form relation `RELATIONS[k]`.
    pub fn relation(k: usize) -> PortOut<L1Ports> {
        assert!(k < RELATIONS.len(), "a K-form relation");
        PortOut::new(k)
    }
}

impl PortSet for L1Ports {
    const INS: usize = 1;
    const GLOBALS: usize = 1;
    const OUTS: usize = RELATIONS.len();

    fn ports() -> Ports {
        Ports {
            ins: vec![Port { name: "tape", shape: tape_shape() }],
            globals: vec!["atoms"],
            outs: RELATIONS.iter().enumerate().map(|(i, (name, _))| Port { name, shape: kform::shape(i) }).collect(),
        }
    }

    /// It asks of its tape port: the documents it needs, by key (P§7.1).
    const ASKED: &'static [usize] = &[0];
}

/// The tape port's rows, P§11.4's document set (path → tape id, grammar; indexed by path, partitioned by document):
/// `(d, path, grammar, source, tape, symbols)`: the document's key (decision 0052 §3, from its path's hash), its path's
/// symbol, its grammar's word, and each blob by its four words.
pub fn tape_shape() -> Shape {
    let c = [ColType::I32, ColType::I32].into_iter().chain(std::iter::repeat_n(ColType::I64, 13)).collect();
    let mut s = Shape::run(c, (0..15).collect(), Some(0));
    s.indexes = vec![rerail_blocks::layer::IndexSpec::Hash(vec![TAPE_PATH as u16])];
    s
}

/// The tape port row's columns: the path's symbol, and the first word of each blob.
pub const TAPE_PATH: usize = 1;
pub const TAPE_SOURCE: usize = 3;
pub const TAPE_TAPE: usize = 7;
pub const TAPE_SYMS: usize = 11;

/// Document `d`'s row in the tape port: at `path` (its symbol), in `grammar`, its blobs `doc` and its symbols blob.
pub fn tape_row(d: i64, path: i64, grammar: i64, doc: &DocIds, syms: &BlobId) -> [i64; 15] {
    let (src, tape, sy) = (doc.source.words(), doc.tape.words(), syms.words());
    [d, path, grammar, src[0], src[1], src[2], src[3], tape[0], tape[1], tape[2], tape[3], sy[0], sy[1], sy[2], sy[3]]
}

/// A tape port row's blobs: its source and tape, and its symbols.
pub fn row_blobs(row: &[i64]) -> (DocIds, BlobId) {
    let b = |c: usize| BlobId::of_words(&row[c..c + 4]);
    (DocIds { source: b(TAPE_SOURCE), tape: b(TAPE_TAPE) }, b(TAPE_SYMS))
}

/// L1: `k0_pass` over its tape port's changed documents, its operations the runner's (`Cx::ops`).
pub struct L1;

/// The want rows' key column (`contract::want_shape`: `(port, key)`, partitioned by key): L1's is a document.
const WANT_KEY: usize = 1;

/// Where a tape port row's module and its blobs lie: `k0::DocsIn::of_rows`'s columns.
pub const DOC_COLS: RowCols = RowCols { module: 0, source: TAPE_SOURCE, tape: TAPE_TAPE, syms: TAPE_SYMS };

/// The tape port's arity (`tape_shape`).
pub const TAPE_ARITY: usize = 15;

/// L1's selection, written where it lies: which documents a run retracts (a parts block) and reads (a runs block of
/// their tape rows, +1), and its asks of the tape port, by `semi_change` of the tape port's change with the want's
/// on the document (`change_rows`, with no want: every document wanted). Each id of the tape port is a row of the
/// selection once at most, and only a change of the want reaches its kept rows: the room is the change's counts
/// (the layers', P§1), or the tape's ids when the want moved.
struct Selected {
    rows: BlockId,
    parts: Parts,
}

impl Selected {
    fn release(self, dir: &mut Directory) {
        dir.release(self.rows);
        self.parts.release(dir);
    }
}

impl L1 {
    /// `k0_pass` over documents `docs`, one launch, into the outputs' runs (+1), their sizes kept in `cache` where the
    /// modules `gone` let go of theirs.
    #[allow(clippy::too_many_arguments)]
    fn pass(&self, cx: &mut Cx<'_>, atoms: BlockId, docs: &DocsIn, gone: Option<&Parts>, out: &Out<L1Ports>, cache: Option<BlockId>, work: &mut Work) -> Result<(), String> {
        let runs: [BlockId; RELATIONS.len()] = core::array::from_fn(|k| out.block(L1Ports::relation(k)));
        work.rows_read += docs.n as u64;
        work.rows_written += k0_pass(cx.ops, cx.dir, atoms, docs, gone, 1, &runs, cache).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl DeltaOp for L1 {
    type Ports = L1Ports;
    type Cache = Sizes;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, L1Ports>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<Sizes>, out: &mut Out<L1Ports>, asks: &mut Asks<L1Ports>) -> Result<Work, String> {
        let work = Work::default();
        // L1's grain is the document: a wanted relation of a document is all of that document's relations
        // (over-demand at its grain, which D1 allows).
        if ins.is_identity() && want.is_none_or(Bistate::is_identity) {
            return Ok(work);
        }
        let atoms = ins.global(L1Ports::ATOMS).block;
        let sel = self.select(cx, ins, want, asks)?;
        // The documents read: the selection's added rows, joined with the blobs' rows by `docs_of`; the directory binds
        // the blocks they name.
        let docs = Runs::new(cx.dir.read("L1's selection", &[sel.rows])?.block(0)).len();
        let docs = DocsIn::of_rows(cx.ops, cx.dir, sel.rows, docs, DOC_COLS, cx.blobs.head());
        let docs = match docs {
            Ok(d) => d,
            Err(e) => {
                sel.release(cx.dir);
                return Err(e);
            }
        };
        let ran = self.retract_and_read(cx, atoms, &docs, &sel.parts, cache, out, work);
        docs.release(cx.dir);
        sel.release(cx.dir);
        ran
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, L1Ports>, out: &mut Out<L1Ports>) -> Result<(), String> {
        // Every document of the tape port: its state's rows, from ∅ (`change_rows`, cold), read whole.
        let atoms = ins.global(L1Ports::ATOMS).block;
        let a = Side { before: None, after: ins.head(L1Ports::TAPE), arity: TAPE_ARITY, col: 0 };
        let n = Read::end(ins.port(L1Ports::TAPE)).get() as usize;
        let rows = open_runs(cx.dir, &tape_shape().encode()?, n);
        let none = match Parts::open(cx.dir, 0) {
            Ok(p) => p,
            Err(e) => {
                cx.dir.release(rows);
                return Err(e);
            }
        };
        let docs = semi::select(cx.ops, cx.dir, &a, None, semi::How::RETRACTED, rows, none).and_then(|()| DocsIn::of_rows(cx.ops, cx.dir, rows, n, DOC_COLS, cx.blobs.head()));
        cx.dir.release(rows);
        cx.dir.release(none);
        let docs = docs?;
        let mut work = Work::default();
        let ran = self.pass(cx, atoms, &docs, None, out, None, &mut work);
        docs.release(cx.dir);
        ran
    }
}

impl L1 {
    /// The run's selection (`Selected`): one launch.
    fn select(&self, cx: &mut Cx<'_>, ins: &In<'_, L1Ports>, want: Option<&Bistate<View<'_>>>, asks: &Asks<L1Ports>) -> Result<Selected, String> {
        let tape = ins.port(L1Ports::TAPE);
        let (before, after) = ins.heads(L1Ports::TAPE);
        let a = Side { before, after, arity: TAPE_ARITY, col: 0 };
        let (gone, added) = tape.counts();
        let (want_gone, moved) = want.map_or((0, false), |w| (w.counts().0, !w.is_identity()));
        let rows_cap = if moved { Read::end(tape.after()).get() as usize } else { gone + added };
        let b = match (want, ins.want_heads()) {
            (Some(w), Some((before, after))) => Some((Side { before, after, arity: Read::arity(w.after()), col: WANT_KEY }, asks.block(L1Ports::TAPE)?)),
            _ => None,
        };
        let rows = open_runs(cx.dir, &tape_shape().encode()?, rows_cap);
        let parts = match Parts::open(cx.dir, gone + want_gone) {
            Ok(p) => p,
            Err(e) => {
                cx.dir.release(rows);
                return Err(e);
            }
        };
        let ran = semi::select(cx.ops, cx.dir, &a, b.as_ref().map(|(s, k)| (s, *k)), semi::How::RETRACTED, rows, parts).and_then(|()| {
            let r = Runs::new(cx.dir.read("L1's selection", &[rows])?.block(0)).needed();
            if r > 0 { Err(format!("L1's selection outgrew its room by {r} rows: its bound is wrong")) } else { Parts::sealed(cx.dir, parts) }
        });
        match ran {
            Ok(parts) => Ok(Selected { rows, parts }),
            Err(e) => {
                cx.dir.release(rows);
                cx.dir.release(parts);
                Err(e)
            }
        }
    }

    /// A delta's retractions and reads, its documents `docs` and its parts `parts` written: the room its runs need
    /// from the cache's sizes, then `drop_parts` over each kept output and one `k0_pass`.
    #[allow(clippy::too_many_arguments)]
    fn retract_and_read(&self, cx: &mut Cx<'_>, atoms: BlockId, docs: &DocsIn, parts: &Parts, cache: &mut CacheW<Sizes>, out: &mut Out<L1Ports>, mut work: Work) -> Result<Work, String> {
        if docs.is_empty() && parts.is_empty() {
            // Only its demand moved (or a document no one wants): nothing to read or retract.
            return Ok(work);
        }
        // The room the run needs (`room_of`), so it never outgrows its runs and runs again: each gone document's kept
        // rows, from the cache's sizes, and each read one's nodes, a bound on its rows. A cache read as ∅ names no
        // gone document's: their retractions are then bounded by each kept relation's rows, read at its head.
        let sized = cache.read(cx)?.is_some();
        let cache = cache.write(cx)?;
        let need = room_of(cx.ops, cx.dir, cache, parts, docs)?;
        for (k, n) in need.into_iter().enumerate() {
            let kept = match out.kept(L1Ports::relation(k)) {
                Some(h) if !sized && !parts.is_empty() => {
                    let l = cx.dir.read("a kept relation's rows", &[h])?;
                    let table = l.table();
                    let v = View::chain(&table, Some(h.0), RELATIONS[k].1)?;
                    Read::end(&v).get() as usize
                }
                _ => 0,
            };
            out.room(cx.dir, L1Ports::relation(k), n as usize + kept)?;
        }
        work.rows_read += parts.len() as u64;
        for k in outs::<L1Ports>() {
            if let Some(kept) = out.kept(k) {
                work.rows_written += drop_parts(cx.ops, cx.dir, kept, parts, -1, out.block(k))?;
            }
        }
        self.pass(cx, atoms, docs, Some(parts), out, Some(cache), &mut work)?;
        Ok(work)
    }
}

#[cfg(test)]
#[path = "l1_tests.rs"]
mod tests;
