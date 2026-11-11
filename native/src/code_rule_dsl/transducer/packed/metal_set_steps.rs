//! A wave's node-set program on Metal (decision 0024 §1, §2): slots of the node space's bitmaps
//! in one shared buffer, the host's leaves written in place, and the whole program as one
//! dispatch (list step 13: S7's program is 533 steps, which were a dispatch each): a threadgroup
//! per file runs every step over the file's words in order. On the device each file starts at a
//! word boundary, so no two threadgroups share a word; leaves and outputs are moved between that
//! layout and the space's. The CPU's `space::NodeSet` operations are the reference.
use super::*;
use metal::MTLSize;

const SOURCE: &str = include_str!("metal_set_steps.metal");
/// A file's threads.
const WIDTH: u64 = 64;

/// A step over slots: a slot is the space's nodes, a bit each.
#[derive(Clone, Copy, Debug)]
pub enum SetStep {
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

impl SetStep {
    fn encoded(self) -> [u32; 4] {
        match self {
            SetStep::Clear { out } => [0, out, out, out],
            SetStep::Fill { out } => [1, out, out, out],
            SetStep::Copy { out, a } => [2, out, a, a],
            SetStep::Or { out, a, b } => [3, out, a, b],
            SetStep::And { out, a, b } => [4, out, a, b],
            SetStep::AndNot { out, a, b } => [5, out, a, b],
            SetStep::Within { out, test } => [6, out, test, test],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProgramArgs {
    steps: u32,
    words: u32,
    files: u32,
    pad: u32,
}

/// The `count` bits of `words` from bit `from`, as words from bit 0.
fn extract(words: &[u64], from: usize, count: usize, out: &mut [u64]) {
    for (at, slot) in out.iter_mut().enumerate() {
        let bit = from + at * 64;
        let (word, shift) = (bit / 64, bit % 64);
        let low = words.get(word).copied().unwrap_or(0) >> shift;
        let high = if shift == 0 { 0 } else { words.get(word + 1).copied().unwrap_or(0) << (64 - shift) };
        let left = count - at * 64;
        *slot = (low | high) & if left >= 64 { u64::MAX } else { (1 << left) - 1 };
    }
}

/// `extract`'s inverse: `bits` (from bit 0) or-ed into `words` from bit `from`.
fn insert(bits: &[u64], from: usize, words: &mut [u64]) {
    for (at, &value) in bits.iter().enumerate() {
        let bit = from + at * 64;
        let (word, shift) = (bit / 64, bit % 64);
        if let Some(slot) = words.get_mut(word) {
            *slot |= value << shift;
        }
        if shift != 0 {
            if let Some(slot) = words.get_mut(word + 1) {
                *slot |= value >> (64 - shift);
            }
        }
    }
}

/// Runs `steps` over `slots` slots of the space of `files` ((first node, node count) each) and
/// `len` nodes, slots `leaves` written first; returns the words of each of `outputs`.
pub fn run(
    files: &[(u32, u32)],
    len: u32,
    slots: usize,
    leaves: &[(usize, &[u64])],
    steps: &[SetStep],
    outputs: &[usize],
) -> Result<Vec<Vec<u64>>, Error> {
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    crate::code_rule_dsl::prof::peak("2a1a.device");
    // The steps' submission within the caller's launch budget, or one of their own.
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let space_words = (len as usize).div_ceil(64).max(1);
    // Each file's words on the device, from a word boundary.
    let mut layout = Vec::with_capacity(files.len());
    let mut total = 0_u32;
    for &(_, count) in files {
        let words = count.div_ceil(64);
        layout.push([total, words, count, 0]);
        total += words;
    }
    let words = (total as usize).max(1);
    let values = MappedBuffer::zeroed::<u64>(device, slots.max(1) * words)?;
    let base = values.buffer.contents().cast::<u64>();
    for &(slot, leaf) in leaves {
        assert!(slot < slots && leaf.len() == space_words, "a leaf fills its slot");
        let slot = unsafe { std::slice::from_raw_parts_mut(base.add(slot * words), words) };
        for (&(first, count), &[at, file_words, ..]) in files.iter().zip(&layout) {
            extract(leaf, first as usize, count as usize, &mut slot[at as usize..(at + file_words) as usize]);
        }
    }
    if !files.is_empty() && !steps.is_empty() {
        let encoded = steps.iter().map(|step| step.encoded()).collect::<Vec<_>>();
        let steps_buffer = MappedBuffer::from_slice(device, &encoded)?;
        let layout_buffer = MappedBuffer::from_slice(device, &layout)?;
        crate::code_rule_dsl::prof::peak("2a1b.filled");
        let pipeline = pipelines()?.remove(0);
        crate::code_rule_dsl::prof::peak("2a1c.pipeline");
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        command.set_label("Rerail set steps");
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("set_program");
        let args = ProgramArgs { steps: steps.len() as u32, words: words as u32, files: files.len() as u32, pad: 0 };
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&values.buffer), 0);
        encoder.set_bytes(1, std::mem::size_of::<ProgramArgs>() as u64, std::ptr::from_ref(&args).cast());
        encoder.set_buffer(2, Some(&steps_buffer.buffer), 0);
        encoder.set_buffer(3, Some(&layout_buffer.buffer), 0);
        encoder.checked_dispatch_thread_groups(command, MTLSize::new(files.len() as u64, 1, 1), MTLSize::new(WIDTH, 1, 1));
        encoder.end_encoding();
        commit_checked(command)?;
        wait_checked(command)?;
        crate::code_rule_dsl::prof::peak("2a1d.ran");
    }
    Ok(outputs
        .iter()
        .map(|&slot| {
            let slot = unsafe { std::slice::from_raw_parts(base.add(slot * words), words) };
            let mut out = vec![0_u64; space_words];
            for (&(first, _), &[at, file_words, ..]) in files.iter().zip(&layout) {
                insert(&slot[at as usize..(at + file_words) as usize], first as usize, &mut out);
            }
            out
        })
        .collect())
}

/// The set steps' pipelines, compiled from source once a process (`label_passes::warm` starts
/// it on a thread when a Metal wave is made, so the wave's set program does not wait for it).
pub(in crate::code_rule_dsl) fn pipelines() -> Result<Vec<metal::ComputePipelineState>, Error> {
    static PIPELINES: std::sync::Mutex<Option<Vec<metal::ComputePipelineState>>> = std::sync::Mutex::new(None);
    let mut cached = PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipelines) = &*cached {
        return Ok(pipelines.clone());
    }
    let pipelines = ["set_program"]
        .iter()
        .map(|name| pipeline_named(SOURCE, name))
        .collect::<Result<Vec<_>, _>>()?;
    *cached = Some(pipelines.clone());
    Ok(pipelines)
}
