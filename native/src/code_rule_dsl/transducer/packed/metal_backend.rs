//! The matcher's anchor scan (an anchor literal's nodes found on the device), in UKL on the process's device
//! through the lane (decision 0440 step C). The runtime it runs on (the device, the buffer budget, launches) is
//! the `rerail-metal` crate's, re-exported here.

use super::{EMIT, Error, KIND, Program, TEXT, TextPredicate, split};
use crate::code_rule_dsl::tape::TapeDoc;
#[allow(unused_imports)]
pub use rerail_metal::runtime::*;
pub use rerail_metal::launch;
/// The Metal tests' allocation counter, where they name it (it moved into `rerail-metal` with the runtime).
#[cfg(any(test, feature = "metal-tests"))]
pub(super) use rerail_metal::buffer_observation::BufferObservation;
use std::collections::BTreeMap;

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
        source.push_str(&format!("switch(source[start+{depth}u]){{"));
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

pub struct PreparedAnchorScan {
    kernel: rerail_metal::lane::Kernel,
    /// The files, resident in one block: the kernel reads each node's kind, span and text there, in place.
    table: crate::code_rule_dsl::residency::slabs::DocTable,
    docs: rerail_metal::lane::HostMapped,
    group_enabled: rerail_metal::lane::Buffer,
    /// {node, group} pairs, appended by the kernel.
    hits: rerail_metal::lane::Buffer,
    hit_count: rerail_metal::lane::Buffer,
    group_programs: Vec<Vec<usize>>,
    file_program_masks: Vec<u32>,
    program_words: usize,
    file_count: usize,
    program_count: usize,
}

/// Prepare one specialized GPU dispatch for exact kind/value anchor selection. The generated
/// kernel groups literals by kind and length, then uses byte tries. It appends only positive hits.
pub fn prepare_anchor_scan(
    programs: &[Program],
    files: &[TapeDoc],
) -> Result<PreparedAnchorScan, Error> {
    prepare_anchor_scan_with_eligibility(programs, files, None)
}

/// `file_program_masks` carries file/check eligibility from SQL into the GPU scan. Each file owns
/// `ceil(programs / 32)` words. This representation does not split large program sets into more
/// dispatches.
pub fn prepare_anchor_scan_with_eligibility(
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
        tests.push_str(&format!("if(kind=={kind}u && end-start=={length}u){{"));
        trie.emit(0, &mut tests);
        tests.push('}');
    }
    use crate::code_rule_dsl::residency::slabs::{DocTable, DocView, admit_docs};
    use rerail_metal::lane;
    // The files resident, read where they lie by the kernel (decision 0353 step 4, 0471): the first one's slab bound,
    // the others reached from it by the slabs' addresses. A batch over several slabs, or of 16z, is refused where
    // buffers have no kernels' address (OpenCL 1.2), as every reader of the block is.
    let table = DocTable::new(admit_docs(files)?).ok_or(Error::BackendCapacity)?;
    if table.nodes != files.iter().map(TapeDoc::transducer_len).sum::<usize>() {
        return Err(Error::BackendFailure);
    }
    u32::try_from(table.nodes).map_err(|_| Error::BackendFailure)?;
    // UKL (decision 0440 step C): one thread a node, its document found among the views.
    let source = format!(
        r#"{view}
typedef struct AnchorArgs {{ uint32_t count; uint32_t documents; }} AnchorArgs;
UKL_REFLECT_STRUCT(AnchorArgs, count, documents)
DEFINE_KERNEL2(anchor_scan, anchors,
               ROBUFFER, DocView,  docs,         "[args.documents]",
               ROBUFFER, uint32_t, groupEnabled, "[args.documents * {group_words}]",
               RWBUFFER, uint32_t, hits,         "[2 * args.count]",
               RWBUFFER, uint32_t, hitCount,     "[1]",
               ARGS,     AnchorArgs, args,       ,
               ROBUFFER, uint8_t,  chunk,        "[slab]",
               GID0,     uint32_t, id,           )
{{
 if(id>=args.count)return;
 uint32_t file=doc_index(docs,args.documents,id);
 DocView d=docs[file];
 UKL_GLOBAL const uint8_t* s=chunk;
 uint32_t i=id-d.node_base,kind=tape_kind(s,d,i),start=tape_start(s,d,i),end=tape_end(s,d,i);
 UKL_GLOBAL const uint8_t* source=tape_source(s,d);
 uint32_t group=0xffffffffu; {tests}
 if(group!=0xffffffffu){{uint32_t word=groupEnabled[file*{group_words}u+(group>>5u)];
  if((word&(1u<<(group&31u)))!=0u){{uint32_t slot=ukl_atomic_add(&hitCount[0],1u);hits[2u*slot]=id;hits[2u*slot+1u]=group;}}}}
}}
"#,
        view = super::metal_program_backend::tape_view(table.format().unwrap_or(0)),
    );
    let kernel = lane::kernel(&source, "anchors", Some(table.format().unwrap_or(0)));
    let device = lane::device()?;
    let docs = if table.views.is_empty() { lane::HostMapped::zeroed::<DocView>(1)? } else { lane::HostMapped::from_slice(&table.views)? };
    let group_enabled = lane::buffer_of(device, &group_enabled);
    let hits = lane::buffer_of(device, &vec![0_u32; 2 * table.nodes]);
    let hit_count = lane::buffer_of(device, &[0_u32]);
    Ok(PreparedAnchorScan {
        kernel,
        table,
        docs,
        group_enabled,
        hits,
        hit_count,
        group_programs,
        file_program_masks: flat_program_masks,
        program_words,
        file_count: files.len(),
        program_count: programs.len(),
    })
}

type AnchorLists = Vec<Vec<Vec<u32>>>;

pub struct AnchorScanProfile {
    pub dispatch_and_wait: std::time::Duration,
    pub reconstruction: std::time::Duration,
    pub hit_count: usize,
}

impl PreparedAnchorScan {
    /// It writes its own buffers (the hit count, the hits), so a run holds the scan alone (`&mut`).
    pub fn run(&mut self) -> Result<AnchorLists, Error> {
        self.run_profiled().map(|(anchors, _)| anchors)
    }

    pub fn run_profiled(&mut self) -> Result<(AnchorLists, AnchorScanProfile), Error> {
        let mut anchors = (0..self.file_count)
            .map(|_| (0..self.program_count).map(|_| Vec::new()).collect())
            .collect::<Vec<Vec<Vec<u32>>>>();
        if self.table.nodes == 0 {
            return Ok((
                anchors,
                AnchorScanProfile {
                    dispatch_and_wait: std::time::Duration::ZERO,
                    reconstruction: std::time::Duration::ZERO,
                    hit_count: 0,
                },
            ));
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct AnchorArgs {
            count: u32,
            documents: u32,
        }
        // SAFETY: two u32s, no padding.
        unsafe impl rerail_metal::lane::Pod for AnchorArgs {}
        let _evaluation = evaluation_scope();
        let dispatch_started = std::time::Instant::now();
        let count = self.table.nodes as u32;
        let block = self.table.block.as_ref().ok_or(Error::BackendFailure)?;
        // One launch is the complete anchor-selection phase, after the hit count is zeroed.
        let mut command = rerail_metal::lane::Command::new("anchor_selection");
        command.fill(&self.hit_count, 0, 1, 0);
        let mut launch = command
            .launch(&self.kernel, "anchor_selection")
            .buffer("docs", self.docs.buffer())
            .buffer("groupEnabled", &self.group_enabled)
            .buffer("hits", &self.hits)
            .buffer("hitCount", &self.hit_count)
            .value("args", &AnchorArgs { count, documents: self.table.views.len() as u32 })
            .buffer("chunk", block.device_buffer());
        for slab in &self.table.slabs {
            launch = launch.uses(slab.device_buffer());
        }
        if let Some(cache) = &self.table.cache {
            launch = launch.uses(cache);
        }
        launch.threads(u64::from(count), 64);
        command.run()?;
        if self.table.cache.as_ref().is_some_and(|cache| rerail_metal::slabs::zcache_failed(cache) != 0) {
            return Err(Error::BackendCapacity);
        }
        let dispatch_and_wait = dispatch_started.elapsed();
        let reconstruction_started = std::time::Instant::now();
        // SAFETY: the command finished; the kernel wrote the count and that many pairs.
        let hit_count = unsafe { self.hit_count.contents().cast::<u32>().read() as usize };
        if hit_count > self.table.nodes {
            return Err(Error::BackendFailure);
        }
        // copy: not block data (the kernel's hits, sorted for the files' order)
        let mut hits = unsafe { std::slice::from_raw_parts(self.hits.contents().cast::<[u32; 2]>(), hit_count) }.to_vec();
        // Atomic append order is undefined. Global node order is file order plus preorder order.
        hits.sort_unstable_by_key(|hit| hit[0]);
        for [node, group] in hits {
            let file = self.table.views.partition_point(|view| view.node_base <= node) - 1;
            let ordinal = node - self.table.views[file].node_base;
            let programs = self
                .group_programs
                .get(group as usize)
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
pub mod launch_tests;
