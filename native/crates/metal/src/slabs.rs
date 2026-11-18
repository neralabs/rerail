//! The slab residency (architecture §5; PIAT §20; decision 0032): stored tapes and their sources
//! resident in slabs the device reads in place. This crate holds the arena (slabs, their budget,
//! retirement and the drop) and the sources; the engine places tapes in it (its
//! `residency::slabs`: reading frames, the reader's derived parts, the checks).
//!
//! - **Slabs** are chunk blocks (`rerail_blocks`, decision 0289 §4): each a block over a lease of
//!   `SLAB_BYTES` (an object larger gets its own class) from the one GPU buffer pool
//!   (`rerail_accel::gpu::shared::lease_block_within`, 0266 §16), so a retired slab's buffer is
//!   recycled whole by the next lease of its class, the residency's or any other user's. Objects are
//!   the block's allocations, at 16 bytes; a slab is append-only until it is retired whole: no
//!   compaction. A recycled slab holds its last lease's bytes, so the block zeroes each room as it
//!   hands it out; a new one reads zero.
//! - **A batch reads one block** (`gather`): a batch whose documents are in several slabs has them
//!   copied into one, each a contiguous range, and the residency then finds them there, so the next
//!   batch over them copies nothing. Kernels bind that one block.
//! - **A resident document** is keyed by its tape's SHA-256 (the store's canonical hash, which
//!   covers the format version). It holds, in one slab: the v9np image byte-identical to its
//!   frame's content (columns at 16-byte offsets, format 11), the source's UTF-8 bytes, and what a
//!   reader derives from them once per admission (decision 0026): each node's parent, the image's
//!   codes at a fixed width (`tape::resident::DeviceCode`) and the source's line starts.
//! - **Admission** reads the misses' frames in one store read (`ContentStore::resident_frames`),
//!   decompresses each straight into its slab and checks the bytes against the tape's and the
//!   source's SHA-256 and the image's header against the source, once. A failed check is an
//!   error, never an empty document.
//! - **A resident source** is a file's bytes alone, keyed by their SHA-256: a parser's input,
//!   which has no tape yet (decision 0067). Block programs read it in place
//!   (`metal_block_program::run_batch`); it shares the slabs, the budget, retirement and the drop
//!   with the documents.
//! - **Leases** are the `Arc`s a batch holds: a retired slab's memory goes when the last batch
//!   reading it lets go.
//! - **Budget.** Slabs are leased within the one Metal budget, with every other GPU buffer; the
//!   residency may hold `RESIDENT_SHARE` of it, the rest being the runs' workspaces. Past either,
//!   the least recently used slabs not read by the admission are retired; a batch that still does
//!   not fit is refused (`Error::BackendCapacity`), and the wave splits it.
//! - **Eviction.** The residency is the pool's evictor (`set_evictor`, called by `lib.rs`): when
//!   another user's buffer needs room that idle buffers cannot give, the least recently used slab
//!   is retired, and its buffer comes back to the pool once the last batch reading it lets go.
//! - **The drop:** two minutes after each full execution (`executed`), every resident document,
//!   slab and compiled pipeline is released (architecture §5, F4).

use crate::Error;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// A slab's bytes (PIAT §20.2.1's default).
pub const SLAB_BYTES: usize = 64 << 20;
/// How long after a full execution everything resident is dropped.
pub const DROP_AFTER: Duration = Duration::from_secs(120);
/// The share of the Metal buffer budget the residency may hold: half, the rest for the runs.
const RESIDENT_SHARE: (usize, usize) = (1, 2);
const ALIGN: usize = 16;

pub fn aligned(length: usize) -> usize {
    length.div_ceil(ALIGN) * ALIGN
}

/// A document as the device reads it: its first byte in the batch's chunk block (`at`, decision 0289 §4), and its
/// parts as byte offsets from there, so a copy into another block changes `at` alone. It is the view the reads
/// written once take (`rerail_clif::view::TapeView`, decision 0310), one definition for the device, the JIT and the
/// tape crate, and its MSL is written from it (`msl`), with every field's offset asserted. `node_base` is the
/// document's first node in a batch's numbering, set by the batch's document table (`DocTable`). The tape is read
/// where it lies, format 15's columns or format 16's arrays (decision 0293): nothing per node is derived beside it.
pub use rerail_clif::view::TapeView as DocView;
const _: () = assert!(std::mem::size_of::<DocView>() == 116);

/// The MSL of `DocView`, for the kernels that read documents in place (`metal_tape.metal`).
pub fn msl() -> String {
    rerail_clif::view::msl()
}

/// A slab: a chunk block over a lease of the pool, and its identity. Its rooms are the block's
/// allocations.
pub struct Slab {
    pub id: u64,
    block: rerail_blocks::Block,
    /// The lease's buffer, as kernels bind it (the block's backing's own).
    buffer: metal::Buffer,
}

impl Slab {
    /// A slab of at least `bytes`, within the Metal budget.
    fn lease(id: u64, bytes: usize) -> Result<Slab, Error> {
        let block = rerail_accel::gpu::shared::lease_block_within(rerail_blocks::block::kind::CHUNK, bytes).map_err(|_| Error::BackendCapacity)?;
        let device = crate::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let buffer = block.gpu_buffer(device);
        Ok(Slab { id, block, buffer })
    }

    /// Its buffer, as kernels bind it.
    pub fn buffer(&self) -> &metal::Buffer {
        &self.buffer
    }

    /// Its bytes as the host writes them.
    pub fn host(&self) -> *mut u8 {
        self.block.base_ptr()
    }

    pub fn byte_length(&self) -> usize {
        self.block.reserved()
    }

    /// A room of `bytes` at 16 bytes, zero, or `None` when the slab cannot hold it.
    fn room(&self, bytes: usize) -> Option<usize> {
        self.block.try_alloc(bytes, ALIGN)
    }
}

/// A resident document: its slab (held: a lease) and where its parts are.
pub struct Resident {
    pub digest: [u8; 32],
    pub slab: Arc<Slab>,
    /// Its view, `at` its range's start in `slab`, `node_base` 0.
    pub view: DocView,
    /// Its range in the slab, every part inside it: what `gather` copies.
    pub at: usize,
    pub len: usize,
    /// Bytes per pool: `tape.resident`, `source.resident`, `tape.derived`.
    pub bytes: [usize; 3],
    /// The tape's length where it is placed (a 16z tape's laid out as format 16).
    pub tape_length: usize,
}

impl Resident {
    /// This document copied into `slab` at `to` (its `len` bytes of room there, 16-byte aligned):
    /// its view's `at` moved with it, its parts being offsets from there.
    fn copied(&self, slab: &Arc<Slab>, to: usize) -> Resident {
        // SAFETY: `to..to + len` is a room of `slab` given to this copy alone; the source range is
        // this document's, written at admission and never again.
        unsafe { std::ptr::copy_nonoverlapping(self.slab.host().add(self.at), slab.host().add(to), self.len) };
        let view = DocView { at: to as u32, ..self.view };
        Resident { digest: self.digest, slab: Arc::clone(slab), view, at: to, len: self.len, bytes: self.bytes, tape_length: self.tape_length }
    }

    /// The bytes at `offset..offset + length` of the document (an offset of its view).
    pub fn bytes_at(&self, offset: u32, length: usize) -> &[u8] {
        let slab = &self.slab;
        let offset = self.at + offset as usize;
        assert!(offset + length <= self.at + self.len && offset + length <= slab.byte_length(), "a resident part inside its range");
        // SAFETY: admission wrote these bytes before publishing the document, and nothing writes
        // them again: a slab is append-only and the document's range is its own.
        unsafe { std::slice::from_raw_parts(slab.host().add(offset), length) }
    }



}

/// A resident source: a file's bytes at a 16-byte offset of its slab (at least 16 bytes of room,
/// so a 16-byte read of an empty file stays inside it).
pub struct ResidentSource {
    pub digest: [u8; 32],
    pub slab: Arc<Slab>,
    pub offset: usize,
    pub len: usize,
}

impl ResidentSource {
    /// Its bytes, as the device reads them.
    pub fn bytes(&self) -> &[u8] {
        assert!(self.offset + self.len <= self.slab.byte_length(), "a resident source inside its slab");
        // SAFETY: admission wrote these bytes before publishing the source, and nothing writes
        // them again: a slab is append-only and the source's range is its own.
        unsafe { std::slice::from_raw_parts(self.slab.host().add(self.offset), self.len) }
    }
}

/// The slabs a batch of sources binds, at most `MAX_SLABS`, and each source's binding; `None`
/// when they span more (the caller splits the batch).
pub fn source_slabs(sources: &[Arc<ResidentSource>]) -> Option<(Vec<Arc<Slab>>, Vec<u32>)> {
    let mut slabs: Vec<Arc<Slab>> = Vec::new();
    let mut at = Vec::with_capacity(sources.len());
    for source in sources {
        let k = match slabs.iter().position(|slab| slab.id == source.slab.id) {
            Some(k) => k,
            None if slabs.len() < MAX_SLABS => {
                slabs.push(Arc::clone(&source.slab));
                slabs.len() - 1
            }
            None => return None,
        };
        at.push(k as u32);
    }
    Some((slabs, at))
}

/// A batch's document table (`batch.doctable`): each document's view with its first node in the
/// batch's numbering, and the one block they are in (decision 0289 §4), `None` for no documents.
pub struct DocTable {
    pub views: Vec<DocView>,
    pub block: Option<Arc<Slab>>,
    pub nodes: usize,
    /// The documents, held for the batch's life.
    pub documents: Vec<Arc<Resident>>,
}

/// The slabs a batch of sources binds (PIAT §20.4.5, Q-R3): block programs take this many slab
/// buffers; a batch over more is split by the wave. A batch of documents binds one (`DocTable`).
pub const MAX_SLABS: usize = 4;

impl DocTable {
    /// The format every view is in, or `None` for an empty or mixed batch: the label kernels are specialized to it
    /// (the reads' format test made a constant, t41).
    pub fn format(&self) -> Option<u32> {
        let first = self.views.first()?.format;
        self.views.iter().all(|view| view.format == first).then_some(first)
    }

    /// The table of `documents`, in order, in one slab (`gather`): `None` when they cannot be.
    pub fn one_block(documents: Vec<Arc<Resident>>) -> Option<Self> {
        Self::new(gather(documents).ok()?)
    }

    /// The table of `documents`, in order, or `None` when they are not all in one slab.
    pub fn new(documents: Vec<Arc<Resident>>) -> Option<Self> {
        let block = documents.first().map(|document| Arc::clone(&document.slab));
        if documents.iter().any(|document| Some(document.slab.id) != block.as_ref().map(|block| block.id)) {
            return None;
        }
        let mut views = Vec::with_capacity(documents.len());
        let mut nodes = 0usize;
        let (mut source, mut lines) = (0usize, 0usize);
        for document in &documents {
            views.push(DocView {
                node_base: u32::try_from(nodes).ok()?,
                source_base: u32::try_from(source).ok()?,
                line_base: u32::try_from(lines).ok()?,
                ..document.view
            });
            nodes += document.view.nodes as usize;
            source += document.view.source_len as usize;
            lines += document.view.line_count as usize;
        }
        u32::try_from(nodes).ok()?;
        u32::try_from(source).ok()?;
        Some(Self { views, block, nodes, documents })
    }
}

struct SlabEntry {
    slab: Arc<Slab>,
    /// The admission clock when a document of it was last asked for.
    last_use: u64,
    /// Whether it holds a loaded tape's home (`tape_room`, decision 0353 step 4a): the tape holds the slab, so
    /// retiring it would free nothing, only lose its count. Never retired.
    tapes: bool,
}

#[derive(Default)]
struct State {
    slabs: Vec<SlabEntry>,
    documents: HashMap<[u8; 32], Arc<Resident>>,
    sources: HashMap<[u8; 32], Arc<ResidentSource>>,
    next_slab: u64,
    clock: u64,
    /// Bytes per pool, of the documents held.
    pools: [usize; 3],
    deadline: Option<Instant>,
}

static STATE: LazyLock<(Mutex<State>, Condvar)> = LazyLock::new(Default::default);

/// What the residency holds now: slabs, documents and bytes per pool.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Held {
    pub slabs: usize,
    pub documents: usize,
    pub sources: usize,
    pub pools: [usize; 3],
}

pub fn held() -> Held {
    let state = STATE.0.lock().expect("residency poisoned");
    Held { slabs: state.slabs.len(), documents: state.documents.len(), sources: state.sources.len(), pools: state.pools }
}

/// The residency's limit: its share of the Metal buffer budget.
fn limit() -> usize {
    crate::runtime::buffer_limit() / RESIDENT_SHARE.1 * RESIDENT_SHARE.0
}





impl State {
    /// A slab with `bytes` free, as (index, offset), zero: the newest slab's room, else a new
    /// slab (retiring the least recently used ones not in `keep` while the budget is short).
    fn room(&mut self, bytes: usize, keep: &[u64]) -> Result<(usize, usize), Error> {
        if let Some(offset) = self.slabs.last().and_then(|last| last.slab.room(bytes)) {
            return Ok((self.slabs.len() - 1, offset));
        }
        // The block's header and alignment take a little of a slab.
        let length = (bytes + 4096).max(SLAB_BYTES);
        loop {
            let held = self.slabs.iter().map(|entry| entry.slab.byte_length()).sum::<usize>();
            if held + length <= limit() {
                // The pool's evictor is this residency, whose lock is held here: it skips, and the
                // retiring is this loop's.
                match Slab::lease(self.next_slab + 1, length) {
                    Ok(slab) => {
                        self.next_slab += 1;
                        let offset = slab.room(bytes).ok_or(Error::BackendCapacity)?;
                        self.slabs.push(SlabEntry { slab: Arc::new(slab), last_use: self.clock, tapes: false });
                        rerail_prof::count("residency.slabs", 1);
                        return Ok((self.slabs.len() - 1, offset));
                    }
                    Err(_) => rerail_prof::count("capacity.residency_slab", 1),
                }
            }
            if !self.retire(keep) {
                return Err(Error::BackendCapacity);
            }
        }
    }

    /// Retires the least recently used slab not in `keep`: its documents are no longer resident,
    /// and its memory goes with the last batch reading it. False when there is none.
    fn retire(&mut self, keep: &[u64]) -> bool {
        let Some(at) = self
            .slabs
            .iter()
            .enumerate()
            .filter(|(_, entry)| !keep.contains(&entry.slab.id) && !entry.tapes)
            .min_by_key(|(_, entry)| entry.last_use)
            .map(|(at, _)| at)
        else {
            return false;
        };
        let entry = self.slabs.remove(at);
        let mut pools = self.pools;
        self.documents.retain(|_, document| {
            let gone = document.slab.id == entry.slab.id;
            if gone {
                for (pool, bytes) in pools.iter_mut().zip(document.bytes) {
                    *pool -= bytes;
                }
            }
            !gone
        });
        self.sources.retain(|_, source| {
            let gone = source.slab.id == entry.slab.id;
            if gone {
                pools[1] -= source_room(source.len);
            }
            !gone
        });
        self.pools = pools;
        rerail_prof::count("residency.retired", 1);
        true
    }
}



/// The room a resident source takes: its bytes and `SOURCE_MARGIN`, 16-byte aligned.
fn source_room(len: usize) -> usize {
    aligned(len.max(1) + SOURCE_MARGIN)
}

/// Zero bytes past each source in its slab (a slab is mapped zeroed, and nothing else is written
/// there): tier 2 reads its words in place, a window past the file's end reading zeros, as the
/// batch's layout (`native_cpu_block::BACK`).
pub const SOURCE_MARGIN: usize = rerail_compiler::native_cpu_block::BACK;

/// Files' bytes resident as sources, in order: hits by SHA-256 from the residency, misses copied
/// into slabs once (the one copy a file's bytes take on their way to the device). The slabs of
/// every source asked for are kept while the misses are placed.
pub fn admit_sources(files: &[&[u8]]) -> Result<Vec<Arc<ResidentSource>>, Error> {
    let digests: Vec<[u8; 32]> = files.iter().map(|bytes| Sha256::digest(bytes).into()).collect();
    admit_sources_digested(files, &digests)
}

/// `admit_sources` with each file's SHA-256 given: the caller's, as a document's tape digest is
/// (decision 0032 §4), so no file's bytes are hashed here. A digest must be its file's.
pub fn admit_sources_digested(files: &[&[u8]], digests: &[[u8; 32]]) -> Result<Vec<Arc<ResidentSource>>, Error> {
    let _admit = rerail_prof::Stage::start("residency.sources");
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    state.clock += 1;
    let clock = state.clock;
    let mut keep = Vec::new();
    for digest in digests {
        if let Some(id) = state.sources.get(digest).map(|source| source.slab.id) {
            keep.push(id);
            if let Some(entry) = state.slabs.iter_mut().find(|entry| entry.slab.id == id) {
                entry.last_use = clock;
            }
        }
    }
    let mut misses = 0usize;
    for (digest, bytes) in digests.iter().zip(files) {
        if state.sources.contains_key(digest) {
            continue;
        }
        misses += 1;
        let room = source_room(bytes.len());
        let (at, offset) = state.room(room, &keep)?;
        let entry = &mut state.slabs[at];
        // SAFETY: `room` gave this range to this source alone, inside the slab's mapping.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), entry.slab.host().add(offset), bytes.len()) };
        entry.last_use = clock;
        let slab = Arc::clone(&entry.slab);
        keep.push(slab.id);
        state.pools[1] += room;
        state.sources.insert(*digest, Arc::new(ResidentSource { digest: *digest, slab, offset, len: bytes.len() }));
    }
    rerail_prof::count("residency.source_hits", files.len() - misses);
    rerail_prof::count("residency.source_misses", misses);
    digests.iter().map(|digest| state.sources.get(digest).cloned().ok_or(Error::BackendCapacity)).collect()
}

/// The digests the residency does not hold (each once); the hits' slabs are marked used.
pub fn misses(digests: &[[u8; 32]]) -> Result<Vec<[u8; 32]>, Error> {
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    state.clock += 1;
    let clock = state.clock;
    let mut missing = Vec::new();
    for digest in digests {
        match state.documents.get(digest).map(|document| document.slab.id) {
            Some(slab) => {
                if let Some(entry) = state.slabs.iter_mut().find(|entry| entry.slab.id == slab) {
                    entry.last_use = clock;
                }
            }
            None if !missing.contains(digest) => missing.push(*digest),
            None => {}
        }
    }
    rerail_prof::count("residency.hits", digests.len() - missing.len());
    rerail_prof::count("residency.misses", missing.len());
    Ok(missing)
}


pub fn resident(digests: &[[u8; 32]]) -> Result<Vec<Arc<Resident>>, Error> {
    let state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    digests.iter().map(|digest| state.documents.get(digest).cloned().ok_or(Error::BackendCapacity)).collect()
}

/// A document to place: its digest, the bytes it takes, and how to write it into a slab at an
/// offset (`bytes` of room there, its own).
pub type Placement<'a> = ([u8; 32], usize, Box<dyn FnOnce(&Arc<Slab>, usize) -> Result<Resident, Error> + 'a>);

/// A room of `bytes` (16-byte aligned) for a tape the store loads (decision 0353 step 4a): its slab and its offset
/// there. The tape is written there once, by its load, and read there by the host and the device; the slab lives as
/// long as a holder of it does, retired from the residency or not.
pub fn tape_room(bytes: usize) -> Result<(Arc<Slab>, usize), Error> {
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    let (at, offset) = state.room(bytes, &[])?;
    let clock = state.clock;
    state.slabs[at].last_use = clock;
    state.slabs[at].tapes = true;
    Ok((Arc::clone(&state.slabs[at].slab), offset))
}

/// Places the misses, keeping the slabs of every document `digests` reads.
pub fn insert(digests: &[[u8; 32]], parts: Vec<Placement<'_>>) -> Result<(), Error> {
    let _admit = rerail_prof::Stage::start("residency.admit");
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    let mut keep = digests.iter().filter_map(|digest| state.documents.get(digest).map(|document| document.slab.id)).collect::<Vec<_>>();
    for (digest, bytes, place) in parts {
        let (at, offset) = state.room(bytes, &keep)?;
        let document = place(&state.slabs[at].slab, offset)?;
        let clock = state.clock;
        let entry = &mut state.slabs[at];
        entry.last_use = clock;
        keep.push(entry.slab.id);
        for (pool, bytes) in state.pools.iter_mut().zip(document.bytes) {
            *pool += bytes;
        }
        state.documents.insert(digest, Arc::new(document));
    }
    Ok(())
}

/// `documents` in one slab, in order: those not in it already are copied there (a contiguous range
/// each), and the residency finds them there from now on, so a later batch over them copies
/// nothing. The newest slab when the copies fit in it, else a new one holding them all.
pub fn gather(documents: Vec<Arc<Resident>>) -> Result<Vec<Arc<Resident>>, Error> {
    let Some(first) = documents.first() else { return Ok(documents) };
    if documents.iter().all(|document| document.slab.id == first.slab.id) {
        return Ok(documents);
    }
    let _gather = rerail_prof::Stage::start("residency.gather");
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    let keep: Vec<u64> = documents.iter().map(|document| document.slab.id).collect();
    let room = |documents: &mut dyn Iterator<Item = &Arc<Resident>>| documents.map(|document| aligned(document.len)).sum::<usize>();
    let newest = state.slabs.last().map(|entry| entry.slab.id);
    let outside = room(&mut documents.iter().filter(|document| Some(document.slab.id) != newest));
    let target = match state.slabs.last().and_then(|last| last.slab.room(outside).map(|offset| (Arc::clone(&last.slab), offset))) {
        Some(found) => found,
        None => {
            // A new slab for every document: none of them is in it yet.
            let (at, offset) = state.room(room(&mut documents.iter()), &keep)?;
            (Arc::clone(&state.slabs[at].slab), offset)
        }
    };
    let (slab, mut to) = target;
    let mut gathered = Vec::with_capacity(documents.len());
    let mut copied = 0usize;
    for document in documents {
        if document.slab.id == slab.id {
            gathered.push(document);
            continue;
        }
        let moved = Arc::new(document.copied(&slab, to));
        to += aligned(document.len);
        copied += document.len;
        // The residency finds it here now; the slab it was in may be retired once no batch reads it.
        if state.documents.get(&document.digest).is_some_and(|held| Arc::ptr_eq(held, &document)) {
            state.documents.insert(document.digest, Arc::clone(&moved));
        }
        gathered.push(moved);
    }
    let clock = state.clock;
    if let Some(entry) = state.slabs.iter_mut().find(|entry| entry.slab.id == slab.id) {
        entry.last_use = clock;
    }
    rerail_prof::count("residency.gathered_kb", copied >> 10);
    Ok(gathered)
}

/// The pool's evictor (`rerail_accel::gpu::shared::set_evictor`): retires the least recently used
/// slab, whatever reads it, when the residency's lock is free now; whether it did. A batch holding
/// the slab keeps its memory until it ends; the documents in it are admitted again when next asked.
pub fn evict() -> bool {
    let Ok(mut state) = STATE.0.try_lock() else { return false };
    state.retire(&[])
}

/// Forgets everything resident: documents, sources and slabs (each slab's memory goes as its last
/// batch lets go). The engine's drop releases its pipelines and pools with it.
pub fn clear() {
    if let Ok(mut state) = STATE.0.lock() {
        state.documents.clear();
        state.sources.clear();
        state.slabs.clear();
        state.pools = [0; 3];
        state.deadline = None;
    }
}

/// A full execution ended: `drop_all` runs `DROP_AFTER` from now, unless another ends first
/// (which moves the deadline). The first call's `drop_all` is the one kept.
pub fn executed(drop_all: fn()) {
    static DROPPER: std::sync::Once = std::sync::Once::new();
    let (lock, wake) = &*STATE;
    if let Ok(mut state) = lock.lock() {
        state.deadline = Some(Instant::now() + DROP_AFTER);
    }
    wake.notify_all();
    DROPPER.call_once(|| {
        rerail_accel::exec::service("rerail-residency-drop", None, move || {
                let (lock, wake) = &*STATE;
                let Ok(mut state) = lock.lock() else { return };
                loop {
                    match state.deadline {
                        None => state = wake.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner()),
                        Some(deadline) if Instant::now() < deadline => {
                            let wait = deadline.saturating_duration_since(Instant::now());
                            state = wake.wait_timeout(state, wait).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
                        }
                        Some(_) => {
                            drop(state);
                            drop_all();
                            state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        }
                    }
                }
            })
            .expect("the residency's drop thread");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The residency's slabs are the pool's: one evicted goes back to it whole, and the next slab leased of its class is
    /// that buffer, its rooms zeroed though it holds the last lease's bytes.
    #[test]
    fn an_evicted_slab_is_recycled_and_its_rooms_read_zero() {
        if crate::METAL_DEVICE.is_none() {
            return;
        }
        // The residency is the process's; other tests of this crate admit sources into it too.
        static ONE: Mutex<()> = Mutex::new(());
        let _one = ONE.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        // An object past `SLAB_BYTES` gets a class of its own: 64 MiB + 1 MiB is in the 80 MiB class.
        let big = vec![0xA5_u8; SLAB_BYTES + (1 << 20)];
        let first = admit_sources(&[&big]).unwrap();
        let address = first[0].slab.buffer().gpu_address();
        assert_eq!(first[0].bytes(), &big[..]);
        drop(first);
        assert!(evict(), "the residency's slab retired");
        assert_eq!(held().sources, 0);
        // A smaller source, of the same class: the buffer comes back, dirty, and its room reads zero past the bytes.
        let small = vec![0x11_u8; SLAB_BYTES + (1 << 19)];
        let again = admit_sources(&[&small]).unwrap();
        assert_eq!(again[0].slab.buffer().gpu_address(), address, "the evicted slab's buffer served the next lease");
        assert_eq!(again[0].bytes(), &small[..]);
        // SAFETY: the room's margin, inside the slab, written by no one but the zeroing.
        let margin = unsafe { std::slice::from_raw_parts(again[0].slab.host().add(again[0].offset + small.len()), SOURCE_MARGIN) };
        assert!(margin.iter().all(|&b| b == 0), "a room in a recycled slab is zero");
        drop(again);
        clear();
    }
}
