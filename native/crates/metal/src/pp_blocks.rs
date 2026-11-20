//! The building blocks of the preprocessor's grid passes (decision 0448 §3), `pp_blocks.metal`:
//! a scan, a segmented scan (add or max), a stable radix sort of 64-bit keys, a search and an
//! expand. Each is sized on the device: its count is a word of a device buffer, a plan kernel
//! makes the levels' counts and the indirect arguments, and the host encodes every dispatch
//! without reading one.
//!
//! Scratch is placed when a block is made, for a capacity the host knows (the arena's), so
//! encoding never allocates. A count past the capacity is the caller's to refuse before.
//!
//! Every store an evaluation's passes use is at an offset in one block, its arena (decision 0468
//! §3, `Arena`): leased once a run within the one budget, and bound at each store's offset.

use crate::Error;
use crate::lane::{self, Buffer};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

/// A store's alignment in its arena: every binding offset and every type's.
const ALIGN: usize = 256;

/// An evaluation's arena (decision 0468 §3): one `rerail_blocks::Block`, leased from the GPU pool
/// within the one budget (`lease_block_within`). Its stores are laid out as the passes' structures
/// declare them (`zeroed`, `from_slice`), then the block is leased once (`lease`), before any pass
/// is encoded; a store's initial words are written into it then. A rung is a larger arena.
#[derive(Clone, Default)]
pub struct Arena(Rc<Inner>);

#[derive(Default)]
struct Inner {
    /// The layout's end: the bytes the stores take.
    end: Cell<usize>,
    /// The stores' initial words, by offset, written when the block is leased.
    init: RefCell<Vec<(usize, Vec<u8>)>>,
    leased: OnceCell<Leased>,
}

struct Leased {
    _block: rerail_blocks::Block,
    buffer: Buffer,
    /// The stores' first byte in the block (past its header), and in the host's mapping.
    base: usize,
    host: *mut u8,
}

/// A store: `len` bytes at offset `at` of its arena's layout.
#[derive(Clone)]
pub struct Buf {
    arena: Arena,
    at: usize,
    len: usize,
}

impl Arena {
    pub fn new() -> Arena {
        Arena::default()
    }

    fn place(&self, len: usize) -> Buf {
        let at = self.0.end.get();
        self.0.end.set(at + len.next_multiple_of(ALIGN));
        Buf { arena: self.clone(), at, len }
    }

    /// A store of `count` `T`s (at least one), zero.
    pub fn zeroed<T>(&self, count: usize) -> Result<Buf, Error> {
        assert!(self.0.leased.get().is_none(), "a store declared after its arena was leased");
        let len = count.max(1).checked_mul(size_of::<T>()).ok_or(Error::BackendCapacity)?;
        Ok(self.place(len))
    }

    /// A store of `values` (at least one `T`'s room).
    pub fn from_slice<T: Copy>(&self, values: &[T]) -> Result<Buf, Error> {
        let b = self.zeroed::<T>(values.len())?;
        // SAFETY: `T` is plain data; its bytes are read once, here.
        let bytes = unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values)) };
        // copy: not block data (a pass's control words or the layout's test input, written into its store)
        self.0.init.borrow_mut().push((b.at, bytes.to_vec()));
        Ok(b)
    }

    /// The bytes the arena's stores take.
    pub fn bytes(&self) -> usize {
        self.0.end.get()
    }

    /// Leases the arena's block, every store zero but for its initial words. Refused
    /// (`BackendCapacity`) where the budget does not hold it: the caller runs a smaller batch.
    pub fn lease(&self) -> Result<(), Error> {
        if self.0.leased.get().is_some() {
            return Ok(());
        }
        let bytes = self.bytes().max(ALIGN);
        let device = lane::device()?;
        let block = rerail_accel::gpu::shared::lease_block_within(rerail_blocks::block::kind::SCRATCH, bytes + 2 * ALIGN + 4096)
            .map_err(|_| Error::BackendCapacity)?;
        // Zeroed: a pooled slab may hold an earlier block's bytes.
        let base = block.try_alloc(bytes, ALIGN).ok_or(Error::BackendCapacity)?;
        let buffer = block.device_buffer(device).map_err(|_| Error::BackendCapacity)?;
        let host = block.base_ptr();
        for (at, words) in self.0.init.borrow_mut().drain(..) {
            // SAFETY: the store's range lies inside the block's allocation `base..base + bytes`.
            // copy: not block data (a store's initial words)
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), host.add(base + at), words.len()) };
        }
        let _ = self.0.leased.set(Leased { _block: block, buffer, base, host });
        Ok(())
    }

    fn leased(&self) -> &Leased {
        self.0.leased.get().expect("an arena's store read before its arena was leased")
    }
}

impl Buf {
    /// The store's first byte in the host's mapping (after its arena is leased).
    pub fn host(&self) -> *mut u8 {
        let l = self.arena.leased();
        // SAFETY: inside the block's allocation.
        unsafe { l.host.add(l.base + self.at) }
    }

    /// The store's length in bytes.
    pub fn byte_length(&self) -> usize {
        self.len
    }
}

/// What a pass binds: a buffer, at a byte offset.
pub trait Bound {
    fn bound(&self) -> (&Buffer, u64);
}

impl Bound for Buf {
    fn bound(&self) -> (&Buffer, u64) {
        let l = self.arena.leased();
        (&l.buffer, (l.base + self.at) as u64)
    }
}

impl<T: Bound + ?Sized> Bound for &T {
    fn bound(&self) -> (&Buffer, u64) {
        (**self).bound()
    }
}

impl Bound for Buffer {
    fn bound(&self) -> (&Buffer, u64) {
        (self, 0)
    }
}

/// The evaluator's one source: the tape's view and its reads written once in CLIF
/// (`<ukl/reads/tape.h>`, the view read where the batch holds it), the front passes
/// (`pp_eval.metal`), these blocks, the work queue (`work_queue.metal`) and the expansion
/// (`pp_expand.metal`); preprocessed for Metal once (`rerail_ukl::metal_source`, decision 0440
/// §3.3) until the passes are UKL themselves (decision 0468 §5).
pub fn source() -> Result<&'static str, Error> {
    static SOURCE: std::sync::LazyLock<Result<String, String>> = std::sync::LazyLock::new(|| {
        let text = format!(
            "#include <metal_stdlib>\nusing namespace metal;\n#define CLIF_VIEW_SPACE UKL_GLOBAL\n{}#include <ukl/reads/tape.h>\n{}\n{}\n{}\n{}",
            rerail_clif::view::ukl(),
            include_str!("pp_eval.metal"),
            include_str!("pp_blocks.metal"),
            include_str!("work_queue.metal"),
            include_str!("pp_expand.metal")
        );
        rerail_ukl::metal_source("pp.metal", &text).map_err(|e| e.to_string())
    });
    SOURCE.as_ref().map(String::as_str).map_err(|_| Error::BackendFailure)
}

/// A dispatch's size: groups known to the host, or indirect arguments at a byte offset.
#[derive(Clone, Copy)]
pub enum Grid<'b> {
    Groups(u64),
    Indirect(&'b dyn Bound, u64),
}

/// The threads of a group.
pub const TG: u64 = 256;
/// A scan block's elements.
const BLOCK: usize = 1024;

/// Encodes passes of `source` into one command (`lane::Command`, decision 0440 step C): every
/// pass of an evaluation in it, so the host reads nothing between them (0448 R9).
pub struct Enc<'a> {
    command: RefCell<lane::Command>,
    pub source: &'a str,
    /// The slabs each pass makes resident: a unit's documents past the bound one (`pp_eval::Unit`).
    pub resident: &'a [std::sync::Arc<crate::slabs::Slab>],
}

impl<'a> Enc<'a> {
    pub fn new(label: &str, source: &'a str, resident: &'a [std::sync::Arc<crate::slabs::Slab>]) -> Enc<'a> {
        Enc { command: RefCell::new(lane::Command::new(label)), source, resident }
    }

    /// The command its passes made, to commit.
    pub fn into_command(self) -> lane::Command {
        self.command.into_inner()
    }

    /// Kernel `name` (with `constant` at function constant 0) over `buffers`, then the words
    /// `words` each at the next index.
    pub fn run(&self, name: &str, constant: Option<u32>, buffers: &[&dyn Bound], words: &[u32], grid: Grid<'_>) -> Result<(), Error> {
        let kernel = lane::kernel(self.source, name, constant);
        let mut command = self.command.borrow_mut();
        let mut launch = command.launch(&kernel, name);
        for (i, b) in buffers.iter().enumerate() {
            let (b, at) = b.bound();
            launch = launch.slot(i as u64, b, at);
        }
        for slab in self.resident {
            launch = launch.uses(slab.device_buffer());
        }
        for (k, w) in words.iter().enumerate() {
            launch = launch.slot_value((buffers.len() + k) as u64, w);
        }
        let group = [TG, 1, 1];
        match grid {
            Grid::Groups(g) => launch.groups([g.max(1), 1, 1], group),
            Grid::Indirect(b, at) => {
                let (b, base) = b.bound();
                // The counts are read when it runs, on Metal, these passes' one backend: no most.
                launch.indirect(b, base + at, [u64::from(u32::MAX), 1, 1], group)
            }
        }
        Ok(())
    }
}

/// An exclusive scan of up to `capacity` u32s, its total in `total`. Only the levels the
/// capacity needs are encoded: 1 launch to 1,024 elements, 3 to 2^20, 5 past it. Level 0 is
/// dispatched at the capacity's blocks; it reads the count itself and writes the plan the later
/// levels' indirect dispatches read.
pub struct Scan {
    levels: u32,
    blocks: u64,
    plan: Buf,
    sums1: Buf,
    out1: Buf,
    sums2: Buf,
    out2: Buf,
    pub total: Buf,
}

impl Scan {
    pub fn new(arena: &Arena, capacity: usize) -> Result<Self, Error> {
        let n1 = capacity.div_ceil(BLOCK).max(1);
        let n2 = n1.div_ceil(BLOCK).max(1);
        if n2 > BLOCK {
            return Err(Error::BackendCapacity);
        }
        let levels = if capacity <= BLOCK { 1 } else if capacity <= BLOCK * BLOCK { 2 } else { 3 };
        Ok(Scan { levels, blocks: n1 as u64, plan: arena.zeroed::<u32>(20)?, sums1: arena.zeroed::<u32>(n1)?, out1: arena.zeroed::<u32>(n1)?, sums2: arena.zeroed::<u32>(n2)?, out2: arena.zeroed::<u32>(n2)?, total: arena.zeroed::<u32>(1)? })
    }

    /// `out` = the exclusive scan of `input`'s first `n[at]` words.
    pub fn encode(&self, enc: &Enc<'_>, n: &dyn Bound, at: u32, input: &dyn Bound, out: &dyn Bound) -> Result<(), Error> {
        let plan = &self.plan;
        let level0 = Grid::Groups(self.blocks);
        match self.levels {
            // One block: its sum is the total.
            1 => enc.run("pb_scan_block", None, &[input, out, &self.total, plan, n], &[0, at], level0),
            // Blocks, then one block of their sums.
            2 => {
                enc.run("pb_scan_block", None, &[input, out, &self.sums1, plan, n], &[0, at], level0)?;
                enc.run("pb_scan_block", None, &[&self.sums1, &self.out1, &self.total, plan, plan], &[1, 0], Grid::Groups(1))?;
                enc.run("pb_scan_add", None, &[out, &self.out1, plan], &[0], Grid::Indirect(plan, 52))
            }
            _ => {
                enc.run("pb_scan_block", None, &[input, out, &self.sums1, plan, n], &[0, at], level0)?;
                enc.run("pb_scan_block", None, &[&self.sums1, &self.out1, &self.sums2, plan, plan], &[1, 0], Grid::Indirect(plan, 28))?;
                enc.run("pb_scan_block", None, &[&self.sums2, &self.out2, &self.total, plan, plan], &[2, 0], Grid::Indirect(plan, 40))?;
                enc.run("pb_scan_add", None, &[&self.out1, &self.out2, plan], &[1], Grid::Indirect(plan, 64))?;
                enc.run("pb_scan_add", None, &[out, &self.out1, plan], &[0], Grid::Indirect(plan, 52))
            }
        }
    }
}

/// The segmented scan's operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegOp {
    Add = 0,
    Max = 1,
}

/// An inclusive segmented scan of up to `capacity` u32s, each with a head flag (a word, 0 or 1).
pub struct SegScan {
    blocks: u64,
    plan: Buf,
    seen0: Buf,
    levels: [(Buf, Buf, Buf, Buf); 2],
    last: (Buf, Buf, Buf, Buf),
}

impl SegScan {
    pub fn new(arena: &Arena, capacity: usize) -> Result<Self, Error> {
        let n1 = capacity.div_ceil(BLOCK).max(1);
        let n2 = n1.div_ceil(BLOCK).max(1);
        if n2 > BLOCK {
            return Err(Error::BackendCapacity);
        }
        // A level's aggregates (values, flags), its scan, and its `seen`.
        let level = |n: usize| -> Result<_, Error> { Ok((arena.zeroed::<u32>(n)?, arena.zeroed::<u32>(n)?, arena.zeroed::<u32>(n)?, arena.zeroed::<u32>(n)?)) };
        Ok(SegScan { blocks: n1 as u64, plan: arena.zeroed::<u32>(20)?, seen0: arena.zeroed::<u32>(capacity)?, levels: [level(n1)?, level(n2)?], last: level(1)? })
    }

    /// `out` = the inclusive scan of `input`'s first `n[at]` words, cut at `heads`.
    pub fn encode(&self, enc: &Enc<'_>, op: SegOp, n: &dyn Bound, at: u32, input: &dyn Bound, heads: &dyn Bound, out: &dyn Bound) -> Result<(), Error> {
        let plan = &self.plan;
        let c = Some(op as u32);
        let ([(v1, f1, o1, s1), (v2, f2, o2, s2)], (v3, f3, _, _)) = (&self.levels, &self.last);
        enc.run("pb_seg_block", c, &[input, heads, out, &self.seen0, &v1, &f1, plan, n], &[0, at], Grid::Groups(self.blocks))?;
        enc.run("pb_seg_block", c, &[&v1, &f1, &o1, &s1, &v2, &f2, plan, plan], &[1, 0], Grid::Indirect(plan, 28))?;
        enc.run("pb_seg_block", c, &[&v2, &f2, &o2, &s2, &v3, &f3, plan, plan], &[2, 0], Grid::Indirect(plan, 40))?;
        enc.run("pb_seg_add", c, &[&o1, &s1, &o2, plan], &[1], Grid::Indirect(plan, 64))?;
        enc.run("pb_seg_add", c, &[out, &self.seen0, &o1, plan], &[0], Grid::Indirect(plan, 52))
    }
}

/// A stable LSD radix sort of up to `capacity` u64 keys, each with a u32 payload (its index
/// before the sort, by `iota`).
pub struct Sort {
    tiles: u64,
    plan: Buf,
    hist: Buf,
    scanned: Buf,
    scan: Scan,
    pub keys_b: Buf,
    pub vals_a: Buf,
    pub vals_b: Buf,
}

impl Sort {
    pub fn new(arena: &Arena, capacity: usize) -> Result<Self, Error> {
        let h = capacity.div_ceil(TG as usize).max(1) * 256;
        Ok(Sort {
            tiles: capacity.div_ceil(TG as usize).max(1) as u64,
            plan: arena.zeroed::<u32>(8)?,
            hist: arena.zeroed::<u32>(h)?,
            scanned: arena.zeroed::<u32>(h)?,
            scan: Scan::new(arena, h)?,
            keys_b: arena.zeroed::<u64>(capacity)?,
            vals_a: arena.zeroed::<u32>(capacity)?,
            vals_b: arena.zeroed::<u32>(capacity)?,
        })
    }

    /// Sorts `keys` (the first `n[at]`) by their low `bits` bits, a multiple of 8, stably. The
    /// keys end in `keys` and their payloads in `vals_a` (the passes are even), or in `keys_b` and
    /// `vals_b`: `sorted` says which.
    pub fn encode(&self, enc: &Enc<'_>, n: &dyn Bound, at: u32, keys: &dyn Bound, bits: u32) -> Result<(), Error> {
        let plan = &self.plan;
        enc.run("pb_iota", None, &[&self.vals_a, plan, n], &[at], Grid::Groups(self.tiles))?;
        for p in 0..bits.div_ceil(8) {
            let (a, b): ([&dyn Bound; 2], [&dyn Bound; 2]) = ([keys, &self.vals_a], [&self.keys_b, &self.vals_b]);
            let ([ki, vi], [ko, vo]) = if p % 2 == 0 { (a, b) } else { (b, a) };
            enc.run("pb_hist", None, &[ki, &self.hist, plan], &[8 * p], Grid::Indirect(plan, 16))?;
            self.scan.encode(enc, plan, 2, &self.hist, &self.scanned)?;
            enc.run("pb_scatter", None, &[ki, vi, ko, vo, &self.scanned, plan], &[8 * p], Grid::Indirect(plan, 16))?;
        }
        Ok(())
    }

    /// Where the sorted keys and payloads are after `bits`: in the input and `vals_a`, or in
    /// `keys_b` and `vals_b`.
    pub fn in_input(bits: u32) -> bool {
        bits.div_ceil(8) % 2 == 0
    }
}

#[cfg(test)]
mod tests;
