//! A wave's node-set program on Metal (decision 0024 §1, §2): slots of the node space's bitmaps
//! in one shared buffer, the host's leaves written in place, and one dispatch per step in one
//! compute encoder (serial, so each step sees the last). The CPU's `space::NodeSet` operations
//! are the reference.
use super::*;
use metal::MTLSize;

const SOURCE: &str = include_str!("metal_set_steps.metal");

/// A step over slots: a slot is `words` 64-bit words of the space.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) enum SetStep {
    Clear { out: u32 },
    /// Every node of the space.
    Fill { out: u32 },
    Copy { out: u32, a: u32 },
    Or { out: u32, a: u32, b: u32 },
    And { out: u32, a: u32, b: u32 },
    AndNot { out: u32, a: u32, b: u32 },
    /// `out`'s nodes kept in files where `test` has a node.
    Within { out: u32, test: u32 },
}

#[repr(C)]
#[derive(Clone, Copy)]
struct StepArgs {
    op: u32,
    out: u32,
    a: u32,
    b: u32,
    words: u32,
    len: u32,
    files: u32,
    pad: u32,
}

/// Runs `steps` over `slots` slots of the space of `files` ((first node, node count) each) and
/// `len` nodes, slots `leaves` written first; returns the words of each of `outputs`.
pub(in crate::code_rule_dsl) fn run(
    files: &[(u32, u32)],
    len: u32,
    slots: usize,
    leaves: &[(usize, &[u64])],
    steps: &[SetStep],
    outputs: &[usize],
) -> Result<Vec<Vec<u64>>, Error> {
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // The steps' submission within the caller's launch budget, or one of their own.
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let words = (len as usize).div_ceil(64).max(1);
    let values = MappedBuffer::zeroed::<u64>(device, slots.max(1) * words)?;
    let base = values.buffer.contents().cast::<u64>();
    for &(slot, leaf) in leaves {
        assert!(slot < slots && leaf.len() == words, "a leaf fills its slot");
        unsafe { std::ptr::copy_nonoverlapping(leaf.as_ptr(), base.add(slot * words), words) };
    }
    let ranges = files.iter().map(|&(first, count)| [first, count]).collect::<Vec<_>>();
    let ranges = MappedBuffer::from_slice(device, if ranges.is_empty() { &[[0_u32, 0]][..] } else { &ranges })?;
    let flags = MappedBuffer::zeroed::<u32>(device, files.len().max(1))?;
    let pipelines = pipelines()?;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    command.set_label("Rerail set steps");
    let encoder = command.new_compute_command_encoder();
    encoder.set_label("Rerail set steps");
    let args = |op: u32, out: u32, a: u32, b: u32| StepArgs {
        op,
        out,
        a,
        b,
        words: words as u32,
        len,
        files: files.len() as u32,
        pad: 0,
    };
    let dispatch = |pipeline: &metal::ComputePipelineState, args: StepArgs, threads: usize| {
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_buffer(0, Some(&values.buffer), 0);
        encoder.set_bytes(1, std::mem::size_of::<StepArgs>() as u64, std::ptr::from_ref(&args).cast());
        encoder.set_buffer(2, Some(&ranges.buffer), 0);
        encoder.set_buffer(3, Some(&flags.buffer), 0);
        let width = pipeline.thread_execution_width().max(1);
        encoder.dispatch_threads(MTLSize::new(threads.max(1) as u64, 1, 1), MTLSize::new(width.min(threads.max(1) as u64), 1, 1));
    };
    for step in steps {
        match *step {
            SetStep::Clear { out } => dispatch(&pipelines[0], args(0, out, out, out), words),
            SetStep::Fill { out } => dispatch(&pipelines[0], args(1, out, out, out), words),
            SetStep::Copy { out, a } => dispatch(&pipelines[0], args(2, out, a, a), words),
            SetStep::Or { out, a, b } => dispatch(&pipelines[0], args(3, out, a, b), words),
            SetStep::And { out, a, b } => dispatch(&pipelines[0], args(4, out, a, b), words),
            SetStep::AndNot { out, a, b } => dispatch(&pipelines[0], args(5, out, a, b), words),
            SetStep::Within { out, test } => {
                dispatch(&pipelines[1], args(0, out, test, test), files.len());
                dispatch(&pipelines[2], args(0, out, test, test), words);
            }
        }
    }
    encoder.end_encoding();
    commit_checked(command)?;
    wait_checked(command)?;
    Ok(outputs
        .iter()
        .map(|&slot| unsafe { std::slice::from_raw_parts(base.add(slot * words), words) }.to_vec())
        .collect())
}

fn pipelines() -> Result<Vec<metal::ComputePipelineState>, Error> {
    static PIPELINES: std::sync::Mutex<Option<Vec<metal::ComputePipelineState>>> = std::sync::Mutex::new(None);
    let mut cached = PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipelines) = &*cached {
        return Ok(pipelines.clone());
    }
    let pipelines = ["set_word", "set_file_any", "set_file_keep"]
        .iter()
        .map(|name| pipeline_named(SOURCE, name))
        .collect::<Result<Vec<_>, _>>()?;
    *cached = Some(pipelines.clone());
    Ok(pipelines)
}
