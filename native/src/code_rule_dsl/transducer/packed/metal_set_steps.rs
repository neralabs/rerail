//! A wave's node-set program on the device (decision 0024 §1, §2; on the process's device, decision 0440 step C): slots of the node space's bitmaps
//! in one shared buffer, the host's leaves written in place, and the whole program as one
//! dispatch (list step 13: S7's program is 533 steps, which were a dispatch each): a threadgroup
//! per file runs every step over the file's words in order. On the device each file starts at a
//! word boundary, so no two threadgroups share a word; leaves and outputs are moved between that
//! layout and the space's. The CPU's `space::NodeSet` operations are the reference.
use rerail_blocks::vector::Vector;
use super::*;
use rerail_metal::lane;

const SOURCE: &str = include_str!("set_steps.uglykl");
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
unsafe impl lane::Pod for ProgramArgs {}

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
        if shift != 0
            && let Some(slot) = words.get_mut(word + 1) {
                *slot |= value >> (64 - shift);
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
) -> Result<Vec<Vector<u64>>, Error> {
    let device = lane::device()?;
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
    let values = lane::buffer_of(device, &vec![0u64; slots.max(1) * words]);
    let base = values.contents().cast::<u64>();
    for &(slot, leaf) in leaves {
        assert!(slot < slots && leaf.len() == space_words, "a leaf fills its slot");
        let slot = unsafe { std::slice::from_raw_parts_mut(base.add(slot * words), words) };
        for (&(first, count), &[at, file_words, ..]) in files.iter().zip(&layout) {
            extract(leaf, first as usize, count as usize, &mut slot[at as usize..(at + file_words) as usize]);
        }
    }
    if !files.is_empty() && !steps.is_empty() {
        let encoded = steps.iter().map(|step| step.encoded()).collect::<Vec<_>>();
        let steps_buffer = lane::buffer_of(device, encoded.as_flattened());
        let layout_buffer = lane::buffer_of(device, layout.as_flattened());
        crate::code_rule_dsl::prof::peak("2a1b.filled");
        let kernel = kernel();
        crate::code_rule_dsl::prof::peak("2a1c.pipeline");
        let args = ProgramArgs { steps: steps.len() as u32, words: words as u32, files: files.len() as u32, pad: 0 };
        let mut command = lane::Command::new("Rerail set steps");
        command
            .launch(&kernel, "set_program")
            .buffer("values", &values)
            .value("p", &args)
            .buffer("steps", &steps_buffer)
            .buffer("files", &layout_buffer)
            .groups([files.len() as u64, 1, 1], [WIDTH, 1, 1]);
        command.run()?;
        crate::code_rule_dsl::prof::peak("2a1d.ran");
    }
    Ok(outputs
        .iter()
        .map(|&slot| {
            let slot = unsafe { std::slice::from_raw_parts(base.add(slot * words), words) };
            let mut out = rerail_blocks::vector![0_u64; space_words];
            for (&(first, _), &[at, file_words, ..]) in files.iter().zip(&layout) {
                insert(&slot[at as usize..(at + file_words) as usize], first as usize, &mut out);
            }
            out
        })
        .collect())
}

/// The set steps' kernel, compiled once a process (`label_passes::warm` makes it on a thread when a wave is made
/// on the device, so the wave's set program does not wait for it).
pub(in crate::code_rule_dsl) fn kernel() -> lane::Kernel {
    lane::kernel_fixed(SOURCE, "set_program", None)
}
