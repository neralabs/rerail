//! The matcher's anchor scan on Metal (an anchor literal's nodes found on the device). The Metal
//! runtime it runs on (the device, the buffer budget, mapped buffers, pipelines, launches) is the
//! `rerail-metal` crate's, re-exported here.

use super::{EMIT, Error, KIND, Program, TEXT, TextPredicate, split};
use crate::code_rule_dsl::tape::TapeDoc;
#[allow(unused_imports)]
pub use rerail_metal::runtime::*;
pub use rerail_metal::launch;
use launch::{CheckedComputeEncoder, commit_checked, wait_checked};
use metal::{CommandQueue, ComputePipelineState, Device, MTLSize};
use std::collections::BTreeMap;

#[repr(C)]
#[derive(Clone, Copy)]
struct AnchorHit {
    node: u32,
    group: u32,
}

#[derive(Default)]
struct LiteralTrie {
    group: Option<u32>,
    children: BTreeMap<u8, LiteralTrie>,
}

impl LiteralTrie {
    fn insert(&mut self, text: &[u8], group: u32) {
        let mut node = self;
        for &byte in text {
            node = node.children.entry(byte).or_default();
        }
        debug_assert!(node.group.is_none_or(|existing| existing == group));
        node.group = Some(group);
    }

    fn emit(&self, depth: usize, source: &mut String) {
        if let Some(group) = self.group {
            source.push_str(&format!("group={group}u;"));
        }
        if self.children.is_empty() {
            return;
        }
        source.push_str(&format!("switch(source[span.x+{depth}u]){{"));
        for (byte, child) in &self.children {
            source.push_str(&format!("case {byte}u:"));
            child.emit(depth + 1, source);
            source.push_str("break;");
        }
        source.push_str("default:break;}");
    }
}

fn anchor_literal(program: &Program) -> Result<(u16, &[u8], u16, u16), Error> {
    if program.words.len() != 4
        || !program.directives.is_empty()
        || !matches!(split(program.words[0]), (KIND, 0, _))
        || !matches!(split(program.words[1]), (TEXT, 0, _))
        || !matches!(split(program.words[2]), (super::CAPTURE, 0, _))
        || !matches!(split(program.words[3]), (EMIT, 0, _))
    {
        return Err(Error::UnsupportedBackend);
    }
    let predicate = split(program.words[1]).2;
    let TextPredicate::Equal(string) = program.predicates[usize::from(predicate)] else {
        return Err(Error::UnsupportedBackend);
    };
    Ok((
        split(program.words[0]).2,
        &program.strings[usize::from(string)],
        split(program.words[2]).2,
        split(program.words[3]).2,
    ))
}

pub(super) struct PreparedAnchorScan {
    pipeline: ComputePipelineState,
    queue: CommandQueue,
    kinds: MappedBuffer,
    spans: MappedBuffer,
    node_files: MappedBuffer,
    group_enabled: MappedBuffer,
    source: MappedBuffer,
    hits: MappedBuffer,
    hit_count: MappedBuffer,
    locations: Vec<(usize, u32)>,
    group_programs: Vec<Vec<usize>>,
    file_program_masks: Vec<u32>,
    program_words: usize,
    file_count: usize,
    program_count: usize,
}

/// Prepare one specialized GPU dispatch for exact kind/value anchor selection. The generated
/// kernel groups literals by kind and length, then uses byte tries. It appends only positive hits.
pub(super) fn prepare_anchor_scan(
    programs: &[Program],
    files: &[TapeDoc],
) -> Result<PreparedAnchorScan, Error> {
    prepare_anchor_scan_with_eligibility(programs, files, None)
}

/// `file_program_masks` carries file/check eligibility from SQL into the GPU scan. Each file owns
/// `ceil(programs / 32)` words. This representation does not split large program sets into more
/// dispatches.
pub(super) fn prepare_anchor_scan_with_eligibility(
    programs: &[Program],
    files: &[TapeDoc],
    file_program_masks: Option<&[Vec<u32>]>,
) -> Result<PreparedAnchorScan, Error> {
    if file_program_masks.is_some_and(|masks| masks.len() != files.len()) {
        return Err(Error::InvalidAnchor);
    }
    let program_words = programs.len().div_ceil(32);
    if file_program_masks.is_some_and(|masks| masks.iter().any(|mask| mask.len() != program_words))
    {
        return Err(Error::InvalidAnchor);
    }
    if let (Some(masks), Some(last)) = (file_program_masks, programs.len().checked_rem(32))
        && last != 0
        && masks
            .iter()
            .any(|mask| mask.last().is_some_and(|word| word >> last != 0))
    {
        return Err(Error::InvalidAnchor);
    }
    let mut flat_program_masks = Vec::with_capacity(files.len().saturating_mul(program_words));
    for file in 0..files.len() {
        if let Some(masks) = file_program_masks {
            flat_program_masks.extend_from_slice(&masks[file]);
        } else {
            flat_program_masks.extend(std::iter::repeat_n(u32::MAX, program_words));
            if !programs.len().is_multiple_of(32)
                && let Some(last) = flat_program_masks.last_mut()
            {
                *last = (1_u32 << (programs.len() % 32)) - 1;
            }
        }
    }

    let mut literal_groups = BTreeMap::<(u16, Vec<u8>), usize>::new();
    let mut group_programs = Vec::<Vec<usize>>::new();
    for (program_index, program) in programs.iter().enumerate() {
        let (kind, text, _, _) = anchor_literal(program)?;
        let next = literal_groups.len();
        let group = *literal_groups.entry((kind, text.to_vec())).or_insert(next);
        if group == group_programs.len() {
            group_programs.push(Vec::new());
        }
        group_programs[group].push(program_index);
    }
    let group_words = group_programs.len().div_ceil(32);
    let mut group_enabled = vec![0_u32; files.len().saturating_mul(group_words)];
    for file in 0..files.len() {
        for (group, members) in group_programs.iter().enumerate() {
            let allowed = members.iter().any(|program| {
                flat_program_masks[file * program_words + program / 32] & (1_u32 << (program % 32))
                    != 0
            });
            if allowed {
                group_enabled[file * group_words + group / 32] |= 1_u32 << (group % 32);
            }
        }
    }

    let mut tries = BTreeMap::<(u16, usize), LiteralTrie>::new();
    for ((kind, text), group) in literal_groups {
        tries
            .entry((kind, text.len()))
            .or_default()
            .insert(&text, group as u32);
    }
    let mut tests = String::new();
    for ((kind, length), trie) in &tries {
        tests.push_str(&format!("if(kind=={kind}u && span.y-span.x=={length}u){{"));
        trie.emit(0, &mut tests);
        tests.push('}');
    }
    let metal_source = format!(
        r#"#include <metal_stdlib>
using namespace metal;
struct AnchorHit {{ uint node; uint group; }};
kernel void anchors(device const ushort* kinds [[buffer(0)]], device const uint2* spans [[buffer(1)]],
 device const uint* nodeFiles [[buffer(2)]], device const uint* groupEnabled [[buffer(3)]],
 device const uchar* source [[buffer(4)]], device AnchorHit* hits [[buffer(5)]],
 device atomic_uint* hitCount [[buffer(6)]], constant uint& count [[buffer(7)]],
 uint id [[thread_position_in_grid]]) {{
 if(id>=count)return; uint kind=kinds[id]; uint2 span=spans[id]; uint group=0xffffffffu; {tests}
 if(group!=0xffffffffu){{uint file=nodeFiles[id]; uint word=groupEnabled[file*{group_words}u+(group>>5u)];
  if((word&(1u<<(group&31u)))!=0u){{uint slot=atomic_fetch_add_explicit(hitCount,1u,memory_order_relaxed);hits[slot]={{id,group}};}}}}
}}"#
    );
    let pipeline = pipeline_named(&metal_source, "anchors")?;
    let device = rerail_metal::device()?;
    let queue = rerail_metal::queue()?;
    let mut source = Vec::new();
    let mut kinds = Vec::new();
    let mut spans = Vec::new();
    let mut node_files = Vec::new();
    let mut locations = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        let base = u32::try_from(source.len()).map_err(|_| Error::BackendFailure)?;
        source.extend_from_slice(file.transducer_source());
        for ordinal in 0..file.transducer_len() {
            let ordinal = u32::try_from(ordinal).map_err(|_| Error::BackendFailure)?;
            let span = file.transducer_span(ordinal);
            let start = u32::try_from(span.start)
                .ok()
                .and_then(|start| base.checked_add(start))
                .ok_or(Error::BackendFailure)?;
            let end = u32::try_from(span.end)
                .ok()
                .and_then(|end| base.checked_add(end))
                .ok_or(Error::BackendFailure)?;
            kinds.push(file.transducer_kind(ordinal));
            spans.push([start, end]);
            node_files.push(u32::try_from(fi).map_err(|_| Error::BackendFailure)?);
            locations.push((fi, ordinal));
        }
    }
    let kinds = MappedBuffer::from_slice(&device, &kinds)?;
    let spans = MappedBuffer::from_slice(&device, &spans)?;
    let node_files = MappedBuffer::from_slice(&device, &node_files)?;
    let group_enabled = MappedBuffer::from_slice(&device, &group_enabled)?;
    let source = MappedBuffer::new(&device, &source)?;
    let hits = MappedBuffer::zeroed::<AnchorHit>(&device, locations.len())?;
    let hit_count = MappedBuffer::zeroed::<u32>(&device, 1)?;
    Ok(PreparedAnchorScan {
        pipeline,
        queue,
        kinds,
        spans,
        node_files,
        group_enabled,
        source,
        hits,
        hit_count,
        locations,
        group_programs,
        file_program_masks: flat_program_masks,
        program_words,
        file_count: files.len(),
        program_count: programs.len(),
    })
}

type AnchorLists = Vec<Vec<Vec<u32>>>;

pub(super) struct AnchorScanProfile {
    pub dispatch_and_wait: std::time::Duration,
    pub reconstruction: std::time::Duration,
    pub hit_count: usize,
}

impl PreparedAnchorScan {
    pub(super) fn run(&self) -> Result<AnchorLists, Error> {
        self.run_profiled().map(|(anchors, _)| anchors)
    }

    pub(super) fn run_profiled(&self) -> Result<(AnchorLists, AnchorScanProfile), Error> {
        let mut anchors = (0..self.file_count)
            .map(|_| (0..self.program_count).map(|_| Vec::new()).collect())
            .collect::<Vec<Vec<Vec<u32>>>>();
        if self.locations.is_empty() {
            return Ok((
                anchors,
                AnchorScanProfile {
                    dispatch_and_wait: std::time::Duration::ZERO,
                    reconstruction: std::time::Duration::ZERO,
                    hit_count: 0,
                },
            ));
        }
        let _execution_guard = metal_execution_guard()?;
        unsafe {
            self.hit_count.buffer.contents().cast::<u32>().write(0);
        }
        let dispatch_started = std::time::Instant::now();
        let command = self.queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("anchor_selection");
        encoder.set_compute_pipeline_state(&self.pipeline);
        encoder.set_buffer(0, Some(&self.kinds.buffer), 0);
        encoder.set_buffer(1, Some(&self.spans.buffer), 0);
        encoder.set_buffer(2, Some(&self.node_files.buffer), 0);
        encoder.set_buffer(3, Some(&self.group_enabled.buffer), 0);
        encoder.set_buffer(4, Some(&self.source.buffer), 0);
        encoder.set_buffer(5, Some(&self.hits.buffer), 0);
        encoder.set_buffer(6, Some(&self.hit_count.buffer), 0);
        let count = u32::try_from(self.locations.len()).map_err(|_| Error::BackendFailure)?;
        encoder.set_bytes(7, 4, (&count as *const u32).cast());
        let width = self.pipeline.thread_execution_width();
        // One dispatch is the complete anchor-selection phase.
        encoder.checked_dispatch_threads(
            command,
            MTLSize {
                width: count as u64,
                height: 1,
                depth: 1,
            },
            MTLSize {
                width,
                height: 1,
                depth: 1,
            },
        );
        encoder.end_encoding();
        commit_checked(command)?;
        wait_checked(command)?;
        let dispatch_and_wait = dispatch_started.elapsed();
        let reconstruction_started = std::time::Instant::now();
        let hit_count = unsafe { self.hit_count.buffer.contents().cast::<u32>().read() as usize };
        if hit_count > self.locations.len() {
            return Err(Error::BackendFailure);
        }
        let mut hits = unsafe {
            std::slice::from_raw_parts(self.hits.buffer.contents().cast::<AnchorHit>(), hit_count)
                .to_vec()
        };
        // Atomic append order is undefined. Global node order is file order plus preorder order.
        hits.sort_unstable_by_key(|hit| hit.node);
        for hit in hits {
            let (file, ordinal) = self.locations[hit.node as usize];
            let programs = self
                .group_programs
                .get(hit.group as usize)
                .ok_or(Error::BackendFailure)?;
            for &program in programs {
                let enabled = self.file_program_masks[file * self.program_words + program / 32]
                    & (1_u32 << (program % 32))
                    != 0;
                if enabled {
                    anchors[file][program].push(ordinal);
                }
            }
        }
        let reconstruction = reconstruction_started.elapsed();
        Ok((
            anchors,
            AnchorScanProfile {
                dispatch_and_wait,
                reconstruction,
                hit_count,
            },
        ))
    }
}

#[cfg(test)]
pub mod launch_tests {
    use super::launch::*;
    use super::*;

    #[test]
    fn production_encoders_cannot_bypass_the_launch_guard() {
        // The engine's Metal files, and every file of the Metal crate (its runtime is the old
        // `metal_backend.rs`, its launch layer `launch.rs`).
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let engine = std::fs::read_dir(root.join("src/code_rule_dsl/transducer/packed")).unwrap();
        let crate_files = std::fs::read_dir(root.join("crates/metal/src")).unwrap();
        for (in_crate, entry) in engine.map(|e| (false, e)).chain(crate_files.map(|e| (true, e))) {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            // `*_tests.rs` files are test modules (mounted under `#[cfg(test)]`), not encoders.
            if !(in_crate || name.starts_with("metal"))
                || !name.ends_with(".rs")
                || name.ends_with("_tests.rs")
                || (in_crate && name == "launch.rs")
            {
                continue;
            }
            let name = if in_crate && name == "runtime.rs" { "metal_backend.rs" } else { name };
            let source = std::fs::read_to_string(&path).unwrap();
            // Up to its first test module (`mod` or, since the crate split, `pub mod`).
            let cut = ["#[cfg(test)]\nmod ", "#[cfg(test)]\npub mod "].iter().filter_map(|m| source.find(m)).min();
            let source = &source[..cut.unwrap_or(source.len())];
            for forbidden in [
                ".dispatch_threads(",
                ".dispatch_thread_groups(",
                ".dispatch_thread_groups_indirect(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{name} bypasses the launch guard: {forbidden}"
                );
            }
            if name != "metal_backend.rs" {
                assert!(
                    !source.contains(".commit()"),
                    "{name} bypasses checked submission"
                );
            } else {
                // Only the upload/readback blit commands can submit without a launch check.
                for prefix in source.split(".commit()").collect::<Vec<_>>().windows(2) {
                    let before = prefix[0];
                    assert!(
                        before.rfind("new_blit_command_encoder")
                            > before.rfind("new_compute_command_encoder"),
                        "unchecked compute submission in {name}"
                    );
                }
            }
        }
    }
}
