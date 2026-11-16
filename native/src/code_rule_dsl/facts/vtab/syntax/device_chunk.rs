//! A batch's chunk block (decision 0289 §1, 0266's `Chunk`): the arrays its views are made of, nested per document
//! in one block the device reads in place (a no-copy shared buffer), so a view the GPU reads is made there
//! (`metal_join`'s `view_expand`), not row by row on the host (`Viewed::row_at`).
//!
//! Each document's entry (`ChunkDoc`) holds its key, its rows' place in each view, and spans of:
//!
//! - the projection's arrays, by local id: `core` (the tape node), `low` (its id's low 32 bits by definition,
//!   empty for ids in preorder), `ord` (its index among its siblings), and the fielded nodes;
//! - the facts' arrays, by tape node: `parent`, `start`, `end`, `kind` (the tape's atom) and `field`;
//! - `local`, each tape node's local id (`NONE` for one left out), which a parent is read through;
//! - `core.rr`'s places (`Shape::Core`'s `Loc` views: `line`, `end_line`, `col`, `docof`), added on a `Loc` view's
//!   first device read (`fill_locs`): the document's first row in them, its spans past its nodes (tokens, the whole
//!   text, lines and end points, as `Projected::span` gives them) and its line starts (`core_rr::Lines`).
//!
//! The kind a view holds is the dialect's canonical code's atom (`Dialect::kind`): each tag's canonical table is in
//! the block once, and an entry points at its own.
//!
//! The projection's arrays are made in the batch's block (0289 §3): the entry points at them, and nothing is copied.
//! The facts' arrays and the canonical tables are copied in once per batch, on the device's first read: `Facts` is
//! shared by every batch and memo hit that reads a document, so its arrays move into the block when a memo entry is a
//! document range copied into the chunk (0266 path, steps 4 and 5).

use super::{Batch, Place, View};
use crate::code_rule_dsl::facts::core::NONE;
use rerail_blocks::{Block, Cells, Span, structure};
use std::sync::Arc;
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
        /// The `Loc` views' first row of the document, and its rows there (nodes, then spans): `fill_locs`.
        pub loc_row: u32,
        pub locs: u32,
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
        /// The spans past the nodes, `[start, end]` each, and the line starts, for the `Loc` views (`fill_locs`).
        pub spans: Span<u32>,
        pub lines: Span<u32>,
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
/// `core.rr`'s places of nodes and spans (`View::Loc`): their first line, last line, start column and document.
pub const VIEW_LOC_LINE: u32 = 5;
pub const VIEW_LOC_END: u32 = 6;
pub const VIEW_LOC_COL: u32 = 7;
pub const VIEW_LOC_DOCOF: u32 = 8;

/// The device's code of `view`, for the views it makes.
pub fn device_view(view: View) -> Option<u32> {
    Some(match view {
        View::Node => VIEW_NODE,
        View::Child => VIEW_CHILD,
        View::Field => VIEW_FIELD,
        View::Core => VIEW_CORE,
        View::Loc(Place::Line) => VIEW_LOC_LINE,
        View::Loc(Place::EndLine) => VIEW_LOC_END,
        View::Loc(Place::Col) => VIEW_LOC_COL,
        View::Loc(Place::Docof) => VIEW_LOC_DOCOF,
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
    /// The batch's block (`Batch::block`), which its projections are in.
    pub block: Arc<Block>,
    pub root: usize,
    /// Whether the `Loc` views' arrays are in the block (`fill_locs`), once asked.
    locs: std::sync::OnceLock<bool>,
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

/// Points `span` at `cells`, which are in `b` (a projection's, made in its batch's block); copied in when they are
/// not (a projection made apart from the batch).
fn point<T: Copy + rerail_blocks::Pod>(b: &Arc<Block>, span: *const Span<T>, cells: &Cells<T>) {
    if Arc::ptr_eq(cells.block(), b) {
        // SAFETY: `cells` are in `b`, as `span` is, and live as long as the batch, which holds both.
        unsafe { Span::set(span, cells.as_ptr(), cells.len()) }
    } else {
        put(b, span, cells);
    }
}

/// The batch's chunk, in the batch's block: every document's arrays the views read.
pub fn build(batch: &Batch) -> Chunk {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "a batch's chunk block".into());
    let b = Arc::clone(&batch.block);
    let root: *mut ChunkRoot = b.alloc_t(1);
    let docs: *mut ChunkDoc = b.alloc_t(batch.docs.len().max(1));
    // Each dialect's canonical codes, once.
    let mut canon: HashMap<u8, (*const u16, usize)> = HashMap::new();
    for (tag, d) in batch.dialects.iter() {
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
            point(&b, &raw const (*d).core, &p.core);
            // Every local's, past the nodes too: a token's id is by definition as well (`Projected::global`).
            match p.defs.as_ref() {
                Some(x) => point(&b, &raw const (*d).low, &x.low),
                None => put(&b, &raw const (*d).low, &[]),
            }
            point(&b, &raw const (*d).ord, p.ordinals());
            point(&b, &raw const (*d).fielded, fielded);
            point(&b, &raw const (*d).local, &p.local);
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
    Chunk { block: b, root: at, locs: std::sync::OnceLock::new() }
}

/// Puts the `Loc` views' arrays into `batch`'s chunk, once: each document's first row in them, its spans past its
/// nodes and its line starts. False when a document's places are not its own lines' (a guest placed in its host
/// file, decision 0055) or the block cannot hold them: the views are then made on the host.
pub fn fill_locs(batch: &Batch, chunk: &Chunk) -> bool {
    *chunk.locs.get_or_init(|| {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "core.rr's places in the chunk block".into());
        if batch.docs.iter().any(|p| p.extra.as_ref().is_none_or(|x| x.host.is_some())) {
            return false;
        }
        let rows = batch.offsets(View::Loc(Place::Line));
        let b = &chunk.block;
        // SAFETY: the root `build` wrote; its document table.
        let docs = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).docs.as_ptr() as *mut ChunkDoc };
        for (i, p) in batch.docs.iter().enumerate() {
            let x = p.extra.as_ref().expect("checked above");
            let (len, locs) = (p.len(), p.locs());
            let starts = x.lines.starts();
            let (Some(sp), Some(ls)) = (b.try_alloc_t::<u32>(2 * (locs - len) as usize + 1), b.try_alloc_t::<u32>(starts.len().max(1))) else {
                return false;
            };
            // SAFETY: `sp` holds `2 * (locs - len)` values and `ls` the starts, just allocated in the block; entry `i`
            // of the table, whose `Loc` fields no reader reads before this returns.
            unsafe {
                for (j, local) in (len..locs).enumerate() {
                    let [s, e] = p.span(local);
                    *sp.add(2 * j) = s;
                    *sp.add(2 * j + 1) = e;
                }
                std::ptr::copy_nonoverlapping(starts.as_ptr(), ls, starts.len());
                let d = docs.add(i);
                (*d).loc_row = rows[i] as u32;
                (*d).locs = locs;
                Span::set(&raw const (*d).spans, sp, 2 * (locs - len) as usize);
                Span::set(&raw const (*d).lines, ls, starts.len());
            }
        }
        true
    })
}

/// The MSL of the chunk's structures, as the kernels read them.
pub fn msl() -> String {
    use super::granules::{INDEX_BITS, SLOT_BITS, SPAN_SLOTS};
    // A span's id past the nodes and tokens (`granules::span_low`): the top slots, down from the last.
    format!("{}constant uint GR_SLOTS = {}u;\nconstant uint GR_INDEX_BITS = {INDEX_BITS}u;\nconstant uint GR_SPAN_SLOTS = {SPAN_SLOTS}u;\n",
            rerail_blocks::msl_of::<ChunkRoot>(), 1u32 << SLOT_BITS)
}

/// NONE, as the kernels test a parent with it.
pub const NO_NODE: u32 = NONE;
