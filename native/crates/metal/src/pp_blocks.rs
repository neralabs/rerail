//! The building blocks of the preprocessor's grid passes (decision 0448 §3), `pp_blocks.metal`:
//! a scan, a segmented scan (add or max), a stable radix sort of 64-bit keys, a search and an
//! expand. Each is sized on the device: its count is a word of a device buffer, a plan kernel
//! makes the levels' counts and the indirect arguments, and the host encodes every dispatch
//! without reading one.
//!
//! Scratch is placed when a block is made, for a capacity the host knows (the arena's), so
//! encoding never allocates. A count past the capacity is the caller's to refuse before.
//!
//! Every store an evaluation's passes use is at an offset in one block, its arena (decision 0460
//! §3, `Arena`): leased once a run within the one budget, and bound at each store's offset.

use crate::Error;
use crate::launch::CheckedComputeEncoder;
use crate::runtime::pipeline_named_constant;
use metal::{BufferRef, CommandBufferRef, MTLSize};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

/// A store's alignment in its arena: every binding offset and every type's.
const ALIGN: usize = 256;

/// An evaluation's arena (decision 0460 §3): one `rerail_blocks::Block`, leased from the GPU pool
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
    buffer: metal::Buffer,
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
        let device = crate::device()?;
        let block = rerail_accel::gpu::shared::lease_block_within(rerail_blocks::block::kind::SCRATCH, bytes + 2 * ALIGN + 4096)
            .map_err(|_| Error::BackendCapacity)?;
        // Zeroed: a pooled slab may hold an earlier block's bytes.
        let base = block.try_alloc(bytes, ALIGN).ok_or(Error::BackendCapacity)?;
        let buffer = block.gpu_buffer(&device);
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
    fn bound(&self) -> (&BufferRef, u64);
}

impl Bound for Buf {
    fn bound(&self) -> (&BufferRef, u64) {
        let l = self.arena.leased();
        (&l.buffer, (l.base + self.at) as u64)
    }
}

impl<T: Bound + ?Sized> Bound for &T {
    fn bound(&self) -> (&BufferRef, u64) {
        (**self).bound()
    }
}

impl Bound for BufferRef {
    fn bound(&self) -> (&BufferRef, u64) {
        (self, 0)
    }
}

impl Bound for metal::Buffer {
    fn bound(&self) -> (&BufferRef, u64) {
        (self, 0)
    }
}

/// The evaluator's one source: the tape's view and its reads written once in CLIF
/// (`<ukl/reads/tape.h>`, the view read where the batch holds it), the persistent grid's barrier
/// and primitives (`<ukl/grid.h>`), the arena's slots (`SLOTS`), the front passes
/// (`pp_eval.metal`), the searches (`pp_blocks.metal`), the work queue (`work_queue.metal`), the
/// expansion's passes (`pp_expand.metal`) and the kernels that run them all (`pp_grid.metal`); preprocessed for Metal once (`rerail_ukl::metal_source`, decision 0440
/// §3.3) until the passes are UKL themselves (decision 0460 §5).
pub fn source() -> Result<&'static str, Error> {
    static SOURCE: std::sync::LazyLock<Result<String, String>> = std::sync::LazyLock::new(|| {
        let text = format!(
            "#include <metal_stdlib>\nusing namespace metal;\n#define CLIF_VIEW_SPACE UKL_GLOBAL\n{}#include <ukl/reads/tape.h>\n#include <ukl/grid.h>\n{}{}\n{}\n{}\n{}\n{}",
            rerail_clif::view::ukl(),
            SLOTS.iter().enumerate().map(|(k, s)| format!("#define S_{s} {k}u\n")).collect::<String>(),
            include_str!("pp_eval.metal"),
            include_str!("pp_blocks.metal"),
            include_str!("work_queue.metal"),
            include_str!("pp_expand.metal"),
            include_str!("pp_grid.metal")
        );
        // A failed source is fatal (no fallback): its reason is said once, where it is made.
        rerail_ukl::metal_source("pp.metal", &text).map_err(|e| {
            eprintln!("the preprocessor's evaluator source: {e}");
            e.to_string()
        })
    });
    SOURCE.as_ref().map(String::as_str).map_err(|_| Error::BackendFailure)
}

/// A dispatch's size: its groups. Every count past it is read on the device (decision 0460 §4),
/// so no dispatch takes indirect arguments.
#[derive(Clone, Copy)]
pub enum Grid {
    Groups(u64),
}

/// The threads of a group.
pub const TG: u64 = 256;

/// Encodes passes of `source` into a command buffer.
pub struct Enc<'a> {
    pub command: &'a CommandBufferRef,
    pub source: &'a str,
}

impl Enc<'_> {
    /// Kernel `name` (with `constant` at function constant 0) over `buffers`, then the words
    /// `words` each at the next index.
    pub fn run(&self, name: &str, constant: Option<u32>, buffers: &[&dyn Bound], words: &[u32], grid: Grid) -> Result<(), Error> {
        let pipe = pipeline_named_constant(self.source, name, constant)?;
        let e = self.command.new_compute_command_encoder();
        // The launch budget records each dispatch by its encoder's label (an unlabelled one's is nil).
        e.set_label(name);
        e.set_compute_pipeline_state(&pipe);
        let bound: Vec<(&BufferRef, u64)> = buffers.iter().map(|b| b.bound()).collect();
        e.checked_set_buffers_at(self.command, &bound);
        for (k, w) in words.iter().enumerate() {
            e.set_bytes((buffers.len() + k) as u64, 4, std::ptr::from_ref(w).cast());
        }
        let group = MTLSize::new(TG, 1, 1);
        let Grid::Groups(g) = grid;
        e.checked_dispatch_thread_groups(self.command, MTLSize::new(g.max(1), 1, 1), group);
        e.end_encoding();
        Ok(())
    }
}


/// A scan's total: the grid's scans (`ukl_grid_scan_n`) need no other store of their own (their
/// groups' sums are the arena's one `SUMS`).
pub struct Scan {
    pub total: Buf,
}

impl Scan {
    pub fn new(arena: &Arena) -> Result<Self, Error> {
        Ok(Scan { total: arena.zeroed::<u32>(1)? })
    }
}

/// A stable LSD radix sort of up to `capacity` u64 keys, each with a u32 payload (its index
/// before the sort): the grid's (`ukl_grid_sort_u64`), its other buffers and its histograms.
pub struct Sort {
    pub keys_b: Buf,
    pub vals_a: Buf,
    pub vals_b: Buf,
    pub hist: Buf,
}

impl Sort {
    pub fn new(arena: &Arena, capacity: usize) -> Result<Self, Error> {
        let tiles = capacity.div_ceil(1024).max(1);
        Ok(Sort {
            keys_b: arena.zeroed::<u64>(capacity)?,
            vals_a: arena.zeroed::<u32>(capacity)?,
            vals_b: arena.zeroed::<u32>(capacity)?,
            hist: arena.zeroed::<u32>(256 * tiles + 1)?,
        })
    }

    /// Where the sorted keys and payloads are after `bits`: in the input and `vals_a`, or in
    /// `keys_b` and `vals_b`.
    pub fn in_input(bits: u32) -> bool {
        bits.div_ceil(8) % 2 == 0
    }
}

/// The groups of the persistent grid at most (decision 0460 §4); a run whose barrier fails runs
/// again with half as many.
pub const GROUPS: u64 = 32;

/// The arena's stores the persistent kernels read, by slot: `pp_grid.metal`'s `S_*`, made from
/// this list (`source`), and a run's offsets (`Lay`). A word slot holds a value, not an offset.
pub const SLOTS: &[&str] = &[
    "GRID", "SUMS", "LAST", "STOP", "N_FILES", "NODES_FLAG", "FILES", "BASES", "TOTAL", "DIR_BASES", "DIR_TOTAL", "DFLAG", "GFLAG", "AFLAG", "DEV", "DRANK", "GRANK", "ARANK", "NDEPTH", "DEV_TOTAL", "N_GROUPS", "N_ARMS",
    "ARM_KEYS", "ASORT_KB", "ASORT_VA", "ASORT_VB", "ASORT_HIST", "GRP_KEYS", "PSORT_KB", "PSORT_VA", "PSORT_VB", "PSORT_HIST", "G_OUT", "G_ARMS",
    "DIR_GROUPS", "GUARDED", "GUARDS", "BAD", "KEYS", "NODES_OUT", "ROWS", "DIR_KEYS",
    "DEFS", "PCOUNT", "BCOUNT", "PFIRST", "IFIRST", "PTOTAL", "ITOTAL", "PKEYS", "ITEMS", "ROOM", "CAP", "VA_LO", "VA_HI",
    "INSTS", "N_INSTS", "DSLOT", "DACTIVE", "N_DIRS", "GSLOT", "GACTIVE", "N_GAPS", "N_BUILTIN", "ND", "NG", "OWNER_IN", "VHEADS",
    "OWNER", "TFLAG", "TRANK", "TTOTAL", "TEXT_NODES", "RFLAG", "RAT", "RTOTAL", "SORT_KEYS", "NAME_KEYS", "VALUES", "N_ROWS",
    "SORT_KB", "SORT_VA", "SORT_VB", "SORT_HIST", "GCOUNTS", "GFIRSTS", "GKEYS", "GSORT_KB", "GSORT_VA", "GSORT_VB", "GSORT_HIST",
    "SSLOT", "GAT", "GTOTAL", "LOOKUPS", "VBAD", "FLOORS", "N_UNITS",
    "THEADS", "DELTA", "DEPTH", "CLOSE", "CLOSE_AT", "CLOSE_TOTAL", "CLOSE_KEYS", "CSORT_KB", "CSORT_VA", "CSORT_VB", "CSORT_HIST",
    "CLOSE_BITS", "CEND", "EHEADS", "REACH", "STATE", "OFLAG", "OID", "OTOTAL", "LISTS", "SLOTS", "CTL", "QCTL", "XFLAG", "XAT",
    "XTOTAL", "XPOS", "XCOVER", "V", "OBASE", "VTOTAL", "OUT", "TOKENS", "ARGS", "WORDS", "SCRATCH", "LIST_CAP", "OUT_CAP", "Q_CAP",
];

fn slot(name: &str) -> usize {
    SLOTS.iter().position(|s| *s == name).unwrap_or_else(|| panic!("no arena slot {name}"))
}

/// A run's layout: each slot's store offset in the arena, or its word.
pub struct Lay(Vec<u32>);

impl Lay {
    pub fn new() -> Lay {
        Lay(vec![0; SLOTS.len()])
    }

    /// Slot `name` at `b`'s offset.
    pub fn at(&mut self, name: &str, b: &Buf) -> &mut Lay {
        self.0[slot(name)] = u32::try_from(b.at).expect("an arena past 4 GiB");
        self
    }

    /// Slot `name` holding `v`.
    pub fn word(&mut self, name: &str, v: u32) -> &mut Lay {
        self.0[slot(name)] = v;
        self
    }

    /// Slot `name`'s offset or word.
    pub fn get(&self, name: &str) -> u32 {
        self.0[slot(name)]
    }

    /// Writes the layout into its store (after the arena is leased).
    pub fn write(&self, into: &Buf) {
        assert!(into.byte_length() >= 4 * self.0.len());
        // SAFETY: the store holds `SLOTS.len()` words, and no command reads it yet.
        unsafe { std::slice::from_raw_parts_mut(into.host().cast::<u32>(), self.0.len()) }.copy_from_slice(&self.0); // copy: not block data (a run's layout words)
    }
}

impl Default for Lay {
    fn default() -> Self {
        Lay::new()
    }
}

impl Bound for Arena {
    /// The arena whole: its stores' first byte, where every slot's offset counts from.
    fn bound(&self) -> (&BufferRef, u64) {
        let l = self.leased();
        (&l.buffer, l.base as u64)
    }
}

#[cfg(test)]
mod tests;
