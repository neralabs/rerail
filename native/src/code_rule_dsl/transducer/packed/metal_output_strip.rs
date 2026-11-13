//! One candidate-owned transform budget and exact GPU text snapshots.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_output_dispatch::{OperatorInput, SOURCE};
use super::metal_parallel::{Scan, encode};
use super::metal_regex_frontier::{
    RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork, StripSnapshot,
};
use super::metal_register_execution::read;
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};

#[derive(Clone, Copy)]
struct Params {
    results: u32,
    captures: u32,
    step: u32,
    source_bytes: u32,
    old_bytes: u32,
    code_words: u32,
    work_limit: u32,
    requests: u32,
}
impl Params {
    fn run(
        self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        groups: usize,
    ) -> Result<(), Error> {
        encode(
            command,
            &pipeline_named(SOURCE, name)?,
            name,
            buffers,
            &[
                self.results,
                self.captures,
                self.step,
                self.source_bytes,
                self.old_bytes,
                self.code_words,
                self.work_limit,
                self.requests,
            ],
            groups.max(1),
            None,
        );
        Ok(())
    }
}
fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if total.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    Ok(total)
}
pub(super) struct StripStep<'a> {
    pub input: OperatorInput<'a>,
    pub text: &'a MappedBuffer,
    pub failures: &'a MappedBuffer,
    pub evaluations: usize,
    pub program: &'a RegexProgram,
}
pub(super) struct StripOutput {
    pool: Option<MappedBuffer>,
    length: u32,
    work: MappedBuffer,
    work_limit: u32,
}
impl StripOutput {
    pub(super) fn new(device: &Device, results: usize, work_limit: u32) -> Result<Self, Error> {
        if work_limit as usize > super::MAX_STEPS_PER_PATH {
            return Err(Error::BackendFailure);
        }
        Ok(Self {
            pool: None,
            length: 0,
            work: MappedBuffer::zeroed::<u32>(device, results)?,
            work_limit,
        })
    }
    pub(super) fn bytes(&self) -> Option<&MappedBuffer> {
        self.pool.as_ref()
    }
    pub(super) fn length(&self) -> u32 {
        self.length
    }
    pub(super) fn execute_step(
        &mut self,
        device: &Device,
        queue: &CommandQueueRef,
        step: StripStep<'_>,
    ) -> Result<(), Error> {
        let StripStep {
            input,
            text,
            failures,
            evaluations,
            program,
        } = step;
        let captures = input.parameters.captures as usize;
        let sizes = Scan::new(device, captures)?;
        let mut p = Params {
            results: input.parameters.results,
            captures: input.parameters.captures,
            step: input.parameters.step,
            source_bytes: 0,
            old_bytes: self.length,
            code_words: program.code_words(),
            work_limit: self.work_limit,
            requests: 0,
        };
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            p.run(
                command,
                "strip_capture_sizes",
                &[
                    &input.data.buffer,
                    &input.captures.buffer,
                    &input.metadata.buffer,
                    &input.owners.buffer,
                    &text.buffer,
                    &program.code.buffer,
                    &sizes.values.buffer,
                    &input.status.buffer,
                ],
                captures.div_ceil(256),
            )?;
            sizes.encode(command)?;
            commit_checked(command)?;
            wait_checked(command)?;
            check(input.status)
        })?;
        let counts = total(&sizes)?;
        if counts[2] as usize > captures || counts[3] != 0 {
            return Err(Error::BackendFailure);
        }
        p.source_bytes = counts[0];
        p.requests = counts[2];
        let snapshot = MappedBuffer::zeroed::<u8>(device, counts[0] as usize)?;
        let mask = MappedBuffer::zeroed::<u32>(device, counts[0] as usize)?;
        let requests = MappedBuffer::zeroed::<RegexRequest>(device, counts[2] as usize)?;
        let ids = MappedBuffer::zeroed::<u32>(device, counts[2] as usize)?;
        let old = self.pool.take();
        self.length = 0;
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            p.run(
                command,
                "strip_snapshot_bytes",
                &[
                    &input.data.buffer,
                    &input.captures.buffer,
                    &text.buffer,
                    &sizes.values.buffer,
                    sizes.total(),
                    &old.as_ref().unwrap_or(input.data).buffer,
                    &snapshot.buffer,
                    &mask.buffer,
                    &input.status.buffer,
                ],
                (counts[0] as usize).div_ceil(256),
            )?;
            p.run(
                command,
                "strip_request_scatter",
                &[
                    &input.data.buffer,
                    &input.captures.buffer,
                    &input.metadata.buffer,
                    &input.owners.buffer,
                    &sizes.values.buffer,
                    sizes.total(),
                    &requests.buffer,
                    &ids.buffer,
                    &input.status.buffer,
                ],
                captures.div_ceil(256),
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            check(input.status)
        })?;
        drop(old);
        let requests = program.execute_strip(
            device,
            queue,
            RegexInput {
                data: input.data,
                status: input.status,
                budget_owners: p.results as usize,
            },
            RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests,
                ids,
                count: counts[2] as usize,
                scratch_slots: counts[1] as usize,
            },
            SharedWork {
                values: &self.work,
                failures,
                evaluations,
                abort_evaluation: true,
            },
            StripSnapshot {
                bytes: &snapshot,
                length: counts[0] as usize,
                mask: &mask,
                work_limit: self.work_limit,
            },
        )?;
        let tiles = (counts[0] as usize).div_ceil(256);
        let kept = Scan::new(device, tiles)?;
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            p.run(
                command,
                "strip_results",
                &[
                    &requests.buffer,
                    &input.metadata.buffer,
                    &failures.buffer,
                    &input.status.buffer,
                ],
                (p.results.max(p.requests) as usize).div_ceil(256),
            )?;
            p.run(
                command,
                "strip_mask_counts",
                &[&mask.buffer, &kept.values.buffer, &input.status.buffer],
                tiles,
            )?;
            kept.encode(command)?;
            commit_checked(command)?;
            wait_checked(command)?;
            check(input.status)
        })?;
        let totals = total(&kept)?;
        if totals[0] > counts[0] || totals[1..] != [0, 0, 0] {
            return Err(Error::BackendFailure);
        }
        let pool = MappedBuffer::zeroed::<u8>(device, totals[0] as usize)?;
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            p.run(
                command,
                "strip_publish_bytes",
                &[
                    &snapshot.buffer,
                    &mask.buffer,
                    &kept.values.buffer,
                    &pool.buffer,
                    &input.status.buffer,
                ],
                tiles,
            )?;
            p.run(
                command,
                "strip_publish_captures",
                &[
                    &input.data.buffer,
                    &input.captures.buffer,
                    &input.metadata.buffer,
                    &input.owners.buffer,
                    &text.buffer,
                    &sizes.values.buffer,
                    sizes.total(),
                    &mask.buffer,
                    &kept.values.buffer,
                    kept.total(),
                    &input.status.buffer,
                ],
                captures.div_ceil(256),
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            check(input.status)
        })?;
        self.pool = Some(pool);
        self.length = totals[0];
        Ok(())
    }
}
