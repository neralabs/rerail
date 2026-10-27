use super::{EMIT, Error, KIND, Program, TEXT, TextPredicate, split};
use crate::code_rule_dsl::tape::TapeDoc;
use metal::{
    Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLResourceOptions, MTLSize,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::mem::size_of_val;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

pub(crate) mod launch;

// G's process-wide buffer counters; nothing charges them until G's buffer budget is ported.
static METAL_BUFFER_BYTES: AtomicUsize = AtomicUsize::new(0);
static METAL_PRIVATE_BYTES: AtomicUsize = AtomicUsize::new(0);

pub(super) fn buffer_usage() -> (usize, usize) {
    (
        METAL_BUFFER_BYTES.load(Ordering::Acquire),
        METAL_PRIVATE_BYTES.load(Ordering::Acquire),
    )
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
    fn zeroed(byte_length: usize) -> Result<Self, Error> {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let wanted = byte_length.max(1);
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
        Ok(Self { pointer, length })
    }

    fn new(bytes: &[u8]) -> Result<Self, Error> {
        let region = Self::zeroed(bytes.len())?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), region.pointer.as_ptr(), bytes.len());
        }
        Ok(region)
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

pub(super) struct MappedBuffer {
    // Drop the Metal reference before the mapping that backs it.
    pub(super) buffer: Buffer,
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

    pub(super) fn from_slice<T: Copy>(device: &Device, values: &[T]) -> Result<Self, Error> {
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        Self::new(device, bytes)
    }

    pub(super) fn zeroed<T>(device: &Device, count: usize) -> Result<Self, Error> {
        let byte_length = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::BackendCapacity)?;
        let mapping = Arc::new(MappedRegion::zeroed(byte_length)?);
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

static METAL_EXECUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(super) struct MetalExecutionGuard {
    _lock: MutexGuard<'static, ()>,
    _launch_scope: launch::LaunchScope,
}

pub(super) fn metal_execution_guard() -> Result<MetalExecutionGuard, Error> {
    let lock = METAL_EXECUTION_LOCK
        .lock()
        .map_err(|_| Error::BackendFailure)?;
    Ok(MetalExecutionGuard {
        _lock: lock,
        _launch_scope: launch::LaunchScope::enter(),
    })
}

thread_local! {
    static PIPELINES: RefCell<HashMap<[u8; 32], ComputePipelineState>> = RefCell::new(HashMap::new());
}

pub(super) fn pipeline_named(source: &str, name: &str) -> Result<ComputePipelineState, Error> {
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
        .map_err(|error| {
            eprintln!("Metal library compilation failed: {error}");
            Error::BackendFailure
        })?;
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

#[cfg(test)]
pub(super) fn pipeline_cache_len() -> usize {
    PIPELINES.with_borrow(HashMap::len)
}

pub(super) struct PreparedAnchorScan {
    pipeline: ComputePipelineState,
    queue: CommandQueue,
    kinds: MappedBuffer,
    spans: MappedBuffer,
    node_files: MappedBuffer,
    group_enabled: MappedBuffer,
    source: MappedBuffer,
    hits: Buffer,
    hit_count: Buffer,
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
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
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
        unsafe {
            self.hit_count.contents().cast::<u32>().write(0);
        }
        let dispatch_started = std::time::Instant::now();
        let command = self.queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&self.pipeline);
        encoder.set_buffer(0, Some(&self.kinds.buffer), 0);
        encoder.set_buffer(1, Some(&self.spans.buffer), 0);
        encoder.set_buffer(2, Some(&self.node_files.buffer), 0);
        encoder.set_buffer(3, Some(&self.group_enabled.buffer), 0);
        encoder.set_buffer(4, Some(&self.source.buffer), 0);
        encoder.set_buffer(5, Some(&self.hits), 0);
        encoder.set_buffer(6, Some(&self.hit_count), 0);
        let count = u32::try_from(self.locations.len()).map_err(|_| Error::BackendFailure)?;
        encoder.set_bytes(7, 4, (&count as *const u32).cast());
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
