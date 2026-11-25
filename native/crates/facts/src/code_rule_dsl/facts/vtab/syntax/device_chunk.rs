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
//!   text, lines and end points, as `Projected::span` gives them), placed on the facts' line starts (`flines`).
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
        /// The spans past the nodes, `[start, end]` each, for the `Loc` views (`fill_locs`).
        pub spans: Span<u32>,
        /// A guest's places (decision 0055), per `Loc` local: its first line, last line and start column in its
        /// file, `Locs::place`'s, through its placement and its host's lines. Empty for a file's own tree, whose
        /// places the kernel finds in `flines`.
        pub placed: Span<u32>,
        /// The facts' line index's bytes (`LineTable::index`), for the `line` view and the `Loc` views (`fill_lines`),
        /// read by the lines' reads (`<ukl/reads/lines.h>`: `Facts::start_line`, `LineTable::place`).
        pub flines: Span<u8>,
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
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
    /// The views' directory (`views_in_place`), once asked; `None` when a document's tape is not where the device
    /// reads it.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    views: std::sync::OnceLock<Option<ViewsInPlace>>,
}

/// A batch's documents as the views' reads find them on the device (decision 0353 step 4d, `ViewDir`): each one's
/// entry, beside its projection's numbering in the batch's block, and its tape where it lies in its slab. A stage
/// reads a view through them; nothing makes the view's rows.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
    /// Whether every document's `Loc` rows are the views' reads' (`ViewEntry::locs`: a file's own tree, or a guest
    /// placed in its host, its places' lines in a slab) and the device reads them, so the `Loc` views are read in place
    /// too.
    pub locs: bool,
    /// The cache a 16z document's reads decode its blocks into (decision 0353 step 4b, `view::cache`), in the batch's
    /// block, which every reader of the batch's views shares, on the host and on the device: none where no tape is
    /// 16z.
    pub cache: Option<crate::code_rule_dsl::facts::vtab::ZCache>,
    /// The directory as the host's join programs read it (`rerail_accel::gpu::shared::on_host`), whether or not a
    /// device reads it too.
    pub host: HostDir,
    /// The tape format of every document read off its tape (15, 16, `F16Z_FORMAT`), or 0 for a batch of several: the
    /// stages that read its views are compiled for it (decision 0576).
    pub format: u32,
    _entries: Vec<super::view_docs::ViewEntry>,
}


/// A views' directory as the host reads it: its host address, and whether it holds every document's lines and `Loc`
/// rows there (the device's refusals of homes it has no address for are not the host's).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[derive(Clone, Copy, Debug)]
pub struct HostDir {
    pub dir: u64,
    pub lines: bool,
    pub locs: bool,
}

/// The batch's cache's counts in the profile when its directory goes (the statistics mode's too, where it is on).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Drop for ViewsInPlace {
    fn drop(&mut self) {
        if let Some(cache) = &self.cache {
            // No reader reads the views once the directory goes.
            for (name, n) in cache.counts().named() {
                crate::code_rule_dsl::prof::count(name, n as usize);
            }
        }
    }
}

/// The batch's views' directory (`ViewsInPlace`), made on its first ask, with or without a device: `None` when a
/// document is in another block or has no tape read in place. Where the device reads it is `ViewsInPlace::dir`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn views_in_place<'a>(batch: &Batch, chunk: &'a Chunk) -> Option<&'a ViewsInPlace> {
    chunk.views.get_or_init(|| build_views(batch, chunk)).as_ref()
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn build_views(batch: &Batch, chunk: &Chunk) -> Option<ViewsInPlace> {
    build_dir(&chunk.block, &docs_of(batch, [0; NVIEWS]))
}

/// One document of a views' directory: its projection, its dialect (none: a text read as text only, no nodes), and
/// its first row in each view (`VIEWS`).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
struct DirDoc<'a> {
    p: &'a super::Projected,
    dialect: Option<&'a super::Dialect>,
    rows: [u32; NVIEWS],
}

/// `batch`'s documents in a directory, each view's rows past `past` of that view.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn docs_of(batch: &Batch, past: [u32; NVIEWS]) -> Vec<DirDoc<'_>> {
    let (nodes, children, fields, locs) = (batch.offsets(View::Node), batch.offsets(View::Child), batch.offsets(View::Field), batch.offsets(View::Loc(Place::Line)));
    batch
        .docs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (n, loc) = (nodes[i] as u32, locs.get(i).map_or(0, |&r| r as u32));
            let own = [n, children[i] as u32, fields[i] as u32, n, n, loc, loc, loc, loc, n, n, n];
            DirDoc { p, dialect: batch.dialects.get(p.tag), rows: std::array::from_fn(|v| past[v] + own[v]) }
        })
        .collect()
}

/// Each view's rows of `batch` (`VIEWS`' order).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn rows_of(batch: &Batch) -> [u32; NVIEWS] {
    let last = |v: View| batch.offsets(v).last().map_or(0, |&r| r as u32);
    let (n, loc) = (last(View::Node), last(View::Loc(Place::Line)));
    [n, last(View::Child), last(View::Field), n, n, loc, loc, loc, loc, n, n, n]
}

/// The views' directory of a chunk's batch and the tape values admitted inside its fixpoint (decision 0431 §8,
/// `values::Layered`): the batch's documents, then each value's batch's, each view's rows of a value past those
/// before it, as a layered source numbers them. Every document is in the batch's block: `None` where one is not, or
/// a tape is not where the device reads it.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn views_over(batch: &Batch, values: &[Arc<Batch>]) -> Option<ViewsInPlace> {
    let mut docs = docs_of(batch, [0; NVIEWS]);
    let mut past = rows_of(batch);
    for v in values {
        docs.extend(docs_of(v, past));
        let more = rows_of(v);
        past = std::array::from_fn(|k| past[k] + more[k]);
    }
    build_dir(&batch.block, &docs)
}

/// The directory of `docs`, made in `b` (`ViewsInPlace`): each one's entry, tape and lines where the device reads them.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn build_dir(b: &Arc<Block>, docs: &[DirDoc<'_>]) -> Option<ViewsInPlace> {
    use crate::code_rule_dsl::tape::placed;
    use rerail_clif::view_reads::{ViewDir, ViewHome, ViewRow};
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the views' directory".into());
    // Made whether or not a device is open (decision 0353 step 3 on the CPU): the host reads it too.
    let device = placed::device();
    // No process device: the join programs run on the host (`rerail_accel::gpu::shared::on_host`), which reads every
    // home where it lies, its device address its host address. Nothing is placed or wrapped for it (decision 0353).
    let host_reads = device.is_none();
    let n = docs.len();
    let rows: *mut ViewRow = b.alloc_t(n.max(1));
    let dir: *mut ViewDir = b.alloc_t(1);
    // The homes the documents' tapes and lines lie in (`ViewHome`), each once, in the order first named.
    let mut homes: Vec<placed::HomeOf> = Vec::new();
    let mut home = |h: placed::HomeOf| -> (u32, u64, bool) {
        let at = h.at;
        let on_device = h.device.is_some() || host_reads;
        let k = homes.iter().position(|o| o.id == h.id).unwrap_or_else(|| {
            homes.push(h);
            homes.len() - 1
        });
        (k as u32, at, on_device)
    };
    let mut entries = Vec::with_capacity(n);
    // What the device reads: every tape of a document with nodes (else no stage reads the directory), every
    // document's lines (else the batch's `line` view is made, counted), its `Loc` rows.
    let (mut on_device, mut lines) = (device.is_some() || host_reads, true);
    // What the device alone refuses (a home it has no address for): the host reads it all the same, by the homes'
    // host addresses (`ViewsInPlace::host`).
    let (mut dev_lines, mut dev_locs) = (true, true);
    // The 16z documents' rows, their blocks and their largest block's entry: their cache made after the rows.
    let (mut zrows, mut zblocks, mut zentry) = (Vec::new(), 0usize, 0usize);
    let mut locs = docs.iter().all(|d| d.p.extra.is_some());
    let mut format: Option<u32> = None;
    for (i, &DirDoc { p, dialect, rows: first_rows }) in docs.iter().enumerate() {
        // Each refusal counted by why: what step 4b (16z laid out apart) and the loads not yet placed leave.
        let refused = |why: &'static str| {
            crate::code_rule_dsl::prof::count(why, 1);
            None
        };
        if !Arc::ptr_eq(p.block(), b) {
            return refused("views.refused.block");
        }
        if dialect.is_none() && p.len() > 0 {
            return refused("views.refused.no_dialect");
        }
        let mut row = ViewRow { rows: first_rows, ..Default::default() };
        if locs && p.locs() > 0 && super::view_docs::ViewEntry::locs(p).is_none() {
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
        // is not read in place, those alone are made.
        let only_locs = p.len() == 0;
        if p.len() > 0 || (locs && p.locs() > 0) {
            let tape_in_place = || -> Result<_, &'static str> {
                let tape = p.facts.tape.as_ref().ok_or("views.refused.no_tape")?;
                let (bytes, view) = tape.in_place().map_err(|_| "views.refused.not_in_place")?;
                Ok((placed::home_of(tape.image().room(), bytes), view, tape.image().format()))
            };
            let (at, view, tape_format) = match tape_in_place() {
                Ok(t) => t,
                Err(why) if !only_locs => return refused(why),
                Err(why) => {
                    crate::code_rule_dsl::prof::count(why, 1);
                    locs = false;
                    // SAFETY: row `i` of the rows just allocated in `b`.
                    unsafe { rows.add(i).write(row) };
                    continue;
                }
            };
            let (tape_home, tape_at, tape_on_device) = home(at);
            if !tape_on_device && device.is_some() {
                // The device's refusal, by why: the host reads it all the same.
                let why = if tape_format == crate::code_rule_dsl::tape::formats::Format::F16Z { "views.refused.unplaced_16z" } else { "views.refused.unplaced" };
                crate::code_rule_dsl::prof::count(why, 1);
                if only_locs { dev_locs = false } else { on_device = false }
            }
            // Its lines, read where they lie (its places' lines: a guest's are its host's, `Locs::place_lines`).
            let table = p.extra.as_deref().map_or(&p.facts.lines, |x| x.place_lines());
            // The lines' index (decision 0612), in the document's tape or in a slab's room beside it.
            let bytes = table.index();
            let (lines_home, lines_at, lines_on_device) = home(placed::home_of(table.room(), bytes));
            // The `Loc` views read the lines' words too (decision 0612): lines the device cannot reach refuse both.
            if !lines_on_device && (lines || locs) && device.is_some() {
                crate::code_rule_dsl::prof::count("views.refused.lines_unplaced", 1);
                (dev_lines, dev_locs) = (false, false);
            }
            let entry = super::view_docs::ViewEntry::new(b, p, dialect);
            row.entry = b.offset_of(entry.desc()) as u64;
            (row.tape, row.lines, row.homes) = (tape_at, lines_at, [tape_home, lines_home]);
            row.view = view;
            format = match format {
                None => Some(view.format),
                Some(f) if f == view.format => Some(f),
                Some(_) => Some(0),
            };
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
    // The 16z documents' cache, in a block of its own (one more home: its reads write it, the batch's block they do
    // not), shared by every reader of the views on either executor: its entry size in each one's view, its reads
    // checked (shared).
    let cache = (!zrows.is_empty()).then(|| {
        let spec = rerail_clif::view::cache::Spec::configured(zblocks, zentry);
        crate::code_rule_dsl::prof::count("zcache.batches", 1);
        crate::code_rule_dsl::prof::count("zcache.blocks", zblocks);
        crate::code_rule_dsl::prof::count("zcache.entries", spec.entries as usize);
        crate::code_rule_dsl::prof::count("zcache.entry_kb", spec.entry_bytes() >> 10);
        let block = Arc::new(Block::anon(rerail_blocks::block::kind::SCRATCH, spec.bytes() + 2 * 4096));
        let at = block.alloc(spec.bytes(), rerail_clif::view::cache::ALIGN);
        // SAFETY: the cache's bytes, just allocated in its block, read by no reader yet.
        spec.init(unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) }).expect("a cache's room of its size");
        for &i in &zrows {
            // SAFETY: row `i`, written above in `b`.
            let row = unsafe { &mut *rows.add(i) };
            (row.view.columns[3], row.view.columns[4]) = (spec.entry_bytes() as u32, 1);
        }
        let buffer = device.and_then(|d| block.device_buffer(d).ok());
        crate::code_rule_dsl::facts::vtab::ZCache { block, at, len: spec.bytes(), buffer }
    });
    // Its home: the cache's block, the device's where the device has it (else the device reads no 16z view).
    let cache_home = cache.as_ref().map(|c| {
        let host = c.block.ptr::<u8>(c.at) as usize;
        let device = c.buffer.as_ref().and_then(|buf| Some((buf.device_address()? + c.at as u64, buf.clone())));
        on_device &= device.is_some() || host_reads;
        home(placed::HomeOf { id: host, host: host as u64, at: 0, device }).0
    });
    let homes_at: *mut ViewHome = b.alloc_t(homes.len().max(1));
    for (k, h) in homes.iter().enumerate() {
        // SAFETY: home `k` of the homes just allocated in `b`.
        unsafe { homes_at.add(k).write(ViewHome { host: h.host, gpu: h.device.as_ref().map_or(if host_reads { h.host } else { 0 }, |d| d.0) }) };
    }
    // The documents' numbers in key order (`ViewDir::keys`): a chunk's tape values follow its batch's documents in
    // the order admitted, not by key.
    let keys_at: *mut u32 = b.alloc_t(n.max(1));
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_by_key(|&i| docs[i as usize].p.key);
    // SAFETY: `n` numbers, just allocated in `b`.
    unsafe { std::ptr::copy_nonoverlapping(order.as_ptr(), keys_at, n) };
    let at = b.offset_of(dir);
    // SAFETY: the directory just allocated in `b`.
    unsafe { dir.write(ViewDir { at: at as u64, ndocs: n as u32, nhomes: homes.len() as u32, rows: b.offset_of(rows) as u64, homes: b.offset_of(homes_at) as u64, cache: cache_home.map_or(0, |k| u64::from(k) + 1), keys: b.offset_of(keys_at) as u64 }) };
    // The device's: the block's buffer made after the last allocation, so it covers them, and each 16z view's cache
    // offset, the cache's device address less its tape's (the kernels read the view where it lies).
    let mut buffers = Vec::new();
    let mut dir_on_device = None;
    if let Some(device) = device.filter(|_| on_device) {
        let buffer = b.device_buffer(device).ok()?;
        let base = buffer.device_address()?;
        let device_of = |k: u32| homes[k as usize].device.as_ref().map_or(0, |d| d.0);
        if let Some(k) = cache_home {
            for &i in &zrows {
                // SAFETY: row `i`, written above in `b`.
                let row = unsafe { &mut *rows.add(i) };
                let rel = device_of(k).wrapping_sub(device_of(row.homes[0]) + row.tape) as i64;
                (row.view.side[0], row.view.side[1]) = (rel as u32, (rel >> 32) as u32);
            }
        }
        buffers.push(buffer);
        // The cache's home is bound apart (`ViewInPlace::cache`), written.
        for h in homes.iter().enumerate().filter(|&(k, _)| Some(k as u32) != cache_home).map(|(_, h)| h) {
            if let Some((_, slab)) = &h.device
                && !buffers.iter().any(|s| s.device_address() == slab.device_address())
            {
                buffers.push(slab.clone());
            }
        }
        dir_on_device = Some(base + at as u64);
    } else if host_reads && on_device {
        // The host's: the directory at its address in the block, each 16z view's cache by its host address less its
        // tape's.
        if let Some(k) = cache_home {
            for &i in &zrows {
                // SAFETY: row `i`, written above in `b`.
                let row = unsafe { &mut *rows.add(i) };
                let rel = homes[k as usize].host.wrapping_sub(homes[row.homes[0] as usize].host + row.tape) as i64;
                (row.view.side[0], row.view.side[1]) = (rel as u32, (rel >> 32) as u32);
            }
        }
        dir_on_device = Some(b.base_ptr() as u64 + at as u64);
    }
    // The host's: the same directory at its host address, its homes read by theirs (`ViewHome::host`), and each 16z
    // view's cache found from the directory's (`ViewDir::cache`), not the device's offset in the row.
    let host = HostDir { dir: b.base_ptr() as u64 + at as u64, lines, locs };
    let lines = lines && dev_lines && dir_on_device.is_some();
    let locs = locs && dev_locs && dir_on_device.is_some();
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, host, cache, format: format.unwrap_or(0), _entries: entries })
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
            (*d).file = p.file;
            (*d).tag = u32::from(p.tag);
            (*d).nodes = p.len();
            (*d).node_row = batch.nodes()[i] as u32;
            (*d).child_row = batch.offsets(View::Child)[i] as u32;
            (*d).field_row = batch.offsets(View::Field)[i] as u32;
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
        (*root).nodes = *batch.nodes().last().unwrap_or(&0) as u32;
        (*root).children = *batch.offsets(View::Child).last().unwrap_or(&0) as u32;
        (*root).fields = *batch.offsets(View::Field).last().unwrap_or(&0) as u32;
        Span::set(&raw const (*root).docs, docs, batch.docs.len());
    }
    let at = b.offset_of(root);
    b.set_root(root);
    Chunk {
        block: b,
        root: at,
        locs: std::sync::OnceLock::new(),
        lines: std::sync::OnceLock::new(),
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        views: std::sync::OnceLock::new(),
    }
}

/// Puts each document's facts' line index into `batch`'s chunk, once, for the `line` view made on the device (decision
/// 0325 step 3: it was read on the host, a row at a time, for an upload). False when the block cannot hold them: the
/// view is then made on the host.
// copy (0353 step 4): the facts' line index copied into the chunk, for a batch whose `line` view is made (its tapes
// not on the device). It goes with the made views: the views' reads read the lines where `LineTable::placed` put them.
pub fn fill_lines(batch: &Batch, chunk: &Chunk) -> bool {
    const LINES: rerail_prof::copies::Site = rerail_prof::copies::Site::new(14, "4", "the facts' line index put into the chunk (fill_lines)");
    *chunk.lines.get_or_init(|| LINES.time(|| in_block(&chunk.block, || {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the facts' lines in the chunk block".into());
        let b = &chunk.block;
        // SAFETY: the root `build` wrote; its document table.
        let docs = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).docs.as_ptr() as *mut ChunkDoc };
        for (i, p) in batch.docs.iter().enumerate() {
            let index = p.facts.lines.index();
            // A word of room past it: the reads load a whole word where a value ends.
            let Some(ls) = b.try_alloc_t::<u8>(index.len() + 8) else { return false };
            // SAFETY: `ls` holds the index and its room, just allocated in the block; entry `i` of the table, whose
            // `flines` no reader reads before this returns.
            unsafe {
                std::ptr::copy_nonoverlapping(index.as_ptr(), ls, index.len());
                std::ptr::write_bytes(ls.add(index.len()), 0, 8);
                Span::set(&raw const (*docs.add(i)).flines, ls, index.len());
            }
        }
        true
    })))
}

/// Puts the `Loc` views' arrays into `batch`'s chunk, once: each document's first row in them and its spans past its
/// nodes, over the facts' line starts (`fill_lines`); a guest's places too (decision 0055: its host's lines, through its placement), as the
/// host finds them. False when a document has no places (no `core.rerail` extra) or the block cannot hold them: the views
/// are then made on the host. A batch with a guest was refused, and its views read on the host for an upload (28 MB
/// on posthog/hogql).
// copy (0353 step 4): the spans past the nodes copied into the chunk for the `Loc` views. It goes: the views' reads
// read the spans where they are written once (Q216).
pub fn fill_locs(batch: &Batch, chunk: &Chunk) -> bool {
    const LOCS: rerail_prof::copies::Site = rerail_prof::copies::Site::new(14, "4", "the Loc views' spans put into the chunk (fill_locs)");
    *chunk.locs.get_or_init(|| LOCS.time(|| in_block(&chunk.block, || {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "core.rerail's places in the chunk block".into());
        if batch.docs.iter().any(|p| p.extra.is_none()) || !fill_lines(batch, chunk) {
            return false;
        }
        let rows = batch.offsets(View::Loc(Place::Line));
        let b = &chunk.block;
        // SAFETY: the root `build` wrote; its document table.
        let docs = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).docs.as_ptr() as *mut ChunkDoc };
        for (i, p) in batch.docs.iter().enumerate() {
            let x = p.extra.as_ref().expect("checked above");
            let (len, locs) = (p.len(), p.locs());
            let Some(sp) = b.try_alloc_t::<u32>(2 * (locs - len) as usize + 1) else {
                return false;
            };
            // SAFETY: `sp` holds `2 * (locs - len)` values, just allocated in the block; entry `i` of the table, whose
            // `Loc` fields no reader reads before this returns.
            unsafe {
                for (j, local) in (len..locs).enumerate() {
                    let [s, e] = p.span(local);
                    *sp.add(2 * j) = s;
                    *sp.add(2 * j + 1) = e;
                }
                let d = docs.add(i);
                (*d).loc_row = rows[i] as u32;
                (*d).locs = locs;
                Span::set(&raw const (*d).spans, sp, 2 * (locs - len) as usize);
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

/// What makes an axis's solve in a chunk: `members` per dialect tag (256) and kind code (1,024) a bit, `None` for
/// every kind (containment); `inside` for every ancestor in the set, else the nearest. Several chunks' rows go in one
/// relation, each chunk's past those before (a chunk's batch and its tape values, decision 0431 §8); the starts are
/// the first's.
pub type ChunkAxis = fn(&[&Chunk], Option<&[u32]>, bool) -> Result<crate::code_rule_dsl::facts::nearest::InChunk, String>;

/// The device's solve of an axis in a chunk: the Metal device's, which it registers at load.
pub static CHUNK_AXIS: rerail_registry::Slot<ChunkAxis> =
    rerail_registry::Slot::new("the device's kind-set axis in a chunk (facts::vtab::syntax::device_chunk::CHUNK_AXIS)");
