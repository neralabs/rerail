use super::{Capture, EMIT, Error, KIND, Output, Program, TEXT, TextPredicate, split};
use crate::policy_dsl::tape::TapeDoc;
use metal::{
    Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLCommandBufferStatus,
    MTLResourceOptions, MTLSize,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::mem::size_of_val;
use std::ptr::NonNull;
use std::sync::Arc;

fn completed_command_buffer(status: MTLCommandBufferStatus) -> Result<(), Error> {
    (status == MTLCommandBufferStatus::Completed)
        .then_some(())
        .ok_or(Error::BackendFailure)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Task {
    kind: u32,
    start: u32,
    end: u32,
    program: u32,
}

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

pub(super) enum MetalProgram {
    Interpreted(ComputePipelineState),
    Jit(ComputePipelineState),
}

/// A page-backed host allocation retained for the complete lifetime of a no-copy Metal buffer.
/// Apple Silicon's shared storage can read this allocation directly; repeated dispatches do not
/// repack sources, tasks, programs, or anchor nodes.
struct MappedRegion {
    pointer: NonNull<u8>,
    length: usize,
}

unsafe impl Send for MappedRegion {}
unsafe impl Sync for MappedRegion {}

impl MappedRegion {
    fn new(bytes: &[u8]) -> Result<Self, Error> {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let wanted = bytes.len().max(1);
        let length = wanted
            .checked_add(page - 1)
            .map(|value| value / page * page)
            .ok_or(Error::BackendFailure)?;
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(Error::BackendFailure);
        }
        let pointer = NonNull::new(raw.cast::<u8>()).ok_or(Error::BackendFailure)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.as_ptr(), bytes.len());
        }
        Ok(Self { pointer, length })
    }
}

impl Drop for MappedRegion {
    fn drop(&mut self) {
        unsafe {
            libc::madvise(
                self.pointer.as_ptr().cast(),
                self.length,
                libc::MADV_DONTNEED,
            );
            libc::munmap(self.pointer.as_ptr().cast(), self.length);
        }
    }
}

struct MappedBuffer {
    // Drop the Metal reference before the mapping that backs it.
    buffer: Buffer,
    _mapping: Arc<MappedRegion>,
}

impl MappedBuffer {
    fn new(device: &Device, bytes: &[u8]) -> Result<Self, Error> {
        let mapping = Arc::new(MappedRegion::new(bytes)?);
        let buffer = device.new_buffer_with_bytes_no_copy(
            mapping.pointer.as_ptr().cast(),
            mapping.length as u64,
            MTLResourceOptions::StorageModeShared,
            None,
        );
        Ok(Self {
            buffer,
            _mapping: mapping,
        })
    }

    fn from_slice<T: Copy>(device: &Device, values: &[T]) -> Result<Self, Error> {
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        Self::new(device, bytes)
    }
}

const INTERPRETER: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Task { uint kind; uint start; uint end; uint program; };
kernel void run(device const Task* tasks [[buffer(0)]], device const uint* words [[buffer(1)]],
 device const uint* pcs [[buffer(2)]], device const uchar* source [[buffer(3)]],
 device const uint* stringStart [[buffer(4)]], device const uint* stringLen [[buffer(5)]],
 device uint* result [[buffer(6)]], constant uint& count [[buffer(7)]], uint id [[thread_position_in_grid]]) {
 if (id >= count) return; Task t=tasks[id]; uint pc=pcs[t.program]; bool keep=true;
 for (uint steps=0; keep && steps<16; ++steps) { uint w=words[pc++]; uint op=w&255, operand=w>>16;
  if (op==1) keep=t.kind==operand;
  else if (op==4) { uint s=stringStart[t.program], n=stringLen[t.program]; keep=(t.end-t.start)==n;
    for(uint i=0; keep && i<n; ++i) keep=source[t.start+i]==source[s+i]; }
  else if (op==32) {} else if (op==64) { result[id]=1; return; } else { keep=false; }
 } result[id]=0;
}"#;

fn literal(program: &Program) -> Result<(u16, &[u8], u16, u16), Error> {
    if program.words.len() != 4
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

thread_local! {
    static PIPELINES: RefCell<HashMap<[u8; 32], ComputePipelineState>> = RefCell::new(HashMap::new());
}

fn pipeline_named(source: &str, name: &str) -> Result<ComputePipelineState, Error> {
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let mut digest = Sha256::new();
    digest.update(device.registry_id().to_le_bytes());
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(source.as_bytes());
    let key: [u8; 32] = digest.finalize().into();
    if let Some(cached) = PIPELINES.with_borrow(|pipelines| pipelines.get(&key).cloned()) {
        return Ok(cached);
    }
    let library = device
        .new_library_with_source(source, &CompileOptions::new())
        .map_err(|_| Error::BackendFailure)?;
    let function = library
        .get_function(name, None)
        .map_err(|_| Error::BackendFailure)?;
    let pipeline = device
        .new_compute_pipeline_state_with_function(&function)
        .map_err(|_| Error::BackendFailure)?;
    PIPELINES.with_borrow_mut(|pipelines| {
        pipelines.insert(key, pipeline.clone());
    });
    Ok(pipeline)
}

fn pipeline(source: &str) -> Result<ComputePipelineState, Error> {
    pipeline_named(source, "run")
}

pub(super) fn compile_interpreted(programs: &[Program]) -> Result<MetalProgram, Error> {
    for program in programs {
        literal(program)?;
    }
    Ok(MetalProgram::Interpreted(pipeline(INTERPRETER)?))
}

pub(super) fn compile_jit(programs: &[Program]) -> Result<MetalProgram, Error> {
    let mut body = String::new();
    for (index, program) in programs.iter().enumerate() {
        let (kind, text, _, _) = literal(program)?;
        body.push_str(&format!(
            "case {index}: keep=t.kind=={kind}u && t.end-t.start=={}u;",
            text.len()
        ));
        for (offset, byte) in text.iter().enumerate() {
            body.push_str(&format!("keep=keep && source[t.start+{offset}u]=={byte}u;"));
        }
        body.push_str("break;");
    }
    let source = format!(
        r#"#include <metal_stdlib>
using namespace metal; struct Task {{ uint kind; uint start; uint end; uint program; }};
kernel void run(device const Task* tasks [[buffer(0)]], device const uchar* source [[buffer(1)]],
 device uint* result [[buffer(2)]], constant uint& count [[buffer(3)]], uint id [[thread_position_in_grid]]) {{
 if(id>=count)return; Task t=tasks[id]; bool keep=false; switch(t.program){{{body}default:break;}} result[id]=keep; }}"#
    );
    Ok(MetalProgram::Jit(pipeline(&source)?))
}

struct InterpreterBuffers {
    words: MappedBuffer,
    pcs: MappedBuffer,
    starts: MappedBuffer,
    lengths: MappedBuffer,
}

/// A reusable dispatch. Preparation performs all packing, mapping, and Metal-buffer creation.
/// `run` measures only submission, synchronization, and deterministic reconstruction.
pub(super) struct PreparedBatch<'a> {
    compiled: &'a MetalProgram,
    programs: &'a [Program],
    queue: CommandQueue,
    tasks: MappedBuffer,
    source: MappedBuffer,
    interpreter: Option<InterpreterBuffers>,
    result: Buffer,
    locations: Vec<(usize, usize, u32, std::ops::Range<usize>)>,
    file_count: usize,
}

pub(super) fn prepare<'a>(
    compiled: &'a MetalProgram,
    programs: &'a [Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<PreparedBatch<'a>, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
    let mut source = Vec::new();
    let mut bases = Vec::new();
    for file in files {
        bases.push(u32::try_from(source.len()).map_err(|_| Error::BackendFailure)?);
        source.extend_from_slice(file.transducer_source());
    }
    let mut tasks = Vec::new();
    let mut locations = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        for (pi, roots) in anchors[fi].iter().enumerate() {
            for &node in roots {
                if node as usize >= file.transducer_len() {
                    return Err(Error::InvalidAnchor);
                }
                let span = file.transducer_span(node);
                let start = u32::try_from(span.start)
                    .ok()
                    .and_then(|start| bases[fi].checked_add(start))
                    .ok_or(Error::BackendFailure)?;
                let end = u32::try_from(span.end)
                    .ok()
                    .and_then(|end| bases[fi].checked_add(end))
                    .ok_or(Error::BackendFailure)?;
                tasks.push(Task {
                    kind: u32::from(file.transducer_kind(node)),
                    start,
                    end,
                    program: u32::try_from(pi).map_err(|_| Error::BackendFailure)?,
                });
                locations.push((fi, pi, node, span));
            }
        }
    }
    let mut interpreter_values = None;
    if matches!(compiled, MetalProgram::Interpreted(_)) {
        let mut words = Vec::new();
        let mut pcs = Vec::new();
        let mut starts = Vec::new();
        let mut lengths = Vec::new();
        for program in programs {
            pcs.push(u32::try_from(words.len()).map_err(|_| Error::BackendFailure)?);
            words.extend_from_slice(&program.words);
            let (_, text, _, _) = literal(program)?;
            starts.push(u32::try_from(source.len()).map_err(|_| Error::BackendFailure)?);
            lengths.push(u32::try_from(text.len()).map_err(|_| Error::BackendFailure)?);
            source.extend_from_slice(text);
        }
        interpreter_values = Some((words, pcs, starts, lengths));
    }
    let tasks = MappedBuffer::from_slice(&device, &tasks)?;
    let source = MappedBuffer::new(&device, &source)?;
    let interpreter = if let Some((words, pcs, starts, lengths)) = interpreter_values {
        Some(InterpreterBuffers {
            words: MappedBuffer::from_slice(&device, &words)?,
            pcs: MappedBuffer::from_slice(&device, &pcs)?,
            starts: MappedBuffer::from_slice(&device, &starts)?,
            lengths: MappedBuffer::from_slice(&device, &lengths)?,
        })
    } else {
        None
    };
    let result = device.new_buffer(
        (locations.len() * std::mem::size_of::<u32>()).max(4) as u64,
        MTLResourceOptions::StorageModeShared,
    );
    Ok(PreparedBatch {
        compiled,
        programs,
        queue,
        tasks,
        source,
        interpreter,
        result,
        locations,
        file_count: files.len(),
    })
}

impl PreparedBatch<'_> {
    pub(super) fn run(&self) -> Result<Vec<Vec<Vec<Output>>>, Error> {
        let actual_count = self.locations.len();
        if actual_count == 0 {
            return Ok((0..self.file_count)
                .map(|_| self.programs.iter().map(|_| Vec::new()).collect())
                .collect());
        }
        let command = self.queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        let state = match self.compiled {
            MetalProgram::Interpreted(state) | MetalProgram::Jit(state) => state,
        };
        encoder.set_compute_pipeline_state(state);
        encoder.set_buffer(0, Some(&self.tasks.buffer), 0);
        let count = u32::try_from(actual_count).map_err(|_| Error::BackendFailure)?;
        match (self.compiled, &self.interpreter) {
            (MetalProgram::Interpreted(_), Some(aux)) => {
                encoder.set_buffer(1, Some(&aux.words.buffer), 0);
                encoder.set_buffer(2, Some(&aux.pcs.buffer), 0);
                encoder.set_buffer(3, Some(&self.source.buffer), 0);
                encoder.set_buffer(4, Some(&aux.starts.buffer), 0);
                encoder.set_buffer(5, Some(&aux.lengths.buffer), 0);
                encoder.set_buffer(6, Some(&self.result), 0);
                encoder.set_bytes(7, 4, (&count as *const u32).cast());
            }
            (MetalProgram::Jit(_), None) => {
                encoder.set_buffer(1, Some(&self.source.buffer), 0);
                encoder.set_buffer(2, Some(&self.result), 0);
                encoder.set_bytes(3, 4, (&count as *const u32).cast());
            }
            _ => return Err(Error::BackendFailure),
        }
        let width = state.thread_execution_width();
        encoder.dispatch_threads(
            MTLSize {
                width: actual_count as u64,
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
        command.commit();
        command.wait_until_completed();
        completed_command_buffer(command.status())?;
        let flags = unsafe {
            std::slice::from_raw_parts(self.result.contents().cast::<u32>(), actual_count)
        };
        let mut output = (0..self.file_count)
            .map(|_| self.programs.iter().map(|_| Vec::new()).collect())
            .collect::<Vec<Vec<Vec<Output>>>>();
        for (matched, &(fi, pi, node, ref span)) in flags.iter().zip(&self.locations) {
            if *matched != 0 {
                let (_, _, capture, label) = literal(&self.programs[pi])?;
                output[fi][pi].push(Output {
                    start: node,
                    label,
                    captures: vec![Capture {
                        label: capture,
                        node,
                        span: span.clone(),
                    }],
                });
            }
        }
        Ok(output)
    }
}

pub(super) fn execute(
    compiled: &MetalProgram,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    prepare(compiled, programs, files, anchors)?.run()
}

pub(super) struct PreparedAnchorScan {
    pipeline: ComputePipelineState,
    queue: CommandQueue,
    kinds: MappedBuffer,
    spans: MappedBuffer,
    source: MappedBuffer,
    hits: Buffer,
    hit_count: Buffer,
    locations: Vec<(usize, u32)>,
    group_programs: Vec<Vec<usize>>,
    file_count: usize,
    program_count: usize,
}

/// Prepare one specialized GPU dispatch for exact kind/value anchor selection. The generated
/// kernel groups literals by kind and length, then uses byte tries. It appends only positive hits.
pub(super) fn prepare_anchor_scan(
    programs: &[Program],
    files: &[TapeDoc],
) -> Result<PreparedAnchorScan, Error> {
    let mut literal_groups = BTreeMap::<(u16, Vec<u8>), usize>::new();
    let mut group_programs = Vec::<Vec<usize>>::new();
    for (program_index, program) in programs.iter().enumerate() {
        let (kind, text, _, _) = literal(program)?;
        let next = literal_groups.len();
        let group = *literal_groups.entry((kind, text.to_vec())).or_insert(next);
        if group == group_programs.len() {
            group_programs.push(Vec::new());
        }
        group_programs[group].push(program_index);
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
 device const uchar* source [[buffer(2)]], device AnchorHit* hits [[buffer(3)]],
 device atomic_uint* hitCount [[buffer(4)]], constant uint& count [[buffer(5)]],
 uint id [[thread_position_in_grid]]) {{
 if(id>=count)return; uint kind=kinds[id]; uint2 span=spans[id]; uint group=0xffffffffu; {tests}
 if(group!=0xffffffffu){{uint slot=atomic_fetch_add_explicit(hitCount,1u,memory_order_relaxed);hits[slot]={{id,group}};}}
}}"#
    );
    let pipeline = pipeline_named(&metal_source, "anchors")?;
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
    let mut source = Vec::new();
    let mut kinds = Vec::new();
    let mut spans = Vec::new();
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
            locations.push((fi, ordinal));
        }
    }
    let kinds = MappedBuffer::from_slice(&device, &kinds)?;
    let spans = MappedBuffer::from_slice(&device, &spans)?;
    let source = MappedBuffer::new(&device, &source)?;
    let hits = device.new_buffer(
        (locations.len() * std::mem::size_of::<AnchorHit>()).max(8) as u64,
        MTLResourceOptions::StorageModeShared,
    );
    let hit_count = device.new_buffer(4, MTLResourceOptions::StorageModeShared);
    Ok(PreparedAnchorScan {
        pipeline,
        queue,
        kinds,
        spans,
        source,
        hits,
        hit_count,
        locations,
        group_programs,
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
        unsafe {
            self.hit_count.contents().cast::<u32>().write(0);
        }
        let dispatch_started = std::time::Instant::now();
        let command = self.queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&self.pipeline);
        encoder.set_buffer(0, Some(&self.kinds.buffer), 0);
        encoder.set_buffer(1, Some(&self.spans.buffer), 0);
        encoder.set_buffer(2, Some(&self.source.buffer), 0);
        encoder.set_buffer(3, Some(&self.hits), 0);
        encoder.set_buffer(4, Some(&self.hit_count), 0);
        let count = u32::try_from(self.locations.len()).map_err(|_| Error::BackendFailure)?;
        encoder.set_bytes(5, 4, (&count as *const u32).cast());
        let width = self.pipeline.thread_execution_width();
        // One dispatch is the complete anchor-selection phase.
        encoder.dispatch_threads(
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
        command.commit();
        command.wait_until_completed();
        completed_command_buffer(command.status())?;
        let dispatch_and_wait = dispatch_started.elapsed();
        let reconstruction_started = std::time::Instant::now();
        let hit_count = unsafe { self.hit_count.contents().cast::<u32>().read() as usize };
        if hit_count > self.locations.len() {
            return Err(Error::BackendFailure);
        }
        let mut hits = unsafe {
            std::slice::from_raw_parts(self.hits.contents().cast::<AnchorHit>(), hit_count).to_vec()
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
                anchors[file][program].push(ordinal);
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
mod tests {
    use super::*;
    use metal::MTLCommandBufferStatus;

    #[test]
    fn failed_command_buffer_is_a_backend_failure() {
        assert_eq!(
            completed_command_buffer(MTLCommandBufferStatus::Error),
            Err(Error::BackendFailure)
        );
    }
}
