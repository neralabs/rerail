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

use crate::contract::{At, CacheW, Cx, DeltaOp, In, Out, Port, Ports, Work};
use rerail_delta_twins::cache as cache_twin;
use crate::dir::{BlockId, Directory};
use crate::drop::{DropOps, drop_parts};
use crate::files::{BlobId, Blobs, DocIds};
use crate::k0::{K0Doc, K0Ops, k0_pass};
use crate::kform::{self, RELATIONS};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;

/// The tape port's place among L1's inputs; the atoms' among its globals.
pub const TAPE: usize = 0;
pub const ATOMS: usize = 0;

/// The tape port's rows: `(d, source, tape, symbols)`, each blob by its four words, by document.
pub fn tape_shape() -> Shape {
    let c = std::iter::once(ColType::I32).chain(std::iter::repeat_n(ColType::I64, 12)).collect();
    Shape::run(c, (0..13).collect(), Some(0))
}

/// Document `doc`'s row in the tape port, as module `d`, with its symbols blob.
pub fn tape_row(d: i64, doc: &DocIds, syms: &BlobId) -> Vec<i64> {
    std::iter::once(d).chain(doc.source.words()).chain(doc.tape.words()).chain(syms.words()).collect()
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
fn doc_at(blobs: &Blobs, dir: &Directory, tape: &View<'_>, id: RowId) -> Result<K0Doc, String> {
    let blob = |at: u16| {
        let w: Vec<i64> = (at..at + 4).map(|c| cell(tape, id, c)).collect();
        blobs.get(dir, &BlobId::of_words(&w))?.ok_or_else(|| "a blob the tape port names, which nothing holds".to_string())
    };
    Ok(K0Doc { module: cell(tape, id, 0), src: blob(1)?, tape: blob(5)?, syms: blob(9)?.block })
}

fn live<'v>(v: &'v View<'_>) -> impl Iterator<Item = RowId> + 'v {
    (0..Read::end(v).get() as u32).map(RowId::new).filter(|&id| Read::live(v, id))
}

impl L1 {
    /// `k0_pass` over documents `docs`, one launch, into the outputs' runs (+1), their sizes kept in `cache` where the
    /// modules `gone` let go of theirs.
    #[allow(clippy::too_many_arguments)]
    fn pass(&self, cx: &mut Cx<'_>, atoms: BlockId, docs: &[K0Doc], gone: &[i64], out: &Out, cache: Option<BlockId>, work: &mut Work) -> Result<(), String> {
        let runs: Vec<BlockId> = (0..RELATIONS.len()).map(|k| out.block(k)).collect();
        work.rows_read += docs.len() as u64;
        work.rows_written += k0_pass(cx.reg, &self.ops, cx.dir, atoms, docs, gone, 1, &runs, cache).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl DeltaOp for L1 {
    fn ports(&self) -> Ports {
        Ports {
            ins: vec![Port { name: "tape", shape: tape_shape() }],
            globals: vec!["atoms"],
            outs: RELATIONS.iter().enumerate().map(|(i, (name, _))| Port { name, shape: kform::shape(i) }).collect(),
        }
    }

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_>, _: Option<&Bistate<View<'_>>>, cache: &mut CacheW, out: &mut Out, _: &mut Out) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let (tape, atoms) = (&ins.ports[TAPE], ins.globals[ATOMS].block);
        let mut parts: Vec<i64> = tape.before().map_or_else(Vec::new, |tb| tape.gone().map(|id| cell(tb, id, 0)).collect()); // copy: derived (the gone documents' ids)
        parts.sort_unstable();
        parts.dedup();
        let docs = tape.added().map(|id| doc_at(cx.blobs, cx.dir, tape.after(), id)).collect::<Result<Vec<_>, _>>()?; // copy: derived (the documents' places, from their blobs)
        // The room the run needs, from the cache's sizes: each gone document's kept rows, and each added one's as they
        // were, an eighth more for an edit that grew.
        if let Some(c) = cache.read(cx)? {
            let need: Vec<usize> = {
                let l = cx.dir.read("L1's cache", &[c])?;
                let sizes = |d: i64| cache_twin::sizes(l.block(0), d as u32);
                let mut need = vec![0usize; RELATIONS.len()];
                for d in parts.iter().copied().chain(docs.iter().map(|d| d.module)) {
                    for (k, &n) in sizes(d).unwrap_or(&[]).iter().enumerate() {
                        need[k] += n as usize;
                    }
                }
                need
            };
            for (k, n) in need.into_iter().enumerate() {
                out.room(cx.dir, k, n + n / 8)?;
            }
        }
        let cache = cache.write(cx)?;
        work.rows_read += parts.len() as u64;
        for k in 0..RELATIONS.len() {
            if let Some(kept) = out.kept(k) {
                work.rows_written += drop_parts(cx.reg, &self.drops, cx.dir, kept, &parts, -1, out.block(k))?;
            }
        }
        self.pass(cx, atoms, &docs, &parts, out, Some(cache), &mut work)?;
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_>, out: &mut Out) -> Result<(), String> {
        let (tape, atoms) = (&ins.ports[TAPE], ins.globals[ATOMS].block);
        let mut work = Work::default();
        let docs = live(tape).map(|id| doc_at(cx.blobs, cx.dir, tape, id)).collect::<Result<Vec<_>, _>>()?; // copy: derived (the documents' places, from their blobs)
        self.pass(cx, atoms, &docs, &[], out, None, &mut work)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "l1_tests.rs"]
mod tests;
