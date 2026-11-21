//! A batch's chunk block (decision 0289 §1, 0266's `Chunk`): the arrays its views are made of, nested per document
//! in one block the device reads in place (a no-copy shared buffer), so a view the GPU reads is made there
//! (`metal_join`'s `view_expand`), not row by row on the host (`Viewed::row_at`).
//!
//! Each document's entry (`ChunkDoc`) holds its key, its rows' place in each view, and spans of:
//!
//! - the projection's arrays, by local id: `core` (the tape node), `low` (its id's low 32 bits by definition,
//!   empty for ids in preorder), `ord` (its index among its siblings), and the fielded nodes;
//! - `up`, the projection's parent of each local id (`Projected::up`, `NONE` above the root and an anchor);
//! - each node's `start`, `end`, `kind` (the tape's atom) and `field`, by local id, read off the tape through its API
//!   (0293) when the chunk is made: an interim, until the device's kernels read the tape in the chunk (0293 step 5);
//! - `core.rerail`'s places (`Shape::Core`'s `Loc` views: `line`, `end_line`, `col`, `docof`), added on a `Loc` view's
//!   first device read (`fill_locs`): the document's first row in them, its spans past its nodes (tokens, the whole
//!   text, lines and end points, as `Projected::span` gives them) and its line starts (`core_rr::Lines`).
//!
//! The kind a view holds is the dialect's canonical code's atom (`Dialect::kind`): each tag's canonical table is in
//! the block once, and an entry points at its own.
//!
//! The projection's arrays are made in the batch's block (0289 §3): the entry points at them, and nothing is copied.
//! `Facts` holds no column since 0293 step 4: a node's kind, span and field are the tape's answers, and the tape image
//! is already in the chunk (0289 §4). Until the kernels read it there, the chunk holds those four answers for the
//! projected nodes, made once per batch on the device's first read. The canonical tables are copied in once a batch.

use super::{Batch, Item, Place, View};
use crate::code_rule_dsl::facts::core::NONE;
use rerail_blocks::{Block, Cells, Span, structure};
use std::sync::Arc;
use std::collections::HashMap;

structure! {
    /// One document of a batch's chunk.
    pub struct ChunkDoc {
        pub key: u32,
        /// The key of the file it is part of (decision 0431 §4, `Projected::file`): `docof`'s.
        pub file: u32,
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
        /// To the spans' 8-byte alignment.
        pub pad: u32,
        pub core: Span<u32>,
        pub low: Span<u32>,
        pub ord: Span<u32>,
        pub fielded: Span<u32>,
        pub up: Span<u32>,
        /// By local id, off the tape (an interim until 0293 step 5).
        pub start: Span<u32>,
        pub end: Span<u32>,
        pub kind: Span<u32>,
        pub field: Span<u16>,
        pub canonical: Span<u16>,
        /// The spans past the nodes, `[start, end]` each, and the line starts, for the `Loc` views (`fill_locs`).
        pub spans: Span<u32>,
        pub lines: Span<u32>,
        /// A guest's places (decision 0055), per `Loc` local: its first line, last line and start column in its
        /// file, `Locs::place`'s, through its placement and its host's lines. Empty for a file's own tree, whose
        /// places the kernel finds in `lines`.
        pub placed: Span<u32>,
        /// The facts' line starts (`LineTable::starts`), for the `line` view (`fill_lines`): a node's line is the
        /// number of them at or before its start (`Facts::start_line`).
        pub flines: Span<u32>,
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
// 4 was a kind-set axis read whole, its pairs copied into the chunk and made ids here. Its solve writes a relation of
// ids of its own now (decision 0353 step 5), read where it lies; the number stays unused, so the others keep theirs.
/// `core.rerail`'s places of nodes and spans (`View::Loc`): their first line, last line, start column and document.
pub const VIEW_LOC_LINE: u32 = 5;
pub const VIEW_LOC_END: u32 = 6;
pub const VIEW_LOC_COL: u32 = 7;
pub const VIEW_LOC_DOCOF: u32 = 8;
/// The `line` view (`View::Line`): each node's id and the line its start is on, by the facts' own lines.
pub const VIEW_LINE: u32 = 9;
/// The `tape`, `ast_depth` and `node_order` views (`View::Tape`, `View::Depth`, `View::Order`), read in place only
/// (no device code makes their rows).
pub const VIEW_TAPE: u32 = 10;
pub const VIEW_DEPTH: u32 = 11;
pub const VIEW_ORDER: u32 = 12;

/// The views' reads' views (`rerail_clif::view_reads::VIEWS`): a directory's document has its first row in each.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const NVIEWS: usize = rerail_clif::view_reads::VIEWS.len();

/// A view's number in the views' reads (`vread`'s `kind`: 1 + its place in `rerail_clif::view_reads::VIEWS`), of its
/// device code.
pub fn read_kind(view: u32) -> u32 {
    match view {
        VIEW_NODE => 1,
        VIEW_CHILD => 2,
        VIEW_FIELD => 3,
        VIEW_CORE => 4,
        VIEW_LINE => 5,
        VIEW_LOC_LINE => 6,
        VIEW_LOC_END => 7,
        VIEW_LOC_COL => 8,
        VIEW_LOC_DOCOF => 9,
        VIEW_TAPE => 10,
        VIEW_DEPTH => 11,
        VIEW_ORDER => 12,
        _ => panic!("view {view} has no read"),
    }
}

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
        View::Line => VIEW_LINE,
        View::Tape => VIEW_TAPE,
        View::Depth => VIEW_DEPTH,
        View::Order => VIEW_ORDER,
        _ => return None,
    })
}

/// A batch's chunk block, and its root's offset in it.
pub struct Chunk {
    /// The batch's block (`Batch::block`), which its projections are in.
    pub block: Arc<Block>,
    pub root: usize,
    /// Whether the `Loc` views' arrays are in the block (`fill_locs`), once asked.
    locs: std::sync::OnceLock<bool>,
    /// Whether the facts' line starts are in the block (`fill_lines`), once asked.
    lines: std::sync::OnceLock<bool>,
}

/// A batch's documents as the views' reads find them on the device (decision 0353 step 4d, `ViewDir`): each one's
/// entry, beside its projection's numbering in the batch's block, and its tape where it lies in its slab. A stage
/// reads a view through them; nothing makes the view's rows.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub struct ViewsInPlace {
    /// The directory's offset in the batch's block (`ViewDir::at`): every executor's reads take it there, the host's at
    /// the block's host address, the device's at its device address.
    pub at: usize,
    /// The directory's GPU address, which a stage's constants point at (`Stage::v_dir`): `None` where there is no
    /// device, or a document's tape is in a home the device does not read (a vector of the host's).
    pub dir: Option<u64>,
    /// The buffers the device's reads reach by address, which a pass that reads a view makes resident: the batch's
    /// block's (the directory's, its entries' and its cache's), and each slab a tape or its lines lie in.
    pub buffers: Vec<rerail_device::device::Buffer>,
    /// Whether the device reads every document's lines in place (each in a slab: `LineTable::placed`), so the `line`
    /// view is read in place too.
    pub lines: bool,
    /// Whether every document's `Loc` rows are the views' reads' (`ViewEntry::locs`: a file's own tree, core.rerail's
    /// lines in the batch's block) and the device reads them, so the `Loc` views are read in place too.
    pub locs: bool,
    /// The cache a 16z document's reads decode its blocks into (decision 0353 step 4b, `view::cache`), in the batch's
    /// block, which every reader of the batch's views shares, on the host and on the device: none where no tape is
    /// 16z.
    /// Each batch's, a layered source's (`views_over`) one for each of its batches that has one.
    pub caches: Vec<crate::code_rule_dsl::facts::vtab::ZCache>,
}


/// The batch's views' directory as the device reads it (`ViewsInPlace`), made on its first ask, with or without a
/// device: `None` when a document is in another block or has no tape read in place. Where the device reads it is
/// `ViewsInPlace::dir`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn views_in_place(batch: &Batch) -> Option<&ViewsInPlace> {
    batch.views.get_or_init(|| build_views(batch)).as_ref()
}

/// The batch's own directory (`made::directory`, its first rows `Batch::firsts`), each document's row of its reads
/// layer written (`Docs::reads`, the host's reads' rows), with what the device needs to read them: whether it reads
/// every tape, lines and `Loc` row where they lie, and the buffers those lie in.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_views(batch: &Batch) -> Option<ViewsInPlace> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the views' directory".into());
    let device = crate::code_rule_dsl::tape::placed::device();
    let docs = &batch.docs;
    batch.firsts(View::Node);
    // The batch's 16z blocks and its largest entry, which its cache is made for (where no row made it first).
    let (mut zblocks, mut zentry) = (0usize, 0usize);
    for p in docs.iter() {
        if let Some(Ok((_, view))) = p.facts.tape.as_ref().map(|t| t.in_place())
            && view.format == rerail_clif::view::F16Z_FORMAT
        {
            zblocks += (view.nodes as usize).div_ceil(rerail_clif::view::F16Z_BLOCK as usize);
            zentry = zentry.max(view.columns[3] as usize);
        }
    }
    // What the device reads: every tape of a document with nodes (else no stage reads the directory), every
    // document's lines (else the batch's `line` view is made, counted), its `Loc` rows.
    let (mut on_device, mut lines, mut locs) = (device.is_some(), true, true);
    let mut slabs: Vec<rerail_device::device::Buffer> = Vec::new();
    for i in 0..docs.len() {
        let r = docs.reads_sized(i, Some((zblocks, zentry))).ok()?;
        locs &= r.locs;
        if r.entry.is_none() {
            continue;
        }
        if !r.tape_device && device.is_some() {
            // The device's refusal, by why: the host reads it all the same.
            let z = docs[i].facts.tape.as_ref().is_some_and(|t| t.image().format() == crate::code_rule_dsl::tape::formats::Format::F16Z);
            crate::code_rule_dsl::prof::count(if z { "views.refused.unplaced_16z" } else { "views.refused.unplaced" }, 1);
            if r.nodes { on_device = false } else { locs = false }
        }
        if !r.lines_device && lines && device.is_some() {
            crate::code_rule_dsl::prof::count("views.refused.lines_unplaced", 1);
            lines = false;
        }
        for slab in &r.slabs {
            if !slabs.iter().any(|s| s.device_address() == slab.device_address()) {
                slabs.push(slab.clone());
            }
        }
    }
    let cache = docs.zcache_made().cloned();
    if let Some(c) = &cache {
        on_device &= c.buffer.as_ref().and_then(|b| b.device_address()).is_some();
    }
    let at = docs.dir_at();
    let mut buffers = Vec::new();
    let mut dir_on_device = None;
    if let Some(device) = device.filter(|_| on_device) {
        // The block's buffer made after its last allocation (the rows' entries), so it covers them; the cache's is
        // bound apart (`ViewInPlace::cache`).
        let buffer = batch.block.device_buffer(device).ok()?;
        dir_on_device = Some(buffer.device_address()? + at as u64);
        buffers.push(buffer);
        buffers.extend(slabs);
    }
    let lines = lines && dir_on_device.is_some();
    let locs = locs && dir_on_device.is_some();
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, caches: cache.into_iter().collect() })
}

/// The views' directory of a chunk's batch and the tape values admitted inside its fixpoint (decision 0431 §8,
/// `values::Layered`): the batch's documents, then each value's batch's, each view's rows of a value past those
/// before it, as a layered source numbers them. Each batch's rows are written through its own reads layer
/// (`views_in_place`: `Docs::reads`, the host's reads' rows), once; this directory names them, each row's first rows
/// past the batches before it and its homes among theirs, and each batch's 16z cache its own (decision 0353 step 4).
/// Every batch is in the chunk's block: `None` where one is not, or a batch's directory is refused.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn views_over(batch: &Batch, values: &[Arc<Batch>]) -> Option<ViewsInPlace> {
    use rerail_clif::view_reads::{ROWS, ViewDir, ViewHome, ViewRow};
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the views' directory".into());
    let b = &batch.block;
    let sources: Vec<&Batch> = std::iter::once(batch).chain(values.iter().map(|v| &**v)).collect();
    let mut own = Vec::with_capacity(sources.len());
    for s in &sources {
        if !Arc::ptr_eq(&s.block, b) {
            crate::code_rule_dsl::prof::count("views.refused.block", 1);
            return None;
        }
        own.push(views_in_place(s)?);
    }
    // SAFETY: each source's directory, in the chunk's block, its rows' reads layers written (`views_in_place`), its
    // first rows counted.
    let (n, nhomes) = sources.iter().fold((0usize, 0usize), |(n, h), s| unsafe {
        let d = s.docs.dir();
        (n + (*d).ndocs as usize, h + (*d).nhomes as usize)
    });
    let (rows, homes, dir): (*mut ViewRow, *mut ViewHome, *mut ViewDir) = (b.alloc_t(n.max(1)), b.alloc_t(nhomes.max(1)), b.alloc_t(1));
    let (mut k, mut h, mut past) = (0usize, 0u32, [0u32; ROWS]);
    for s in &sources {
        let d = s.docs.dir().cast_mut();
        // SAFETY: as above; the rows and homes just allocated in the chunk's block, `n` and `nhomes` of them.
        unsafe {
            let (srows, shomes, sn, sh) = (ViewDir::block_rows(d), ViewDir::homes_mut(d), (*d).ndocs as usize, (*d).nhomes);
            for i in 0..sn {
                // copy: derived (the layered directory's row: the batch's row, its first rows and homes renumbered)
                let mut row: ViewRow = srows.add(i).read();
                row.rows = std::array::from_fn(|v| past[v] + row.rows[v]);
                row.homes = row.homes.map(|x| x + h);
                rows.add(k).write(row);
                k += 1;
            }
            for j in 0..sh as usize {
                homes.add(h as usize + j).write(shomes.add(j).read());
            }
            let ends = (*d).ends;
            past = std::array::from_fn(|v| past[v] + ends[v]);
            h += sh;
        }
    }
    let at = b.offset_of(dir);
    // The chunk's batch's cache is the directory's; each value's rows name their own in their views (`TapeView::side`).
    // SAFETY: the chunk's batch's directory, as above.
    let cache = unsafe { (*batch.docs.dir()).cache };
    // SAFETY: the directory, just allocated in the chunk's block, made whole.
    unsafe { dir.write(ViewDir { at: at as u64, ndocs: n as u32, nhomes: nhomes as u32, rows: b.offset_of(rows) as u64, homes: b.offset_of(homes) as u64, cache, ends: past, counted: 1, pad: 0 }) };
    // The device's where it reads every batch's: the block's buffer made after the last allocation, so it covers
    // them, and each batch's slabs (a batch's first buffer is its block's, covered by this one).
    let device = crate::code_rule_dsl::tape::placed::device();
    let mut buffers: Vec<rerail_device::device::Buffer> = Vec::new();
    let mut dir_on_device = None;
    if let Some(device) = device.filter(|_| own.iter().all(|o| o.dir.is_some())) {
        let buffer = b.device_buffer(device).ok()?;
        dir_on_device = Some(buffer.device_address()? + at as u64);
        buffers.push(buffer);
        for slab in own.iter().flat_map(|o| o.buffers.iter().skip(1)) {
            if !buffers.iter().any(|s| s.device_address() == slab.device_address()) {
                buffers.push(slab.clone());
            }
        }
    }
    let lines = dir_on_device.is_some() && own.iter().all(|o| o.lines);
    let locs = dir_on_device.is_some() && own.iter().all(|o| o.locs);
    let caches = own.iter().flat_map(|o| o.caches.iter().cloned()).collect();
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, caches })
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

/// `n` values made by `at` (by local id) put in `b`, and `span` set to them.
fn made<T: Copy>(b: &Block, span: *const Span<T>, n: usize, at: impl Fn(usize) -> T) {
    let p: *mut T = b.alloc_t(n.max(1));
    // SAFETY: `p` holds `n` items, just allocated in `b`, as `span` is.
    unsafe {
        for l in 0..n {
            p.add(l).write(at(l));
        }
        Span::set(span, p, n);
    }
}

/// `n` values written by `fill` in `b`, and `span` set to them.
fn made_with<T: Copy + Default>(b: &Block, span: *const Span<T>, n: usize, fill: impl FnOnce(&mut [T])) {
    let p: *mut T = b.alloc_t(n.max(1));
    // SAFETY: `p` holds `n` items, just allocated in `b` and set to their default before `fill` reads or writes them.
    unsafe {
        for k in 0..n {
            p.add(k).write(T::default());
        }
        fill(std::slice::from_raw_parts_mut(p, n));
        Span::set(span, p, n);
    }
}

/// `n` values from `values`, in order, put in `b`, and `span` set to them.
fn made_in_order<T: Copy>(b: &Block, span: *const Span<T>, n: usize, values: impl Iterator<Item = T>) {
    let p: *mut T = b.alloc_t(n.max(1));
    let mut k = 0;
    // SAFETY: `p` holds `n` items, just allocated in `b`, as `span` is.
    unsafe {
        for v in values.take(n) {
            p.add(k).write(v);
            k += 1;
        }
        Span::set(span, p, k);
    }
}

/// The batch's chunk, in the batch's block: every document's arrays the views read.
// copy (0353 step 4): the node columns (`core`, `up`, `ord`, `fielded`, `start`, `end`, `kind`, `field`), the canonical codes
// and the projection's arrays copied or made here. They go: the kernels read the placed tapes (decision 0353).
pub fn build(batch: &Batch) -> Chunk {
    const COLUMNS: rerail_prof::copies::Site = rerail_prof::copies::Site::new(3, "4", "the chunk's node columns and codes (device_chunk::build)");
    COLUMNS.time(|| in_block(&batch.block, || build_in(batch)))
}

/// What `f` puts into `block`, with the bytes the block grew by (a copy's size, `rerail_prof::copies`).
fn in_block<T>(block: &Block, f: impl FnOnce() -> T) -> (T, usize) {
    let before = block.used();
    let out = f();
    (out, block.used().saturating_sub(before))
}

fn build_in(batch: &Batch) -> Chunk {
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
            (*d).key = p.key();
            (*d).file = p.file();
            (*d).tag = u32::from(p.tag());
            (*d).nodes = p.len();
            (*d).node_row = batch.nodes().at(i) as u32;
            (*d).child_row = batch.firsts(View::Child).at(i) as u32;
            (*d).field_row = batch.firsts(View::Field).at(i) as u32;
            (*d).nfielded = p.fielded();
            (*d).preorder = u32::from(p.defs.is_none());
            // The tape node and the parent of each local id, made from the rank index and the tape (the host reads
            // them there, decision 0353 step 2): node columns, as `start` and `end` are, until the kernels read the
            // placed tapes.
            let n = p.len() as usize;
            made_in_order(&b, &raw const (*d).core, n, p.nodes().map(|(_, x)| x));
            made_in_order(&b, &raw const (*d).up, n, p.walk().map(|(_, _, q)| q));
            let core: &[u32] = (*d).core.as_slice();
            let ids = |l: usize| core[l];
            // Every local's, past the nodes too: a token's id is by definition as well (`Projected::global`).
            // The ids' low bits, made by one scan of the granule table (nodes and tokens each come in local order).
            match p.table() {
                Some(table) => {
                    let tokens = p.ranks().count(Item::Token) as usize;
                    made_with(&b, &raw const (*d).low, n + tokens, |out| {
                        let (mut a, mut k) = (0, n);
                        table.scan(p.ranks(), |_, low, item| {
                            let at = if item == Item::Node { &mut a } else { &mut k };
                            out[*at] = low;
                            *at += 1;
                        });
                    });
                }
                None => put(&b, &raw const (*d).low, &[]),
            }
            made_in_order(&b, &raw const (*d).ord, n, p.places().map(|(_, idx, _)| idx));
            made_in_order(&b, &raw const (*d).fielded, p.fielded() as usize, p.nodes().filter(|&(_, x)| p.ranks().is(Item::Field, x)).map(|(l, _)| l));
            made(&b, &raw const (*d).start, n, |l| f.start(ids(l)));
            made(&b, &raw const (*d).end, n, |l| f.end(ids(l)));
            made(&b, &raw const (*d).kind, n, |l| f.kind(ids(l)));
            made(&b, &raw const (*d).field, n, |l| f.field(ids(l)));
            match canon.get(&p.tag()) {
                Some(&(c, n)) => Span::set(&raw const (*d).canonical, c, n),
                None => Span::set(&raw const (*d).canonical, std::ptr::null::<u16>(), 0),
            }
        }
    }
    // SAFETY: the root, just allocated; its spans point into `b`.
    unsafe {
        (*root).ndocs = batch.docs.len() as u32;
        (*root).nodes = batch.nodes().total() as u32;
        (*root).children = batch.firsts(View::Child).total() as u32;
        (*root).fields = batch.firsts(View::Field).total() as u32;
        Span::set(&raw const (*root).docs, docs, batch.docs.len());
    }
    let at = b.offset_of(root);
    b.set_root(root);
    Chunk {
        block: b,
        root: at,
        locs: std::sync::OnceLock::new(),
        lines: std::sync::OnceLock::new(),
    }
}

/// Puts each document's facts' line starts into `batch`'s chunk, once, for the `line` view made on the device (decision
/// 0325 step 3: it was read on the host, a row at a time, for an upload). False when the block cannot hold them: the
/// view is then made on the host.
// copy (0353 step 4): the facts' line starts copied into the chunk, for a batch whose `line` view is made (its tapes
// not on the device). It goes with the made views: the views' reads read the lines where `LineTable::placed` put them.
pub fn fill_lines(batch: &Batch, chunk: &Chunk) -> bool {
    const LINES: rerail_prof::copies::Site = rerail_prof::copies::Site::new(14, "4", "the facts' line starts put into the chunk (fill_lines)");
    *chunk.lines.get_or_init(|| LINES.time(|| in_block(&chunk.block, || {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the facts' lines in the chunk block".into());
        let b = &chunk.block;
        // SAFETY: the root `build` wrote; its document table.
        let docs = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).docs.as_ptr() as *mut ChunkDoc };
        for (i, p) in batch.docs.iter().enumerate() {
            let starts = p.facts.lines.starts();
            let Some(ls) = b.try_alloc_t::<u32>(starts.len().max(1)) else { return false };
            // SAFETY: `ls` holds the starts, just allocated in the block; entry `i` of the table, whose `flines` no
            // reader reads before this returns.
            unsafe {
                for (j, &s) in starts.iter().enumerate() {
                    *ls.add(j) = s;
                }
                Span::set(&raw const (*docs.add(i)).flines, ls, starts.len());
            }
        }
        true
    })))
}

/// Puts the `Loc` views' arrays into `batch`'s chunk, once: each document's first row in them, its spans past its
/// nodes and its line starts; a guest's places too (decision 0055: its host's lines, through its placement), as the
/// host finds them. False when a document has no places (no `core.rerail` extra) or the block cannot hold them: the views
/// are then made on the host. A batch with a guest was refused, and its views read on the host for an upload (28 MB
/// on posthog/hogql).
// copy (0353 step 4): the line starts and the spans past the nodes copied into the chunk for the `Loc` views. It goes:
// the views' reads read the lines in place, and the spans where they are written once (Q216).
pub fn fill_locs(batch: &Batch, chunk: &Chunk) -> bool {
    const LOCS: rerail_prof::copies::Site = rerail_prof::copies::Site::new(14, "4", "the Loc views' lines and spans put into the chunk (fill_locs)");
    *chunk.locs.get_or_init(|| LOCS.time(|| in_block(&chunk.block, || {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "core.rerail's places in the chunk block".into());
        if batch.docs.iter().any(|p| p.extra.is_none()) {
            return false;
        }
        let rows = batch.firsts(View::Loc(Place::Line));
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
                (*d).loc_row = rows.at(i) as u32;
                (*d).locs = locs;
                Span::set(&raw const (*d).spans, sp, 2 * (locs - len) as usize);
                Span::set(&raw const (*d).lines, ls, starts.len());
            }
            if x.host.is_some() {
                let Some(pl) = b.try_alloc_t::<u32>(3 * locs as usize + 1) else { return false };
                // SAFETY: `pl` holds `3 * locs` values, just allocated in the block; entry `i` of the table, as above.
                unsafe {
                    for (local, span) in p.spans().take(locs as usize) {
                        let (line, last, col) = x.place(span);
                        let at = pl.add(3 * local as usize);
                        (*at, *at.add(1), *at.add(2)) = (line, last, col);
                    }
                    Span::set(&raw const (*docs.add(i)).placed, pl, 3 * locs as usize);
                }
            }
        }
        true
    })))
}

/// The UKL of the chunk's structures, as the kernels read them (decision 0440 step C).
pub fn ukl() -> String {
    use super::granules::{INDEX_BITS, SLOT_BITS, SPAN_SLOTS};
    // A span's id past the nodes and tokens (`granules::span_low`): the top slots, down from the last.
    format!("{}#define GR_SLOTS {}u\n#define GR_INDEX_BITS {INDEX_BITS}u\n#define GR_SPAN_SLOTS {SPAN_SLOTS}u\n",
            rerail_blocks::ukl_of::<ChunkRoot>(), 1u32 << SLOT_BITS)
}

/// NONE, as the kernels test a parent with it.
pub const NO_NODE: u32 = NONE;
