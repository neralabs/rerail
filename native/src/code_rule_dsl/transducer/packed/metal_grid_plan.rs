//! Broad stages over resident input. No stage owns a private scheduler.
use super::metal_grid::{MetalExecutionGrid, MetalGridInput};
use super::{Error, Program};
use std::ops::Range;

pub(super) const THREADGROUP_WIDTH: u32 = 256;
pub(super) const CHECKED_FRONTIER_WAVES: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StageKind {
    ClassifyRoots,
    CountDirectBlocks,
    CountBoundedBlocks,
    CountRepeatedBlocks,
    SeedDynamicFrontier,
    ScheduleDynamicFrontier,
    AdvanceDynamicFrontier,
    CompactPredicates,
    ReduceFailures,
    ScanSemanticCounts,
    SelectOutputPage,
    ScatterTypedPage,
}

impl StageKind {
    pub(super) fn unimplemented(self) -> Error {
        Error::UnsupportedFeature(match self {
            Self::ClassifyRoots => "Metal root classification is not implemented",
            Self::CountDirectBlocks => "Metal direct block counts are not implemented",
            Self::CountBoundedBlocks => "Metal bounded block counts are not implemented",
            Self::CountRepeatedBlocks => "Metal repeated block counts are not implemented",
            Self::SeedDynamicFrontier => "Metal compact frontier seeds are not implemented",
            Self::ScheduleDynamicFrontier => "Metal ready-batch scheduling is not implemented",
            Self::AdvanceDynamicFrontier => {
                "Metal structural successor execution is not implemented"
            }
            Self::CompactPredicates => "Metal shared predicate frontiers are not implemented",
            Self::ReduceFailures => "Metal ordered failure reduction is not implemented",
            Self::ScanSemanticCounts => "Metal semantic count scans are not implemented",
            Self::SelectOutputPage => "Metal exact output boundaries are not implemented",
            Self::ScatterTypedPage => "Metal typed page scatter is not implemented",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SignatureBlocks {
    pub signature: u32,
    pub route: u32,
    pub blocks: Range<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorkDomain {
    Roots(u32),
    Blocks(Vec<SignatureBlocks>),
    CompactFrontier { checked_waves: u32 },
    SemanticSpans(u32),
    TypedPages,
}

/// Each reference names an output of an earlier broad stage. Resident input is external.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResourceKind {
    ResidentInput,
    ClassifiedRoots,
    BlockCounts,
    PredicateCounts,
    Candidates,
    StructuralFrontier,
    ReadyBatches,
    PredicateFrontier,
    OutputContinuation,
    AcceptedMatches,
    Failures,
    ReducedFailures,
    SemanticOffsets,
    ExactPageBoundary,
    TypedPage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResourceInput {
    pub kind: ResourceKind,
    pub producer: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrontierKind {
    Structural,
    Predicate,
}

/// The span index follows semantic_span_order, rather than signature block order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OutputBoundary {
    pub semantic_span: u32,
    pub block_lane: u32,
    /// Next rank in saved ordered outputs for this root, distinct from the raw candidate ordinal.
    pub local_output_ordinal: u32,
    pub frontier_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContinuationPoint {
    Frontier { kind: FrontierKind, generation: u64 },
    Output(OutputBoundary),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProgressBoundary {
    None,
    Frontier(FrontierKind),
    Output,
}

impl ProgressBoundary {
    pub(super) fn check_advance(
        self,
        before: ContinuationPoint,
        after: ContinuationPoint,
    ) -> Result<(), Error> {
        let advances = match (self, before, after) {
            (
                Self::Frontier(expected),
                ContinuationPoint::Frontier {
                    kind: old_kind,
                    generation: old,
                },
                ContinuationPoint::Frontier {
                    kind: new_kind,
                    generation: new,
                },
            ) => old_kind == expected && new_kind == expected && new > old,
            (Self::Output, ContinuationPoint::Output(old), ContinuationPoint::Output(new)) => {
                // Output pages must consume an exact prefix without restarting semantic work.
                old.frontier_generation == new.frontier_generation
                    && (new.semantic_span, new.block_lane, new.local_output_ordinal)
                        > (old.semantic_span, old.block_lane, old.local_output_ordinal)
            }
            _ => false,
        };
        if advances {
            Ok(())
        } else {
            Err(Error::BackendFailure)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ContinuationContract {
    pub checkpoint: ResourceKind,
    pub resume_at: StageKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StageContract {
    pub inputs: Vec<ResourceInput>,
    pub outputs: Vec<ResourceKind>,
    pub resume_input: Option<ResourceKind>,
    pub continuation: Option<ContinuationContract>,
    pub progress: ProgressBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlannedStage {
    pub kind: StageKind,
    pub work: WorkDomain,
    pub after: Option<usize>,
    pub contract: StageContract,
}

/// One immutable input for the complete evaluation, independent of publication batches.
pub(super) struct ResidentInput {
    pub input: MetalGridInput,
    pub witness_limits: Vec<u32>,
}

impl ResidentInput {
    pub(super) fn new(input: MetalGridInput, limits: Option<&[u32]>) -> Result<Self, Error> {
        let programs = input.packed.programs.len();
        let witness_limits = match limits {
            Some(limits) if limits.len() == programs => limits.to_vec(),
            Some(_) => return Err(Error::InvalidAnchor),
            None => vec![u32::MAX; programs],
        };
        Ok(Self {
            input,
            witness_limits,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecutionPlan {
    pub stages: Vec<PlannedStage>,
    pub readiness: super::metal_ready_proof::ReadyProof,
}

impl ExecutionPlan {
    pub(super) fn new(resident: &ResidentInput, programs: &[Program]) -> Result<Self, Error> {
        super::metal_grid_contract::validate(resident)?;
        let grid = &resident.input.grid;
        validate_grid(grid)?;
        if !resident.input.packed.tasks.is_empty()
            || resident.input.packed.header.task_count != 0
            || resident.witness_limits.len() != programs.len()
            || resident.input.packed.programs.len() != programs.len()
        {
            return Err(Error::InvalidAnchor);
        }
        let shapes = resident.input.program_shapes(programs)?;
        for block in &grid.blocks {
            let first = block.block_span_start as usize;
            for span in &grid.block_spans[first..first + block.block_span_count as usize] {
                let evaluation = grid.evaluation_spans[span.evaluation_span as usize];
                let shape = shapes
                    .get(evaluation.program as usize)
                    .ok_or(Error::InvalidAnchor)?;
                if shape.dispatch_signature != block.dispatch_signature
                    || shape.route != block.route
                    || shape.signature != evaluation.signature
                    || shape.class != evaluation.class
                {
                    return Err(Error::InvalidAnchor);
                }
            }
        }
        let readiness = super::metal_ready_proof::ReadyProof::new(programs)?;
        readiness.validate(resident)?;
        Self::for_resident(resident, readiness)
    }

    /// Reject incomplete or contradictory plans before the executor acquires the Metal guard.
    pub(super) fn validate(&self, resident: &ResidentInput) -> Result<(), Error> {
        super::metal_grid_contract::validate(resident)?;
        validate_grid(&resident.input.grid)?;
        self.readiness.validate(resident)?;
        if !resident.input.packed.tasks.is_empty()
            || resident.input.packed.header.task_count != 0
            || resident.witness_limits.len() != resident.input.packed.programs.len()
            || *self != Self::for_resident(resident, self.readiness.clone())?
        {
            return Err(Error::InvalidAnchor);
        }
        Ok(())
    }

    fn for_resident(
        resident: &ResidentInput,
        readiness: super::metal_ready_proof::ReadyProof,
    ) -> Result<Self, Error> {
        let grid = &resident.input.grid;
        let mut direct = Vec::new();
        let mut bounded = Vec::new();
        let mut dynamic = Vec::new();
        let mut repeated = Vec::new();
        let register_classes = resident
            .input
            .packed
            .programs
            .iter()
            .map(|program| super::metal_register_codegen::register_class(&resident.input, program))
            .collect::<Result<Vec<_>, _>>()?;
        for (index, block) in grid.blocks.iter().enumerate() {
            let span = grid.block_spans[block.block_span_start as usize];
            let evaluation = grid.evaluation_spans[span.evaluation_span as usize];
            let groups = if readiness.repetitions().contains_key(&evaluation.program) {
                &mut repeated
            } else {
                if evaluation.class > 4 {
                    return Err(Error::InvalidAnchor);
                }
                match register_classes[evaluation.program as usize] {
                    Some(super::metal_register_codegen::RegisterClass::Direct) => &mut direct,
                    Some(super::metal_register_codegen::RegisterClass::Bounded) => &mut bounded,
                    None => &mut dynamic,
                }
            };
            append_group(groups, index as u32, block.dispatch_signature, block.route);
        }
        let mut plan = Self {
            stages: Vec::new(),
            readiness,
        };
        plan.push(
            StageKind::ClassifyRoots,
            WorkDomain::Roots(grid.header.root_count),
        );
        if !direct.is_empty() {
            plan.push(StageKind::CountDirectBlocks, WorkDomain::Blocks(direct));
        }
        if !bounded.is_empty() {
            plan.push(StageKind::CountBoundedBlocks, WorkDomain::Blocks(bounded));
        }
        if !repeated.is_empty() {
            plan.push(StageKind::CountRepeatedBlocks, WorkDomain::Blocks(repeated));
        }
        if !dynamic.is_empty() {
            plan.push(StageKind::SeedDynamicFrontier, WorkDomain::Blocks(dynamic));
            plan.push(
                StageKind::ScheduleDynamicFrontier,
                WorkDomain::CompactFrontier {
                    checked_waves: CHECKED_FRONTIER_WAVES,
                },
            );
            plan.push(
                StageKind::AdvanceDynamicFrontier,
                WorkDomain::CompactFrontier {
                    checked_waves: CHECKED_FRONTIER_WAVES,
                },
            );
        }
        if grid.evaluation_spans.iter().any(|span| {
            let program = &resident.input.packed.programs[span.program as usize];
            program.value_count != 0 || program.capture_count != 0
        }) {
            plan.push(
                StageKind::CompactPredicates,
                WorkDomain::CompactFrontier {
                    checked_waves: CHECKED_FRONTIER_WAVES,
                },
            );
        }
        plan.push(
            StageKind::ReduceFailures,
            WorkDomain::SemanticSpans(grid.header.evaluation_span_count),
        );
        plan.push(
            StageKind::ScanSemanticCounts,
            WorkDomain::SemanticSpans(grid.header.block_span_count),
        );
        plan.push(StageKind::SelectOutputPage, WorkDomain::TypedPages);
        plan.push(StageKind::ScatterTypedPage, WorkDomain::TypedPages);
        Ok(plan)
    }

    fn push(&mut self, kind: StageKind, work: WorkDomain) {
        use ResourceKind::*;
        let (required, outputs, progress): (&[_], &[_], _) = match kind {
            StageKind::ClassifyRoots => {
                (&[ResidentInput], &[ClassifiedRoots], ProgressBoundary::None)
            }
            StageKind::CountDirectBlocks
            | StageKind::CountBoundedBlocks
            | StageKind::CountRepeatedBlocks => (
                &[ResidentInput, ClassifiedRoots],
                &[BlockCounts, Candidates, Failures],
                ProgressBoundary::None,
            ),
            StageKind::SeedDynamicFrontier => (
                &[ResidentInput, ClassifiedRoots],
                &[StructuralFrontier, Failures],
                ProgressBoundary::None,
            ),
            StageKind::ScheduleDynamicFrontier => (
                &[ResidentInput, StructuralFrontier],
                &[ReadyBatches, Failures],
                ProgressBoundary::None,
            ),
            StageKind::AdvanceDynamicFrontier => (
                &[ResidentInput, StructuralFrontier, ReadyBatches],
                &[
                    StructuralFrontier,
                    BlockCounts,
                    Candidates,
                    AcceptedMatches,
                    Failures,
                ],
                ProgressBoundary::Frontier(FrontierKind::Structural),
            ),
            StageKind::CompactPredicates => (
                &[ResidentInput, Candidates, BlockCounts],
                &[AcceptedMatches, PredicateCounts, Failures],
                ProgressBoundary::Frontier(FrontierKind::Predicate),
            ),
            StageKind::ReduceFailures => (&[Failures], &[ReducedFailures], ProgressBoundary::None),
            StageKind::ScanSemanticCounts => (
                &[
                    if self
                        .stages
                        .iter()
                        .any(|stage| stage.kind == StageKind::CompactPredicates)
                    {
                        PredicateCounts
                    } else {
                        BlockCounts
                    },
                    ReducedFailures,
                ],
                &[SemanticOffsets],
                ProgressBoundary::None,
            ),
            StageKind::SelectOutputPage => (
                &[ResidentInput, SemanticOffsets],
                &[ExactPageBoundary],
                ProgressBoundary::None,
            ),
            StageKind::ScatterTypedPage => (
                &[
                    ResidentInput,
                    ReducedFailures,
                    SemanticOffsets,
                    ExactPageBoundary,
                    AcceptedMatches,
                ],
                &[TypedPage],
                ProgressBoundary::Output,
            ),
        };
        let mut inputs = Vec::new();
        for &kind in required {
            if kind == ResidentInput {
                inputs.push(ResourceInput {
                    kind,
                    producer: None,
                });
            } else {
                // Counts and failures from every broad class participate in final reduction.
                for (producer, stage) in self.stages.iter().enumerate() {
                    if stage.contract.outputs.contains(&kind) {
                        inputs.push(ResourceInput {
                            kind,
                            producer: Some(producer),
                        });
                    }
                }
            }
        }
        let resume_input = match kind {
            StageKind::ScheduleDynamicFrontier | StageKind::AdvanceDynamicFrontier => {
                Some(StructuralFrontier)
            }
            StageKind::CompactPredicates => Some(PredicateFrontier),
            StageKind::SelectOutputPage => Some(OutputContinuation),
            _ => None,
        };
        let continuation = match kind {
            StageKind::AdvanceDynamicFrontier => Some(ContinuationContract {
                checkpoint: StructuralFrontier,
                resume_at: StageKind::ScheduleDynamicFrontier,
            }),
            StageKind::CompactPredicates => Some(ContinuationContract {
                checkpoint: PredicateFrontier,
                resume_at: StageKind::CompactPredicates,
            }),
            StageKind::ScatterTypedPage => Some(ContinuationContract {
                checkpoint: OutputContinuation,
                resume_at: StageKind::SelectOutputPage,
            }),
            _ => None,
        };
        let mut outputs = outputs.to_vec();
        if let Some(continuation) = continuation
            && !outputs.contains(&continuation.checkpoint)
        {
            outputs.push(continuation.checkpoint);
        }
        self.stages.push(PlannedStage {
            kind,
            work,
            after: self.stages.len().checked_sub(1),
            contract: StageContract {
                inputs,
                outputs,
                resume_input,
                continuation,
                progress,
            },
        });
    }
}

fn append_group(groups: &mut Vec<SignatureBlocks>, index: u32, signature: u32, route: u32) {
    if let Some(last) = groups.last_mut()
        && last.signature == signature
        && last.route == route
        && last.blocks.end == index
    {
        last.blocks.end += 1;
    } else {
        groups.push(SignatureBlocks {
            signature,
            route,
            blocks: index..index + 1,
        });
    }
}

fn validate_grid(grid: &MetalExecutionGrid) -> Result<(), Error> {
    if grid.header.root_count as usize != grid.roots.len()
        || grid.header.format_version != super::metal_grid::METAL_GRID_FORMAT_VERSION
        || grid.header.selector_count as usize != grid.selectors.len()
        || grid.header.selector_data_count as usize != grid.selector_data.len()
        || grid.header.evaluation_span_count as usize != grid.evaluation_spans.len()
        || grid.header.block_span_count as usize != grid.block_spans.len()
        || grid.header.block_count as usize != grid.blocks.len()
        || grid.semantic_span_order.len() != grid.block_spans.len()
    {
        return Err(Error::InvalidAnchor);
    }
    let mut covered = vec![false; grid.block_spans.len()];
    let mut ordinal = 0_u32;
    for &index in &grid.semantic_span_order {
        let span = grid
            .block_spans
            .get(index as usize)
            .ok_or(Error::InvalidAnchor)?;
        if covered[index as usize] || span.semantic_start != ordinal {
            return Err(Error::InvalidAnchor);
        }
        covered[index as usize] = true;
        ordinal = ordinal
            .checked_add(span.root_count)
            .ok_or(Error::BackendCapacity)?;
    }
    if ordinal != grid.header.semantic_tuple_count {
        return Err(Error::InvalidAnchor);
    }
    let mut covered_spans = 0_usize;
    for (index, block) in grid.blocks.iter().enumerate() {
        let first = block.block_span_start as usize;
        let last = first
            .checked_add(block.block_span_count as usize)
            .ok_or(Error::InvalidAnchor)?;
        if first != covered_spans
            || block.block_span_count == 0
            || !(1..=THREADGROUP_WIDTH).contains(&block.logical_count)
        {
            return Err(Error::InvalidAnchor);
        }
        let spans = grid
            .block_spans
            .get(first..last)
            .ok_or(Error::InvalidAnchor)?;
        let mut lane = 0_u32;
        for span in spans {
            let evaluation = grid
                .evaluation_spans
                .get(span.evaluation_span as usize)
                .ok_or(Error::InvalidAnchor)?;
            if span.execution_block != index as u32
                || span.block_lane_start != lane
                || evaluation.route != block.route
                || span.root_count == 0
            {
                return Err(Error::InvalidAnchor);
            }
            lane = lane
                .checked_add(span.root_count)
                .ok_or(Error::BackendCapacity)?;
        }
        if lane != block.logical_count {
            return Err(Error::InvalidAnchor);
        }
        covered_spans = last;
    }
    if covered_spans != grid.block_spans.len() {
        return Err(Error::InvalidAnchor);
    }
    Ok(())
}
