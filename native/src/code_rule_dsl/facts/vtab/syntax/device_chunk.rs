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
//! - `core.rr`'s places (`Shape::Core`'s `Loc` views: `line`, `end_line`, `col`, `docof`), added on a `Loc` view's
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
/// `core.rr`'s places of nodes and spans (`View::Loc`): their first line, last line, start column and document.
pub const VIEW_LOC_LINE: u32 = 5;
pub const VIEW_LOC_END: u32 = 6;
pub const VIEW_LOC_COL: u32 = 7;
pub const VIEW_LOC_DOCOF: u32 = 8;
/// The `line` view (`View::Line`): each node's id and the line its start is on, by the facts' own lines.
pub const VIEW_LINE: u32 = 9;

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
    /// The views' directory (`views_in_place`), once asked; `None` when a document's tape is not where the device
    /// reads it.
    #[cfg(target_os = "macos")]
    views: std::sync::OnceLock<Option<ViewsInPlace>>,
}

/// A batch's documents as the views' reads find them on the device (decision 0353 step 4d, `ViewDir`): each one's
/// entry, beside its projection's numbering in the batch's block, and its tape where it lies in its slab. A stage
/// reads a view through them; nothing makes the view's rows.
#[cfg(target_os = "macos")]
pub struct ViewsInPlace {
    /// The directory's GPU address, which a stage's constants point at (`Stage::v_dir`).
    pub dir: u64,
    /// The buffers the reads reach by address, which a pass that reads a view makes resident: the batch's block's
    /// (the one the addresses are of), and each slab a tape lies in.
    pub buffers: Vec<metal::Buffer>,
    /// Whether every document's lines are in a slab (`LineTable::placed`), so the `line` view is read in place too.
    pub lines: bool,
    /// Whether every document's `Loc` rows are the views' reads' (`ViewEntry::locs`: a file's own tree, core.rr's
    /// lines in the batch's block), so the `Loc` views are read in place too.
    pub locs: bool,
    /// The cache a 16z document's reads decode its blocks into (decision 0353 step 4b, `view::cache`), which every
    /// stage that reads the batch's views shares: none where no tape is 16z.
    pub cache: Option<metal::Buffer>,
    _entries: Vec<super::view_docs::ViewEntry>,
}


/// The batch's cache's counts in the profile when its directory goes (the statistics mode's too, where it is on).
#[cfg(target_os = "macos")]
impl Drop for ViewsInPlace {
    fn drop(&mut self) {
        if let Some(cache) = &self.cache {
            // SAFETY: the cache's shared buffer, all of it; no stage reads the views once the directory goes.
            let c = rerail_clif::view::cache::Counts::checked(unsafe { std::slice::from_raw_parts(cache.contents().cast::<u8>(), cache.length() as usize) });
            for (name, n) in c.named() {
                crate::code_rule_dsl::prof::count(name, n as usize);
            }
        }
    }
}

/// The batch's views' directory (`ViewsInPlace`), made on its first ask: `None` when a document's tape is not in a
/// slab (a tape held in a vector of its own), or the batch has no device.
#[cfg(target_os = "macos")]
pub fn views_in_place<'a>(batch: &Batch, chunk: &'a Chunk) -> Option<&'a ViewsInPlace> {
    chunk.views.get_or_init(|| build_views(batch, chunk)).as_ref()
}

#[cfg(target_os = "macos")]
fn build_views(batch: &Batch, chunk: &Chunk) -> Option<ViewsInPlace> {
    build_dir(&chunk.block, &docs_of(batch, [0; 9]))
}

/// One document of a views' directory: its projection, its dialect, and its first row in each view (`VIEWS`).
#[cfg(target_os = "macos")]
struct DirDoc<'a> {
    p: &'a Arc<super::Projected>,
    dialect: &'a super::Dialect,
    rows: [u32; 9],
}

/// `batch`'s documents in a directory, each view's rows past `past` of that view.
#[cfg(target_os = "macos")]
fn docs_of(batch: &Batch, past: [u32; 9]) -> Vec<DirDoc<'_>> {
    let (nodes, children, fields, locs) = (batch.offsets(View::Node), batch.offsets(View::Child), batch.offsets(View::Field), batch.offsets(View::Loc(Place::Line)));
    batch
        .docs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (n, loc) = (nodes[i] as u32, locs.get(i).map_or(0, |&r| r as u32));
            let own = [n, children[i] as u32, fields[i] as u32, n, n, loc, loc, loc, loc];
            DirDoc { p, dialect: &batch.dialects[p.tag], rows: std::array::from_fn(|v| past[v] + own[v]) }
        })
        .collect()
}

/// Each view's rows of `batch` (`VIEWS`' order).
#[cfg(target_os = "macos")]
fn rows_of(batch: &Batch) -> [u32; 9] {
    let last = |v: View| batch.offsets(v).last().map_or(0, |&r| r as u32);
    let (n, loc) = (last(View::Node), last(View::Loc(Place::Line)));
    [n, last(View::Child), last(View::Field), n, n, loc, loc, loc, loc]
}

/// The views' directory of a chunk's batch and the tape values admitted inside its fixpoint (decision 0431 §8,
/// `values::Layered`): the batch's documents, then each value's batch's, each view's rows of a value past those
/// before it, as a layered source numbers them. Every document is in the batch's block: `None` where one is not, or
/// a tape is not where the device reads it.
#[cfg(target_os = "macos")]
pub fn views_over(batch: &Batch, values: &[Arc<Batch>]) -> Option<ViewsInPlace> {
    let mut docs = docs_of(batch, [0; 9]);
    let mut past = rows_of(batch);
    for v in values {
        docs.extend(docs_of(v, past));
        let more = rows_of(v);
        past = std::array::from_fn(|k| past[k] + more[k]);
    }
    build_dir(&batch.block, &docs)
}

/// The directory of `docs`, made in `b` (`ViewsInPlace`): each one's entry, tape and lines where the device reads them.
#[cfg(target_os = "macos")]
fn build_dir(b: &Arc<Block>, docs: &[DirDoc<'_>]) -> Option<ViewsInPlace> {
    use rerail_clif::view_reads::{ViewDir, ViewRow};
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the views' directory".into());
    let device = crate::code_rule_dsl::tape::placed::device()?;
    let n = docs.len();
    let rows: *mut ViewRow = b.alloc_t(n.max(1));
    let dir: *mut ViewDir = b.alloc_t(1);
    let mut slabs: Vec<metal::Buffer> = Vec::new();
    let mut entries = Vec::with_capacity(n);
    let mut lines = true;
    // The 16z documents' rows, their blocks and their largest block's entry: their cache made after the rows.
    let (mut zrows, mut zblocks, mut zentry) = (Vec::new(), 0usize, 0usize);
    let mut locs = docs.iter().all(|d| d.p.extra.is_some());
    for (i, &DirDoc { p, dialect, rows: first_rows }) in docs.iter().enumerate() {
        // Each refusal counted by why: what step 4b (16z laid out apart) and the loads not yet placed leave.
        let refused = |why: &'static str| {
            crate::code_rule_dsl::prof::count(why, 1);
            None
        };
        if !Arc::ptr_eq(p.block(), b) {
            return refused("views.refused.block");
        }
        let mut row = ViewRow { rows: first_rows, ..Default::default() };
        if locs && p.locs() > 0 && super::view_docs::ViewEntry::locs(b, p).is_none() {
            // Why, by the guest's placement where it has one.
            let why = match p.extra.as_ref().and_then(|x| x.host.as_ref()).map(|(_, place)| place) {
                Some(super::Placement::Copied { .. }) => "views.refused.locs.guest_copied",
                Some(super::Placement::Mapped { .. }) => "views.refused.locs.guest_mapped",
                Some(super::Placement::At(_)) => "views.refused.locs.guest_at",
                Some(super::Placement::Segments(_)) => "views.refused.locs.guest_segments",
                None => "views.refused.locs",
            };
            crate::code_rule_dsl::prof::count(why, 1);
            locs = false;
        }
        // A document with no nodes has rows only in the `Loc` views (a withheld one's text and lines): where its tape
        // is not on the device, those alone are made.
        let only_locs = p.len() == 0;
        if p.len() > 0 || (locs && p.locs() > 0) {
            let tape_on_device = || -> Result<_, &'static str> {
                let tape = p.facts.tape.as_ref().ok_or("views.refused.no_tape")?;
                let (bytes, view) = tape.in_place().map_err(|_| "views.refused.not_in_place")?;
                let room = tape.image().room().ok_or(if tape.image().format() == crate::code_rule_dsl::tape::formats::Format::F16Z { "views.refused.unplaced_16z" } else { "views.refused.unplaced" })?;
                let (address, slab) = crate::code_rule_dsl::tape::placed::on_device(room, bytes).ok_or("views.refused.not_in_room")?;
                Ok((address, slab, view))
            };
            let (address, slab, view) = match tape_on_device() {
                Ok(on) => on,
                Err(why) if !only_locs => return refused(why),
                Err(why) => {
                    crate::code_rule_dsl::prof::count(why, 1);
                    locs = false;
                    // SAFETY: row `i` of the rows just allocated in `b`.
                    unsafe { rows.add(i).write(row) };
                    continue;
                }
            };
            if !slabs.iter().any(|s| s.gpu_address() == slab.gpu_address()) {
                slabs.push(slab);
            }
            // Its lines where the device reads them: else the batch's `line` view is made (counted).
            let table = &p.facts.lines;
            let words = table.words();
            // SAFETY: the words as bytes, their own range.
            let bytes = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), 4 * words.len()) };
            match table.room().and_then(|room| crate::code_rule_dsl::tape::placed::on_device(room, bytes)) {
                Some((address, slab)) => {
                    row.lines = address;
                    if !slabs.iter().any(|s| s.gpu_address() == slab.gpu_address()) {
                        slabs.push(slab);
                    }
                }
                None if lines => {
                    crate::code_rule_dsl::prof::count("views.refused.lines_unplaced", 1);
                    lines = false;
                }
                None => {}
            }
            let entry = super::view_docs::ViewEntry::new(b, p, dialect);
            row.entry = b.offset_of(entry.desc()) as u64;
            row.tape = address;
            row.view = view;
            if view.format == rerail_clif::view::F16Z_FORMAT {
                zrows.push(i);
                zblocks += (view.nodes as usize).div_ceil(rerail_clif::view::F16Z_BLOCK as usize);
                zentry = zentry.max(view.columns[3] as usize);
            }
            entries.push(entry);
        }
        // SAFETY: row `i` of the rows just allocated in `b`.
        unsafe { rows.add(i).write(row) };
    }
    // The 16z documents' cache, shared by every thread of every stage that reads the views: each one's view names it
    // by its address less its tape's, its entry size the cache's, its reads checked (shared).
    let cache = (!zrows.is_empty()).then(|| {
        let spec = rerail_clif::view::cache::Spec::configured(zblocks, zentry);
        crate::code_rule_dsl::prof::count("zcache.batches", 1);
        crate::code_rule_dsl::prof::count("zcache.blocks", zblocks);
        crate::code_rule_dsl::prof::count("zcache.entries", spec.entries as usize);
        crate::code_rule_dsl::prof::count("zcache.entry_kb", spec.entry_bytes() >> 10);
        let cache = device.new_buffer(spec.bytes() as u64, metal::MTLResourceOptions::StorageModeShared);
        // SAFETY: the buffer's bytes, shared with the host, read by no stage yet.
        spec.init(unsafe { std::slice::from_raw_parts_mut(cache.contents().cast::<u8>(), spec.bytes()) }).expect("a cache's buffer of its size");
        for &i in &zrows {
            // SAFETY: row `i`, written above in `b`.
            let row = unsafe { &mut *rows.add(i) };
            let rel = cache.gpu_address().wrapping_sub(row.tape) as i64;
            (row.view.side[0], row.view.side[1]) = (rel as u32, (rel >> 32) as u32);
            (row.view.columns[3], row.view.columns[4]) = (spec.entry_bytes() as u32, 1);
        }
        cache
    });
    // The block's buffer made after the last allocation, so it covers them: the addresses are its.
    let buffer = b.gpu_buffer(device);
    let base = buffer.gpu_address();
    // SAFETY: the directory just allocated in `b`.
    unsafe { dir.write(ViewDir { chunk: base, ndocs: n as u32, pad: 0, rows: base + b.offset_of(rows) as u64 }) };
    let mut buffers = vec![buffer];
    buffers.extend(slabs);
    Some(ViewsInPlace { dir: base + b.offset_of(dir) as u64, buffers, lines, locs, cache, _entries: entries })
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
            (*d).key = p.key;
            (*d).tag = u32::from(p.tag);
            (*d).nodes = p.len();
            (*d).node_row = batch.nodes[i] as u32;
            (*d).child_row = batch.children[i] as u32;
            (*d).field_row = batch.fields[i] as u32;
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
    Chunk {
        block: b,
        root: at,
        locs: std::sync::OnceLock::new(),
        lines: std::sync::OnceLock::new(),
        #[cfg(target_os = "macos")]
        views: std::sync::OnceLock::new(),
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
                    *ls.add(j) = u32::try_from(s).expect("a line start within 4 GB");
                }
                Span::set(&raw const (*docs.add(i)).flines, ls, starts.len());
            }
        }
        true
    })))
}

/// Puts the `Loc` views' arrays into `batch`'s chunk, once: each document's first row in them, its spans past its
/// nodes and its line starts; a guest's places too (decision 0055: its host's lines, through its placement), as the
/// host finds them. False when a document has no places (no `core.rr` extra) or the block cannot hold them: the views
/// are then made on the host. A batch with a guest was refused, and its views read on the host for an upload (28 MB
/// on posthog/hogql).
// copy (0353 step 4): the line starts and the spans past the nodes copied into the chunk for the `Loc` views. It goes:
// the views' reads read the lines in place, and the spans where they are written once (Q216).
pub fn fill_locs(batch: &Batch, chunk: &Chunk) -> bool {
    const LOCS: rerail_prof::copies::Site = rerail_prof::copies::Site::new(14, "4", "the Loc views' lines and spans put into the chunk (fill_locs)");
    *chunk.locs.get_or_init(|| LOCS.time(|| in_block(&chunk.block, || {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "core.rr's places in the chunk block".into());
        if batch.docs.iter().any(|p| p.extra.is_none()) {
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

/// The MSL of the chunk's structures, as the kernels read them.
pub fn msl() -> String {
    use super::granules::{INDEX_BITS, SLOT_BITS, SPAN_SLOTS};
    // A span's id past the nodes and tokens (`granules::span_low`): the top slots, down from the last.
    format!("{}constant uint GR_SLOTS = {}u;\nconstant uint GR_INDEX_BITS = {INDEX_BITS}u;\nconstant uint GR_SPAN_SLOTS = {SPAN_SLOTS}u;\n",
            rerail_blocks::msl_of::<ChunkRoot>(), 1u32 << SLOT_BITS)
}

/// NONE, as the kernels test a parent with it.
pub const NO_NODE: u32 = NONE;
