//! The slab residency (architecture §5; PIAT §20; decision 0032): stored tapes and their sources
//! resident in slabs the device reads in place.
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

use crate::code_rule_dsl::tape::Grammar;
use crate::code_rule_dsl::tape::resident::{self, DeviceCode};
use crate::code_rule_dsl::transducer::packed::Error;
use crate::code_rule_dsl::transducer::packed::metal_backend::MappedBuffer;
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// A slab's bytes (PIAT §20.2.1's default).
pub(in crate::code_rule_dsl) const SLAB_BYTES: usize = 64 << 20;
/// How long after a full execution everything resident is dropped.
pub(in crate::code_rule_dsl) const DROP_AFTER: Duration = Duration::from_secs(120);
/// The share of the Metal buffer budget the residency may hold: half, the rest for the runs.
const RESIDENT_SHARE: (usize, usize) = (1, 2);
const ALIGN: usize = 16;

fn aligned(length: usize) -> usize {
    length.div_ceil(ALIGN) * ALIGN
}

/// A document as the device reads it: byte offsets into its slab, as `DocView` in
/// `metal_tape.metal`. `node_base` is the document's first node in a batch's numbering, set by
/// the batch's document table (`DocTable`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct DocView {
    /// The batch's slab binding the document is in (`DocTable::slabs`).
    pub(in crate::code_rule_dsl) slab: u32,
    pub(in crate::code_rule_dsl) nodes: u32,
    pub(in crate::code_rule_dsl) node_base: u32,
    /// `size`'s, `off`'s and `tail`'s widths in bytes, at bits 0, 4 and 8; bit 12: tails measured
    /// from the node's own start.
    pub(in crate::code_rule_dsl) widths: u32,
    pub(in crate::code_rule_dsl) code: u32,
    pub(in crate::code_rule_dsl) size: u32,
    pub(in crate::code_rule_dsl) base: u32,
    pub(in crate::code_rule_dsl) off: u32,
    pub(in crate::code_rule_dsl) tail: u32,
    /// Each node's parent (u32, `u32::MAX` at the root).
    pub(in crate::code_rule_dsl) parent: u32,
    /// The fixed-width codes (`DeviceCode`).
    pub(in crate::code_rule_dsl) codes: u32,
    pub(in crate::code_rule_dsl) source: u32,
    pub(in crate::code_rule_dsl) source_len: u32,
    /// Line starts (u32, document-local bytes), the first 0.
    pub(in crate::code_rule_dsl) lines: u32,
    pub(in crate::code_rule_dsl) line_count: u32,
    /// The code table's length.
    pub(in crate::code_rule_dsl) code_count: u32,
    /// Each node's first entry in `kids` (u32, `nodes + 1` of them), and the children of every
    /// node in node order (u32, document-local; `kid_count` of them, a tree's `nodes - 1`): the
    /// child lists a reader builds at admission, for the interpreter's child by index.
    pub(in crate::code_rule_dsl) kid_start: u32,
    pub(in crate::code_rule_dsl) kids: u32,
    pub(in crate::code_rule_dsl) kid_count: u32,
    /// The document's place in a batch's numbering of children, source bytes and lines, as a
    /// packed batch concatenates them (set by `DocTable`).
    pub(in crate::code_rule_dsl) child_base: u32,
    pub(in crate::code_rule_dsl) source_base: u32,
    pub(in crate::code_rule_dsl) line_base: u32,
    /// Each code's first entry in `by_code` (u32, `code_count + 1` of them), and the nodes of
    /// every code in node order (u32, document-local, `nodes` of them): a pass's roots in this
    /// document are the nodes of the codes its rule admits (`label_passes::Roots`), read in place.
    pub(in crate::code_rule_dsl) code_start: u32,
    pub(in crate::code_rule_dsl) by_code: u32,
}
const _: () = assert!(std::mem::size_of::<DocView>() == 96);

/// A slab: its buffer and identity. Its bump position is the residency's (`State`).
pub(in crate::code_rule_dsl) struct Slab {
    pub(in crate::code_rule_dsl) id: u64,
    pub(in crate::code_rule_dsl) buffer: MappedBuffer,
}

/// A resident document: its slab (held: a lease) and where its parts are.
pub(in crate::code_rule_dsl) struct Resident {
    pub(in crate::code_rule_dsl) digest: [u8; 32],
    pub(in crate::code_rule_dsl) slab: Arc<Slab>,
    /// Its view with `slab` and `node_base` 0.
    pub(in crate::code_rule_dsl) view: DocView,
    /// Bytes per pool: `tape.resident`, `source.resident`, `tape.derived`.
    pub(in crate::code_rule_dsl) bytes: [usize; 3],
    /// The tape image's length, as stored.
    pub(in crate::code_rule_dsl) tape_length: usize,
}

impl Resident {
    /// The bytes at `offset..offset + length` of its slab.
    pub(in crate::code_rule_dsl) fn bytes_at(&self, offset: u32, length: usize) -> &[u8] {
        let slab = &self.slab.buffer;
        assert!(offset as usize + length <= slab.byte_length(), "a resident part inside its slab");
        // SAFETY: admission wrote these bytes before publishing the document, and nothing writes
        // them again: a slab is append-only and the document's range is its own.
        unsafe { std::slice::from_raw_parts(slab.host().add(offset as usize), length) }
    }

    /// The fixed-width code table (`DeviceCode`), as the device reads it.
    pub(in crate::code_rule_dsl) fn codes(&self) -> &[DeviceCode] {
        let bytes = self.bytes_at(self.view.codes, 24 * self.view.code_count as usize);
        // SAFETY: admission wrote `code_count` `DeviceCode`s at this 16-byte offset.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<DeviceCode>(), self.view.code_count as usize) }
    }

    /// Each code's first entry in the nodes by code, `code_count + 1` of them.
    pub(in crate::code_rule_dsl) fn code_starts(&self) -> &[u32] {
        let length = self.view.code_count as usize + 1;
        let bytes = self.bytes_at(self.view.code_start, 4 * length);
        // SAFETY: as `codes`, u32s at a 16-byte offset.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u32>(), length) }
    }

    /// The tape image as stored.
    pub(in crate::code_rule_dsl) fn image(&self) -> &[u8] {
        self.bytes_at(self.view.code - resident::HEADER_LEN as u32, self.tape_length)
    }
}

/// A resident source: a file's bytes at a 16-byte offset of its slab (at least 16 bytes of room,
/// so a 16-byte read of an empty file stays inside it).
pub(in crate::code_rule_dsl) struct ResidentSource {
    pub(in crate::code_rule_dsl) digest: [u8; 32],
    pub(in crate::code_rule_dsl) slab: Arc<Slab>,
    pub(in crate::code_rule_dsl) offset: usize,
    pub(in crate::code_rule_dsl) len: usize,
}

impl ResidentSource {
    /// Its bytes, as the device reads them.
    pub(in crate::code_rule_dsl) fn bytes(&self) -> &[u8] {
        assert!(self.offset + self.len <= self.slab.buffer.byte_length(), "a resident source inside its slab");
        // SAFETY: admission wrote these bytes before publishing the source, and nothing writes
        // them again: a slab is append-only and the source's range is its own.
        unsafe { std::slice::from_raw_parts(self.slab.buffer.host().add(self.offset), self.len) }
    }
}

/// The slabs a batch of sources binds, at most `MAX_SLABS`, and each source's binding; `None`
/// when they span more (the caller splits the batch).
pub(in crate::code_rule_dsl) fn source_slabs(sources: &[Arc<ResidentSource>]) -> Option<(Vec<Arc<Slab>>, Vec<u32>)> {
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
pub(in crate::code_rule_dsl) struct DocTable {
    pub(in crate::code_rule_dsl) views: Vec<DocView>,
    pub(in crate::code_rule_dsl) slabs: Vec<Arc<Slab>>,
    pub(in crate::code_rule_dsl) nodes: usize,
    /// The documents, held for the batch's life.
    pub(in crate::code_rule_dsl) documents: Vec<Arc<Resident>>,
}

/// The slabs a batch binds (PIAT §20.4.5, Q-R3): kernels take this many slab buffers; a batch
/// over more is split by the wave.
pub(in crate::code_rule_dsl) const MAX_SLABS: usize = 4;

impl DocTable {
    /// The table of `documents`, in order, or `None` when they span more than `MAX_SLABS` slabs.
    pub(in crate::code_rule_dsl) fn new(documents: Vec<Arc<Resident>>) -> Option<Self> {
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
pub(in crate::code_rule_dsl) struct Held {
    pub(in crate::code_rule_dsl) slabs: usize,
    pub(in crate::code_rule_dsl) documents: usize,
    pub(in crate::code_rule_dsl) sources: usize,
    pub(in crate::code_rule_dsl) pools: [usize; 3],
}

pub(in crate::code_rule_dsl) fn held() -> Held {
    let state = STATE.0.lock().expect("residency poisoned");
    Held { slabs: state.slabs.len(), documents: state.documents.len(), sources: state.sources.len(), pools: state.pools }
}

/// The residency's limit: its share of the Metal buffer budget.
fn limit() -> usize {
    crate::code_rule_dsl::transducer::packed::metal_backend::buffer_limit() / RESIDENT_SHARE.1 * RESIDENT_SHARE.0
}

fn invalid(message: impl std::fmt::Display) -> Error {
    eprintln!("residency: {message}");
    crate::code_rule_dsl::prof::count("residency.corrupt", 1);
    Error::BackendFailure
}

/// Sizes of a document's parts, from its frames' raw lengths, the image's header and the source's
/// newlines (read by one streamed decode that keeps nothing).
struct Plan {
    tape: usize,
    source: usize,
    nodes: usize,
    codes: usize,
    lines: usize,
}

impl Plan {
    fn bytes(&self) -> usize {
        aligned(self.tape) + aligned(self.source) + self.derived()
    }

    /// The reader's state: parents, codes, line starts, child lists and nodes by code.
    fn derived(&self) -> usize {
        aligned(4 * self.nodes)
            + aligned(24 * self.codes)
            + aligned(4 * self.lines)
            + aligned(4 * (self.nodes + 1))
            + aligned(4 * self.nodes.saturating_sub(1))
            + aligned(4 * (self.codes + 1))
            + aligned(4 * self.nodes)
    }
}

fn plan(frames: &crate::content_store::ResidentFrames) -> Result<Plan, Error> {
    let mut header = [0u8; resident::HEADER_LEN];
    zstd::stream::read::Decoder::new(&frames.tape_frame[..])
        .and_then(|mut decoder| decoder.read_exact(&mut header))
        .map_err(|error| invalid(format!("tape frame header: {error}")))?;
    let (nodes, codes) = resident::header_counts(&header).map_err(invalid)?;
    let mut decoder = zstd::stream::read::Decoder::new(&frames.source_frame[..]).map_err(|error| invalid(format!("source frame: {error}")))?;
    let mut lines = 1usize;
    let mut chunk = [0u8; 1 << 16];
    loop {
        let read = decoder.read(&mut chunk).map_err(|error| invalid(format!("source frame: {error}")))?;
        if read == 0 {
            break;
        }
        lines += chunk[..read].iter().filter(|&&byte| byte == b'\n').count();
    }
    Ok(Plan { tape: frames.tape_length, source: frames.source_length, nodes, codes, lines })
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
                        crate::code_rule_dsl::prof::count("residency.slabs", 1);
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
        crate::code_rule_dsl::prof::count("residency.retired", 1);
        true
    }
}

/// The documents of the tapes `digests` (SHA-256), resident, in order: hits from the residency,
/// misses admitted from `store` in one read. `grammar` gives each tape's grammar by
/// `tape_objects.grammar_id`. A digest the store has no tape for is an error.
pub(in crate::code_rule_dsl) fn admit(
    store: &ContentStore,
    digests: &[[u8; 32]],
    grammar: &dyn Fn(i64) -> Result<Arc<Grammar>, String>,
) -> Result<Vec<Arc<Resident>>, Error> {
    let missing = misses(digests)?;
    if !missing.is_empty() {
        let frames = crate::code_rule_dsl::prof::time("residency.read", || store.resident_frames(&missing)).map_err(invalid)?;
        let mut planned = Vec::with_capacity(missing.len());
        for (digest, frames) in missing.iter().zip(&frames) {
            let frames = frames.as_ref().ok_or_else(|| invalid("a tape the store does not hold"))?;
            planned.push((*digest, plan(frames)?, grammar(frames.grammar_id).map_err(invalid)?, frames));
        }
        insert(digests, planned.iter().map(|(digest, plan, grammar, frames)| (*digest, plan, &**grammar, Parts::Frames(frames))).collect())?;
    }
    resident(digests)
}

/// The documents of in-memory tapes, resident, in order: hits from the residency, misses copied
/// from each document's image and source (a batch whose tapes were restored before it ran).
pub(in crate::code_rule_dsl) fn admit_docs(docs: &[crate::code_rule_dsl::tape::TapeDoc]) -> Result<Vec<Arc<Resident>>, Error> {
    let digests = docs.iter().map(|doc| doc.tape_digest().map_err(invalid)).collect::<Result<Vec<_>, _>>()?;
    let missing = misses(&digests)?;
    if !missing.is_empty() {
        let mut owned = Vec::with_capacity(missing.len());
        for digest in &missing {
            let doc = &docs[digests.iter().position(|candidate| candidate == digest).expect("a miss is one of the digests")];
            let (image, source, grammar) = doc.resident_parts().map_err(invalid)?;
            if image.len() < resident::HEADER_LEN {
                return Err(invalid("a tape image shorter than its header"));
            }
            let (nodes, codes) = resident::header_counts(&image[..resident::HEADER_LEN]).map_err(invalid)?;
            let lines = 1 + source.bytes().filter(|&byte| byte == b'\n').count();
            let plan = Plan { tape: image.len(), source: source.len(), nodes, codes, lines };
            owned.push((*digest, plan, Arc::clone(grammar), image, source));
        }
        insert(&digests, owned.iter().map(|(digest, plan, grammar, image, source)| (*digest, plan, &**grammar, Parts::Bytes(image, source.as_bytes()))).collect())?;
    }
    resident(&digests)
}

/// The room a resident source takes: its bytes, 16-byte aligned, at least 16.
fn source_room(len: usize) -> usize {
    aligned(len.max(1))
}

/// Files' bytes resident as sources, in order: hits by SHA-256 from the residency, misses copied
/// into slabs once (the one copy a file's bytes take on their way to the device). The slabs of
/// every source asked for are kept while the misses are placed.
pub(in crate::code_rule_dsl) fn admit_sources(files: &[&[u8]]) -> Result<Vec<Arc<ResidentSource>>, Error> {
    let digests: Vec<[u8; 32]> = files.iter().map(|bytes| Sha256::digest(bytes).into()).collect();
    let _admit = crate::code_rule_dsl::prof::Stage::start("residency.sources");
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    state.clock += 1;
    let clock = state.clock;
    let mut keep = Vec::new();
    for digest in &digests {
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
    crate::code_rule_dsl::prof::count("residency.source_hits", files.len() - misses);
    crate::code_rule_dsl::prof::count("residency.source_misses", misses);
    digests.iter().map(|digest| state.sources.get(digest).cloned().ok_or(Error::BackendCapacity)).collect()
}

/// The digests the residency does not hold (each once); the hits' slabs are marked used.
fn misses(digests: &[[u8; 32]]) -> Result<Vec<[u8; 32]>, Error> {
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
    crate::code_rule_dsl::prof::count("residency.hits", digests.len() - missing.len());
    crate::code_rule_dsl::prof::count("residency.misses", missing.len());
    Ok(missing)
}

/// Places the misses' parts, keeping the slabs of every document `digests` reads.
fn insert(digests: &[[u8; 32]], parts: Vec<([u8; 32], &Plan, &Grammar, Parts<'_>)>) -> Result<(), Error> {
    let _admit = crate::code_rule_dsl::prof::Stage::start("residency.admit");
    let mut state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    let mut keep = digests.iter().filter_map(|digest| state.documents.get(digest).map(|document| document.slab.id)).collect::<Vec<_>>();
    for (digest, plan, grammar, parts) in parts {
        let (at, offset) = state.room(plan.bytes(), &keep)?;
        let document = place(&state.slabs[at].slab, offset, &digest, parts, plan, grammar)?;
        let clock = state.clock;
        let entry = &mut state.slabs[at];
        entry.used = offset + plan.bytes();
        entry.last_use = clock;
        keep.push(entry.slab.id);
        for (pool, bytes) in state.pools.iter_mut().zip(document.bytes) {
            *pool += bytes;
        }
        state.documents.insert(digest, Arc::new(document));
    }
    Ok(())
}

fn resident(digests: &[[u8; 32]]) -> Result<Vec<Arc<Resident>>, Error> {
    let state = STATE.0.lock().map_err(|_| Error::BackendFailure)?;
    digests.iter().map(|digest| state.documents.get(digest).cloned().ok_or(Error::BackendCapacity)).collect()
}

/// Writes a document's parts into `slab` at `offset` (`plan.bytes()` of room), checked.
/// Where a document's bytes come from: a store's frames, decompressed into the slab, or an image
/// and source already in memory, copied.
enum Parts<'a> {
    Frames(&'a crate::content_store::ResidentFrames),
    Bytes(&'a [u8], &'a [u8]),
}

fn place(
    slab: &Arc<Slab>,
    offset: usize,
    digest: &[u8; 32],
    parts: Parts<'_>,
    plan: &Plan,
    grammar: &Grammar,
) -> Result<Resident, Error> {
    let base = slab.buffer.host();
    // SAFETY: `room` gave this range to this document alone, inside the slab's mapping.
    let region = |at: usize, length: usize| unsafe { std::slice::from_raw_parts_mut(base.add(at), length) };
    let tape_at = offset;
    let source_at = tape_at + aligned(plan.tape);
    let parent_at = source_at + aligned(plan.source);
    let codes_at = parent_at + aligned(4 * plan.nodes);
    let lines_at = codes_at + aligned(24 * plan.codes);
    let kid_start_at = lines_at + aligned(4 * plan.lines);
    let kids_at = kid_start_at + aligned(4 * (plan.nodes + 1));
    let code_start_at = kids_at + aligned(4 * plan.nodes.saturating_sub(1));
    let by_code_at = code_start_at + aligned(4 * (plan.codes + 1));
    let tape = region(tape_at, plan.tape);
    let source = region(source_at, plan.source);
    match parts {
        // Decompressed straight into the slab, to the frames' raw lengths exactly.
        Parts::Frames(frames) => {
            if zstd::bulk::decompress_to_buffer(&frames.tape_frame, tape).map_err(|error| invalid(format!("tape frame: {error}")))? != plan.tape {
                return Err(invalid("tape frame length"));
            }
            if zstd::bulk::decompress_to_buffer(&frames.source_frame, source).map_err(|error| invalid(format!("source frame: {error}")))? != plan.source {
                return Err(invalid("source frame length"));
            }
            if <[u8; 32]>::from(Sha256::digest(&*source)) != frames.source_sha256 {
                return Err(invalid("a resident source's digest"));
            }
        }
        Parts::Bytes(image, text) => {
            tape.copy_from_slice(image);
            source.copy_from_slice(text);
        }
    }
    let tape = &*tape;
    let source = &*source;
    if <[u8; 32]>::from(Sha256::digest(tape)) != *digest {
        return Err(invalid("a resident tape's digest"));
    }
    let layout = resident::layout(tape).map_err(invalid)?;
    if layout.source_len != source.len()
        || layout.nodes as usize != plan.nodes
        || layout.code_count != plan.codes
        || <[u8; 32]>::from(Sha256::digest(source)) != layout.source_digest
        || std::str::from_utf8(source).is_err()
    {
        return Err(invalid("a tape and its source disagree"));
    }
    // SAFETY: the parent region is `4 * nodes` bytes at a 16-byte offset in the mapping.
    let parents = unsafe { std::slice::from_raw_parts_mut(base.add(parent_at).cast::<u32>(), plan.nodes) };
    resident::parents(tape, &layout, parents).map_err(invalid)?;
    let codes = resident::device_codes(tape, &layout, grammar).map_err(invalid)?;
    // SAFETY: as the parents', `24 * codes` bytes, and `DeviceCode` is 24 bytes, 2-aligned.
    unsafe { std::ptr::copy_nonoverlapping(codes.as_ptr(), base.add(codes_at).cast::<DeviceCode>(), codes.len()) };
    // SAFETY: as the parents', `4 * lines` bytes.
    let lines = unsafe { std::slice::from_raw_parts_mut(base.add(lines_at).cast::<u32>(), plan.lines) };
    lines[0] = 0;
    let mut line = 1;
    for (at, _) in source.iter().enumerate().filter(|(_, byte)| **byte == b'\n') {
        lines[line] = at as u32 + 1;
        line += 1;
    }
    // Child lists from the parents: counts, their prefix, then each node into its parent's list
    // in node order (a parent's children ascend).
    // SAFETY: as the parents', `4 * (nodes + 1)` and `4 * (nodes - 1)` bytes.
    let kid_start = unsafe { std::slice::from_raw_parts_mut(base.add(kid_start_at).cast::<u32>(), plan.nodes + 1) };
    let kids = unsafe { std::slice::from_raw_parts_mut(base.add(kids_at).cast::<u32>(), plan.nodes.saturating_sub(1)) };
    kid_start.fill(0);
    let mut kid_count = 0usize;
    for &parent in parents.iter() {
        if parent != u32::MAX {
            kid_start[parent as usize + 1] += 1;
            kid_count += 1;
        }
    }
    if kid_count != kids.len() {
        return Err(invalid("a tape that is not one tree"));
    }
    for node in 0..plan.nodes {
        kid_start[node + 1] += kid_start[node];
    }
    let mut fill = kid_start[..plan.nodes].to_vec();
    for (node, &parent) in parents.iter().enumerate() {
        if parent != u32::MAX {
            kids[fill[parent as usize] as usize] = node as u32;
            fill[parent as usize] += 1;
        }
    }
    // Nodes by code, as the child lists: counts, their prefix, then each node in node order.
    // SAFETY: as the parents', `4 * (codes + 1)` and `4 * nodes` bytes; the code column is
    // `nodes` u16s at a 16-byte offset in the image.
    let node_codes = unsafe { std::slice::from_raw_parts(tape.as_ptr().add(layout.code).cast::<u16>(), plan.nodes) };
    let code_start = unsafe { std::slice::from_raw_parts_mut(base.add(code_start_at).cast::<u32>(), plan.codes + 1) };
    let by_code = unsafe { std::slice::from_raw_parts_mut(base.add(by_code_at).cast::<u32>(), plan.nodes) };
    code_start.fill(0);
    for &code in node_codes {
        let slot = code_start.get_mut(usize::from(code) + 1).ok_or_else(|| invalid("a node's code past the table"))?;
        *slot += 1;
    }
    for code in 0..plan.codes {
        code_start[code + 1] += code_start[code];
    }
    let mut fill = code_start[..plan.codes].to_vec();
    for (node, &code) in node_codes.iter().enumerate() {
        by_code[fill[usize::from(code)] as usize] = node as u32;
        fill[usize::from(code)] += 1;
    }
    let widths = u32::from(layout.widths[0]) | u32::from(layout.widths[1]) << 4 | u32::from(layout.widths[2]) << 8 | u32::from(layout.from_start) << 12;
    let at = |offset: usize| u32::try_from(offset).map_err(|_| invalid("a slab offset past 4 GiB"));
    let view = DocView {
        slab: 0,
        nodes: layout.nodes,
        node_base: 0,
        widths,
        code: at(tape_at + layout.code)?,
        size: at(tape_at + layout.size)?,
        base: at(tape_at + layout.base)?,
        off: at(tape_at + layout.off)?,
        tail: at(tape_at + layout.tail)?,
        parent: at(parent_at)?,
        codes: at(codes_at)?,
        source: at(source_at)?,
        source_len: at(plan.source)?,
        lines: at(lines_at)?,
        line_count: at(plan.lines)?,
        code_count: at(plan.codes)?,
        kid_start: at(kid_start_at)?,
        kids: at(kids_at)?,
        kid_count: at(kid_count)?,
        child_base: 0,
        source_base: 0,
        line_base: 0,
        code_start: at(code_start_at)?,
        by_code: at(by_code_at)?,
    };
    crate::code_rule_dsl::prof::count("residency.decompressed_kb", (plan.tape + plan.source) >> 10);
    Ok(Resident {
        digest: *digest,
        slab: Arc::clone(slab),
        view,
        bytes: [aligned(plan.tape), aligned(plan.source), plan.derived()],
        tape_length: plan.tape,
    })
}

/// Releases everything resident: documents, slabs (as their last batch lets go) and compiled
/// pipelines.
pub(in crate::code_rule_dsl) fn drop_all() {
    if let Ok(mut state) = STATE.0.lock() {
        state.documents.clear();
        state.sources.clear();
        state.slabs.clear();
        state.pools = [0; 3];
        state.deadline = None;
    }
    crate::code_rule_dsl::transducer::packed::metal_backend::drop_pipelines();
    crate::code_rule_dsl::transducer::packed::metal_blocks::drop_all();
    crate::code_rule_dsl::transducer::packed::drop_label_pipelines();
    crate::code_rule_dsl::prof::count("residency.dropped", 1);
}

/// A full execution ended: everything resident is dropped `DROP_AFTER` from now, unless another
/// ends first (which moves the deadline).
pub(in crate::code_rule_dsl) fn executed() {
    static DROPPER: std::sync::Once = std::sync::Once::new();
    let (lock, wake) = &*STATE;
    if let Ok(mut state) = lock.lock() {
        state.deadline = Some(Instant::now() + DROP_AFTER);
    }
    wake.notify_all();
    DROPPER.call_once(|| {
        std::thread::Builder::new()
            .name("rerail-residency-drop".into())
            .spawn(|| {
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
mod tests;
