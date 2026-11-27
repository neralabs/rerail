//! A `Chunk` block: a chunk's documents (their tapes) and every relation made from them, plus the memo entries
//! copied in for documents that hit. It is a `Facts` block of kind `CHUNK` whose root also holds `docs` and
//! `hits`.
//!
//! A memo entry is one document's rows of the relations the memo keeps, as relations of their own. It is built
//! in a scratch block and moved whole: into the journal (one `memcpy`), and on a hit from the journal into the
//! next run's chunk (one `memcpy`). No entry is ever decoded.

use crate::relation::RelationW;
use crate::block::{Block, kind};
use crate::facts::{Facts, Schema, root_of};
use rerail_delta_core::flat::{Rel, RelSpan};
use crate::relation::{self, Relation, Spec, View};
use crate::structure;
use std::sync::{Arc, Mutex};

structure! {
    /// One document: its key and its tape's columns.
    pub struct Doc {
        pub key: u32,
        pub nodes: u32,
        pub kind: RelSpan<u16>,
        pub parent: RelSpan<u32>,
        pub name: RelSpan<u32>,
    }
}

structure! {
    pub struct MemoEntry {
        pub key: u32,
        pub nrel: u32,
        pub digest: u64,
        pub rels: RelSpan<Rel<Relation>>,
    }
}

impl MemoEntry {
    pub fn rel(&self, i: usize) -> View<'_> {
        View::full(self.rels.as_slice()[i].get().expect("entry relation"))
    }
}

/// A document as a chunk is handed it.
pub struct DocIn<'a> {
    pub key: u32,
    pub kind: &'a [u16],
    pub parent: &'a [u32],
    pub name: &'a [u32],
}

/// Opens a chunk for its documents and up to `nhits` memo hits.
pub fn open(schema: Arc<Schema>, reserve: usize, docs: &[DocIn], nhits: usize) -> Facts {
    let f = Facts::open(kind::CHUNK, 0, schema, reserve, None);
    let b = &f.block;
    let dp: *mut Doc = b.alloc_t(docs.len().max(1));
    let hp: *mut Rel<MemoEntry> = b.alloc_t(nhits.max(1));
    for (i, d) in docs.iter().enumerate() {
        let n = d.kind.len();
        let k: *mut u16 = b.alloc_t(n);
        let p: *mut u32 = b.alloc_t(n);
        let m: *mut u32 = b.alloc_t(n);
        unsafe {
            std::ptr::copy_nonoverlapping(d.kind.as_ptr(), k, n);
            std::ptr::copy_nonoverlapping(d.parent.as_ptr(), p, n);
            std::ptr::copy_nonoverlapping(d.name.as_ptr(), m, n);
            let doc = dp.add(i);
            (*doc).key = d.key;
            (*doc).nodes = n as u32;
            b.span(&raw const (*doc).kind, k, n);
            b.span(&raw const (*doc).parent, p, n);
            b.span(&raw const (*doc).name, m, n);
        }
    }
    let root = root_of(b) as *const _ as *mut crate::facts::FactsRoot;
    unsafe {
        b.span(&raw const (*root).docs, dp, docs.len());
        b.span(&raw const (*root).hits, hp, nhits);
    }
    f
}

pub fn docs(b: &Block) -> &[Doc] {
    root_of(b).docs.as_slice()
}

pub fn hits(b: &Block) -> impl Iterator<Item = &MemoEntry> {
    let r = root_of(b);
    r.hits.as_slice()[..r.nhits.load(std::sync::atomic::Ordering::Acquire) as usize].iter().map(|p| p.get().expect("hit"))
}

/// Builds one document's memo entry in a scratch block: the rows of each kept relation, as given, frozen with
/// the relation's spec. Returns the scratch block and the entry's range and offset in it.
pub fn build_entry(key: u32, digest: u64, rels: &[(&Spec, Vec<i64>)]) -> (Block, usize, usize, usize) {
    let s = crate::alloc::host(kind::SCRATCH, 64 << 20);
    let start = s.mark(64);
    let e: *mut MemoEntry = s.alloc_t(1);
    let tbl: *mut Rel<Relation> = s.alloc_t(rels.len());
    let lock = Mutex::new(());
    for (i, (spec, flat)) in rels.iter().enumerate() {
        let r = Relation::create(&s, spec, None);
        relation::RelW { block: &s, rel: r, lock: &lock }.extend(flat);
        relation::freeze(&s, r, spec, None);
        unsafe { s.rel(tbl.add(i), r) };
    }
    unsafe {
        (*e).key = key;
        (*e).nrel = rels.len() as u32;
        (*e).digest = digest;
        s.span(&raw const (*e).rels, tbl, rels.len());
    }
    let end = s.used();
    let off = s.offset_of(e);
    (s, start, end, off)
}

/// Copies an entry's range in (from the journal), and records it among the chunk's hits.
pub fn add_hit<'f>(f: &'f Facts, src: &Block, start: usize, end: usize, entry: usize) -> &'f MemoEntry {
    let dst = f.block.copy_in(src.bytes(start, end), start);
    let e: *const MemoEntry = f.block.ptr(dst + entry - start);
    let root = root_of(&f.block);
    let i = root.nhits.fetch_add(1, std::sync::atomic::Ordering::AcqRel) as usize;
    unsafe {
        f.block.rel(&root.hits.as_slice()[i], e);
        &*e
    }
}
