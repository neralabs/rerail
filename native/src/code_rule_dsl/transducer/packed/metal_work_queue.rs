//! The device work queue (list step 17, decision 0027): device-side enqueue, on the process's device (decision
//! 0440 step C). A persistent dispatch's workers take items from a queue and push the items their work creates;
//! idle workers take those up, across every threadgroup, so the device, not the host, decides what runs next and
//! how long the loop lasts. A kernel that uses it includes `<ukl/workqueue.h>` (`ukl_wq_take`, `ukl_wq_push`,
//! `ukl_wq_done`, `ukl_wq_idle`).
//!
//! A GPU promises no progress between threadgroups, so no worker waits on another without a
//! bound (the source says how). The dispatch is sized to what the device keeps resident; a
//! worker that has polled long enough for nothing exits, and if items remain once all have, the
//! host launches the same dispatch again over the same queue (`run`).
use super::*;
use rerail_metal::lane;

/// A worker group's threads.
pub(in crate::code_rule_dsl) const WIDTH: u64 = 256;
/// At most this many worker groups: about what an M-series GPU keeps resident.
const GROUPS: u64 = 64;
/// An idle worker's polls before it exits.
pub(in crate::code_rule_dsl) const POLLS: u32 = 4096;
/// Launches of one queue before it is a failure (a re-launch is the rare case).
const LAUNCHES: usize = 8;

/// A linear queue: `slots` (an item + 1 per slot), `control` (head, tail, active, overflow).
pub(in crate::code_rule_dsl) struct Queue {
    pub(in crate::code_rule_dsl) slots: super::blocks::Block,
    pub(in crate::code_rule_dsl) control: super::blocks::Block,
    pub(in crate::code_rule_dsl) capacity: u32,
}

impl Queue {
    /// A queue of `capacity` slots holding `initial`.
    pub(in crate::code_rule_dsl) fn new(capacity: usize, initial: &[u32]) -> Result<Self, Error> {
        let capacity = capacity.max(initial.len()).max(1);
        if capacity >= u32::MAX as usize / 2 {
            return Err(Error::BackendCapacity);
        }
        let mut slots = vec![0_u32; capacity];
        for (slot, item) in slots.iter_mut().zip(initial) {
            *slot = item + 1;
        }
        let count = initial.len() as u32;
        Ok(Self {
            slots: super::blocks::upload(&slots)?,
            control: super::blocks::upload(&[0, count, count, 0_u32])?,
            capacity: capacity as u32,
        })
    }

    /// The slots pushed into (the tail).
    pub(in crate::code_rule_dsl) fn used(&self) -> u32 {
        self.control()[1].min(self.capacity)
    }

    /// head, tail, active, overflow.
    fn control(&self) -> [u32; 4] {
        let words = self.control.read::<u32>(4);
        [words[0], words[1], words[2], words[3]]
    }
}

/// The worker groups for `items` initial items.
pub(in crate::code_rule_dsl) fn groups(items: usize) -> u64 {
    (items as u64).div_ceil(WIDTH * 4).clamp(1, GROUPS)
}

/// What a queue's run took: its launches, the command buffers' GPU seconds, the host's wall
/// seconds from the first encode to the last completion, and where they went (`profile::PHASES`;
/// the buffers are the caller's).
pub(in crate::code_rule_dsl) struct Ran {
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) gpu: f64,
    pub(in crate::code_rule_dsl) wall: f64,
    pub(in crate::code_rule_dsl) phases: [f64; 7],
}

/// Runs `kernel` (its arguments bound by `bind`) over `queue` until no item is active: one launch, or more where
/// every worker exited with items left.
pub(in crate::code_rule_dsl) fn run(
    kernel: &lane::Kernel,
    queue: &Queue,
    groups: u64,
    label: &str,
    bind: impl for<'c> Fn(lane::Launch<'c>) -> lane::Launch<'c>,
) -> Result<Ran, Error> {
    use super::metal_backend::launch::host_seconds;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let mut phases = [0.0f64; 7];
    let started = host_seconds();
    let mut gpu = 0.0;
    for launch in 1..=LAUNCHES {
        let at = host_seconds();
        let mut command = lane::Command::new(label);
        bind(command.launch(kernel, label)).groups([groups, 1, 1], [WIDTH, 1, 1]);
        phases[2] += host_seconds() - at;
        let times = command.run()?;
        let [scheduled, start, end] = times.host;
        if end > 0.0 {
            phases[3] += (scheduled - times.committed).max(0.0);
            phases[4] += (start - scheduled).max(0.0);
            phases[6] += (times.committed + times.wall - end).max(0.0);
        }
        phases[5] += times.device;
        gpu += times.device;
        let [_, _, active, overflow] = queue.control();
        if overflow != 0 {
            return Err(Error::BackendCapacity);
        }
        if active == 0 {
            crate::code_rule_dsl::prof::count("queue.launches", launch);
            return Ok(Ran { launches: launch, gpu, wall: host_seconds() - started, phases });
        }
    }
    Err(Error::BackendFailure)
}
