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
/// The `op` view (`View::Op`): each numbered node's anonymous children, its id and their name atoms, read in place
/// only (decision 0353 row 7).
pub const VIEW_OP: u32 = 13;

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
        VIEW_OP => 13,
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
        View::Op => VIEW_OP,
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
    /// Whether every document's `Loc` rows are the views' reads' (`ViewEntry::locs`: a file's own tree, or a guest
    /// placed in its host by its placement or its spans placed, its places' lines in the batch's block) and the device
    /// reads them, so the `Loc` views are read in place too.
    pub locs: bool,
    /// Whether the device reads every document's own source in place (each in a slab: decision 0353 step 7, row 13),
    /// so a node's text is read there (`view_text_span`, decision 0287 §2). Where one is not, a stage that reads a
    /// node's text is the host's; the directory is still the device's for every other read.
    pub sources: bool,
    /// The cache a 16z document's reads decode its blocks into (decision 0353 step 4b, `view::cache`), in the batch's
    /// block, which every reader of the batch's views shares, on the host and on the device: none where no tape is
    /// 16z.
    /// Each batch's, a layered source's (`views_over`) one for each of its batches that has one.
    pub caches: Vec<crate::code_rule_dsl::facts::vtab::ZCache>,
    /// The directory as the host's join programs read it (`rerail_accel::gpu::shared::on_host`), whether or not a
    /// device reads it too.
    pub host: HostDir,
    /// The tape format of every document read off its tape (15, 16, `F16Z_FORMAT`), or 0 for a batch of several: the
    /// stages that read its views are compiled for it (decision 0576).
    pub format: u32,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl ViewsInPlace {
    /// Whether the join programs that read a view here run on the host: bound to it (`on_host`), or with no process
    /// device.
    pub fn read_on_host() -> bool {
        rerail_accel::gpu::shared::is_on_host() || rerail_accel::gpu::shared::layer().is_none()
    }
    /// Whether the directory here (`read_on_host`'s, or the device's) holds every document's lines, and every
    /// document's `Loc` rows.
    pub fn holds(&self) -> (bool, bool) {
        if Self::read_on_host() { (self.host.lines, self.host.locs) } else { (self.lines, self.locs) }
    }
    /// View `view`'s read in place as the join programs here read it (`read_on_host`): on the host, the directory at
    /// its host address with no buffer made resident (its cache's included); on the device, at the device's, with
    /// its buffers. `None` where the device has no address for it.
    pub fn read_of(&self, view: u32, keep: Arc<dyn std::any::Any + Send + Sync>) -> Option<crate::code_rule_dsl::facts::vtab::ViewInPlace> {
        let (dir, buffers, caches) = if Self::read_on_host() {
            (self.host.dir, Vec::new(), self.caches.iter().map(|c| crate::code_rule_dsl::facts::vtab::ZCache { buffer: None, ..c.clone() }).collect())
        } else {
            (self.dir?, self.buffers.clone(), self.caches.clone())
        };
        let mut read = crate::code_rule_dsl::facts::vtab::ViewInPlace { dir, kind: read_kind(view), buffers, caches, failed: 0, format: self.format, keep };
        read.failed = read.cache_failed();
        Some(read)
    }
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

/// A batch's cache for `blocks` 16z blocks of entries up to `entry` bytes (`super::batch_cache`): in the device's
/// memory where there is a device, so its host reads and its device's share it (decision 0353 step 4b), else the
/// host's (`ZCache::host`, in the run's room). The device's is a lease of its pool, which its ledger holds while work
/// that reads it runs (decision 0491 step 2).
pub(super) fn batch_cache(blocks: usize, entry: usize) -> rerail_tape_format::f16::ZCache {
    let spec = rerail_clif::view::cache::Spec::configured(blocks, entry);
    crate::code_rule_dsl::prof::count("zcache.batches", 1);
    crate::code_rule_dsl::prof::count("zcache.blocks", blocks);
    crate::code_rule_dsl::prof::count("zcache.entries", spec.entries as usize);
    crate::code_rule_dsl::prof::count("zcache.entry_kb", spec.entry_bytes() >> 10);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Some(device) = crate::code_rule_dsl::tape::placed::device() {
        let _ = device;
        if let Ok(lease) = rerail_accel::gpu::shared::lease(spec.bytes()) {
            let at = lease.host();
            // SAFETY: the lease's bytes (at least `spec.bytes()`), shared with the host, read by no one yet.
            spec.init(unsafe { std::slice::from_raw_parts_mut(at, spec.bytes()) }).expect("a cache's buffer of its size");
            // SAFETY: the cache just made in the lease, which the cache holds.
            return unsafe { rerail_tape_format::f16::ZCache::over(at, &spec, Box::new(lease)) };
        }
    }
    rerail_tape_format::f16::ZCache::host(blocks, entry)
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl ViewsInPlace {
    /// Its `node` view as a stage reads it in place, `keep` keeping its directory's rows, and the view's rows in all
    /// (`ViewDir::ends`), read in `block` (the batch's, which the directory lies in), as the join programs here read
    /// it (`read_of`): `None` where the device reads none of it (`dir`).
    pub fn node_view(&self, block: &rerail_blocks::Block, keep: Arc<dyn std::any::Any + Send + Sync>) -> Option<(crate::code_rule_dsl::facts::vtab::ViewInPlace, usize)> {
        let v = self.read_of(VIEW_NODE, keep)?;
        // SAFETY: the directory lies in `block` at `at` (`build_views`, `views_over`), its counts written.
        let nodes = unsafe { (*block.ptr::<rerail_clif::view_reads::ViewDir>(self.at)).ends[0] } as usize;
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
    // The documents' one tape format, 0 for several (decision 0576), of those with nodes.
    let mut format: Option<u32> = None;
    for p in docs.iter() {
        let Some(Ok((_, view))) = p.facts.tape.as_ref().map(|t| t.in_place()) else { continue };
        if p.len() > 0 {
            format = Some(same_format(format, view.format));
        }
    }
    // What the device reads: every tape of a document with nodes (else no stage reads the directory), every
    // document's lines (else the batch's `line` view is made, counted), its `Loc` rows.
    let (mut on_device, lines, mut locs, mut sources) = (device.is_some(), true, true, true);
    // What the device alone refuses (a home it has no address for): the host reads it all the same, by the homes'
    // host addresses (`ViewsInPlace::host`).
    let (mut dev_lines, mut dev_locs) = (true, true);
    let mut slabs: Vec<rerail_device::device::Buffer> = Vec::new();
    for i in 0..docs.len() {
        // Its tape's rows' derived columns its program reads, made before its entry is written, which points at them
        // (`ViewDoc::gaps`, …).
        super::tape_rows::derive(batch, i);
        let r = docs.reads(i).ok()?;
        locs &= r.locs;
        if r.entry.is_none() {
            continue;
        }
        if !r.tape_device && device.is_some() {
            // The device's refusal, by why: the host reads it all the same.
            let z = docs[i].facts.tape.as_ref().is_some_and(|t| t.image().format() == crate::code_rule_dsl::tape::formats::Format::F16Z);
            crate::code_rule_dsl::prof::count(if z { "views.refused.unplaced_16z" } else { "views.refused.unplaced" }, 1);
            if r.nodes { on_device = false } else { dev_locs = false }
        }
        // The `Loc` views read the lines' words too (decision 0612): lines the device cannot reach refuse both.
        if !r.lines_device && (lines || locs) && device.is_some() {
            crate::code_rule_dsl::prof::count("views.refused.lines_unplaced", 1);
            (dev_lines, dev_locs) = (false, false);
        }
        if !r.source_device && r.nodes && device.is_some() {
            crate::code_rule_dsl::prof::count("views.refused.source_unplaced", 1);
            sources = false;
        }
        for slab in &r.slabs {
            if !slabs.iter().any(|s| s.device_address() == slab.device_address()) {
                slabs.push(slab.clone());
            }
        }
    }
    // The codes' directory (decision 0697), which every row's `node` and `field` reads read.
    if let Some(codes) = device.and_then(|_| docs.codes_device()) {
        slabs.push(codes.clone());
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
    // The host's: the same directory at its host address, its homes read by theirs (`ViewHome::host`), and each 16z
    // view's cache found from the directory's (`ViewDir::cache`), not the device's offset in the row.
    let host = HostDir { dir: batch.block.base_ptr() as u64 + at as u64, lines, locs };
    let lines = lines && dev_lines && dir_on_device.is_some();
    let locs = locs && dev_locs && dir_on_device.is_some();
    let sources = sources && dir_on_device.is_some();
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, sources, host, caches: cache.into_iter().collect(), format: format.unwrap_or(0) })
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
    // The documents' keys in row order (`ViewDir::keys`), which a search by key reads alone: the chunk's batch's first,
    // in key order, then each values batch's, in the order admitted (t50 step 9, Metal at 512 MB: a value admitted with
    // a key below the chunk's was never found by its parents' walk, `tp_ident` and `tp_list` 9 rows short).
    let keys: *mut u32 = b.alloc_t(n.max(1));
    for i in 0..n {
        // SAFETY: row `i` of the `n` written above, and key `i` of the `n` just allocated in the chunk's block.
        unsafe { keys.add(i).write((*rows.add(i)).key) };
    }
    // SAFETY: the chunk's batch's directory, as above.
    let nbatch = unsafe { (*batch.docs.dir()).ndocs };
    let at = b.offset_of(dir);
    // The chunk's batch's cache is the directory's; each value's rows name their own in their views (`TapeView::side`).
    // SAFETY: the chunk's batch's directory, as above.
    let cache = unsafe { (*batch.docs.dir()).cache };
    // So is its codes' home (the binary's directory, decision 0697): the chunk's batch's homes are the first.
    // SAFETY: as above.
    let codes = unsafe { (*batch.docs.dir()).codes };
    // SAFETY: the directory, just allocated in the chunk's block, made whole.
    unsafe { dir.write(ViewDir { at: at as u64, ndocs: n as u32, nhomes: nhomes as u32, rows: b.offset_of(rows) as u64, homes: b.offset_of(homes) as u64, cache, nbatch, keys: b.offset_of(keys) as u64, ends: past, counted: 1, codes }) };
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
    // The host's: the same directory at its host address, each batch's rows' homes read by theirs (`ViewHome::host`).
    let host = HostDir { dir: b.base_ptr() as u64 + at as u64, lines: own.iter().all(|o| o.host.lines), locs: own.iter().all(|o| o.host.locs) };
    let lines = dir_on_device.is_some() && own.iter().all(|o| o.lines);
    let locs = dir_on_device.is_some() && own.iter().all(|o| o.locs);
    let sources = dir_on_device.is_some() && own.iter().all(|o| o.sources);
    let caches = own.iter().flat_map(|o| o.caches.iter().cloned()).collect();
    // The batches' one format, else 0, which the stages read whatever the format (decision 0576).
    let format = own.iter().map(|o| o.format).reduce(|a, b| same_format(Some(a), b)).unwrap_or(0);
    Some(ViewsInPlace { at, dir: dir_on_device, buffers, lines, locs, sources, host, caches, format })
}
