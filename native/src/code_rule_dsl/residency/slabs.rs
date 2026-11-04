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
    pub(in crate::code_rule_dsl) reserved: u32,
}
const _: () = assert!(std::mem::size_of::<DocView>() == 64);

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

    /// The tape image as stored.
    pub(in crate::code_rule_dsl) fn image(&self) -> &[u8] {
        self.bytes_at(self.view.code - resident::HEADER_LEN as u32, self.tape_length)
    }
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
        for document in &documents {
            let slab = match slabs.iter().position(|slab| slab.id == document.slab.id) {
                Some(at) => at,
                None if slabs.len() < MAX_SLABS => {
                    slabs.push(Arc::clone(&document.slab));
                    slabs.len() - 1
                }
                None => return None,
            };
            views.push(DocView { slab: slab as u32, node_base: u32::try_from(nodes).ok()?, ..document.view });
            nodes += document.view.nodes as usize;
        }
        u32::try_from(nodes).ok()?;
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
    pub(in crate::code_rule_dsl) pools: [usize; 3],
}

pub(in crate::code_rule_dsl) fn held() -> Held {
    let state = STATE.0.lock().expect("residency poisoned");
    Held { slabs: state.slabs.len(), documents: state.documents.len(), pools: state.pools }
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
        aligned(self.tape) + aligned(self.source) + aligned(4 * self.nodes) + aligned(24 * self.codes) + aligned(4 * self.lines)
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
    let (lock, _) = &*STATE;
    let missing = {
        let mut state = lock.lock().map_err(|_| Error::BackendFailure)?;
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
        missing
    };
    if !missing.is_empty() {
        crate::code_rule_dsl::prof::count("residency.misses", missing.len());
        let frames = crate::code_rule_dsl::prof::time("residency.read", || store.resident_frames(&missing)).map_err(invalid)?;
        let _admit = crate::code_rule_dsl::prof::Stage::start("residency.admit");
        let mut state = lock.lock().map_err(|_| Error::BackendFailure)?;
        // The slabs of documents this admission reads stay.
        let mut keep = digests.iter().filter_map(|digest| state.documents.get(digest).map(|document| document.slab.id)).collect::<Vec<_>>();
        for (digest, frames) in missing.iter().zip(frames) {
            let frames = frames.ok_or_else(|| invalid("a tape the store does not hold"))?;
            let grammar = grammar(frames.grammar_id).map_err(invalid)?;
            let plan = plan(&frames)?;
            let (at, offset) = state.room(plan.bytes(), &keep)?;
            let document = place(&state.slabs[at].slab, offset, digest, &frames, &plan, &grammar)?;
            let clock = state.clock;
            let entry = &mut state.slabs[at];
            entry.used = offset + plan.bytes();
            entry.last_use = clock;
            keep.push(entry.slab.id);
            for (pool, bytes) in state.pools.iter_mut().zip(document.bytes) {
                *pool += bytes;
            }
            state.documents.insert(*digest, Arc::new(document));
        }
    }
    let state = lock.lock().map_err(|_| Error::BackendFailure)?;
    digests.iter().map(|digest| state.documents.get(digest).cloned().ok_or(Error::BackendCapacity)).collect()
}

/// Writes a document's parts into `slab` at `offset` (`plan.bytes()` of room), checked.
fn place(
    slab: &Arc<Slab>,
    offset: usize,
    digest: &[u8; 32],
    frames: &crate::content_store::ResidentFrames,
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
    // Decompressed straight into the slab, to the frames' raw lengths exactly.
    let tape = region(tape_at, plan.tape);
    if zstd::bulk::decompress_to_buffer(&frames.tape_frame, tape).map_err(|error| invalid(format!("tape frame: {error}")))? != plan.tape {
        return Err(invalid("tape frame length"));
    }
    let source = region(source_at, plan.source);
    if zstd::bulk::decompress_to_buffer(&frames.source_frame, source).map_err(|error| invalid(format!("source frame: {error}")))? != plan.source {
        return Err(invalid("source frame length"));
    }
    let tape = &*tape;
    let source = &*source;
    if <[u8; 32]>::from(Sha256::digest(tape)) != *digest || <[u8; 32]>::from(Sha256::digest(source)) != frames.source_sha256 {
        return Err(invalid("a resident object's digest"));
    }
    let layout = resident::layout(tape).map_err(invalid)?;
    if layout.source_len != source.len() || layout.nodes as usize != plan.nodes || layout.code_count != plan.codes || std::str::from_utf8(source).is_err() {
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
        reserved: 0,
    };
    crate::code_rule_dsl::prof::count("residency.decompressed_kb", (plan.tape + plan.source) >> 10);
    Ok(Resident {
        digest: *digest,
        slab: Arc::clone(slab),
        view,
        bytes: [aligned(plan.tape), aligned(plan.source), aligned(4 * plan.nodes) + aligned(24 * plan.codes) + aligned(4 * plan.lines)],
        tape_length: plan.tape,
    })
}

/// Releases everything resident: documents, slabs (as their last batch lets go) and compiled
/// pipelines.
pub(in crate::code_rule_dsl) fn drop_all() {
    if let Ok(mut state) = STATE.0.lock() {
        state.documents.clear();
        state.slabs.clear();
        state.pools = [0; 3];
        state.deadline = None;
    }
    crate::code_rule_dsl::transducer::packed::metal_backend::drop_pipelines();
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
