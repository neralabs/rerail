//! Shared terminal capture comparisons, before transforms and accepted limits.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_capture_arguments::{ArgumentInput, QueryArguments, direct};
use super::metal_grid_plan::{ResidentInput, StageKind};
use super::metal_output_dispatch::OperatorInput;
use metal::Device;
use std::rc::Rc;

pub(super) fn validate(resident: &ResidentInput) -> Result<(), Error> {
    let batch = &resident.input.packed;
    for span in &resident.input.grid.evaluation_spans {
        let pg = batch.programs[span.program as usize];
        let shared = || StageKind::CompactPredicates.unimplemented();
        let values =
            &batch.values[pg.value_start as usize..(pg.value_start + pg.value_count) as usize];
        if values.iter().any(|value| !matches!(value.kind, 0..=3)) {
            return Err(shared());
        }
    }
    Ok(())
}

/// Immutable code and source-derived tables have one owner per complete evaluation.
pub(super) struct PredicatePrograms<'a> {
    resident: &'a ResidentInput,
    data: &'a MappedBuffer,
    regex: Option<Rc<super::metal_regex_frontier::RegexProgram>>,
    kinds: Option<Rc<super::metal_capture_groups::CaptureKinds>>,
    query_general: Vec<u8>,
    query_regex: Vec<bool>,
    inline_general: u8,
    inline_regex: bool,
}
struct PredicateFeatures {
    query_general: Vec<u8>,
    query_regex: Vec<bool>,
    inline_general: u8,
    inline_regex: bool,
    output_regex: bool,
}
impl<'a> PredicatePrograms<'a> {
    fn features(
        resident: &ResidentInput,
        active: &std::collections::BTreeSet<u32>,
    ) -> Result<PredicateFeatures, Error> {
        let batch = &resident.input.packed;
        if active.iter().any(|&id| id as usize >= batch.programs.len()) {
            return Err(Error::BackendFailure);
        }
        let steps = active
            .iter()
            .map(|&id| {
                let pg = batch.programs[id as usize];
                if pg.flags & 1 != 0 {
                    pg.capture_count as usize
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0);
        let mut query_general = vec![0u8; steps];
        let mut query_regex = vec![false; steps];
        let mut inline_general = 0;
        let mut inline_regex = false;
        let mut output_regex = false;
        let value_feature = |pg: super::metal_batch_abi::MetalProgram, index: u32| {
            let value = batch.values[(pg.value_start + index) as usize];
            (
                u8::from(
                    value.kind <= 1
                        && batch.strings[(pg.string_start + value.operand) as usize].byte_length
                            > 64,
                ),
                value.kind >= 2,
            )
        };
        let assertion_feature = |pg: super::metal_batch_abi::MetalProgram,
                                 a: &super::packed_batch::AssertionRecord|
         -> Result<(u8, bool), Error> {
            Ok(match a.kind {
                0 | 2 | 4 | 6 => (1, false),
                3 | 5 => (2, false),
                1 => value_feature(pg, a.second),
                _ => return Err(Error::BackendFailure),
            })
        };
        for &id in active {
            let pg = batch.programs[id as usize];
            if pg.flags & 1 != 0 {
                for (step, a) in batch.assertions
                    [pg.capture_start as usize..(pg.capture_start + pg.capture_count) as usize]
                    .iter()
                    .enumerate()
                {
                    let (g, r) = assertion_feature(pg, a)?;
                    query_general[step] |= g;
                    query_regex[step] |= r;
                }
            }
            output_regex |= batch.assertion_data
                [pg.output_start as usize..pg.output_start as usize + pg.output_count as usize * 6]
                .chunks_exact(6)
                .any(|op| op[0] == 3);
            for &word in &batch.words
                [pg.structural_start as usize..(pg.structural_start + pg.structural_count) as usize]
            {
                let (op, _, arg) = super::split(word);
                let feature = if op == super::TEXT {
                    Some(value_feature(pg, u32::from(arg)))
                } else if op == super::ASSERT && pg.flags & 1 == 0 {
                    Some(assertion_feature(
                        pg,
                        &batch.assertions[(pg.capture_start + u32::from(arg)) as usize],
                    )?)
                } else {
                    None
                };
                if let Some((g, r)) = feature {
                    inline_general |= g;
                    inline_regex |= r;
                }
            }
        }
        Ok(PredicateFeatures {
            query_general,
            query_regex,
            inline_general,
            inline_regex,
            output_regex,
        })
    }

    /// Completed GPU program membership selects work; immutable code retains one owner.
    pub(super) fn scoped(&self, active: &std::collections::BTreeSet<u32>) -> Result<Self, Error> {
        let PredicateFeatures {
            query_general,
            query_regex,
            inline_general,
            inline_regex,
            output_regex,
        } = Self::features(self.resident, active)?;
        if ((output_regex || inline_regex || query_regex.iter().any(|&r| r))
            && self.regex.is_none())
            || ((inline_general & 2 != 0 || query_general.iter().any(|&g| g & 2 != 0))
                && self.kinds.is_none())
        {
            return Err(Error::BackendFailure);
        }
        Ok(Self {
            resident: self.resident,
            data: self.data,
            regex: self.regex.clone(),
            kinds: self.kinds.clone(),
            query_general,
            query_regex,
            inline_general,
            inline_regex,
        })
    }
    pub(super) fn output_regex(&self) -> Result<&super::metal_regex_frontier::RegexProgram, Error> {
        self.regex.as_deref().ok_or(Error::BackendFailure)
    }
    pub(super) fn new(
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &'a ResidentInput,
        data: &'a MappedBuffer,
    ) -> Result<Self, Error> {
        let batch = &resident.input.packed;
        let active = resident
            .input
            .grid
            .evaluation_spans
            .iter()
            .map(|s| s.program)
            .collect::<std::collections::BTreeSet<_>>();
        let PredicateFeatures {
            query_general,
            query_regex,
            inline_general,
            inline_regex,
            output_regex,
        } = Self::features(resident, &active)?;
        let regex = if output_regex || inline_regex || query_regex.iter().any(|&r| r) {
            Some(Rc::new(super::metal_regex_frontier::RegexProgram::new(
                device, batch,
            )?))
        } else {
            None
        };
        let kinds = if inline_general & 2 != 0 || query_general.iter().any(|&g| g & 2 != 0) {
            Some(Rc::new(super::metal_capture_groups::CaptureKinds::new(
                device, queue, resident, data,
            )?))
        } else {
            None
        };
        Ok(Self {
            resident,
            data,
            regex,
            kinds,
            query_general,
            query_regex,
            inline_general,
            inline_regex,
        })
    }
}

pub(super) struct CapturePredicates<'a> {
    programs: &'a PredicatePrograms<'a>,
    counts: MappedBuffer,
    steps: u32,
    work: MappedBuffer,
    regex_steps: Vec<bool>,
    evaluations: usize,
    general_steps: Vec<u8>,
}
impl<'a> CapturePredicates<'a> {
    pub(super) fn new(
        device: &Device,
        programs: &'a PredicatePrograms<'a>,
        results: usize,
    ) -> Result<Option<Self>, Error> {
        if programs.query_general.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            programs,
            counts: MappedBuffer::zeroed::<[u32; 4]>(device, results)?,
            work: MappedBuffer::zeroed::<u32>(device, results)?,
            steps: u32::try_from(programs.query_general.len())
                .map_err(|_| Error::BackendCapacity)?,
            regex_steps: programs.query_regex.clone(),
            general_steps: programs.query_general.clone(),
            evaluations: programs.resident.input.grid.evaluation_spans.len(),
        }))
    }
    pub(super) fn execute(
        &self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        input: &OperatorInput<'_>,
        failures: &MappedBuffer,
    ) -> Result<(), Error> {
        let arguments = QueryArguments::new(input, resident.input.packed.nodes.len())?;
        self.execute_arguments(device, queue, resident, &arguments.input(), failures, true)
    }
    pub(super) fn new_inline(
        device: &Device,
        programs: &'a PredicatePrograms<'a>,
        results: usize,
        work: MappedBuffer,
    ) -> Result<Self, Error> {
        if work.buffer.length() < results.checked_mul(4).ok_or(Error::BackendCapacity)? as u64 {
            return Err(Error::BackendFailure);
        }
        Ok(Self {
            programs,
            counts: MappedBuffer::zeroed::<[u32; 4]>(device, results)?,
            steps: 1,
            work,
            regex_steps: vec![programs.inline_regex],
            general_steps: vec![programs.inline_general],
            evaluations: programs.resident.input.grid.evaluation_spans.len(),
        })
    }
    pub(super) fn outcome_counts(&self) -> &MappedBuffer {
        &self.counts
    }
    pub(super) fn work_values(&self) -> &MappedBuffer {
        &self.work
    }
    pub(super) fn execute_arguments(
        &self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        input: &ArgumentInput<'_>,
        failures: &MappedBuffer,
        query: bool,
    ) -> Result<(), Error> {
        if !std::ptr::eq(resident, self.programs.resident)
            || !std::ptr::eq(input.data, self.programs.data)
        {
            return Err(Error::BackendFailure);
        }
        input.validate_storage(
            resident.input.packed.nodes.len(),
            query,
            [
                (&self.counts, input.parameters.results as usize, 16),
                (&self.work, input.parameters.results as usize, 4),
                (failures, self.evaluations, 4),
            ],
        )?;
        check(input.status)?;
        let mut groups = None;
        let mut p = input.parameters;
        for step in 0..self.steps {
            p.step = step;
            let command = queue.new_command_buffer();
            if query {
                direct(
                    command,
                    "capture_predicate_clear",
                    &[
                        &input.data.buffer,
                        &input.metadata.buffer,
                        &input.specs.buffer,
                        &self.counts.buffer,
                        &input.status.buffer,
                    ],
                    &p,
                    p.results as usize,
                )?;
            } else {
                direct(
                    command,
                    "capture_inline_predicate_clear",
                    &[&self.counts.buffer],
                    &p,
                    p.results as usize,
                )?;
            }
            let step_input = ArgumentInput {
                parameters: p,
                ..*input
            };
            step_input.encode_validation(command, query)?;
            let command = if groups.is_none() && self.general_steps[step as usize] != 0 {
                super::metal_backend::launch::commit_checked(command)?;
                super::metal_backend::launch::wait_checked(command)?;
                check(input.status)?;
                groups = Some(super::metal_capture_groups::CaptureGroups::prepare(
                    device,
                    queue,
                    &step_input,
                )?);
                queue.new_command_buffer()
            } else {
                command
            };
            direct(
                command,
                "capture_predicate_text",
                &[
                    &input.data.buffer,
                    &input.captures.buffer,
                    &input.metadata.buffer,
                    &input.owners.buffer,
                    &self.counts.buffer,
                    &input.status.buffer,
                    &input.specs.buffer,
                ],
                &p,
                p.captures as usize,
            )?;
            let command = if self.general_steps[step as usize] != 0 {
                let groups = groups.as_ref().ok_or(Error::BackendFailure)?;
                if self.general_steps[step as usize] & 2 != 0 {
                    let kinds = self.programs.kinds.as_ref().ok_or(Error::BackendFailure)?;
                    let kind_params = p;
                    kind_params.run(
                        command,
                        "capture_predicate_kind",
                        &[
                            &input.data.buffer,
                            &input.captures.buffer,
                            &input.metadata.buffer,
                            &input.owners.buffer,
                            &self.counts.buffer,
                            &kinds.code.buffer,
                            &kinds.ancestors.buffer,
                            &input.status.buffer,
                            &input.specs.buffer,
                        ],
                        p.captures as usize,
                    )?;
                }
                super::metal_backend::launch::commit_checked(command)?;
                super::metal_backend::launch::wait_checked(command)?;
                check(input.status)?;
                if self.general_steps[step as usize] & 1 != 0 {
                    let step_input = ArgumentInput {
                        parameters: p,
                        ..*input
                    };
                    super::metal_capture_literals::execute(
                        device,
                        queue,
                        &step_input,
                        groups,
                        &self.counts,
                    )?;
                }
                queue.new_command_buffer()
            } else {
                command
            };
            let command = if self.regex_steps[step as usize] {
                let regex = self.programs.regex.as_ref().ok_or(Error::BackendFailure)?;
                let scan = super::metal_parallel::Scan::new(device, p.captures as usize)?;
                direct(
                    command,
                    "capture_predicate_regex_counts",
                    &[
                        &input.data.buffer,
                        &regex.code.buffer,
                        &input.captures.buffer,
                        &input.metadata.buffer,
                        &input.owners.buffer,
                        &scan.values.buffer,
                        &input.status.buffer,
                        &input.specs.buffer,
                    ],
                    &p,
                    p.captures as usize,
                )?;
                scan.encode(command)?;
                super::metal_backend::launch::commit_checked(command)?;
                super::metal_backend::launch::wait_checked(command)?;
                check(input.status)?;
                let totals = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
                if totals.contains(&u32::MAX) {
                    return Err(Error::BackendCapacity);
                }
                if totals[0] > p.captures || totals[2..] != [0, 0] {
                    return Err(Error::BackendFailure);
                }
                let requests = MappedBuffer::zeroed::<super::metal_regex_frontier::RegexRequest>(
                    device,
                    totals[0] as usize,
                )?;
                let ids = MappedBuffer::zeroed::<u32>(device, totals[0] as usize)?;
                let command = queue.new_command_buffer();
                direct(
                    command,
                    "capture_predicate_regex_scatter",
                    &[
                        &input.data.buffer,
                        &regex.code.buffer,
                        &input.captures.buffer,
                        &input.metadata.buffer,
                        &input.owners.buffer,
                        &scan.values.buffer,
                        scan.total(),
                        &requests.buffer,
                        &ids.buffer,
                        &input.status.buffer,
                        &input.specs.buffer,
                    ],
                    &p,
                    p.captures as usize,
                )?;
                super::metal_backend::launch::commit_checked(command)?;
                super::metal_backend::launch::wait_checked(command)?;
                check(input.status)?;
                let requests = regex.execute(
                    device,
                    queue,
                    super::metal_regex_frontier::RegexInput {
                        data: input.data,
                        status: input.status,
                        budget_owners: p.results as usize,
                    },
                    super::metal_regex_frontier::RegexBatch {
                        scratch_plan:
                            super::metal_regex_frontier::ScratchPlan::CooperativePredicates,
                        requests,
                        ids,
                        count: totals[0] as usize,
                        scratch_slots: totals[1] as usize,
                    },
                    super::metal_regex_frontier::SharedWork {
                        values: &self.work,
                        failures,
                        evaluations: self.evaluations,
                        abort_evaluation: p.abort_evaluation != 0,
                    },
                )?;
                let command = queue.new_command_buffer();
                direct(
                    command,
                    "capture_predicate_regex_results",
                    &[
                        &input.data.buffer,
                        &requests.buffer,
                        scan.total(),
                        &input.metadata.buffer,
                        &self.counts.buffer,
                        &failures.buffer,
                        &input.status.buffer,
                        &input.specs.buffer,
                    ],
                    &p,
                    totals[0] as usize,
                )?;
                super::metal_backend::launch::commit_checked(command)?;
                super::metal_backend::launch::wait_checked(command)?;
                check(input.status)?;
                queue.new_command_buffer()
            } else {
                command
            };
            direct(
                command,
                "capture_predicate_accept",
                &[
                    &input.data.buffer,
                    &input.metadata.buffer,
                    &self.counts.buffer,
                    &failures.buffer,
                    &input.status.buffer,
                    &input.specs.buffer,
                ],
                &p,
                p.results as usize,
            )?;
            super::metal_backend::launch::commit_checked(command)?;
            super::metal_backend::launch::wait_checked(command)?;
            check(input.status)?;
        }
        Ok(())
    }
}

fn check(status: &MappedBuffer) -> Result<(), Error> {
    match super::metal_register_execution::read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
