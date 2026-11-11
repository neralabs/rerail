//! The slab residency (architecture §5; PIAT §20; decision 0032): stored tapes and their sources
//! resident in slabs the device reads in place. This crate holds the arena (slabs, their budget,
//! retirement and the drop) and the sources; the engine places tapes in it (its
//! `residency::slabs`: reading frames, the reader's derived parts, the checks).
//!
//! - **Slabs** are anonymous mappings of `SLAB_BYTES` (an object larger gets its own), each with
//!   one retained no-copy shared `MTLBuffer` (`MappedBuffer::slab`). Objects are bump-allocated at
//!   16 bytes; a slab is append-only until it is retired whole: no compaction, no partial
//!   `MADV_DONTNEED`.
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
//! - **Budget.** Slabs are reserved against the Metal buffer budget before they are mapped (as
//!   every buffer is); the residency may hold `RESIDENT_SHARE` of it, the rest being the runs'
//!   workspaces. Past it, the least recently used slabs not read by the admission are retired; a
//!   batch that still does not fit is refused (`Error::BackendCapacity`), and the wave splits it.
//! - **The drop:** two minutes after each full execution (`executed`), every resident document,
//!   slab and compiled pipeline is released (architecture §5, F4).

use crate::Error;
use crate::runtime::MappedBuffer;
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

/// A document as the device reads it: byte offsets into its slab, as `DocView` in
/// `metal_tape.metal`. `node_base` is the document's first node in a batch's numbering, set by
/// the batch's document table (`DocTable`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DocView {
    /// The batch's slab binding the document is in (`DocTable::slabs`).
    pub slab: u32,
    pub nodes: u32,
    pub node_base: u32,
    /// `size`'s, `off`'s and `tail`'s widths in bytes, at bits 0, 4 and 8; bit 12: tails measured
    /// from the node's own start.
    pub widths: u32,
    pub code: u32,
    pub size: u32,
    pub base: u32,
    pub off: u32,
    pub tail: u32,
    /// Each node's parent (u32, `u32::MAX` at the root).
    pub parent: u32,
    /// The fixed-width codes (`DeviceCode`).
    pub codes: u32,
    pub source: u32,
    pub source_len: u32,
    /// Line starts (u32, document-local bytes), the first 0.
    pub lines: u32,
    pub line_count: u32,
    /// The code table's length.
    pub code_count: u32,
    /// Each node's first entry in `kids` (u32, `nodes + 1` of them), and the children of every
    /// node in node order (u32, document-local; `kid_count` of them, a tree's `nodes - 1`): the
    /// child lists a reader builds at admission, for the interpreter's child by index.
    pub kid_start: u32,
    pub kids: u32,
    pub kid_count: u32,
    /// The document's place in a batch's numbering of children, source bytes and lines, as a
    /// packed batch concatenates them (set by `DocTable`).
    pub child_base: u32,
    pub source_base: u32,
    pub line_base: u32,
    /// Each code's first entry in `by_code` (u32, `code_count + 1` of them), and the nodes of
    /// every code in node order (u32, document-local, `nodes` of them): a pass's roots in this
    /// document are the nodes of the codes its rule admits (`label_passes::Roots`), read in place.
    pub code_start: u32,
    pub by_code: u32,
}
const _: () = assert!(std::mem::size_of::<DocView>() == 96);

/// A slab: its buffer and identity. Its bump position is the residency's (`State`).
pub struct Slab {
    pub id: u64,
    pub buffer: MappedBuffer,
}

/// A resident document: its slab (held: a lease) and where its parts are.
pub struct Resident {
    pub digest: [u8; 32],
    pub slab: Arc<Slab>,
    /// Its view with `slab` and `node_base` 0.
    pub view: DocView,
    /// Bytes per pool: `tape.resident`, `source.resident`, `tape.derived`.
    pub bytes: [usize; 3],
    /// The tape image's length, as stored.
    pub tape_length: usize,
}

impl Resident {
    /// The bytes at `offset..offset + length` of its slab.
    pub fn bytes_at(&self, offset: u32, length: usize) -> &[u8] {
        let slab = &self.slab.buffer;
        assert!(offset as usize + length <= slab.byte_length(), "a resident part inside its slab");
        // SAFETY: admission wrote these bytes before publishing the document, and nothing writes
        // them again: a slab is append-only and the document's range is its own.
        unsafe { std::slice::from_raw_parts(slab.host().add(offset as usize), length) }
    }

    /// Each code's first entry in the nodes by code, `code_count + 1` of them.
    pub fn code_starts(&self) -> &[u32] {
        let length = self.view.code_count as usize + 1;
        let bytes = self.bytes_at(self.view.code_start, 4 * length);
        // SAFETY: as `codes`, u32s at a 16-byte offset.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u32>(), length) }
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
        assert!(self.offset + self.len <= self.slab.buffer.byte_length(), "a resident source inside its slab");
        // SAFETY: admission wrote these bytes before publishing the source, and nothing writes
        // them again: a slab is append-only and the source's range is its own.
        unsafe { std::slice::from_raw_parts(self.slab.buffer.host().add(self.offset), self.len) }
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

/// A batch's document table (`batch.doctable`): each document's view with its slab binding and
/// first node in the batch's numbering, and the slabs bound, at most `MAX_SLABS`.
pub struct DocTable {
    pub views: Vec<DocView>,
    pub slabs: Vec<Arc<Slab>>,
    pub nodes: usize,
    /// The documents, held for the batch's life.
    pub documents: Vec<Arc<Resident>>,
}

/// The slabs a batch binds (PIAT §20.4.5, Q-R3): kernels take this many slab buffers; a batch
/// over more is split by the wave.
pub const MAX_SLABS: usize = 4;

impl DocTable {
    /// The table of `documents`, in order, or `None` when they span more than `MAX_SLABS` slabs.
    pub fn new(documents: Vec<Arc<Resident>>) -> Option<Self> {
        let mut slabs: Vec<Arc<Slab>> = Vec::new();
        let mut views = Vec::with_capacity(documents.len());
        let mut nodes = 0usize;
        let (mut children, mut source, mut lines) = (0usize, 0usize, 0usize);
        for document in &documents {
            let slab = match slabs.iter().position(|slab| slab.id == document.slab.id) {
                Some(at) => at,
                None if slabs.len() < MAX_SLABS => {
                    slabs.push(Arc::clone(&document.slab));
                    slabs.len() - 1
                }
                None => return None,
            };
            views.push(DocView {
                slab: slab as u32,
                node_base: u32::try_from(nodes).ok()?,
                child_base: u32::try_from(children).ok()?,
                source_base: u32::try_from(source).ok()?,
                line_base: u32::try_from(lines).ok()?,
                ..document.view
            });
            nodes += document.view.nodes as usize;
            children += document.view.kid_count as usize;
            source += document.view.source_len as usize;
            lines += document.view.line_count as usize;
        }
        u32::try_from(nodes).ok()?;
        u32::try_from(source).ok()?;
        Some(Self { views, slabs, nodes, documents })
    }
}

struct SlabEntry {
    slab: Arc<Slab>,
    used: usize,
    /// The admission clock when a document of it was last asked for.
    last_use: u64,
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
    /// A slab with `bytes` free, as (index, offset): the newest slab's room, else a new slab
    /// (retiring the least recently used ones not in `keep` while the budget is short).
    fn room(&mut self, bytes: usize, keep: &[u64]) -> Result<(usize, usize), Error> {
        if let Some(last) = self.slabs.last()
            && last.slab.buffer.byte_length() - last.used >= bytes
        {
            return Ok((self.slabs.len() - 1, last.used));
        }
        let length = bytes.max(SLAB_BYTES);
        loop {
            let held = self.slabs.iter().map(|entry| entry.slab.buffer.byte_length()).sum::<usize>();
            if held + length <= limit() {
                match MappedBuffer::slab(length) {
                    Ok(buffer) => {
                        self.next_slab += 1;
                        let slab = Arc::new(Slab { id: self.next_slab, buffer });
                        self.slabs.push(SlabEntry { slab, used: 0, last_use: self.clock });
                        rerail_prof::count("residency.slabs", 1);
                        return Ok((self.slabs.len() - 1, 0));
                    }
                    Err(Error::BackendCapacity) => {}
                    Err(error) => return Err(error),
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
            .filter(|(_, entry)| !keep.contains(&entry.slab.id))
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

/// Readable bytes past each source in its slab: tier 2 reads its words in place, a token's text
/// in 8-byte loads up to 64 bytes past the file's end (masked by its length).
pub const SOURCE_MARGIN: usize = 64;

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
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), entry.slab.buffer.host().add(offset), bytes.len()) };
        entry.used = offset + room;
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
        entry.used = offset + bytes;
        entry.last_use = clock;
        keep.push(entry.slab.id);
        for (pool, bytes) in state.pools.iter_mut().zip(document.bytes) {
            *pool += bytes;
        }
        state.documents.insert(digest, Arc::new(document));
    }
    Ok(())
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
