//! The slab residency's documents (decision 0032): stored tapes placed in the Metal crate's slabs
//! (`rerail_metal::slabs`: the arena, its budget, retirement, the sources and the drop). Here:
//! reading the misses' frames in one store read (`ContentStore::resident_frames`), decompressing
//! each straight into its slab (a 16z tape as stored), the reader's derived parts (format 15's
//! codes at a fixed width and line starts; format 16's and 16z's codes are its language's table, once a
//! slab, and its lines its tape's; nothing per node), and the checks of the
//! bytes against the tape's and the source's SHA-256 and of the image's header against the
//! source, once. A failed check is an error, never an empty document.

use crate::code_rule_dsl::tape::Grammar;
use crate::code_rule_dsl::tape::resident::{self, DeviceCode};
use crate::code_rule_dsl::transducer::packed::Error;
use crate::content_store::ContentStore;
#[allow(unused_imports)]
pub use rerail_metal::slabs::*;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// A resident document's parts that are the engine's types: its code table and its image.
pub trait ResidentParts {
    fn resident(&self) -> &Resident;

    /// The fixed-width code table (`DeviceCode`), as the device reads it.
    fn codes(&self) -> &[DeviceCode] {
        let bytes = self.resident().code_bytes();
        // SAFETY: admission wrote `code_count` `DeviceCode`s at this 16-byte offset.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<DeviceCode>(), self.resident().view.code_count as usize) }
    }

    /// The tape where it is placed, as stored.
    fn image(&self) -> &[u8] {
        self.resident().bytes_at(self.resident().view.tape, self.resident().tape_length)
    }
}

impl ResidentParts for Resident {
    fn resident(&self) -> &Resident {
        self
    }
}

fn invalid(message: impl std::fmt::Display) -> Error {
    eprintln!("residency: {message}");
    crate::code_rule_dsl::prof::count("residency.corrupt", 1);
    Error::BackendFailure
}

/// Sizes of a document's parts, from its frames' raw lengths, the image's header and (format 15) the
/// source's newlines, read by one streamed decode that keeps nothing. Format 16 holds its newline index
/// (decision 0336 §6): no line is counted for it. A 16z tape is placed as stored, and read through a
/// cache of its decoded blocks (decision 0353 step 4b).
struct Plan {
    tape: usize,
    source: usize,
    nodes: usize,
    codes: usize,
    lines: usize,
    stored: resident::Stored,
    /// Format 16's and 16z's code table: its language's, once a slab (decision 0336 §7).
    table: Option<Table>,
}

impl Plan {
    fn bytes(&self) -> usize {
        aligned(self.tape) + aligned(self.source) + self.derived()
    }

    /// The reader's state beside the tape: format 15's codes at a fixed width and line starts. Format 16's are its
    /// language's table (the slab's) and its tape's newline index. Nothing per node.
    fn derived(&self) -> usize {
        if self.table.is_some() { 0 } else { aligned(24 * self.codes) + aligned(4 * self.lines) }
    }
}

/// The key of a format 16 tape's language table in a slab: its tag and its table's revision.
fn table_of(head: &[u8], codes: usize) -> Result<Table, Error> {
    let (tag, revision) = (*head.get(8).ok_or_else(|| invalid("a short format 16 header"))?, head[9]);
    Ok(Table { key: u32::from(tag) | u32::from(revision) << 8, len: 24 * codes })
}

/// The plan of an image whose first bytes (at least a format 15 header's, or the whole image) are
/// `head`, `tape` bytes long as stored.
fn planned(head: &[u8], tape: usize, source: usize, lines: usize) -> Result<Plan, Error> {
    let stored = resident::stored(head).map_err(invalid)?;
    // Every format's header states its source's length at the same place: the tape's reads hold every span to it,
    // so it must be the source the document holds (decision 0463).
    const AT: usize = crate::code_rule_dsl::tape::header::SOURCE_LEN;
    let stated = head.get(AT..AT + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
    if stated != Some(source as u64) {
        return Err(invalid(format!("a tape whose header states a source of {stated:?} bytes, held with one of {source}")));
    }
    let (nodes, codes) = match stored {
        resident::Stored::F15 => resident::header_counts(head).map_err(invalid)?,
        // Format 16's and 16z's headers agree on where the node count is.
        resident::Stored::F16 | resident::Stored::F16Z => {
            let nodes = head.get(24..32).ok_or_else(|| invalid("a short format 16 header"))?;
            let nodes = usize::try_from(u64::from_le_bytes(nodes.try_into().expect("8 bytes"))).map_err(invalid)?;
            (nodes, resident::lang_of(head).map_err(invalid)?.table().len())
        }
    };
    let table = if stored == resident::Stored::F15 { None } else { Some(table_of(head, codes)?) };
    Ok(Plan { tape, source, nodes, codes, lines, stored, table })
}

fn plan(frames: &crate::content_store::ResidentFrames) -> Result<Plan, Error> {
    let head = frames.tape_frame.head(resident::HEADER_LEN.min(frames.tape_length));
    let stored = resident::stored(head).map_err(invalid)?;
    let mut lines = 0usize;
    if stored == resident::Stored::F15 {
        lines = 1 + frames.source_frame.segment().bytes().iter().filter(|&&byte| byte == b'\n').count();
    }
    planned(head, frames.tape_length, frames.source_length, lines)
}

/// The documents of the tapes `digests` (SHA-256), resident, in order: hits from the residency,
/// misses admitted from `store` in one read. `grammar` gives each tape's grammar by
/// `tape_objects.grammar_id`. A digest the store has no tape for is an error.
pub fn admit(
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
        insert_parts(digests, planned.iter().map(|(digest, plan, grammar, frames)| (*digest, plan, &**grammar, Parts::Frames(frames))).collect())?;
    }
    resident(digests)
}

/// The documents of in-memory tapes, resident, in order: hits from the residency, misses copied
/// from each document's image and source (a batch whose tapes were restored before it ran).
pub fn admit_docs(docs: &[crate::code_rule_dsl::tape::TapeDoc]) -> Result<Vec<Arc<Resident>>, Error> {
    // A digest not known yet serializes the image: a miss's admission copies those bytes.
    let (digests, mut images): (Vec<_>, Vec<_>) = docs.iter().map(|doc| doc.tape_digest_and_image().map_err(invalid)).collect::<Result<Vec<_>, _>>()?.into_iter().unzip();
    let missing = misses(&digests)?;
    if !missing.is_empty() {
        let mut owned = Vec::with_capacity(missing.len());
        for digest in &missing {
            let at = digests.iter().position(|candidate| candidate == digest).expect("a miss is one of the digests");
            let (image, source, grammar) = docs[at].resident_parts(images[at].take()).map_err(invalid)?;
            let stored = resident::stored(&image).map_err(invalid)?;
            let lines = if stored == resident::Stored::F15 { 1 + source.bytes().filter(|&byte| byte == b'\n').count() } else { 0 };
            let plan = planned(&image, image.len(), source.len(), lines)?;
            owned.push((*digest, plan, Arc::clone(grammar), image, source));
        }
        // The hits' images, serialized for their digests, are not copied.
        drop(images);
        insert_parts(&digests, owned.iter().map(|(digest, plan, grammar, image, source)| (*digest, plan, &**grammar, Parts::Bytes(image, source.as_bytes()))).collect())?;
    }
    resident(&digests)
}

/// Places the misses' parts, keeping the slabs of every document `digests` reads.
fn insert_parts(digests: &[[u8; 32]], parts: Vec<([u8; 32], &Plan, &Grammar, Parts<'_>)>) -> Result<(), Error> {
    let placements = parts
        .into_iter()
        .map(|(digest, plan, grammar, parts)| -> Placement<'_> {
            (digest, plan.bytes(), plan.table, Box::new(move |slab: &Arc<Slab>, offset: usize, table: Option<TableAt>| place(slab, offset, table, &digest, parts, plan, grammar)))
        })
        .collect();
    insert(digests, placements)
}

/// Writes a document's parts into `slab` at `offset` (`plan.bytes()` of room), checked.
/// Where a document's bytes come from: a store's frames, decompressed into the slab, or an image
/// and source already in memory, copied.
#[derive(Clone, Copy)]
enum Parts<'a> {
    Frames(&'a crate::content_store::ResidentFrames),
    Bytes(&'a [u8], &'a [u8]),
}

/// A format 16 document's language table is the slab's (`table`): written here where the slab does not hold it yet.
fn place(
    slab: &Arc<Slab>,
    offset: usize,
    table: Option<TableAt>,
    digest: &[u8; 32],
    parts: Parts<'_>,
    plan: &Plan,
    grammar: &Grammar,
) -> Result<Placed, Error> {
    let base = slab.host();
    // SAFETY: `room` gave this range to this document alone, inside the slab's mapping.
    let region = |at: usize, length: usize| unsafe { std::slice::from_raw_parts_mut(base.add(at), length) };
    let tape_at = offset;
    let source_at = tape_at + aligned(plan.tape);
    let codes_at = source_at + aligned(plan.source);
    let lines_at = codes_at + aligned(24 * plan.codes);
    let tape = region(tape_at, plan.tape);
    let source = region(source_at, plan.source);
    match parts {
        // Decompressed straight into the slab, to the frames' raw lengths exactly (a 16z tape as stored), then
        // checked: the store's bytes are the tape's and the source's whose digests the store holds.
        Parts::Frames(frames) => {
            frames.tape_frame.copy_into(tape).map_err(|error| invalid(format!("tape frame: {error}")))?;
            frames.source_frame.copy_into(source).map_err(|error| invalid(format!("source frame: {error}")))?;
            let source_sha256: [u8; 32] = Sha256::digest(&*source).into();
            if source_sha256 != frames.source_sha256 {
                return Err(invalid("a resident source's digest"));
            }
            // The key is the tape's identity: its bytes as stored and its source's (`HashKey::tape_identity`).
            if crate::code_rule_dsl::tape::tape_identity(&source_sha256, tape) != *digest {
                return Err(invalid("a resident tape's digest"));
            }
        }
        // A document in memory: its digest was made from these same bytes (`TapeDoc::tape_digest`), so
        // there is nothing read to check them against.
        // copy (0353 step 7): an in-memory tape and its source copied into a slab (row 13). It goes: a parse writes
        // its tape into its room (step 4a), and the residency takes that room as it lies.
        Parts::Bytes(image, text) => {
            const INTO_SLAB: rerail_prof::copies::Site = rerail_prof::copies::Site::new(13, "7", "an in-memory tape and its source copied into a slab (residency::place)");
            INTO_SLAB.time(|| {
                // copy (0353 step 7): the tape (row 13).
                tape.copy_from_slice(image);
                // copy (0353 step 7): its source (row 13).
                source.copy_from_slice(text);
                ((), image.len() + text.len())
            });
        }
    }
    let tape = &*tape;
    let source = &*source;
    if std::str::from_utf8(source).is_err() {
        return Err(invalid("a resident source that is not UTF-8"));
    }
    let count = |n: usize| u32::try_from(n).map_err(|_| invalid("a count past u32"));
    // Parts as offsets from the document's start (`at`, decision 0289 §4): a copy into another block moves `at`
    // alone.
    let at = |part: usize| u32::try_from(part - offset).map_err(|_| invalid("a part past 4 GiB of its document"));
    let mut view = DocView {
        at: u32::try_from(offset).map_err(|_| invalid("a slab offset past 4 GiB"))?,
        nodes: count(plan.nodes)?,
        node_base: 0,
        tape: at(tape_at)?,
        ..DocView::default()
    };
    let mut new_table = None;
    match plan.stored {
        resident::Stored::F15 => {
            let layout = resident::layout(tape).map_err(invalid)?;
            if layout.source_len != source.len() || layout.nodes as usize != plan.nodes || layout.code_count != plan.codes {
                return Err(invalid("a tape and its source disagree"));
            }
            view.format = 15;
            view.widths = u32::from(layout.widths[0]) | u32::from(layout.widths[1]) << 4 | u32::from(layout.widths[2]) << 8 | u32::from(layout.from_start) << 12;
            view.columns = [at(tape_at + layout.code)?, at(tape_at + layout.size)?, at(tape_at + layout.base)?, at(tape_at + layout.off)?, at(tape_at + layout.tail)?];
            let codes = resident::device_codes(tape, &layout, grammar).map_err(invalid)?;
            if codes.len() != plan.codes {
                return Err(invalid("a tape's code table"));
            }
            // SAFETY: `24 * codes` bytes at a 16-byte offset of the mapping, and `DeviceCode` is 24 bytes, 2-aligned.
            unsafe { std::ptr::copy_nonoverlapping(codes.as_ptr(), base.add(codes_at).cast::<DeviceCode>(), codes.len()) };
            view.codes = at(codes_at)?;
            // Format 15's line starts, placed beside it.
            // SAFETY: as the codes', `4 * lines` bytes.
            let lines = unsafe { std::slice::from_raw_parts_mut(base.add(lines_at).cast::<u32>(), plan.lines) };
            lines[0] = 0;
            let mut line = 1;
            for (at, _) in source.iter().enumerate().filter(|(_, byte)| **byte == b'\n') {
                lines[line] = at as u32 + 1;
                line += 1;
            }
            view.lines = at(lines_at)?;
            view.line_count = count(plan.lines)?;
        }
        // A 16z tape as stored (decision 0353 step 4b): its view names its piece index, side lists' part starts,
        // checkpoints, payload and newline index where they lie here, its id, and its entry size; the batch's table
        // gives it a cache (`DocTable`).
        resident::Stored::F16Z => {
            let lang = resident::lang_of(tape).map_err(invalid)?;
            // SAFETY: the room `place` writes, read only while this view is made, inside the slab's mapping.
            let room = InSlab(tape.as_ptr(), tape.len());
            let z = resident::Tape16Z::from_bytes(resident::Home::Placed(Arc::new(room))).map_err(invalid)?;
            if z.len() != plan.nodes || z.len() == 0 || z.source_len() != source.len() as u64 || tape[9] != lang.revision() {
                return Err(invalid("a tape and its source or language disagree"));
            }
            let zv = z.view(lang).map_err(invalid)?;
            let here = |offset: u32| at(tape_at + offset as usize);
            view.format = zv.format;
            view.widths = zv.widths;
            view.columns = [here(zv.columns[0])?, here(zv.columns[1])?, here(zv.columns[2])?, zv.columns[3], 0];
            view.stride = here(zv.stride)?;
            view.side = [0, 0, zv.side[2], zv.side[3]];
            (view.cut, view.per) = (zv.cut, zv.per);
            (view.lines, view.line_count) = (here(zv.lines)?, zv.line_count);
            // The presence after the tape, in its home (decision 0434 §4, step 6): the kernels' `tape_has_kind`.
            view.presence = if zv.presence == 0 { 0 } else { here(zv.presence)? };
            view.codes = slab_table(table, lang, grammar, plan, offset, &mut new_table)?;
        }
        resident::Stored::F16 => {
            let p = resident::placement(tape).map_err(invalid)?;
            let lang = resident::lang_of(tape).map_err(invalid)?;
            let source_len = u64::from_le_bytes(tape[16..24].try_into().expect("8 bytes"));
            let (cut, per, w) = lang.device();
            if p.n != plan.nodes || p.n == 0 || source_len != source.len() as u64 || tape[9] != lang.revision() || p.widths != w {
                return Err(invalid("a tape and its source or language disagree"));
            }
            // The tape's own view (`Placement::view`), its offsets from the document's start: where its blocks,
            // side lists and newline index lie (which answers lines: nothing is counted here).
            let placed = p.view(lang, tape_at - offset).map_err(invalid)?;
            debug_assert_eq!((placed.cut, placed.per), (cut, per));
            view.format = 16;
            (view.widths, view.columns, view.stride, view.side, view.parts) = (placed.widths, placed.columns, placed.stride, placed.side, placed.parts);
            (view.cut, view.per, view.lines, view.line_count) = (placed.cut, placed.per, placed.lines, placed.line_count);
            view.presence = placed.presence;
            view.codes = slab_table(table, lang, grammar, plan, offset, &mut new_table)?;
        }
    }
    view.source = at(source_at)?;
    view.source_len = count(plan.source)?;
    view.code_count = count(plan.codes)?;
    crate::code_rule_dsl::prof::count("residency.decompressed_kb", (plan.tape + plan.source) >> 10);
    let document = Resident {
        digest: *digest,
        slab: Arc::clone(slab),
        view,
        at: offset,
        len: plan.bytes(),
        bytes: [aligned(plan.tape), aligned(plan.source), plan.derived()],
        tape_length: plan.tape,
        table: plan.table,
    };
    Ok(Placed { document, table: new_table })
}

/// A format 16 or 16z document's language table: the slab's (`table`), made into `new_table` where the slab does
/// not hold it yet (decision 0336 §7); its offset from the document's start.
fn slab_table(table: Option<TableAt>, lang: &crate::code_rule_dsl::tape::f16::Lang, grammar: &Grammar, plan: &Plan, offset: usize, new_table: &mut Option<Vec<u8>>) -> Result<u32, Error> {
    let table = table.ok_or_else(|| invalid("a format 16 document without its language's table"))?;
    if !table.held {
        let codes = resident::device_codes_of(lang.table(), grammar).map_err(invalid)?;
        if codes.len() != plan.codes {
            return Err(invalid("a tape's code table"));
        }
        // SAFETY: `DeviceCode` is plain data, 24 bytes each.
        *new_table = Some(unsafe { std::slice::from_raw_parts(codes.as_ptr().cast::<u8>(), 24 * codes.len()) }.to_vec());
    }
    Ok(codes_offset(offset, table.at))
}

/// A room's bytes borrowed while its view is made (`Tape16Z::from_bytes` over them, in place).
#[derive(Debug)]
struct InSlab(*const u8, usize);
// SAFETY: read only, while the slab that holds the bytes is held by the placing thread.
unsafe impl Send for InSlab {}
unsafe impl Sync for InSlab {}
impl resident::Room for InSlab {
    fn bytes(&self) -> &[u8] {
        // SAFETY: as `InSlab`'s maker vouches.
        unsafe { std::slice::from_raw_parts(self.0, self.1) }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Releases everything resident: documents, slabs (as their last batch lets go) and compiled
/// pipelines.
pub fn drop_all() {
    clear();
    crate::code_rule_dsl::transducer::packed::metal_backend::drop_pipelines();
    rerail_metal::pool::drop_all();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    rerail_accel::gpu::shared::drop_idle();
    if let Some(device) = crate::code_rule_dsl::transducer::packed::device::matcher() {
        device.drop_label_pipelines();
    }
    crate::code_rule_dsl::prof::count("residency.dropped", 1);
}

/// A full execution ended: everything resident is dropped `DROP_AFTER` from now, unless another
/// ends first (which moves the deadline).
pub fn executed() {
    rerail_metal::slabs::executed(drop_all);
}

// Its tests run in the engine's test binary (its facade of this module mounts `slabs/tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).
