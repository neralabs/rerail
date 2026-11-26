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
use crate::contract::{Asks, At, CacheW, Cx, DeltaOp, In, Out, Port, PortGlobal, PortIn, PortOut, PortSet, Ports, Work, outs, want_rows};
use crate::dir::{BlockId, Directory};
use crate::drop::{DropOps, drop_parts};
use crate::files::{BlobId, BlobRows, DocIds};
use crate::k0::{K0Doc, K0Ops, k0_pass};
use crate::kform::{self, RELATIONS};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;
use std::collections::BTreeSet;

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
pub fn tape_row(d: i64, path: i64, grammar: i64, doc: &DocIds, syms: &BlobId) -> Vec<i64> {
    [d, path, grammar].into_iter().chain(doc.source.words()).chain(doc.tape.words()).chain(syms.words()).collect()
}

/// A tape port row's blobs: its source and tape, and its symbols.
pub fn row_blobs(row: &[i64]) -> (DocIds, BlobId) {
    let b = |c: usize| BlobId::of_words(&row[c..c + 4]);
    (DocIds { source: b(TAPE_SOURCE), tape: b(TAPE_TAPE) }, b(TAPE_SYMS))
}

/// L1: `k0_pass` over its tape port's changed documents.
pub struct L1 {
    pub ops: K0Ops,
    pub drops: DropOps,
}

fn cell(v: &View<'_>, id: RowId, c: u16) -> i64 {
    Read::get(v, id, ColumnIndex::new(c))
}

/// The document of tape row `id` of `tape`, its blobs where `blobs` has them.
fn doc_at(blobs: &BlobRows, dir: &Directory, tape: &View<'_>, id: RowId) -> Result<K0Doc, String> {
    let blob = |at: u16| {
        let w: Vec<i64> = (at..at + 4).map(|c| cell(tape, id, c)).collect();
        blobs.get(dir, &BlobId::of_words(&w))?.ok_or_else(|| "a blob the tape port names, which nothing holds".to_string())
    };
    Ok(K0Doc { module: cell(tape, id, 0), src: blob(TAPE_SOURCE as u16)?, tape: blob(TAPE_TAPE as u16)?, syms: blob(TAPE_SYMS as u16)?.block })
}

fn live<'v>(v: &'v View<'_>) -> impl Iterator<Item = RowId> + 'v {
    (0..Read::end(v).get()).map(RowId::new).filter(|&id| Read::live(v, id))
}

impl L1 {
    /// `k0_pass` over documents `docs`, one launch, into the outputs' runs (+1), their sizes kept in `cache` where the
    /// modules `gone` let go of theirs.
    #[allow(clippy::too_many_arguments)]
    fn pass(&self, cx: &mut Cx<'_>, atoms: BlockId, docs: &[K0Doc], gone: &[i64], out: &Out<L1Ports>, cache: Option<BlockId>, work: &mut Work) -> Result<(), String> {
        let runs: Vec<BlockId> = outs::<L1Ports>().map(|k| out.block(k)).collect();
        work.rows_read += docs.len() as u64;
        work.rows_written += k0_pass(cx.reg, &self.ops, cx.dir, atoms, docs, gone, 1, &runs, cache).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl DeltaOp for L1 {
    type Ports = L1Ports;
    type Cache = Sizes;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, L1Ports>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<Sizes>, out: &mut Out<L1Ports>, asks: &mut Asks<L1Ports>) -> Result<Work, String> {
        let mut work = Work::default();
        // The documents wanted on each side (`None`: all). L1's grain is the document: a wanted relation of a document
        // is all of that document's relations (over-demand at its grain, which D1 allows).
        let docs_of = |v: &View<'_>| want_rows(v).into_iter().map(|w| w.1).collect::<BTreeSet<i64>>();
        let (wb, wa) = match want {
            None => (None, None),
            Some(w) => (Some(w.before().map_or_else(BTreeSet::new, docs_of)), Some(docs_of(w.after()))),
        };
        if ins.is_identity() && wb == wa {
            return Ok(work);
        }
        let (was, now) = (|d: i64| wb.as_ref().is_none_or(|s| s.contains(&d)), |d: i64| wa.as_ref().is_none_or(|s| s.contains(&d)));
        let (newly, dropped): (Vec<i64>, Vec<i64>) = match (&wb, &wa) {
            (Some(b), Some(a)) => (a.difference(b).copied().collect(), b.difference(a).copied().collect()), // copy: derived (the want's change, by document)
            _ => (Vec::new(), Vec::new()),
        };
        // Demand out (P§7.1): the documents newly wanted asked of the tape port, those no longer wanted let go.
        if !newly.is_empty() || !dropped.is_empty() {
            let w = asks.writer(cx.dir, L1Ports::TAPE)?;
            for (&d, diff) in newly.iter().map(|d| (d, 1)).chain(dropped.iter().map(|d| (d, -1))) {
                w.push(&[d], diff);
            }
        }
        let (tape, atoms) = (ins.port(L1Ports::TAPE), ins.global(L1Ports::ATOMS).block);
        // Retracted: the documents gone from the tape port that were wanted, and those no longer wanted.
        let mut parts: Vec<i64> = tape.before().map_or_else(Vec::new, |tb| tape.gone().map(|id| cell(tb, id, 0)).filter(|&d| was(d)).collect()); // copy: derived (the gone documents' ids)
        parts.extend(&dropped);
        parts.sort_unstable();
        parts.dedup();
        // Read: the documents added to the tape port that are wanted, and those newly wanted that it holds unchanged (a
        // newly wanted relation is an initial run of it, P§5).
        let mut ids: Vec<RowId> = tape.added().filter(|&id| now(cell(tape.after(), id, 0))).collect(); // copy: derived (the documents to read)
        let added: BTreeSet<RowId> = ids.iter().copied().collect(); // copy: derived (the added rows' ids)
        for &d in &newly {
            ids.extend(Read::matching(tape.after(), &[ColumnIndex::new(0)], &[d]).filter(|id| !added.contains(id)));
        }
        let docs = ids.into_iter().map(|id| doc_at(cx.blobs, cx.dir, tape.after(), id)).collect::<Result<Vec<_>, _>>()?; // copy: derived (the documents' places, from their blobs)
        if docs.is_empty() && parts.is_empty() {
            // Only its demand moved (or a document no one wants): nothing to read or retract.
            return Ok(work);
        }
        // The room the run needs, from the cache's sizes: each gone document's kept rows, and each added one's as they
        // were, an eighth more for an edit that grew.
        let need = cache.body(cx, |sizes| {
            let mut need = vec![0usize; RELATIONS.len()];
            for d in parts.iter().copied().chain(docs.iter().map(|d| d.module)) {
                for (k, &n) in sizes.of(d as u32).unwrap_or(&[]).iter().enumerate() {
                    need[k] += n as usize;
                }
            }
            need
        })?;
        for (k, n) in need.into_iter().flatten().enumerate() {
            out.room(cx.dir, L1Ports::relation(k), n + n / 8)?;
        }
        let cache = cache.write(cx)?;
        work.rows_read += parts.len() as u64;
        for k in outs::<L1Ports>() {
            if let Some(kept) = out.kept(k) {
                work.rows_written += drop_parts(cx.reg, &self.drops, cx.dir, kept, &parts, -1, out.block(k))?;
            }
        }
        self.pass(cx, atoms, &docs, &parts, out, Some(cache), &mut work)?;
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, L1Ports>, out: &mut Out<L1Ports>) -> Result<(), String> {
        let (tape, atoms) = (ins.port(L1Ports::TAPE), ins.global(L1Ports::ATOMS).block);
        let mut work = Work::default();
        let docs = live(tape).map(|id| doc_at(cx.blobs, cx.dir, tape, id)).collect::<Result<Vec<_>, _>>()?; // copy: derived (the documents' places, from their blobs)
        self.pass(cx, atoms, &docs, &[], out, None, &mut work)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "l1_tests.rs"]
mod tests;
