//! The sole guarded executor for broad stages over shared resident input.
use super::metal_backend::{MetalExecutionGuard, metal_execution_guard};
use super::metal_grid_plan::{ContinuationPoint, ExecutionPlan, ResidentInput, StageKind};
use super::{Error, PackedExecution};

pub(super) type PageConsumer<'a> = dyn FnMut(PackedExecution, usize) -> Result<(), Error> + 'a;

pub(super) struct GuardedExecutor {
    resident: ResidentInput,
    plan: ExecutionPlan,
    partition_span_limit: u32,
    output_bounds: super::metal_output_pages::PageBounds,
    _guard: MetalExecutionGuard,
}

impl GuardedExecutor {
    pub(super) fn new(resident: ResidentInput, plan: ExecutionPlan) -> Result<Self, Error> {
        plan.validate(&resident)?;
        Ok(Self {
            resident,
            plan,
            partition_span_limit: u32::MAX,
            output_bounds: super::metal_output_pages::PageBounds::default(),
            _guard: metal_execution_guard()?,
        })
    }

    /// A smaller scratch capacity can partition execution, but never immutable input.
    pub(super) fn with_partition_span_limit(mut self, limit: u32) -> Result<Self, Error> {
        if limit == 0 {
            return Err(Error::BackendCapacity);
        }
        self.partition_span_limit = limit;
        Ok(self)
    }

    #[cfg(test)]
    pub(super) fn with_output_bounds(
        mut self,
        bounds: super::metal_output_pages::PageBounds,
    ) -> Self {
        self.output_bounds = bounds;
        self
    }

    pub(super) fn execute(self) -> Result<PackedExecution, Error> {
        super::metal_register_execution::execute(
            &self.resident,
            &self.plan,
            self.partition_span_limit,
        )?
        .with_bounds(self.output_bounds)?
        .execute()
    }

    pub(super) fn execute_pages(self, consumer: &mut PageConsumer<'_>) -> Result<(), Error> {
        super::metal_register_execution::execute(
            &self.resident,
            &self.plan,
            self.partition_span_limit,
        )?
        .with_bounds(self.output_bounds)?
        .execute_pages(consumer)
    }

    /// Check semantic progress and return the broad stage that consumes the checkpoint.
    pub(super) fn check_continuation(
        &self,
        stage_index: usize,
        before: ContinuationPoint,
        after: ContinuationPoint,
    ) -> Result<StageKind, Error> {
        let stage = self
            .plan
            .stages
            .get(stage_index)
            .ok_or(Error::InvalidAnchor)?;
        for point in [before, after] {
            if let ContinuationPoint::Output(boundary) = point {
                let grid = &self.resident.input.grid;
                let index = boundary.semantic_span as usize;
                if index == grid.semantic_span_order.len() {
                    if boundary.block_lane != 0 || boundary.local_output_ordinal != 0 {
                        return Err(Error::InvalidAnchor);
                    }
                } else {
                    let span = grid
                        .semantic_span_order
                        .get(index)
                        .and_then(|&span| grid.block_spans.get(span as usize))
                        .ok_or(Error::InvalidAnchor)?;
                    if boundary.block_lane < span.block_lane_start
                        || boundary.block_lane - span.block_lane_start >= span.root_count
                    {
                        return Err(Error::InvalidAnchor);
                    }
                }
            }
        }
        stage.contract.progress.check_advance(before, after)?;
        stage
            .contract
            .continuation
            .map(|continuation| continuation.resume_at)
            .ok_or(Error::BackendFailure)
    }
}
