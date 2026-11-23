//! The label passes' parents (`label_passes.uglykl`'s `label_parents`, decision 0475): the batch's every node's parent,
//! once, which the label phases read. The engine runs it on the device; here, rerail-ukl-vec's Rust of it over a batch
//! of format 16 documents, read as the engine's batch reads them: each where it lies, its view at its offset from the
//! batch's chunk, its nodes numbered from its `node_base`.

use crate::parent::Doc;
use rerail_clif::view::TapeView;

/// `label_passes`' `Shape` (its arguments' struct), of which `label_parents` reads `fileCount` and `recursive`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Shape {
    pub node_count: u32,
    pub words: u32,
    pub result_arena: u32,
    pub task_count: u32,
    pub item_count: u32,
    pub target_count: u32,
    pub round: u32,
    pub finish: u32,
    /// The region's words: the files' node words, each file's from `region(file)`.
    pub recursive: u32,
    pub last_level: u32,
    pub file_count: u32,
    pub steps: u32,
    pub profile: u32,
    pub queue: u32,
    pub budget: u32,
    pub lanes: u32,
    pub frames: u32,
}

/// The kernel's threads a group (`PHASE_WORDS`): a thread a word of 32 nodes.
pub const GROUP: u32 = 64;

/// A batch of documents, each read where it lies: the chunk is the first one's bytes, and each document's view holds
/// its offset from there (the difference of their addresses, as the engine's views reach a document in another slab
/// of the batch, decision 0471) and its first node's number.
pub struct Batch<'a> {
    chunk: *const u8,
    pub views: Vec<TapeView>,
    _docs: std::marker::PhantomData<&'a Doc>,
}

impl<'a> Batch<'a> {
    pub fn new(docs: &[&'a Doc]) -> Batch<'a> {
        let chunk = docs.first().map_or(std::ptr::null(), |d| d.bytes.as_ptr());
        let mut base = 0u32;
        let views = docs.iter().map(|d| {
            let mut v = d.view;
            v.set_offset((d.bytes.as_ptr() as isize).wrapping_sub(chunk as isize) as i64);
            v.node_base = base;
            base += d.nodes().div_ceil(32) * 32;
            v
        }).collect();
        Batch { chunk, views, _docs: std::marker::PhantomData }
    }

    /// File `f`'s first word in the region's numbering (`FORMULA_REGION`).
    pub fn region(&self, f: usize) -> u32 {
        self.views[f].node_base / 32 + 2 * f as u32
    }

    /// The region's words (`Shape::recursive`).
    pub fn words(&self) -> u32 {
        self.views.iter().enumerate().map(|(f, v)| self.region(f) + v.nodes.div_ceil(32)).max().unwrap_or(0)
    }
}

/// `label_parents` as Rust over `b`: each file's nodes' parents (file-local, `NONE` at the root).
///
/// # Safety
///
/// `k` is a kernel of `label_parents`' arguments.
pub unsafe fn run_parents(k: crate::Kernel, b: &Batch) -> Vec<Vec<u32>> {
    let words = b.words();
    let shape = Shape { recursive: words, file_count: b.views.len() as u32, ..Shape::default() };
    let mut parents = vec![0u32; 32 * words as usize];
    let (dp, pp, cp) = (b.views.as_ptr().cast::<u8>(), parents.as_mut_ptr().cast::<u8>().cast_const(), b.chunk);
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let p = |x: &*const u8| (&raw const *x).cast::<u8>();
    let argv = [p(&dp), p(&pp), (&raw const shape).cast::<u8>(), p(&cp), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>()];
    unsafe { crate::launch(k, &argv, words.div_ceil(GROUP), GROUP) };
    (0..b.views.len()).map(|f| {
        let at = 32 * b.region(f) as usize;
        parents[at..at + b.views[f].nodes as usize].to_vec()
    }).collect()
}

/// The label passes' source as the engine assembles it for the device (`metal_program_backend::VIEW_PIECES`), the
/// format left to its specialization constant (`UKL_CONSTANT_0`, a define on the CPU): the view's struct, the
/// format's read, the tape's reads, the device's tape helpers, and the passes.
pub fn source() -> String {
    format!(
        "{}UKL_CONSTANT_U32(tape_format)\n#define clif_view(v,a) ((tape_format!=0u&&(a)=={}u)?tape_format:(v)[(a)/4u])\n#include <ukl/reads/tape.h>\n{}{}",
        rerail_clif::view::ukl(),
        std::mem::offset_of!(TapeView, format),
        include_str!("../../../src/code_rule_dsl/transducer/packed/device_tape.h"),
        include_str!("../../device_metal/src/code_rule_dsl/transducer/packed/label_passes.uglykl"),
    )
}

