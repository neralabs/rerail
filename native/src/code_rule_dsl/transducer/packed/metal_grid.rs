use super::execution_shape::{compile_entry_shapes, execution_class};
use super::packed_batch::PackedBatch;
use super::{Error, Program};
use crate::code_rule_dsl::tape::TapeDoc;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

pub const METAL_GRID_FORMAT_VERSION: u32 = 1;
pub(super) const METAL_GRID_BLOCK_WIDTH: u32 = 256;
pub const EXECUTION_ROUTE_COUNT: usize = 11;

pub const SELECTOR_RANGE: u32 = 1;
pub const SELECTOR_RUNS: u32 = 2;
pub const SELECTOR_BITS: u32 = 3;
pub const SELECTOR_EXPLICIT: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetalGridHeader {
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
pub struct RootRecord {
    pub file: u32,
    pub node: u32,
    pub semantic_root_ordinal: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RootSelector {
    pub kind: u32,
    pub file: u32,
    pub data_start: u32,
    pub data_count: u32,
    pub root_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvaluationSpan {
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
pub struct BlockSpan {
    pub evaluation_span: u32,
    pub execution_block: u32,
    pub selector_offset: u32,
    pub root_count: u32,
    pub block_lane_start: u32,
    pub semantic_start: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionRange {
    pub block_start: u32,
    pub block_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionBlock {
    pub block_span_start: u32,
    pub block_span_count: u32,
    pub dispatch_signature: u32,
    pub route: u32,
    pub logical_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GridProgramShape {
    pub signature: u32,
    pub class: u32,
    pub route: u32,
    pub dispatch_signature: u32,
}

pub fn compile_program_shapes(programs: &[Program]) -> Vec<GridProgramShape> {
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

pub struct MetalGridInput {
    pub packed: Arc<PackedBatch>,
    pub grid: MetalExecutionGrid,
    pub children: super::metal_child_index::ChildIndex,
}

impl MetalGridInput {
    pub fn program_shapes(
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
    pub fn build_streamed_with_upload(
        programs: &[Program],
        files: &[crate::code_rule_dsl::tape::TapeSource],
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

    pub fn build(
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
    pub fn build_shared(
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
pub struct MetalExecutionGrid {
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
pub struct SelectorEncoding {
    pub kind: u32,
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
    pub fn block_indices_for_range(
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

    pub fn build(
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

    pub fn selector_root(&self, selector: u32, offset: u32) -> Result<u32, Error> {
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

    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of_val(self.selectors.as_slice())
            + std::mem::size_of_val(self.evaluation_spans.as_slice())
            + std::mem::size_of_val(self.block_spans.as_slice())
            + std::mem::size_of_val(self.blocks.as_slice())
            + std::mem::size_of_val(self.semantic_span_order.as_slice())
    }

    pub fn validate(&self, shapes: &[GridProgramShape]) -> Result<(), Error> {
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

#[cfg(any(test, feature = "test-hooks"))]
pub fn merge_roots(lists: &[Vec<u32>]) -> Vec<u32> {
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

pub fn encode_selector(ordinals: &[u32]) -> Result<SelectorEncoding, Error> {
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

pub fn append_encoded_selector(
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

pub fn decode_selector(selector: RootSelector, data: &[u32], offset: u32) -> Result<u32, Error> {
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

// cratify: `tests` is rerail-engine's (it mounts it)
