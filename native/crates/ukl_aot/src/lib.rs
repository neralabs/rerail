//! UKL kernels as Rust ahead of time (decision 0567 step 3), to compare the Rust rerail-ukl-vec writes of a kernel
//! with the same kernel vectorized by hand.
//!
//! Each kernel is here three times:
//!
//! - `{kernel}_w{W}`: rerail-ukl-vec's Rust (`rs.rs`) of its vector tape at `W` lanes, NEON intrinsics, checked in
//!   (rerail-ukl-vec's tests write it again and fail if it differs);
//! - `hand::{kernel}`: the same kernel written by hand with NEON intrinsics, as a host kernel would be;
//! - `scalar::{kernel}`: one thread at a time, the reference.
//!
//! The tests run all three on the same inputs; `examples/nearest_bench.rs` times them.

#[cfg(target_arch = "aarch64")]
pub mod hand;
#[cfg(target_arch = "aarch64")]
pub mod mix_lanes_w8;
#[cfg(target_arch = "aarch64")]
pub mod nearest_jump_w8;
#[cfg(target_arch = "aarch64")]
pub mod parent_back_w8;
#[cfg(target_arch = "aarch64")]
pub mod parent_group_w8;
#[cfg(target_arch = "aarch64")]
pub mod parent_link_w8;
#[cfg(target_arch = "aarch64")]
pub mod parent_step_w8;
#[cfg(target_arch = "aarch64")]
pub mod wide_lanes_w8;
pub mod parent;
pub mod scalar;
#[cfg(test)]
mod tests;

/// A group's launch, as a kernel's Rust reads it: its group's position, its threads' (each dimension's), and the
/// groups'.
#[derive(Clone, Copy, Debug)]
pub struct Launch {
    pub group: [u32; 3],
    pub block: [u32; 3],
    pub groups: [u32; 3],
}

/// A kernel's Rust: one group's threads.
pub type Kernel = unsafe fn(*const *const u8, &Launch);

/// Kernel `k` over `groups` groups of `block` threads (1-D), `argv` its arguments (each the address of its bytes).
///
/// # Safety
///
/// `argv` holds `k`'s arguments, its buffers as large as the launch reads and writes.
pub unsafe fn launch(k: Kernel, argv: &[*const u8], groups: u32, block: u32) {
    for g in 0..groups {
        let l = Launch { group: [g, 0, 0], block: [block, 1, 1], groups: [groups, 1, 1] };
        unsafe { k(argv.as_ptr(), &l) };
    }
}

/// A state's bit that says its target is the cell's answer (`nearest.uglykl`'s `FINAL`).
pub const FINAL: u32 = 0x8000_0000;
/// No ancestor in the set (`NONE`).
pub const NONE: u32 = u32::MAX;

/// `nearest_jump`'s arguments' struct (`NearestArgs`), as its kernel reads its bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NearestArgs {
    pub nodes: u32,
    pub cells: u32,
    pub pad0: u32,
    pub pad1: u32,
}

/// Sets of one forest's nodes, and each cell's (set, node) start state and answer, for `nearest_jump`.
pub struct Forest {
    pub nodes: u32,
    pub sets: u32,
    /// Each cell's state before the kernel: its parent, final if the parent is in the set.
    pub start: Vec<u32>,
    /// Each cell's answer: its nearest strict ancestor in its set, final, or `NONE`.
    pub want: Vec<u32>,
}

impl Forest {
    /// `sets` sets of `nodes` nodes whose parents `parent` gives (a node's parent below it), each set's members
    /// `member`'s.
    pub fn new(nodes: u32, sets: u32, parent: impl Fn(u32) -> Option<u32>, member: impl Fn(u32, u32) -> bool) -> Forest {
        let (mut start, mut want) = (Vec::new(), Vec::new());
        for s in 0..sets {
            for i in 0..nodes {
                start.push(match parent(i) {
                    None => NONE,
                    Some(q) if member(s, q) => FINAL | q,
                    Some(q) => q,
                });
                let mut a = parent(i);
                while let Some(q) = a.filter(|&q| !member(s, q)) {
                    a = parent(q);
                }
                want.push(a.map_or(NONE, |q| FINAL | q));
            }
        }
        Forest { nodes, sets, start, want }
    }

    /// A random forest of `nodes` nodes (a node's parent one of those below it, near it if `deep`), `sets` sets
    /// each holding about one node in `sparse`.
    pub fn random(nodes: u32, sets: u32, sparse: u32, deep: bool, seed: u64) -> Forest {
        let hash = |a: u64, b: u64| {
            let mut x = seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
            x ^= x >> 33;
            x
        };
        let parent = |i: u32| (i > 0).then(|| if deep { i - 1 - (hash(1, u64::from(i)) % u64::from(i.min(4))) as u32 } else { (hash(1, u64::from(i)) % u64::from(i)) as u32 });
        let member = |s: u32, i: u32| hash(u64::from(s) + 2, u64::from(i)) % u64::from(sparse) == 0;
        Forest::new(nodes, sets, parent, member)
    }

    pub fn cells(&self) -> u32 {
        self.nodes * self.sets
    }
}

/// `nearest_jump`'s Rust `k` run on `state` (each cell's), `block` threads a group: the most steps a thread took.
///
/// # Safety
///
/// `state` is `args.cells` cells.
pub unsafe fn nearest_run(k: Kernel, state: &mut [u32], args: NearestArgs, block: u32) -> u32 {
    let mut stats = 0u32;
    let (sp, tp) = (state.as_mut_ptr(), &raw mut stats);
    // The shim's two arguments past the kernel's: the groups' counts (none) and their first's position.
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let argv = [(&raw const sp).cast::<u8>(), (&raw const tp).cast::<u8>(), (&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>()];
    let groups = args.cells.div_ceil(block);
    unsafe { launch(k, &argv, groups, block) };
    stats
}

/// `mix_lanes`'s arguments' struct (`MixArgs`, `kernels/mix.uglykl`), as its kernel reads its bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MixArgs {
    pub n: u32,
    pub rounds: u32,
    pub pad1: u32,
    pub pad2: u32,
}

/// `mix_lanes` (its Rust `k`) over `input`, `rounds` its uniform loop's, in groups of `block` threads: each value's
/// answer.
///
/// # Safety
///
/// `k` is a kernel of `mix_lanes`'s arguments.
pub unsafe fn mix_run(k: Kernel, input: &[u32], rounds: u32, block: u32) -> Vec<u32> {
    let mut out = vec![0u32; input.len()];
    let (ip, op) = (input.as_ptr(), out.as_mut_ptr());
    let args = MixArgs { n: input.len() as u32, rounds, pad1: 0, pad2: 0 };
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let argv = [(&raw const ip).cast::<u8>(), (&raw const op).cast::<u8>(), (&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>()];
    unsafe { launch(k, &argv, args.n.div_ceil(block), block) };
    out
}

/// `wide_lanes`'s arguments' struct (`WideArgs`, `kernels/wide.uglykl`), as its kernel reads its bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WideArgs {
    pub n: u32,
    pub pad: u32,
    pub k: u64,
}

/// The rows of `wide_lanes`'s answers (`WIDE_ROWS`): row `r`'s answer of thread `i` at `r * n + i`.
pub const WIDE_ROWS: usize = 26;

/// `wide_lanes` (its Rust `k`) over `a`, `b` and `c` (each a thread's), `k` its uniform value, in groups of `block`
/// threads: each row's answers.
///
/// # Safety
///
/// `k` is a kernel of `wide_lanes`'s arguments, and `a`, `b` and `c` have one length.
pub unsafe fn wide_run(kernel: Kernel, a: &[u64], b: &[u64], c: &[u32], k: u64, block: u32) -> Vec<u64> {
    let mut out = vec![0u64; a.len() * WIDE_ROWS];
    let (ap, bp, cp, op) = (a.as_ptr(), b.as_ptr(), c.as_ptr(), out.as_mut_ptr());
    let args = WideArgs { n: a.len() as u32, pad: 0, k };
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let argv = [(&raw const ap).cast::<u8>(), (&raw const bp).cast::<u8>(), (&raw const cp).cast::<u8>(), (&raw const op).cast::<u8>(), (&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>()];
    unsafe { launch(kernel, &argv, args.n.div_ceil(block), block) };
    out
}
