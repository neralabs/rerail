use super::execution_shape::{compile_entry_shapes, execution_class};
use super::packed_batch::PackedBatch;
use super::{Error, Program};
use crate::policy_dsl::tape::TapeDoc;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

pub(super) const METAL_GRID_FORMAT_VERSION: u32 = 1;
pub(super) const METAL_GRID_BLOCK_WIDTH: u32 = 256;
const EXECUTION_ROUTE_COUNT: usize = 11;

const SELECTOR_RANGE: u32 = 1;
const SELECTOR_RUNS: u32 = 2;
const SELECTOR_BITS: u32 = 3;
const SELECTOR_EXPLICIT: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalGridHeader {
    pub format_version: u32,
    pub root_count: u32,
    pub selector_count: u32,
    pub selector_data_count: u32,
    pub evaluation_span_count: u32,
    pub block_span_count: u32,
    pub block_count: u32,
    pub semantic_tuple_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RootRecord {
    pub file: u32,
    pub node: u32,
    pub semantic_root_ordinal: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RootSelector {
    pub kind: u32,
    pub file: u32,
    pub data_start: u32,
    pub data_count: u32,
    pub root_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct EvaluationSpan {
    pub evaluation: u32,
    pub file: u32,
    pub program: u32,
    pub signature: u32,
    pub class: u32,
    pub route: u32,
    pub selector: u32,
    pub root_count: u32,
    pub semantic_start: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct BlockSpan {
    pub evaluation_span: u32,
    pub execution_block: u32,
    pub selector_offset: u32,
    pub root_count: u32,
    pub block_lane_start: u32,
    pub semantic_start: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ExecutionRange {
    pub block_start: u32,
    pub block_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ExecutionBlock {
    pub block_span_start: u32,
    pub block_span_count: u32,
    pub dispatch_signature: u32,
    pub route: u32,
    pub logical_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct GridProgramShape {
    pub signature: u32,
    pub class: u32,
    pub route: u32,
    pub dispatch_signature: u32,
}

pub(super) fn compile_program_shapes(programs: &[Program]) -> Vec<GridProgramShape> {
    let mut signatures = BTreeMap::<Vec<u32>, u32>::new();
    let entries = compile_entry_shapes(programs);
    programs
        .iter()
        .enumerate()
        .map(|(program_index, program)| {
            let mut cursor_ids = BTreeMap::new();
            let mut key = Vec::new();
            for &instruction in &program.words {
                let (opcode, _, operand) = super::split(instruction);
                key.push(instruction & 0xffff);
                let shape = if matches!(opcode, super::SAVE_CURSOR | super::RESTORE_CURSOR) {
                    let next = cursor_ids.len() as u32;
                    *cursor_ids.entry(operand).or_insert(next)
                } else if matches!(opcode, super::FORK | super::GOTO) {
                    u32::from(operand)
                } else {
                    0
                };
                key.push(shape);
            }
            key.extend([
                u32::MAX,
                program.value_programs.len() as u32,
                program.capture_programs.len() as u32,
                program.output_program.instructions.len() as u32,
                entries[program_index].cursor_slots,
                u32::from(program.tree_sitter_query),
            ]);
            for value in &program.value_programs {
                let value = super::execution_shape::value_execution_signature(value);
                key.push(value.len() as u32);
                key.extend(value);
            }
            for capture in &program.capture_programs {
                use super::value::CaptureProgram::*;
                let shape = match capture {
                    Equal { negated, any, .. } => [0, u32::from(*negated), u32::from(*any), 0, 0],
                    Text { negated, any, .. } => [1, u32::from(*negated), u32::from(*any), 0, 0],
                    AnyOf {
                        strings,
                        negated,
                        any,
                        ..
                    } => [
                        2,
                        u32::from(*negated),
                        u32::from(*any),
                        strings.len() as u32,
                        0,
                    ],
                    Kind {
                        kinds,
                        negated,
                        any,
                        empty_all,
                        ..
                    } => [
                        3,
                        u32::from(*negated),
                        u32::from(*any),
                        kinds.len() as u32,
                        u32::from(*empty_all),
                    ],
                    Contains {
                        strings,
                        negated,
                        any,
                        ..
                    } => [
                        4,
                        u32::from(*negated),
                        u32::from(*any),
                        strings.len() as u32,
                        0,
                    ],
                    Relation {
                        kinds,
                        ancestor,
                        negated,
                        ..
                    } => [
                        5,
                        u32::from(*negated),
                        u32::from(*ancestor),
                        kinds.len() as u32,
                        0,
                    ],
                    SameLine { negated, .. } => [6, u32::from(*negated), 0, 0, 0],
                };
                key.extend(shape);
            }
            for output in &program.output_program.instructions {
                use super::output::OutputInstruction::*;
                key.push(match output {
                    MatchProperty { .. } => 0,
                    CaptureProperty { .. } => 1,
                    Offset { .. } => 2,
                    Strip { .. } => 3,
                    SelectAdjacent { .. } => 4,
                    Downcase { .. } => 5,
                    LanguageFromMime { .. } => 6,
                });
                if let Strip { regex, .. } = output {
                    key.extend(super::execution_shape::value_execution_signature(
                        &super::value::ValueProgram::Regex(regex.clone()),
                    ));
                }
            }
            let next = signatures.len() as u32;
            let route = entries[program_index].route as u32;
            let signature = *signatures.entry(key).or_insert(next);
            GridProgramShape {
                signature,
                class: execution_class(program) as u32,
                route,
                dispatch_signature: if super::execution_shape::register_program_paths(program)
                    .is_some()
                {
                    signature | 0x80000000
                } else {
                    entries[program_index].signature
                },
            }
        })
        .collect()
}

pub(super) struct MetalGridInput {
    pub packed: Arc<PackedBatch>,
    pub grid: MetalExecutionGrid,
    pub children: super::metal_child_index::ChildIndex,
}

impl MetalGridInput {
    pub(super) fn program_shapes(
        &self,
        programs: &[Program],
    ) -> Result<Vec<GridProgramShape>, Error> {
        self.children.validate(&self.packed)?;
        if programs.len() != self.packed.programs.len() {
            return Err(Error::InvalidAnchor);
        }
        Ok(input_shapes(programs, &self.children))
    }
    #[cfg(target_os = "macos")]
    pub(super) fn build_streamed_with_upload(
        programs: &[Program],
        files: &[crate::policy_dsl::tape::TapeSource],
        anchors: &[Vec<Arc<[u32]>>],
        after_file: impl FnMut(&mut PackedBatch) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        let packed = Arc::new(PackedBatch::new_streamed_grid_inputs_with_upload(
            programs, files, anchors, after_file,
        )?);
        let children = super::metal_child_index::ChildIndex::new(Arc::clone(&packed))?;
        let grid = MetalExecutionGrid::build_lists(anchors, &input_shapes(programs, &children))?;
        Ok(Self {
            packed,
            grid,
            children,
        })
    }

    pub(super) fn build(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<Self, Error> {
        let packed = Arc::new(PackedBatch::new_grid_inputs(programs, files, anchors)?);
        let children = super::metal_child_index::ChildIndex::new(Arc::clone(&packed))?;
        let grid = MetalExecutionGrid::build(anchors, &input_shapes(programs, &children))?;
        if !packed.tasks.is_empty() {
            return Err(Error::InvalidAnchor);
        }
        Ok(Self {
            packed,
            grid,
            children,
        })
    }
    #[cfg(target_os = "macos")]
    pub(super) fn build_shared(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Arc<[u32]>>],
    ) -> Result<Self, Error> {
        let packed = Arc::new(PackedBatch::new_shared_grid_inputs(
            programs, files, anchors,
        )?);
        let children = super::metal_child_index::ChildIndex::new(Arc::clone(&packed))?;
        let grid = MetalExecutionGrid::build_lists(anchors, &input_shapes(programs, &children))?;
        if !packed.tasks.is_empty() {
            return Err(Error::InvalidAnchor);
        }
        Ok(Self {
            packed,
            grid,
            children,
        })
    }
}

fn input_shapes(
    programs: &[Program],
    children: &super::metal_child_index::ChildIndex,
) -> Vec<GridProgramShape> {
    let mut shapes = compile_program_shapes(programs);
    for (index, shape) in shapes.iter_mut().enumerate() {
        if children.eligible(index) {
            shape.dispatch_signature = shape.signature | 0x80000000;
        }
    }
    shapes
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalExecutionGrid {
    pub header: MetalGridHeader,
    pub roots: Vec<RootRecord>,
    pub selectors: Vec<RootSelector>,
    pub selector_data: Vec<u32>,
    pub evaluation_spans: Vec<EvaluationSpan>,
    pub block_spans: Vec<BlockSpan>,
    pub blocks: Vec<ExecutionBlock>,
    pub route_ranges: [ExecutionRange; EXECUTION_ROUTE_COUNT],
    pub semantic_span_order: Vec<u32>,
}

#[derive(Clone)]
struct SelectorEncoding {
    kind: u32,
    data: Vec<u32>,
    root_count: u32,
}

#[derive(Clone, Copy)]
struct SpanChunk {
    evaluation_span: u32,
    selector_offset: u32,
    root_count: u32,
    semantic_start: u32,
}

impl MetalExecutionGrid {
    pub(super) fn block_indices_for_range(
        &self,
        range: std::ops::Range<u32>,
    ) -> Result<Vec<u32>, Error> {
        if range.start > range.end || range.end > self.header.semantic_tuple_count {
            return Err(Error::InvalidAnchor);
        }
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let first = self.semantic_span_order.partition_point(|&index| {
            let span = self.block_spans[index as usize];
            span.semantic_start + span.root_count <= range.start
        });
        let mut blocks = self.semantic_span_order[first..]
            .iter()
            .copied()
            .take_while(|&index| self.block_spans[index as usize].semantic_start < range.end)
            .map(|index| self.block_spans[index as usize].execution_block)
            .collect::<Vec<_>>();
        blocks.sort_unstable();
        blocks.dedup();
        Ok(blocks)
    }

    pub(super) fn build(
        roots_by_file_program: &[Vec<Vec<u32>>],
        shapes: &[GridProgramShape],
    ) -> Result<Self, Error> {
        Self::build_lists(roots_by_file_program, shapes)
    }

    fn build_lists<R: AsRef<[u32]>>(
        roots_by_file_program: &[Vec<R>],
        shapes: &[GridProgramShape],
    ) -> Result<Self, Error> {
        let mut grid = Self::default();
        let mut selector_candidates = HashMap::<u64, Vec<u32>>::new();
        let mut semantic_start = 0_u32;
        let mut evaluation = 0_u32;

        for (file_index, program_roots) in roots_by_file_program.iter().enumerate() {
            if program_roots.len() != shapes.len() {
                return Err(Error::InvalidAnchor);
            }
            validate_root_lists(program_roots)?;
            let root_list_hashes = program_roots
                .iter()
                .map(|roots| root_list_hash(roots.as_ref()))
                .collect::<Vec<_>>();
            let unique_root_lists = unique_root_lists(program_roots, &root_list_hashes);
            let file_roots = merge_root_slices(&unique_root_lists);
            let root_base = checked_u32(grid.roots.len())?;
            for (root_ordinal, node) in file_roots.iter().copied().enumerate() {
                grid.roots.push(RootRecord {
                    file: checked_u32(file_index)?,
                    node,
                    semantic_root_ordinal: root_base
                        .checked_add(checked_u32(root_ordinal)?)
                        .ok_or(Error::BackendCapacity)?,
                });
            }

            let mut file_selectors = HashMap::<u64, Vec<(usize, u32)>>::new();
            for (program_index, roots) in program_roots.iter().enumerate() {
                let roots = roots.as_ref();
                if roots.is_empty() {
                    continue;
                }
                let root_hash = root_list_hashes[program_index];
                let selector = file_selectors.get(&root_hash).and_then(|candidates| {
                    candidates.iter().find_map(|(previous, selector)| {
                        (program_roots[*previous].as_ref() == roots).then_some(*selector)
                    })
                });
                let selector = if let Some(selector) = selector {
                    selector
                } else {
                    let ordinals = roots_to_ordinals(roots, &file_roots, root_base)?;
                    let encoding = encode_selector(&ordinals)?;
                    let selector = intern_selector(
                        &mut grid,
                        &mut selector_candidates,
                        checked_u32(file_index)?,
                        encoding,
                    )?;
                    file_selectors
                        .entry(root_hash)
                        .or_default()
                        .push((program_index, selector));
                    selector
                };
                let root_count = checked_u32(roots.len())?;
                let shape = shapes[program_index];
                grid.evaluation_spans.push(EvaluationSpan {
                    evaluation,
                    file: checked_u32(file_index)?,
                    program: checked_u32(program_index)?,
                    signature: shape.signature,
                    class: shape.class,
                    route: shape.route,
                    selector,
                    root_count,
                    semantic_start,
                });
                semantic_start = semantic_start
                    .checked_add(root_count)
                    .ok_or(Error::BackendCapacity)?;
                evaluation = evaluation.checked_add(1).ok_or(Error::BackendCapacity)?;
            }
        }

        grid.build_blocks(shapes)?;
        grid.header = MetalGridHeader {
            format_version: METAL_GRID_FORMAT_VERSION,
            root_count: checked_u32(grid.roots.len())?,
            selector_count: checked_u32(grid.selectors.len())?,
            selector_data_count: checked_u32(grid.selector_data.len())?,
            evaluation_span_count: checked_u32(grid.evaluation_spans.len())?,
            block_span_count: checked_u32(grid.block_spans.len())?,
            block_count: checked_u32(grid.blocks.len())?,
            semantic_tuple_count: semantic_start,
        };
        grid.validate(shapes)?;
        Ok(grid)
    }

    fn build_blocks(&mut self, shapes: &[GridProgramShape]) -> Result<(), Error> {
        let mut chunks = BTreeMap::<(u32, u32), Vec<SpanChunk>>::new();
        for (span_index, span) in self.evaluation_spans.iter().copied().enumerate() {
            if span.root_count != 0 {
                chunks
                    .entry((span.route, shapes[span.program as usize].dispatch_signature))
                    .or_default()
                    .push(SpanChunk {
                        evaluation_span: checked_u32(span_index)?,
                        selector_offset: 0,
                        root_count: span.root_count,
                        semantic_start: span.semantic_start,
                    });
            }
        }

        for ((route, dispatch_signature), signature_chunks) in chunks {
            let mut block_start = checked_u32(self.block_spans.len())?;
            let mut logical_count = 0_u32;
            for mut chunk in signature_chunks {
                while chunk.root_count != 0 {
                    let available = METAL_GRID_BLOCK_WIDTH - logical_count;
                    let take = chunk.root_count.min(available);
                    self.block_spans.push(BlockSpan {
                        evaluation_span: chunk.evaluation_span,
                        execution_block: u32::MAX,
                        selector_offset: chunk.selector_offset,
                        root_count: take,
                        block_lane_start: logical_count,
                        semantic_start: chunk.semantic_start,
                    });
                    logical_count += take;
                    chunk.selector_offset += take;
                    chunk.root_count -= take;
                    chunk.semantic_start += take;
                    if logical_count == METAL_GRID_BLOCK_WIDTH {
                        self.push_block(block_start, route, dispatch_signature, logical_count)?;
                        block_start = checked_u32(self.block_spans.len())?;
                        logical_count = 0;
                    }
                }
            }
            if logical_count != 0 {
                self.push_block(block_start, route, dispatch_signature, logical_count)?;
            }
        }

        for route in 0..EXECUTION_ROUTE_COUNT as u32 {
            let start = self.blocks.partition_point(|block| block.route < route);
            let end = self.blocks.partition_point(|block| block.route <= route);
            self.route_ranges[route as usize] = ExecutionRange {
                block_start: checked_u32(start)?,
                block_count: checked_u32(end - start)?,
            };
        }
        self.semantic_span_order = (0..checked_u32(self.block_spans.len())?).collect();
        self.semantic_span_order
            .sort_unstable_by_key(|index| self.block_spans[*index as usize].semantic_start);
        Ok(())
    }

    fn push_block(
        &mut self,
        block_span_start: u32,
        route: u32,
        dispatch_signature: u32,
        logical_count: u32,
    ) -> Result<(), Error> {
        let block_index = checked_u32(self.blocks.len())?;
        for span in &mut self.block_spans[block_span_start as usize..] {
            span.execution_block = block_index;
        }
        self.blocks.push(ExecutionBlock {
            block_span_start,
            block_span_count: checked_u32(self.block_spans.len())?
                .checked_sub(block_span_start)
                .ok_or(Error::BackendFailure)?,
            dispatch_signature,
            route,
            logical_count,
            reserved: 0,
        });
        Ok(())
    }

    pub(super) fn selector_root(&self, selector: u32, offset: u32) -> Result<u32, Error> {
        let selector = *self
            .selectors
            .get(selector as usize)
            .ok_or(Error::InvalidAnchor)?;
        if offset >= selector.root_count {
            return Err(Error::InvalidAnchor);
        }
        let data = self
            .selector_data
            .get(selector.data_start as usize..(selector.data_start + selector.data_count) as usize)
            .ok_or(Error::InvalidAnchor)?;
        decode_selector(selector, data, offset)
    }

    pub(super) fn metadata_bytes(&self) -> usize {
        std::mem::size_of_val(self.selectors.as_slice())
            + std::mem::size_of_val(self.evaluation_spans.as_slice())
            + std::mem::size_of_val(self.block_spans.as_slice())
            + std::mem::size_of_val(self.blocks.as_slice())
            + std::mem::size_of_val(self.semantic_span_order.as_slice())
    }

    fn validate(&self, shapes: &[GridProgramShape]) -> Result<(), Error> {
        let mut expected_semantic = 0_u32;
        for index in &self.semantic_span_order {
            let span = self
                .block_spans
                .get(*index as usize)
                .ok_or(Error::InvalidAnchor)?;
            if span.semantic_start != expected_semantic {
                return Err(Error::InvalidAnchor);
            }
            let evaluation = self
                .evaluation_spans
                .get(span.evaluation_span as usize)
                .ok_or(Error::InvalidAnchor)?;
            let end = span
                .selector_offset
                .checked_add(span.root_count)
                .ok_or(Error::BackendCapacity)?;
            if end > evaluation.root_count {
                return Err(Error::InvalidAnchor);
            }
            expected_semantic = expected_semantic
                .checked_add(span.root_count)
                .ok_or(Error::BackendCapacity)?;
        }
        if expected_semantic != self.header.semantic_tuple_count {
            return Err(Error::InvalidAnchor);
        }
        let mut ranged_blocks = 0_u32;
        for (route, range) in self.route_ranges.iter().copied().enumerate() {
            if range.block_start != ranged_blocks {
                return Err(Error::InvalidAnchor);
            }
            let end = range
                .block_start
                .checked_add(range.block_count)
                .ok_or(Error::BackendCapacity)?;
            if self.blocks[range.block_start as usize..end as usize]
                .iter()
                .any(|block| block.route as usize != route)
            {
                return Err(Error::InvalidAnchor);
            }
            ranged_blocks = end;
        }
        if ranged_blocks as usize != self.blocks.len() {
            return Err(Error::InvalidAnchor);
        }
        for (block_index, block) in self.blocks.iter().enumerate() {
            if block.logical_count == 0 || block.logical_count > METAL_GRID_BLOCK_WIDTH {
                return Err(Error::InvalidAnchor);
            }
            let spans = self
                .block_spans
                .get(
                    block.block_span_start as usize
                        ..(block.block_span_start + block.block_span_count) as usize,
                )
                .ok_or(Error::InvalidAnchor)?;
            let mut lane = 0_u32;
            for span in spans {
                if span.execution_block as usize != block_index {
                    return Err(Error::InvalidAnchor);
                }
                if span.block_lane_start != lane {
                    return Err(Error::InvalidAnchor);
                }
                let evaluation = self.evaluation_spans[span.evaluation_span as usize];
                let shape = shapes
                    .get(evaluation.program as usize)
                    .ok_or(Error::InvalidAnchor)?;
                if evaluation.route != block.route
                    || shape.dispatch_signature != block.dispatch_signature
                {
                    return Err(Error::InvalidAnchor);
                }
                lane += span.root_count;
            }
            if lane != block.logical_count {
                return Err(Error::InvalidAnchor);
            }
        }
        Ok(())
    }
}

fn checked_u32(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}

fn validate_root_lists<R: AsRef<[u32]>>(lists: &[R]) -> Result<(), Error> {
    for roots in lists {
        if roots.as_ref().windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Error::InvalidAnchor);
        }
    }
    Ok(())
}

fn root_list_hash(roots: &[u32]) -> u64 {
    roots.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, root| {
        (hash ^ u64::from(*root)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn unique_root_lists<'a, R: AsRef<[u32]>>(lists: &'a [R], hashes: &[u64]) -> Vec<&'a [u32]> {
    let mut candidates = HashMap::<u64, Vec<usize>>::new();
    let mut unique = Vec::new();
    for (index, roots) in lists.iter().enumerate() {
        let hash = hashes[index];
        if candidates.get(&hash).is_some_and(|previous| {
            previous
                .iter()
                .any(|other| lists[*other].as_ref() == roots.as_ref())
        }) {
            continue;
        }
        candidates.entry(hash).or_default().push(index);
        unique.push(roots.as_ref());
    }
    unique
}

fn merge_root_slices(lists: &[&[u32]]) -> Vec<u32> {
    let mut heap = BinaryHeap::<Reverse<(u32, usize, usize)>>::new();
    for (list, roots) in lists.iter().enumerate() {
        if let Some(root) = roots.first() {
            heap.push(Reverse((*root, list, 0)));
        }
    }
    let mut merged = Vec::new();
    while let Some(Reverse((root, list, offset))) = heap.pop() {
        if merged.last() != Some(&root) {
            merged.push(root);
        }
        let next = offset + 1;
        if let Some(root) = lists[list].get(next) {
            heap.push(Reverse((*root, list, next)));
        }
    }
    merged
}

#[cfg(test)]
fn merge_roots(lists: &[Vec<u32>]) -> Vec<u32> {
    merge_root_slices(&lists.iter().map(Vec::as_slice).collect::<Vec<_>>())
}

fn roots_to_ordinals(roots: &[u32], universe: &[u32], base: u32) -> Result<Vec<u32>, Error> {
    let mut ordinals = Vec::with_capacity(roots.len());
    let mut universe_offset = 0_usize;
    for root in roots {
        while universe
            .get(universe_offset)
            .is_some_and(|candidate| candidate < root)
        {
            universe_offset += 1;
        }
        if universe.get(universe_offset) != Some(root) {
            return Err(Error::InvalidAnchor);
        }
        ordinals.push(
            base.checked_add(checked_u32(universe_offset)?)
                .ok_or(Error::BackendCapacity)?,
        );
    }
    Ok(ordinals)
}

fn encode_selector(ordinals: &[u32]) -> Result<SelectorEncoding, Error> {
    let first = *ordinals.first().ok_or(Error::InvalidAnchor)?;
    let last = *ordinals.last().ok_or(Error::InvalidAnchor)?;
    if last - first + 1 == ordinals.len() as u32 {
        return Ok(SelectorEncoding {
            kind: SELECTOR_RANGE,
            data: vec![first],
            root_count: checked_u32(ordinals.len())?,
        });
    }

    let mut runs = Vec::new();
    let mut run_start = first;
    let mut run_offset = 0_u32;
    let mut previous = first;
    for ordinal in &ordinals[1..] {
        if *ordinal != previous + 1 {
            let run_count = previous - run_start + 1;
            runs.extend([run_start, run_count, run_offset]);
            run_offset += run_count;
            run_start = *ordinal;
        }
        previous = *ordinal;
    }
    runs.extend([run_start, previous - run_start + 1, run_offset]);

    let explicit = ordinals.to_vec();
    let (mut kind, mut data) = [(SELECTOR_RUNS, runs), (SELECTOR_EXPLICIT, explicit)]
        .into_iter()
        .min_by_key(|(kind, data)| (data.len(), *kind))
        .ok_or(Error::BackendFailure)?;
    let bitmap_words = ((u64::from(last) - u64::from(first)) / 32 + 1) as usize;
    let blocks = bitmap_words.div_ceil(32);
    if 2 + blocks + bitmap_words < data.len() {
        let mut bitmap = vec![0u32; bitmap_words];
        for &ordinal in ordinals {
            let bit = ordinal - first;
            bitmap[bit as usize / 32] |= 1 << (bit % 32);
        }
        data.clear();
        data.push(first);
        data.push(checked_u32(blocks)?);
        let mut rank = 0u32;
        for block in bitmap.chunks(32) {
            data.push(rank);
            rank += block.iter().map(|word| word.count_ones()).sum::<u32>();
        }
        data.extend(bitmap);
        kind = SELECTOR_BITS;
    }
    Ok(SelectorEncoding {
        kind,
        data,
        root_count: checked_u32(ordinals.len())?,
    })
}

pub(super) fn append_encoded_selector(
    ordinals: &[u32],
    data: &mut Vec<u32>,
) -> Result<RootSelector, Error> {
    let encoding = encode_selector(ordinals)?;
    let selector = RootSelector {
        kind: encoding.kind,
        file: 0,
        data_start: checked_u32(data.len())?,
        data_count: checked_u32(encoding.data.len())?,
        root_count: encoding.root_count,
        reserved: 0,
    };
    data.extend(encoding.data);
    Ok(selector)
}

fn intern_selector(
    grid: &mut MetalExecutionGrid,
    candidates: &mut HashMap<u64, Vec<u32>>,
    file: u32,
    encoding: SelectorEncoding,
) -> Result<u32, Error> {
    let mut hasher = DefaultHasher::new();
    file.hash(&mut hasher);
    encoding.kind.hash(&mut hasher);
    encoding.root_count.hash(&mut hasher);
    encoding.data.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some(indexes) = candidates.get(&hash) {
        for index in indexes {
            let old = grid.selectors[*index as usize];
            let old_data = &grid.selector_data
                [old.data_start as usize..(old.data_start + old.data_count) as usize];
            if old.file == file
                && old.kind == encoding.kind
                && old.root_count == encoding.root_count
                && old_data == encoding.data
            {
                return Ok(*index);
            }
        }
    }
    let index = checked_u32(grid.selectors.len())?;
    let data_start = checked_u32(grid.selector_data.len())?;
    grid.selector_data.extend_from_slice(&encoding.data);
    grid.selectors.push(RootSelector {
        kind: encoding.kind,
        file,
        data_start,
        data_count: checked_u32(encoding.data.len())?,
        root_count: encoding.root_count,
        reserved: 0,
    });
    candidates.entry(hash).or_default().push(index);
    Ok(index)
}

fn decode_selector(selector: RootSelector, data: &[u32], offset: u32) -> Result<u32, Error> {
    match selector.kind {
        SELECTOR_RANGE => data
            .first()
            .and_then(|first| first.checked_add(offset))
            .ok_or(Error::InvalidAnchor),
        SELECTOR_RUNS => {
            let run_count = data.len() / 3;
            let mut low = 0_usize;
            let mut high = run_count;
            while low < high {
                let middle = (low + high) / 2;
                if data[middle * 3 + 2] <= offset {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let run = low.checked_sub(1).ok_or(Error::InvalidAnchor)?;
            let start = data[run * 3];
            let count = data[run * 3 + 1];
            let prefix = data[run * 3 + 2];
            let relative = offset.checked_sub(prefix).ok_or(Error::InvalidAnchor)?;
            if relative >= count {
                return Err(Error::InvalidAnchor);
            }
            start.checked_add(relative).ok_or(Error::BackendCapacity)
        }
        SELECTOR_BITS => {
            let base = *data.first().ok_or(Error::InvalidAnchor)?;
            let blocks = *data.get(1).ok_or(Error::InvalidAnchor)? as usize;
            if blocks == 0 {
                return Err(Error::InvalidAnchor);
            }
            let ranks = data.get(2..2 + blocks).ok_or(Error::InvalidAnchor)?;
            let bitmap = data.get(2 + blocks..).ok_or(Error::InvalidAnchor)?;
            if bitmap.len() > blocks * 32 || bitmap.len() <= (blocks - 1) * 32 {
                return Err(Error::InvalidAnchor);
            }
            let block = ranks
                .partition_point(|&rank| rank <= offset)
                .checked_sub(1)
                .ok_or(Error::InvalidAnchor)?;
            let mut remaining = offset - ranks[block];
            for (local, &word) in bitmap[block * 32..bitmap.len().min((block + 1) * 32)]
                .iter()
                .enumerate()
            {
                let count = word.count_ones();
                if remaining < count {
                    let mut bits = word;
                    for _ in 0..remaining {
                        bits &= bits - 1;
                    }
                    return base
                        .checked_add(((block * 32 + local) * 32) as u32 + bits.trailing_zeros())
                        .ok_or(Error::BackendCapacity);
                }
                remaining -= count;
            }
            Err(Error::InvalidAnchor)
        }
        SELECTOR_EXPLICIT => data
            .get(offset as usize)
            .copied()
            .ok_or(Error::InvalidAnchor),
        _ => Err(Error::InvalidAnchor),
    }
}

const _: [(); 32] = [(); std::mem::size_of::<MetalGridHeader>()];
const _: [(); 12] = [(); std::mem::size_of::<RootRecord>()];
const _: [(); 24] = [(); std::mem::size_of::<RootSelector>()];
const _: [(); 36] = [(); std::mem::size_of::<EvaluationSpan>()];
const _: [(); 24] = [(); std::mem::size_of::<BlockSpan>()];
const _: [(); 8] = [(); std::mem::size_of::<ExecutionRange>()];
const _: [(); 24] = [(); std::mem::size_of::<ExecutionBlock>()];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_child_proofs_preserve_unique_and_repeated_roots_on_every_platform() {
        use super::super::{CAPTURE, CHILD, EMIT, KIND, NAMED, word};
        use crate::policy_dsl::tape::{Grammar, TapeImage};
        use ast_grep_core::tree_sitter::StrDoc;
        use ast_grep_language::{LanguageExt, SupportLang};
        let source = "def f():\n    pass\nf(a, b)\n";
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let file =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let language = SupportLang::Python.get_ts_language();
        let unique = Program::compile_tree_sitter_query_pattern(
            "(function_definition body: (block) @body)",
            &language,
        )
        .unwrap();
        let repeated = Program::new(
            vec![
                word(KIND, 0, language.id_for_node_kind("argument_list", true)),
                word(CHILD, NAMED, 0),
                word(KIND, 0, language.id_for_node_kind("identifier", true)),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"name".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [unique, repeated];
        let roots = ["function_definition", "argument_list"].map(|kind| {
            let kind = language.id_for_node_kind(kind, true);
            (0..file.transducer_len() as u32)
                .filter(|&node| file.transducer_kind(node) == kind)
                .collect::<Vec<_>>()
        });
        assert!(roots.iter().all(|roots| roots.len() == 1));
        let input = MetalGridInput::build(&programs, &[file], &[roots.to_vec()]).unwrap();
        let shapes = input.program_shapes(&programs).unwrap();
        assert_ne!(shapes[0].dispatch_signature & 0x80000000, 0);
        assert_eq!(shapes[1].dispatch_signature & 0x80000000, 0);
        assert_eq!(input.grid.header.semantic_tuple_count, 2);
        for (program, root) in roots.iter().enumerate() {
            let span = input
                .grid
                .evaluation_spans
                .iter()
                .find(|span| span.program == program as u32)
                .unwrap();
            let ordinal = input.grid.selector_root(span.selector, 0).unwrap();
            assert_eq!(input.grid.roots[ordinal as usize].node, root[0]);
        }
        let copied = Arc::new((*input.packed).clone());
        assert!(matches!(
            input.children.validate(&copied),
            Err(Error::InvalidAnchor)
        ));
    }

    fn shape(signature: u32) -> GridProgramShape {
        GridProgramShape {
            signature,
            class: signature % 3,
            route: signature % EXECUTION_ROUTE_COUNT as u32,
            dispatch_signature: signature,
        }
    }

    #[test]
    fn semantic_windows_select_only_intersecting_signature_blocks() {
        let grid = MetalExecutionGrid::build(
            &[
                vec![
                    (0..600).collect(),
                    (0..600).step_by(2).collect(),
                    vec![7, 11, 19],
                ],
                vec![vec![2, 4, 8], vec![2, 4, 8], vec![]],
            ],
            &[shape(1), shape(1), shape(2)],
        )
        .unwrap();
        for first in 0..=grid.header.semantic_tuple_count {
            for length in [0, 1, 17, 256, 800] {
                let last = first
                    .saturating_add(length)
                    .min(grid.header.semantic_tuple_count);
                let selected = grid.block_indices_for_range(first..last).unwrap();
                let expected = grid
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| {
                        first < last
                            && grid.block_spans[block.block_span_start as usize
                                ..(block.block_span_start + block.block_span_count) as usize]
                                .iter()
                                .any(|span| {
                                    span.semantic_start < last
                                        && span.semantic_start + span.root_count > first
                                })
                    })
                    .map(|(index, _)| index as u32)
                    .collect::<Vec<_>>();
                assert_eq!(selected, expected, "{first}..{last}");
            }
        }
        assert!(
            grid.block_indices_for_range(std::ops::Range { start: 2, end: 1 })
                .is_err()
        );
        assert!(
            grid.block_indices_for_range(0..grid.header.semantic_tuple_count + 1)
                .is_err()
        );
    }

    #[test]
    fn grid_preserves_every_semantic_tuple_without_tuple_records() {
        let inputs = vec![
            vec![
                (0..600).collect::<Vec<_>>(),
                (0..600).step_by(2).collect::<Vec<_>>(),
                vec![7, 11, 19],
            ],
            vec![vec![2, 4, 8], vec![2, 4, 8], vec![]],
        ];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(1), shape(1), shape(2)]).unwrap();
        assert_eq!(grid.header.root_count, 603);
        assert_eq!(grid.header.semantic_tuple_count, 909);
        assert_eq!(grid.header.evaluation_span_count, 5);
        assert_eq!(
            grid.selectors.len(),
            4,
            "equal selectors are interned per file"
        );
        assert!(grid.blocks.iter().all(|block| block.logical_count <= 256));
        assert_eq!(
            grid.route_ranges
                .iter()
                .map(|range| range.block_count)
                .sum::<u32>(),
            grid.header.block_count
        );
        assert!(grid.blocks.len() < grid.header.semantic_tuple_count as usize / 8);

        let mut actual = Vec::new();
        for semantic_span in &grid.semantic_span_order {
            let block_span = grid.block_spans[*semantic_span as usize];
            let evaluation = grid.evaluation_spans[block_span.evaluation_span as usize];
            for offset in 0..block_span.root_count {
                let root = grid
                    .selector_root(evaluation.selector, block_span.selector_offset + offset)
                    .unwrap();
                actual.push((evaluation.file, evaluation.program, root));
            }
        }
        let mut expected = Vec::new();
        let mut file_bases = Vec::new();
        let mut base = 0_u32;
        for programs in &inputs {
            file_bases.push(base);
            base += merge_roots(programs).len() as u32;
        }
        for (file, programs) in inputs.iter().enumerate() {
            let universe = merge_roots(programs);
            for (program, roots) in programs.iter().enumerate() {
                for root in roots {
                    let ordinal = file_bases[file] + universe.binary_search(root).unwrap() as u32;
                    expected.push((file as u32, program as u32, ordinal));
                }
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn selectors_choose_bounded_random_access_encodings() {
        let range = encode_selector(&[10, 11, 12, 13]).unwrap();
        assert_eq!(range.kind, SELECTOR_RANGE);
        let runs = encode_selector(&[0, 1, 2, 3, 4, 5, 100, 101, 102, 103, 104, 105]).unwrap();
        assert_eq!(runs.kind, SELECTOR_RUNS);
        let explicit = encode_selector(&[0, 1_000, 2_000]).unwrap();
        assert_eq!(explicit.kind, SELECTOR_EXPLICIT);
        let roots = (0..5000).filter(|index| index % 3 != 1).collect::<Vec<_>>();
        let mut data = Vec::new();
        let bits = append_encoded_selector(&roots, &mut data).unwrap();
        assert_eq!(bits.kind, SELECTOR_BITS);
        assert!(data.len() * 4 < roots.len() * 4);
        for (offset, &root) in roots.iter().enumerate() {
            assert_eq!(decode_selector(bits, &data, offset as u32).unwrap(), root);
        }
        for malformed in [&data[..1], &data[..4], &[0, 0][..]] {
            assert!(decode_selector(bits, malformed, 0).is_err());
        }
    }

    #[test]
    fn execution_signatures_include_predicate_graph_and_output_shape() {
        use ast_grep_language::{LanguageExt, SupportLang};
        let language = SupportLang::Python.get_ts_language();
        let queries = [
            r#"((identifier) @name (#set! role "name"))"#,
            r#"((identifier) @name (#set! @name role "name"))"#,
            r#"((identifier) @name (#match? @name "^a"))"#,
            r#"((identifier) @name (#match? @name "^z"))"#,
            r#"((identifier) @name (#not-match? @name "^a"))"#,
        ];
        let programs = queries
            .iter()
            .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        let shapes = compile_program_shapes(&programs);
        assert_ne!(shapes[0].signature, shapes[1].signature);
        assert_eq!(shapes[2].signature, shapes[3].signature);
        assert_ne!(shapes[2].signature, shapes[4].signature);
        use super::super::value::{ByteNfa, ValueProgram};
        let left = ValueProgram::Regex(ByteNfa::compile("a|b").unwrap());
        let right = ValueProgram::Regex(ByteNfa::compile("a+").unwrap());
        assert_ne!(
            super::super::execution_shape::value_execution_signature(&left),
            super::super::execution_shape::value_execution_signature(&right)
        );
        use super::super::{
            EMIT, NEXT, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR, SAVE_CURSOR, word,
        };
        let cursors = [0, 1].map(|restore| {
            Program::new(
                vec![
                    word(SAVE_CURSOR, 0, 0),
                    word(NEXT, 1, 0),
                    word(SAVE_CURSOR, 0, 1),
                    word(RESTORE_CURSOR, 0, restore),
                    word(REPEAT_BEGIN, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(REPEAT_END, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"match".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        });
        let entries = compile_entry_shapes(&cursors);
        assert_ne!(entries[0].signature, entries[1].signature);
    }

    #[test]
    fn malformed_roots_and_selector_ranges_fail_closed() {
        assert!(matches!(
            MetalExecutionGrid::build(&[vec![vec![2, 1]]], &[shape(0)]),
            Err(Error::InvalidAnchor)
        ));
        let grid = MetalExecutionGrid::build(&[vec![vec![1, 2, 3]]], &[shape(0)]).unwrap();
        assert!(matches!(
            grid.selector_root(0, 3),
            Err(Error::InvalidAnchor)
        ));
        assert!(matches!(
            grid.selector_root(1, 0),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    fn one_route_keeps_exact_signatures_in_separate_threadgroups() {
        let anchors = vec![vec![vec![0], vec![1], vec![2]]];
        let shapes = vec![
            GridProgramShape {
                signature: 41,
                class: 2,
                route: 4,
                dispatch_signature: 17,
            },
            GridProgramShape {
                signature: 73,
                class: 4,
                route: 4,
                dispatch_signature: 73,
            },
            GridProgramShape {
                signature: 41,
                class: 2,
                route: 4,
                dispatch_signature: 17,
            },
        ];
        let grid = MetalExecutionGrid::build(&anchors, &shapes).unwrap();
        assert_eq!(grid.blocks.len(), 2);
        assert_eq!(grid.blocks[0].dispatch_signature, 17);
        assert_eq!(grid.blocks[0].logical_count, 2);
        assert_eq!(grid.blocks[1].dispatch_signature, 73);
        assert_eq!(grid.blocks[1].logical_count, 1);
        let ordinals = grid
            .semantic_span_order
            .iter()
            .map(|physical| grid.block_spans[*physical as usize].semantic_start)
            .collect::<Vec<_>>();
        assert_eq!(ordinals, [0, 1, 2]);
        let mut malformed = grid;
        malformed.blocks[0].dispatch_signature = 73;
        assert!(matches!(
            malformed.validate(&shapes),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    fn large_evaluation_splits_only_at_actual_block_boundaries() {
        let inputs = vec![vec![
            (0..3).collect::<Vec<_>>(),
            (0..1024).collect::<Vec<_>>(),
        ]];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(4), shape(4)]).unwrap();
        assert_eq!(grid.header.semantic_tuple_count, 1027);
        assert_eq!(grid.blocks.len(), 5);
        assert_eq!(grid.block_spans.len(), 6);
        assert_eq!(
            grid.semantic_span_order
                .iter()
                .map(|&index| grid.block_spans[index as usize].semantic_start)
                .collect::<Vec<_>>(),
            vec![0, 3, 256, 512, 768, 1024]
        );
    }

    #[test]
    fn metadata_scales_with_spans_and_blocks_instead_of_tuples() {
        let roots = (0..100_000).collect::<Vec<_>>();
        let inputs = vec![vec![roots.clone(), roots.clone(), roots]];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(4), shape(4), shape(4)]).unwrap();
        assert_eq!(grid.header.semantic_tuple_count, 300_000);
        assert_eq!(grid.selectors.len(), 1);
        assert!(grid.metadata_bytes() < 90_000);
    }
}
