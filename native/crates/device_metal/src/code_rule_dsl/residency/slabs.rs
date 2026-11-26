//! The slab residency's documents (decision 0032): stored tapes placed in the Metal crate's slabs
//! (`rerail_metal::slabs`: the arena, its budget, retirement, the sources and the drop). Here:
//! reading the misses' frames in one store read (`ContentStore::resident_frames`), decompressing
//! each straight into its slab (a 16z tape as stored), the reader's derived parts (none: its codes are its
//! language's table, once a slab, and its lines its tape's; nothing per node), and the checks of the
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

/// Sizes of a document's parts, from its frames' raw lengths and the image's header, read by one streamed decode
/// that keeps nothing. The tape holds its line index (decision 0336 §6), and its code table is its language's, the
/// slab's (decision 0336 §7): nothing is derived beside it. A 16z tape is placed as stored, and read through a cache
/// of its decoded blocks (decision 0353 step 4b).
struct Plan {
    tape: usize,
    source: usize,
    nodes: usize,
    codes: usize,
    stored: resident::Stored,
    /// The code table: its language's, once a slab (decision 0336 §7).
    table: Table,
}

impl Plan {
    fn bytes(&self) -> usize {
        aligned(self.tape) + aligned(self.source)
    }
}

/// The key of a format 16 tape's language table in a slab: its tag and its table's revision.
fn table_of(head: &[u8], codes: usize) -> Result<Table, Error> {
    let (tag, revision) = (*head.get(8).ok_or_else(|| invalid("a short format 16 header"))?, head[9]);
    Ok(Table { key: u32::from(tag) | u32::from(revision) << 8, len: 24 * codes })
}

/// The plan of an image whose first bytes (at least its header's, `resident::HEAD`, or the whole image) are `head`,
/// `tape` bytes long as stored.
fn planned(head: &[u8], tape: usize, source: usize) -> Result<Plan, Error> {
    let stored = resident::stored(head).map_err(invalid)?;
    // Format 16's and 16z's headers state the source's length at the same place: the tape's reads hold every span
    // to it, so it must be the source the document holds (decision 0463).
    const AT: usize = rerail_tape_format::f16::head::SOURCE_LEN;
    let stated = head.get(AT..AT + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
    if stated != Some(source as u64) {
        return Err(invalid(format!("a tape whose header states a source of {stated:?} bytes, held with one of {source}")));
    }
    // Format 16's and 16z's headers agree on where the node count is.
    let nodes = head.get(rerail_tape_format::f16::head::NODES..rerail_tape_format::f16::head::NODES + 8).ok_or_else(|| invalid("a short format 16 header"))?;
    let nodes = usize::try_from(u64::from_le_bytes(nodes.try_into().expect("8 bytes"))).map_err(invalid)?;
    let codes = resident::lang_of(head).map_err(invalid)?.table().len();
    let table = table_of(head, codes)?;
    Ok(Plan { tape, source, nodes, codes, stored, table })
}

fn plan(frames: &crate::content_store::ResidentFrames) -> Result<Plan, Error> {
    let head = frames.tape_frame.head(resident::HEAD.min(frames.tape_length));
    planned(head, frames.tape_length, frames.source_length)
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
            let grammar = grammar(frames.grammar_id).map_err(invalid)?;
            planned.push((*digest, plan(frames)?, grammar, frames));
        }
        insert_parts(digests, planned.iter().map(|(digest, plan, grammar, frames)| (*digest, plan, &**grammar, Parts::Frames(frames))).collect())?;
    }
    resident(digests)
}

/// The documents of in-memory tapes, resident, in order: hits from the residency, misses taken where their tapes and
/// sources lie (decision 0353 step 7, row 13): each in a slab's room (`TapeDoc::in_place`, a document's constructor
/// houses both, `TapeImage::housed` and `placed::housed`), its view naming them there. Nothing is copied: a document
/// held apart (no device room, or format 15) is refused, and read where it is, on the CPU.
pub fn admit_docs(docs: &[crate::code_rule_dsl::tape::TapeDoc]) -> Result<Vec<Arc<Resident>>, Error> {
    let digests = docs.iter().map(|doc| doc.tape_digest_and_image().map(|(digest, _)| digest).map_err(invalid)).collect::<Result<Vec<_>, _>>()?;
    let missing = misses(&digests)?;
    if !missing.is_empty() {
        let mut lying = Vec::with_capacity(missing.len());
        for digest in &missing {
            let at = digests.iter().position(|candidate| candidate == digest).expect("a miss is one of the digests");
            let doc = &docs[at];
            let Some((room, stored, source)) = doc.in_place() else {
                crate::code_rule_dsl::prof::count("residency.apart", 1);
                return Err(Error::BackendCapacity);
            };
            let (Some((slab, _, _)), Some(source_room)) = (crate::code_rule_dsl::tape::placed::slab_room(room), source.home().room().and_then(crate::code_rule_dsl::tape::placed::slab_room)) else {
                crate::code_rule_dsl::prof::count("residency.apart", 1);
                return Err(Error::BackendCapacity);
            };
            let plan = planned(stored, stored.len(), source.len())?;
            lying.push((*digest, plan, Arc::clone(doc.grammar()), slab, stored, source_room, source.as_bytes()));
        }
        insert_lying(
            lying
                .iter()
                .map(|(digest, plan, grammar, slab, stored, (source_slab, source_at, _), source)| -> Lying<'_> {
                    let digest = *digest;
                    let place = move |table: TableAt| place_lying(slab, stored, (source_slab, *source_at, source), table, &digest, plan, grammar);
                    (digest, Arc::clone(slab), plan.table, Box::new(place))
                })
                .collect(),
        )?;
    }
    resident(&digests)
}

/// A document taken where its tape and source lie (`admit_docs`): `stored`, the tape as stored in `slab`, and its
/// source in its slab at its offset. Its view names the tape at its own place (the document's first byte) and the
/// source by its offset from there, an i64: in another slab it is the slabs' device addresses apart (0471), as a
/// batch reaches a document in another slab. Its language's table is its slab's (`slab_table`): nothing is derived
/// beside it.
fn place_lying(
    slab: &Arc<Slab>,
    stored: &[u8],
    (source_slab, source_at, source): (&Arc<Slab>, usize, &[u8]),
    table: TableAt,
    digest: &[u8; 32],
    plan: &Plan,
    grammar: &Grammar,
) -> Result<Placed, Error> {
    let offset = (stored.as_ptr() as usize).checked_sub(slab.host() as usize).filter(|&at| at + stored.len() <= slab.byte_length()).ok_or_else(|| invalid("a placed tape outside its slab"))?;
    let source_offset = if source_slab.id == slab.id {
        source_at as i64 - offset as i64
    } else {
        let (Some(from), Some(to)) = (slab.device_buffer().device_address(), source_slab.device_buffer().device_address()) else {
            return Err(Error::BackendCapacity);
        };
        (to as i64).wrapping_sub(from as i64).wrapping_add(source_at as i64).wrapping_sub(offset as i64)
    };
    let mut document = describe(slab, offset, stored, source, source_offset, table, plan, grammar)?;
    document.document.digest = *digest;
    // Its range: the tape (all its view's parts but the source).
    document.document.len = stored.len();
    document.document.source = (source_slab.id != slab.id).then(|| Arc::clone(source_slab));
    // Its tape and source are their rooms' (the tapes' pool): it takes no resident bytes of its own.
    document.document.bytes = [0; 3];
    Ok(document)
}

/// Places the misses' parts, keeping the slabs of every document `digests` reads.
fn insert_parts(digests: &[[u8; 32]], parts: Vec<([u8; 32], &Plan, &Grammar, Parts<'_>)>) -> Result<(), Error> {
    let placements = parts
        .into_iter()
        .map(|(digest, plan, grammar, parts)| -> Placement<'_> {
            (digest, plan.bytes(), plan.table, Box::new(move |slab: &Arc<Slab>, offset: usize, table: TableAt| place(slab, offset, table, &digest, parts, plan, grammar)))
        })
        .collect();
    insert(digests, placements)
}

/// Where a document's bytes come from: a store's frames, decompressed into the slab (a document in memory is taken
/// where it lies, `place_lying`).
#[derive(Clone, Copy)]
enum Parts<'a> {
    Frames(&'a crate::content_store::ResidentFrames),
}

/// A format 16 document's language table is the slab's (`table`): written here where the slab does not hold it yet.
fn place(
    slab: &Arc<Slab>,
    offset: usize,
    table: TableAt,
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
    }
    let source_offset = (source_at - tape_at) as i64;
    let mut placed = describe(slab, offset, tape, source, source_offset, table, plan, grammar)?;
    placed.document.digest = *digest;
    placed.document.bytes = [aligned(plan.tape), aligned(plan.source), 0];
    placed.document.len = plan.bytes();
    Ok(placed)
}

/// A document's view and record, its tape at `offset` of `slab` (its first byte) and its source `source_offset` from
/// there.
#[allow(clippy::too_many_arguments)]
fn describe(
    slab: &Arc<Slab>,
    offset: usize,
    tape: &[u8],
    source: &[u8],
    source_offset: i64,
    table: TableAt,
    plan: &Plan,
    grammar: &Grammar,
) -> Result<Placed, Error> {
    let tape_at = offset;
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
        // A 16z tape as stored (decision 0353 step 4b): its view names its piece index, side lists' part starts,
        // checkpoints, payload and newline index where they lie here, its id, and its entry size; the batch's table
        // gives it a cache (`DocTable`).
        resident::Stored::F16Z => {
            let lang = resident::lang_of(tape).map_err(invalid)?;
            // SAFETY: the room `place` writes, read only while this view is made, inside the slab's mapping.
            let room = InSlab(tape.as_ptr(), tape.len());
            let z = resident::Tape16Z::from_bytes(resident::Home::Placed(Arc::new(room))).map_err(invalid)?;
            if z.len() != plan.nodes || z.is_empty() || z.source_len() != source.len() as u64 || tape[9] != lang.revision() {
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
    view.set_source_offset(source_offset);
    view.source_len = count(plan.source)?;
    view.code_count = count(plan.codes)?;
    crate::code_rule_dsl::prof::count("residency.decompressed_kb", (plan.tape + plan.source) >> 10);
    let document = Resident {
        digest: [0; 32],
        slab: Arc::clone(slab),
        source: None,
        view,
        at: offset,
        len: plan.tape,
        bytes: [0; 3],
        tape_length: plan.tape,
        table: plan.table,
    };
    Ok(Placed { document, table: new_table })
}

/// A format 16 or 16z document's language table: the slab's (`table`), made into `new_table` where the slab does
/// not hold it yet (decision 0336 §7); its offset from the document's start.
fn slab_table(table: TableAt, lang: &crate::code_rule_dsl::tape::f16::Lang, grammar: &Grammar, plan: &Plan, offset: usize, new_table: &mut Option<Vec<u8>>) -> Result<u32, Error> {
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
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
