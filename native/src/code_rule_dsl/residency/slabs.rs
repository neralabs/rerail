//! The slab residency's documents (decision 0032): stored tapes placed in the Metal crate's slabs
//! (`rerail_metal::slabs`: the arena, its budget, retirement, the sources and the drop). Here:
//! reading the misses' frames in one store read (`ContentStore::resident_frames`), decompressing
//! each straight into its slab, the reader's derived parts (each node's parent, the image's codes
//! at a fixed width, the source's line starts, child lists, nodes by code), and the checks of the
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
pub(in crate::code_rule_dsl) trait ResidentParts {
    fn resident(&self) -> &Resident;

    /// The fixed-width code table (`DeviceCode`), as the device reads it.
    fn codes(&self) -> &[DeviceCode] {
        let bytes = self.resident().bytes_at(self.resident().view.codes, 24 * self.resident().view.code_count as usize);
        // SAFETY: admission wrote `code_count` `DeviceCode`s at this 16-byte offset.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<DeviceCode>(), self.resident().view.code_count as usize) }
    }

    /// The tape image as stored.
    fn image(&self) -> &[u8] {
        self.resident().bytes_at(self.resident().view.code - resident::HEADER_LEN as u32, self.resident().tape_length)
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
    let header = crate::content_store::decompress_head(&frames.tape_frame, resident::HEADER_LEN)
        .map_err(|error| invalid(format!("tape frame header: {error}")))?;
    let (nodes, codes) = resident::header_counts(&header).map_err(invalid)?;
    let mut lines = 1usize;
    crate::content_store::decompress_chunks(&frames.source_frame, |chunk| {
        lines += chunk.iter().filter(|&&byte| byte == b'\n').count();
    })
    .map_err(|error| invalid(format!("source frame: {error}")))?;
    Ok(Plan { tape: frames.tape_length, source: frames.source_length, nodes, codes, lines })
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
        insert_parts(&digests, owned.iter().map(|(digest, plan, grammar, image, source)| (*digest, plan, &**grammar, Parts::Bytes(image, source.as_bytes()))).collect())?;
    }
    resident(&digests)
}

/// Places the misses' parts, keeping the slabs of every document `digests` reads.
fn insert_parts(digests: &[[u8; 32]], parts: Vec<([u8; 32], &Plan, &Grammar, Parts<'_>)>) -> Result<(), Error> {
    let placements = parts
        .into_iter()
        .map(|(digest, plan, grammar, parts)| -> Placement<'_> {
            (digest, plan.bytes(), Box::new(move |slab: &Arc<Slab>, offset: usize| place(slab, offset, &digest, parts, plan, grammar)))
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
    let source_sha256: [u8; 32] = Sha256::digest(match parts {
        // Decompressed straight into the slab, to the frames' raw lengths exactly.
        Parts::Frames(frames) => {
            crate::content_store::decompress_into(&frames.tape_frame, tape).map_err(|error| invalid(format!("tape frame: {error}")))?;
            crate::content_store::decompress_into(&frames.source_frame, source).map_err(|error| invalid(format!("source frame: {error}")))?;
            &*source
        }
        Parts::Bytes(image, text) => {
            tape.copy_from_slice(image);
            source.copy_from_slice(text);
            &*source
        }
    })
    .into();
    if let Parts::Frames(frames) = parts
        && source_sha256 != frames.source_sha256
    {
        return Err(invalid("a resident source's digest"));
    }
    let tape = &*tape;
    let source = &*source;
    // The key is the tape's identity: its bytes and its source's (`HashKey::tape_identity`).
    if crate::code_rule_dsl::tape::tape_identity(&source_sha256, tape) != *digest {
        return Err(invalid("a resident tape's digest"));
    }
    let layout = resident::layout(tape).map_err(invalid)?;
    if layout.source_len != source.len()
        || layout.nodes as usize != plan.nodes
        || layout.code_count != plan.codes
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

#[cfg(test)]
pub mod tests;
