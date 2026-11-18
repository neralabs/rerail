//! The slab residency's documents (decision 0032): stored tapes placed in the Metal crate's slabs
//! (`rerail_metal::slabs`: the arena, its budget, retirement, the sources and the drop). Here:
//! reading the misses' frames in one store read (`ContentStore::resident_frames`), decompressing
//! each straight into its slab (a 16z tape laid out as format 16), the reader's derived parts (format
//! 15's codes at a fixed width and line starts; format 16's codes are its language's table, once a
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

    /// The tape where it is placed: as stored, or a 16z tape laid out as format 16.
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
/// (decision 0336 §6): no line is counted for it. A 16z tape is laid out as format 16
/// where it is placed (its head copied, its pieces decoded on the device: `Decodes`): `tape` is that length, and `z` its stored bytes,
/// read once (a store's frame) to plan and place it.
struct Plan {
    tape: usize,
    source: usize,
    nodes: usize,
    codes: usize,
    lines: usize,
    stored: resident::Stored,
    z: Option<resident::Tape16Z>,
    /// Format 16's code table: its language's, once a slab (decision 0336 §7).
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
/// `head`, `tape` bytes long as stored, and of its 16z tape where it is one (`z`, read whole).
fn planned(head: &[u8], tape: usize, source: usize, lines: usize, z: Option<Vec<u8>>) -> Result<Plan, Error> {
    let stored = resident::stored(head).map_err(invalid)?;
    let (tape, nodes, codes, z) = match stored {
        resident::Stored::F15 => {
            let (nodes, codes) = resident::header_counts(head).map_err(invalid)?;
            (tape, nodes, codes, None)
        }
        resident::Stored::F16 => {
            let nodes = head.get(24..32).ok_or_else(|| invalid("a short format 16 header"))?;
            let nodes = usize::try_from(u64::from_le_bytes(nodes.try_into().expect("8 bytes"))).map_err(invalid)?;
            (tape, nodes, resident::lang_of(head).map_err(invalid)?.table().len(), None)
        }
        resident::Stored::F16Z => {
            let z = resident::Tape16Z::from_bytes(z.ok_or_else(|| invalid("a 16z tape not read whole"))?).map_err(invalid)?;
            (z.decoded_len(), z.len(), resident::lang_of(head).map_err(invalid)?.table().len(), Some(z))
        }
    };
    let table = if stored == resident::Stored::F15 { None } else { Some(table_of(head, codes)?) };
    Ok(Plan { tape, source, nodes, codes, lines, stored, z, table })
}

fn plan(frames: &crate::content_store::ResidentFrames) -> Result<Plan, Error> {
    let head = frames.tape_frame.head(resident::HEADER_LEN.min(frames.tape_length))
        .map_err(|error| invalid(format!("tape frame header: {error}")))?;
    let stored = resident::stored(&head).map_err(invalid)?;
    let mut lines = 0usize;
    if stored == resident::Stored::F15 {
        lines = 1;
        frames.source_frame.chunks(|chunk| {
            lines += chunk.iter().filter(|&&byte| byte == b'\n').count();
        })
        .map_err(|error| invalid(format!("source frame: {error}")))?;
    }
    // A 16z tape is read whole: its block index sizes its format 16 layout, and its blocks are laid out from it.
    let z = if stored == resident::Stored::F16Z {
        let mut whole = vec![0u8; frames.tape_length];
        frames.tape_frame.copy_into(&mut whole).map_err(|error| invalid(format!("tape frame: {error}")))?;
        Some(whole)
    } else {
        None
    };
    planned(&head, frames.tape_length, frames.source_length, lines, z)
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
            let z = (stored == resident::Stored::F16Z).then(|| image.clone());
            let plan = planned(&image, image.len(), source.len(), lines, z)?;
            owned.push((*digest, plan, Arc::clone(grammar), image, source));
        }
        // The hits' images, serialized for their digests, are not copied.
        drop(images);
        insert_parts(&digests, owned.iter().map(|(digest, plan, grammar, image, source)| (*digest, plan, &**grammar, Parts::Bytes(image, source.as_bytes()))).collect())?;
    }
    resident(&digests)
}

/// Places the misses' parts, keeping the slabs of every document `digests` reads. The 16z documents' pieces are
/// decoded on the device, in one command, before any document is published (`Decodes`).
fn insert_parts(digests: &[[u8; 32]], parts: Vec<([u8; 32], &Plan, &Grammar, Parts<'_>)>) -> Result<(), Error> {
    let decodes = std::cell::RefCell::new(Decodes::default());
    let placements = parts
        .into_iter()
        .map(|(digest, plan, grammar, parts)| -> Placement<'_> {
            let decodes = &decodes;
            (digest, plan.bytes(), plan.table, Box::new(move |slab: &Arc<Slab>, offset: usize, table: Option<TableAt>| place(slab, offset, table, &digest, parts, plan, grammar, &mut decodes.borrow_mut())))
        })
        .collect();
    insert(digests, placements, &mut || std::mem::take(&mut *decodes.borrow_mut()).run())
}

/// The device's decode of 16z's pieces (decision 0336 §8; 0156 E26's shape): `slabs/decode16z.metal`.
const DECODE16Z: &str = include_str!("slabs/decode16z.metal");
/// SIMD groups (pieces) a threadgroup.
const DECODE_GROUPS: u64 = 4;

/// One piece's decode as the kernel takes it (`PieceRun` in `decode16z.metal`): its stored bytes in the
/// admission's upload, LZ4 or raw, and where its array's and side list's bytes go in the slab.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PieceRun {
    src: u32,
    src_len: u32,
    lz4: u32,
    fixed: u32,
    fixed_len: u32,
    side: u32,
    side_len: u32,
    pad: u32,
}

/// The 16z documents of one admission, their pieces to decode on the device: their stored bytes, uploaded once,
/// and each slab's pieces.
#[derive(Default)]
struct Decodes {
    stored: Vec<u8>,
    slabs: Vec<(Arc<Slab>, Vec<PieceRun>)>,
}

impl Decodes {
    /// The pieces of `z`, whose format 16 tape lies at `tape_at` in `slab`.
    fn add(&mut self, slab: &Arc<Slab>, tape_at: usize, z: &resident::Tape16Z) -> Result<(), Error> {
        let base = self.stored.len();
        self.stored.extend_from_slice(z.as_bytes());
        self.stored.resize(aligned(self.stored.len()), 0);
        let k = match self.slabs.iter().position(|(held, _)| held.id == slab.id) {
            Some(k) => k,
            None => {
                self.slabs.push((Arc::clone(slab), Vec::new()));
                self.slabs.len() - 1
            }
        };
        let u = |n: usize| u32::try_from(n).map_err(|_| invalid("a 16z piece past 4 GiB"));
        for job in z.piece_jobs() {
            self.slabs[k].1.push(PieceRun {
                src: u(base + job.src.start)?,
                src_len: u(job.src.len())?,
                lz4: u32::from(job.lz4),
                fixed: u(tape_at + job.fixed.start)?,
                fixed_len: u(job.fixed.len())?,
                side: u(tape_at + job.side.start)?,
                side_len: u(job.side.len())?,
                pad: 0,
            });
        }
        Ok(())
    }

    /// Every piece decoded into its slab: one command, a dispatch a slab, waited for. A piece that does not decode
    /// to its length refuses the admission.
    fn run(mut self) -> Result<(), Error> {
        use rerail_metal::launch::{CheckedComputeEncoder as _, LaunchScope, commit_checked, wait_checked};
        // A document of no nodes has no pieces: nothing to dispatch for it.
        self.slabs.retain(|(_, runs)| !runs.is_empty());
        if self.slabs.is_empty() {
            return Ok(());
        }
        let source = format!("#include <metal_stdlib>\nusing namespace metal;\n{DECODE16Z}");
        let pipeline = rerail_metal::runtime::pipeline_named(&source, "decode16z")?;
        let stored = rerail_metal::pool::upload(&self.stored)?;
        let bad = rerail_metal::pool::zeroed::<u32>(1)?;
        let runs = self.slabs.iter().map(|(_, runs)| rerail_metal::pool::upload(runs)).collect::<Result<Vec<_>, _>>()?;
        let _scope = LaunchScope::enter();
        let queue = rerail_metal::queue()?;
        metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            command.set_label("Rerail 16z decode");
            let encoder = command.new_compute_command_encoder();
            encoder.set_label("Rerail 16z decode");
            encoder.set_compute_pipeline_state(&pipeline);
            encoder.set_buffer(1, Some(stored.buffer()), 0);
            encoder.set_buffer(3, Some(bad.buffer()), 0);
            for ((slab, pieces), runs) in self.slabs.iter().zip(&runs) {
                let count = pieces.len() as u32;
                encoder.set_buffer(0, Some(runs.buffer()), 0);
                encoder.set_buffer(2, Some(slab.buffer()), 0);
                encoder.set_bytes(4, 4, std::ptr::from_ref(&count).cast());
                encoder.checked_dispatch_thread_groups(command, metal::MTLSize::new(u64::from(count).div_ceil(DECODE_GROUPS), 1, 1), metal::MTLSize::new(32 * DECODE_GROUPS, 1, 1));
            }
            encoder.end_encoding();
            if let Err(refused) = commit_checked(command) {
                rerail_metal::launch::release_refused(command);
                return Err(refused);
            }
            wait_checked(command)
        })?;
        // SAFETY: the command finished; the flag is a u32 the kernel wrote, or zero.
        if unsafe { *bad.host().cast::<u32>() } != 0 {
            return Err(invalid("a 16z piece that does not decode to its length"));
        }
        crate::code_rule_dsl::prof::count("residency.decoded_16z_pieces", self.slabs.iter().map(|(_, runs)| runs.len()).sum::<usize>());
        Ok(())
    }
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
    decodes: &mut Decodes,
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
        // Decompressed straight into the slab, to the frames' raw lengths exactly (a 16z tape is laid out
        // below, once checked), then checked: the store's bytes are the tape's and the source's whose digests
        // the store holds.
        Parts::Frames(frames) => {
            if plan.z.is_none() {
                frames.tape_frame.copy_into(tape).map_err(|error| invalid(format!("tape frame: {error}")))?;
            }
            frames.source_frame.copy_into(source).map_err(|error| invalid(format!("source frame: {error}")))?;
            let source_sha256: [u8; 32] = Sha256::digest(&*source).into();
            if source_sha256 != frames.source_sha256 {
                return Err(invalid("a resident source's digest"));
            }
            // The key is the tape's identity: its bytes as stored and its source's (`HashKey::tape_identity`).
            let stored = plan.z.as_ref().map_or(&*tape, |z| z.as_bytes());
            if crate::code_rule_dsl::tape::tape_identity(&source_sha256, stored) != *digest {
                return Err(invalid("a resident tape's digest"));
            }
        }
        // A document in memory: its digest was made from these same bytes (`TapeDoc::tape_digest`), so
        // there is nothing read to check them against.
        Parts::Bytes(image, text) => {
            if plan.z.is_none() {
                tape.copy_from_slice(image);
            }
            source.copy_from_slice(text);
        }
    }
    // A 16z tape, checked as stored, is laid out as format 16 where it is placed: its head (header, part starts,
    // checkpoints, newline index; stored as they are) copied here, its pieces decoded on the device before the
    // document is published (`Decodes`). The checks below read the head alone.
    if let Some(z) = &plan.z {
        z.decode_head_into(tape).map_err(invalid)?;
        decodes.add(slab, tape_at, z)?;
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
        resident::Stored::F16 | resident::Stored::F16Z => {
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
            // The language's table: the slab's, made here where the slab does not hold it yet.
            let table = table.ok_or_else(|| invalid("a format 16 document without its language's table"))?;
            if !table.held {
                let codes = resident::device_codes_of(lang.table(), grammar).map_err(invalid)?;
                if codes.len() != plan.codes {
                    return Err(invalid("a tape's code table"));
                }
                // SAFETY: `DeviceCode` is plain data, 24 bytes each.
                new_table = Some(unsafe { std::slice::from_raw_parts(codes.as_ptr().cast::<u8>(), 24 * codes.len()) }.to_vec());
            }
            view.codes = codes_offset(offset, table.at);
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

/// Releases everything resident: documents, slabs (as their last batch lets go) and compiled
/// pipelines.
pub fn drop_all() {
    clear();
    crate::code_rule_dsl::transducer::packed::metal_backend::drop_pipelines();
    rerail_metal::pool::drop_all();
    #[cfg(target_os = "macos")]
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
// engine registers as it loads (the languages, their `.rr` libraries).
