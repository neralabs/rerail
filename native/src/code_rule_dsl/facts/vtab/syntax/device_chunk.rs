//! A batch's chunk block (decision 0289 §1, 0266's `Chunk`): the arrays its views are made of, nested per document
//! in one block the device reads in place (a no-copy shared buffer), so a view the GPU reads is made there
//! (`metal_join`'s `view_expand`), not row by row on the host (`Viewed::row_at`).
//!
//! Each document's entry (`ChunkDoc`) holds its key, its rows' place in each view, and spans of:
//!
//! - the projection's arrays, by local id: `core` (the tape node), `low` (its id's low 32 bits by definition,
//!   empty for ids in preorder), `ord` (its index among its siblings), and the fielded nodes;
//! - the facts' arrays, by tape node: `parent`, `start`, `end`, `kind` (the tape's atom) and `field`;
//! - `local`, each tape node's local id (`NONE` for one left out), which a parent is read through.
//!
//! The kind a view holds is the dialect's canonical code's atom (`Dialect::kind`): each tag's canonical table is in
//! the block once, and an entry points at its own.
//!
//! The arrays are copied in once per batch, on the device's first read. §3 of 0289 makes them the projection's and
//! the facts' own storage, and the copy goes.

use super::{Batch, View};
use crate::code_rule_dsl::facts::core::NONE;
use rerail_blocks::{Block, Span, block::kind, structure};
use std::collections::HashMap;

structure! {
    /// One document of a batch's chunk.
    pub struct ChunkDoc {
        pub key: u32,
        pub tag: u32,
        /// Nodes (local ids), and the first row of the document in `node` (also `core_node`'s), `child` and `field`.
        pub nodes: u32,
        pub node_row: u32,
        pub child_row: u32,
        pub field_row: u32,
        pub nfielded: u32,
        /// 1 when ids are local ids (`RERAIL_NODE_IDS=preorder`): `low` is then empty.
        pub preorder: u32,
        pub core: Span<u32>,
        pub low: Span<u32>,
        pub ord: Span<u32>,
        pub fielded: Span<u32>,
        pub local: Span<u32>,
        pub parent: Span<u32>,
        pub start: Span<u32>,
        pub end: Span<u32>,
        pub kind: Span<u32>,
        pub field: Span<u16>,
        pub canonical: Span<u16>,
    }
}

structure! {
    /// A batch's chunk: its documents, in the batch's order, and each view's rows by document (`ndocs + 1` each).
    pub struct ChunkRoot {
        pub ndocs: u32,
        pub nodes: u32,
        pub children: u32,
        pub fields: u32,
        pub docs: Span<ChunkDoc>,
    }
}

/// A view the device makes from a chunk (`metal_join`'s `VIEW_*`), with the source columns it has.
pub const VIEW_NODE: u32 = 0;
pub const VIEW_CHILD: u32 = 1;
pub const VIEW_FIELD: u32 = 2;
pub const VIEW_CORE: u32 = 3;
/// A kind-set axis or containment read whole (`Within`): its batch's solve, as pairs of node rows (inner << 32 |
/// outer) copied into the chunk, each made the two nodes' ids.
pub const VIEW_AXIS: u32 = 4;

/// The device's code of `view`, for the views it makes.
pub fn device_view(view: View) -> Option<u32> {
    Some(match view {
        View::Node => VIEW_NODE,
        View::Child => VIEW_CHILD,
        View::Field => VIEW_FIELD,
        View::Core => VIEW_CORE,
        _ => return None,
    })
}

/// `pairs` copied into the chunk: their offset in the block, or `None` when its reservation cannot hold them.
pub fn put_pairs(chunk: &Chunk, pairs: &[u64]) -> Option<usize> {
    let p: *mut u64 = chunk.block.try_alloc_t(pairs.len().max(1))?;
    // SAFETY: `p` holds `pairs.len()` items, just allocated in the block.
    unsafe { std::ptr::copy_nonoverlapping(pairs.as_ptr(), p, pairs.len()) };
    Some(chunk.block.offset_of(p))
}

/// A batch's chunk block, and its root's offset in it.
pub struct Chunk {
    pub block: Block,
    pub root: usize,
}

/// Copies `src` into the block and points `span` at it.
fn put<T: Copy>(b: &Block, span: *const Span<T>, src: &[T]) {
    let p: *mut T = b.alloc_t(src.len().max(1));
    // SAFETY: `p` holds `src.len()` items, just allocated in `b`, as `span` is.
    unsafe {
        std::ptr::copy_nonoverlapping(src.as_ptr(), p, src.len());
        Span::set(span, p, src.len());
    }
}

/// The batch's chunk: every document's arrays the views read.
pub fn build(batch: &Batch) -> Chunk {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "a batch's chunk block".into());
    // What the arrays take, with room for alignment, and for the axes' pairs read whole (`put_pairs`, up to 1 GB):
    // the block reserves address space, not memory.
    let bytes: usize = batch.docs.iter().map(|p| 4 * (6 * p.facts.len() + 4 * p.core.len()) + 2 * p.facts.len() + 4096).sum::<usize>()
        + batch.dialects.values().map(|d| 2 * d.canonical.len() + 64).sum::<usize>()
        + (1 << 30);
    let b = Block::anon(kind::CHUNK, bytes.next_power_of_two());
    let root: *mut ChunkRoot = b.alloc_t(1);
    let docs: *mut ChunkDoc = b.alloc_t(batch.docs.len().max(1));
    // Each dialect's canonical codes, once.
    let mut canon: HashMap<u8, (*const u16, usize)> = HashMap::new();
    for (&tag, d) in &batch.dialects {
        let p: *mut u16 = b.alloc_t(d.canonical.len().max(1));
        // SAFETY: just allocated for `canonical.len()` items.
        unsafe { std::ptr::copy_nonoverlapping(d.canonical.as_ptr(), p, d.canonical.len()) };
        canon.insert(tag, (p, d.canonical.len()));
    }
    for (i, p) in batch.docs.iter().enumerate() {
        let f = &*p.facts;
        // SAFETY: entry `i` of the table just allocated; every span set below points into `b`.
        unsafe {
            let d = docs.add(i);
            (*d).key = p.key;
            (*d).tag = u32::from(p.tag);
            (*d).nodes = p.len();
            (*d).node_row = batch.nodes[i] as u32;
            (*d).child_row = batch.children[i] as u32;
            (*d).field_row = batch.fields[i] as u32;
            let fielded = p.fielded();
            (*d).nfielded = fielded.len() as u32;
            (*d).preorder = u32::from(p.defs.is_none());
            put(&b, &raw const (*d).core, &p.core);
            put(&b, &raw const (*d).low, p.defs.as_ref().map_or(&[][..], |x| &x.low[..p.core.len().min(x.low.len())]));
            put(&b, &raw const (*d).ord, p.ordinals());
            put(&b, &raw const (*d).fielded, fielded);
            put(&b, &raw const (*d).local, &p.local);
            put(&b, &raw const (*d).parent, &f.parent[..]);
            put(&b, &raw const (*d).start, &f.start[..]);
            put(&b, &raw const (*d).end, &f.end[..]);
            put(&b, &raw const (*d).kind, &f.kind);
            put(&b, &raw const (*d).field, &f.field);
            match canon.get(&p.tag) {
                Some(&(c, n)) => Span::set(&raw const (*d).canonical, c, n),
                None => Span::set(&raw const (*d).canonical, std::ptr::null::<u16>(), 0),
            }
        }
    }
    // SAFETY: the root, just allocated; its spans point into `b`.
    unsafe {
        (*root).ndocs = batch.docs.len() as u32;
        (*root).nodes = *batch.nodes.last().unwrap_or(&0) as u32;
        (*root).children = *batch.children.last().unwrap_or(&0) as u32;
        (*root).fields = *batch.fields.last().unwrap_or(&0) as u32;
        Span::set(&raw const (*root).docs, docs, batch.docs.len());
    }
    let at = b.offset_of(root);
    b.set_root(root);
    Chunk { block: b, root: at }
}

/// The MSL of the chunk's structures, as the kernels read them.
pub fn msl() -> String {
    rerail_blocks::msl_of::<ChunkRoot>()
}

/// NONE, as the kernels test a parent with it.
pub const NO_NODE: u32 = NONE;
