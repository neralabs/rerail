//! Construct one shared execution input and enter the sole broad-stage executor.
use super::metal_backend::launch::LaunchScope;
use super::metal_grid::MetalGridInput;
use super::metal_grid_executor::{GuardedExecutor, PageConsumer};
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::*;
use crate::policy_dsl::tape::TapeSource;

fn executor(
    input: MetalGridInput,
    programs: &[Program],
    limits: Option<&[u32]>,
) -> Result<GuardedExecutor, Error> {
    let resident = ResidentInput::new(input, limits)?;
    let plan = ExecutionPlan::new(&resident, programs)?;
    GuardedExecutor::new(resident, plan)
}

pub(super) fn execute_packed_with_result_limits(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<PackedExecution, Error> {
    let _evaluation = LaunchScope::enter();
    validate_dimensions(
        programs.len(),
        files.len(),
        anchors.iter().map(Vec::len),
        result_limits,
    )?;
    executor(
        MetalGridInput::build(programs, files, anchors)?,
        programs,
        result_limits,
    )?
    .execute()
}

pub(super) fn execute_outcomes(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<BatchOutcome, Error> {
    execute_packed_with_result_limits(programs, files, anchors, None)?.into_outcome()
}

#[cfg(test)]
pub(super) fn execute(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_outcomes(programs, files, anchors)?.into_result()
}

pub(super) fn execute_shared_corpus(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Arc<[u32]>>],
    result_limits: &[u32],
) -> Result<PackedExecution, Error> {
    let _evaluation = LaunchScope::enter();
    validate_dimensions(
        programs.len(),
        files.len(),
        anchors.iter().map(Vec::len),
        Some(result_limits),
    )?;
    executor(
        MetalGridInput::build_shared(programs, files, anchors)?,
        programs,
        Some(result_limits),
    )?
    .execute()
}

pub(super) fn execute_shared_corpus_pages(
    programs: &[Program],
    files: &[TapeSource],
    anchors: &[Vec<Arc<[u32]>>],
    result_limits: &[u32],
    consumer: &mut PageConsumer<'_>,
) -> Result<(), Error> {
    let _evaluation = LaunchScope::enter();
    validate_dimensions(
        programs.len(),
        files.len(),
        anchors.iter().map(Vec::len),
        Some(result_limits),
    )?;
    let input = MetalGridInput::build_streamed_with_upload(programs, files, anchors, |_| Ok(()))?;
    executor(input, programs, Some(result_limits))?.execute_pages(consumer)
}

fn validate_dimensions(
    programs: usize,
    files: usize,
    anchors: impl ExactSizeIterator<Item = usize>,
    limits: Option<&[u32]>,
) -> Result<(), Error> {
    if anchors.len() != files
        || anchors.into_iter().any(|count| count != programs)
        || limits.is_some_and(|limits| limits.len() != programs)
    {
        return Err(Error::InvalidAnchor);
    }
    Ok(())
}
