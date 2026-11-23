//! A batch's views as the device reads them in place (decision 0353 step 4): its documents' directory
//! (`ViewsInPlace`, `ViewDir`), each document's entry beside its projection's numbering in the batch's block and its
//! tape where it lies in its slab, which every view's reads read. Nothing makes a view's rows: a view the device does
//! not read in place is not given to it (`Viewed::device_view`).

use super::{Batch, Place, View};
use std::sync::Arc;

/// A view the device reads (`metal_join`'s `VIEW_*`), with the source columns it has.
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
    /// Whether every document's `Loc` rows are the views' reads' (`ViewEntry::locs`: a file's own tree, core.rerail's
    /// lines in the batch's block) and the device reads them, so the `Loc` views are read in place too.
    pub locs: bool,
    /// The cache a 16z document's reads decode its blocks into (decision 0353 step 4b, `view::cache`), in the batch's
    /// block, which every reader of the batch's views shares, on the host and on the device: none where no tape is
    /// 16z.
    /// Each batch's, a layered source's (`views_over`) one for each of its batches that has one.
    pub caches: Vec<crate::code_rule_dsl::facts::vtab::ZCache>,
    /// The tape format of every document read off its tape (15, 16, `F16Z_FORMAT`), or 0 for a batch of several: the
    /// stages that read its views are compiled for it (decision 0576).
    pub format: u32,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl ViewsInPlace {
    /// Its `node` view as a stage reads it in place, `keep` keeping its directory's rows, and the view's rows in all
    /// (`ViewDir::ends`), read in `block` (the batch's, which the directory lies in): `None` where the device reads
    /// none of it (`dir`).
    pub fn node_view(&self, block: &rerail_blocks::Block, keep: Arc<dyn std::any::Any + Send + Sync>) -> Option<(crate::code_rule_dsl::facts::vtab::ViewInPlace, usize)> {
        let dir = self.dir?;
        // SAFETY: the directory lies in `block` at `at` (`build_views`, `views_over`), its counts written.
        let nodes = unsafe { (*block.ptr::<rerail_clif::view_reads::ViewDir>(self.at)).ends[0] } as usize;
        let mut v = crate::code_rule_dsl::facts::vtab::ViewInPlace { dir, kind: read_kind(VIEW_NODE), buffers: self.buffers.clone(), caches: self.caches.clone(), failed: 0, format: self.format, keep };
        v.failed = v.cache_failed();
        Some((v, nodes))
    }
}


/// The batch's views' directory as the device reads it (`ViewsInPlace`), made on its first ask, with or without a
/// device: `None` when a document is in another block or has no tape read in place. Where the device reads it is
/// `ViewsInPlace::dir`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn views_in_place(batch: &Batch) -> Option<&ViewsInPlace> {
    batch.views.get_or_init(|| build_views(batch)).as_ref()
}

/// The batch's own directory (`made::directory`, its first rows `Batch::firsts`), each document's row of its reads
/// layer written (`Docs::reads`, the host's reads' rows), with what the device needs to read them: whether it reads
/// every tape, lines and `Loc` row where they lie, and the buffers those lie in.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn build_views(batch: &Batch) -> Option<ViewsInPlace> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the views' directory".into());
    let device = crate::code_rule_dsl::tape::placed::device();
    let docs = &batch.docs;
    batch.firsts(View::Node);
    // The batch's 16z blocks and its largest entry, which its cache is made for (where no row made it first).
    let (mut zblocks, mut zentry) = (0usize, 0usize);
    // And the documents' one tape format, 0 for several (decision 0576), of those with nodes.
    let mut format: Option<u32> = None;
    for p in docs.iter() {
        let Some(Ok((_, view))) = p.facts.tape.as_ref().map(|t| t.in_place()) else { continue };
        if p.len() > 0 {
            format = Some(same_format(format, view.format));
        }
        if view.format == rerail_clif::view::F16Z_FORMAT {
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
        // The `Loc` views read the lines' words too (decision 0612): lines the device cannot reach refuse both.
        if !r.lines_device && (lines || locs) && device.is_some() {
            crate::code_rule_dsl::prof::count("views.refused.lines_unplaced", 1);
            (lines, locs) = (false, false);
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
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, caches: cache.into_iter().collect(), format: format.unwrap_or(0) })
}

/// The one format of `so_far` and `next`, or 0 for several (decision 0576).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn same_format(so_far: Option<u32>, next: u32) -> u32 {
    match so_far {
        None => next,
        Some(f) if f == next => f,
        Some(_) => 0,
    }
}

/// The views' directory of a chunk's batch and the tape values admitted inside its fixpoint (decision 0431 §8,
/// `values::Layered`): the batch's documents, then each value's batch's, each view's rows of a value past those
/// before it, as a layered source numbers them. Each batch's rows are written through its own reads layer
/// (`views_in_place`: `Docs::reads`, the host's reads' rows), once; this directory names them, each row's first rows
/// past the batches before it and its homes among theirs, and each batch's 16z cache its own (decision 0353 step 4).
/// Every batch is in the chunk's block: `None` where one is not, or a batch's directory is refused.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
    // The batches' one format, else 0, which the stages read whatever the format (decision 0576).
    let format = own.iter().map(|o| o.format).reduce(|a, b| same_format(Some(a), b)).unwrap_or(0);
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, caches, format })
}
