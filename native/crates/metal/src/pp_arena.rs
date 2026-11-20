//! The preprocessor's evaluation arena and its kernels' one source (decisions 0448 §3, 0460):
//! every store an evaluation's passes use is at an offset in one block (`Arena`), leased once a
//! run within the one budget and bound at each store's offset; the layout (`Lay`) names each
//! store's offset by slot (`SLOTS`); and the passes are UKL (`pp_*.uglykl`), launched on the
//! process's device through the lane (`Enc`, `crate::lane`).

use crate::Error;
use crate::lane::{self, Buffer};
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
        let buffer = block.device_buffer(device).map_err(|_| Error::BackendFailure)?;
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

/// The evaluator's one source, UKL (decision 0460 §5): the tape's view and its reads written once
/// in CLIF (`<ukl/reads/tape.h>`, the view read where the batch holds it), the persistent grid's
/// barrier and primitives (`<ukl/grid.h>`), the work queue (`<ukl/workqueue.h>`), the arena's
/// slots (`SLOTS`, the source's `S_*`), the front passes (`pp_eval.uglykl`), the searches
/// (`pp_search.uglykl`), the expansion's passes (`pp_expand.uglykl`), the kernels that run them
/// all (`pp_grid.uglykl`) and the tests' probes (`pp_probes.uglykl`). The kernel store
/// preprocesses it for the device's backend.
pub fn source() -> &'static str {
    static SOURCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "#include <ukl/backend.h>\n#define CLIF_VIEW_SPACE UKL_GLOBAL\n{}#include <ukl/reads/tape.h>\n#include <ukl/bits.h>\n#include <ukl/grid.h>\n#include <ukl/workqueue.h>\n{}{}\n{}\n{}\n{}\n{}\n{}\n{}",
            rerail_clif::view::ukl(),
            SLOTS.iter().enumerate().map(|(k, s)| format!("#define S_{s} {k}u\n")).collect::<String>(),
            include_str!("pp_eval.uglykl"),
            include_str!("pp_search.uglykl"),
            include_str!("pp_expand.uglykl"),
            include_str!("pp_p1.uglykl"),
            include_str!("pp_if.uglykl"),
            include_str!("pp_grid.uglykl"),
            include_str!("pp_probes.uglykl")
        )
    });
    &SOURCE
}

/// The kernel `name` of the evaluator's source, made once a process (a failed compile is fatal).
pub fn kernel(name: &str) -> &'static lane::Kernel {
    static MADE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static lane::Kernel>>> = std::sync::OnceLock::new();
    let mut made = MADE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    made.entry(name.to_owned()).or_insert_with(|| Box::leak(Box::new(lane::kernel_fixed(source(), name, None))))
}

/// The threads of a group.
pub const TG: u64 = 256;

/// A probe's scalars (`pp_probes.uglykl`'s `PpProbeArgs`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProbeArgs(pub [u32; 4]);
unsafe impl lane::Pod for ProbeArgs {}

/// The launches of one command, on the process's device (`lane::Command`).
pub struct Enc<'a> {
    command: RefCell<lane::Command<'a>>,
}

impl<'a> Enc<'a> {
    pub fn new(label: &str) -> Enc<'a> {
        Enc { command: RefCell::new(lane::Command::new(label)) }
    }

    /// Kernel `name` over `groups` groups of `TG` threads, its buffer rows `rows` (each a row's
    /// name and what it binds), and its `args` where it has them.
    pub fn run(&self, name: &str, rows: &[(&'a str, &'a dyn Bound)], args: Option<ProbeArgs>, groups: u64) {
        let mut command = self.command.borrow_mut();
        let mut launch = command.launch(&**kernel(name), name);
        for &(row, b) in rows {
            let (buffer, offset) = b.bound();
            launch = launch.buffer_at(row, buffer, offset);
        }
        if let Some(a) = args {
            launch = launch.value("args", &a);
        }
        launch.groups([groups.max(1), 1, 1], [TG, 1, 1]);
    }

    /// The launches it holds (those within the budget).
    pub fn len(&self) -> usize {
        self.command.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Commits and waits: its time on the device, in seconds.
    pub fn finish(self) -> Result<f64, Error> {
        Ok(self.command.into_inner().run()?.device)
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

/// The persistent grid's groups with no core count (decision 0460 §4).
pub const GROUPS_UNKNOWN: u64 = 8;

/// The persistent grid's groups at most: a preprocessor unit is small, and a barrier costs more with
/// every group, so past 32 groups a run is slower (decision 0460: 80 groups took 168 ms of GPU time
/// over the corpus, 32 took 147 ms).
pub const GROUPS_MOST: u64 = 32;

/// The groups of the persistent grid (decision 0460 §4): the device's resident count at the grid
/// kernels' threadgroup memory (`Device::resident_groups`: its GPU cores, IORegistry's
/// `gpu-core-count` on a Mac, times the groups a core holds), at most `GROUPS_MOST`; `GROUPS_UNKNOWN`
/// with no count. A run
/// whose barrier fails runs again with half as many.
pub fn groups() -> u64 {
    static G: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *G.get_or_init(|| {
        let Ok(device) = lane::device() else { return GROUPS_UNKNOWN };
        let local = kernel("pp_setup").threadgroup_bytes();
        device.resident_groups(TG as u32, local).map_or(GROUPS_UNKNOWN, |g| g.min(GROUPS_MOST))
    })
}

/// The arena's stores the persistent kernels read, by slot: `pp_grid.uglykl`'s `S_*`, made from
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
    "P1_NODES", "P1_NDIRS", "P1_NGAPS", "P1_DFIRST", "P1_GFIRST", "P1_SIZE", "P1_CSIZE", "P1_GC", "P1_GHEADS", "P1_GSHIFT",
    "P1_USIZE", "P1_UTOTAL", "P1_BASE", "DIR_ACTS", "RTYPES", "N_STACK", "STK_MATCH", "STK_SRC", "STK_CHANGED", "STK_ROUNDS",
    "NQ", "P1_GCOUNT", "P1_GRPF", "P1_NGRP", "P1_GA", "P1_TAKEN", "P1_OWN", "P1_PAR", "P1_OWN2", "P1_PAR2", "P1_JCHANGED", "P1_ICHANGED",
    "P1_JUMPS", "P1_ITERS",
    "P1_DEPTHN", "N_FT", "P1_FTKEYS", "P1_FTFILES", "P1_FOUNDBY", "P1_MAXDEPTH", "P1_LIVE",
    "P1_MADE", "P1_MADEF", "P1_ENTER", "P1_CAND", "P1_ONCE", "P1_NEWF", "P1_NEWAT", "P1_NNEW", "P1_REQS", "P1_NREQ", "P1_FLAGS", "P1_NEED", "P1_UFIRST",
    "NC", "DC", "GC", "TC", "P1_UFILE",
    "RSTART", "RUNX", "RUNTOT", "RUN_END", "CNT", "CNT_CAP", "TR_SK", "TR_NK", "TR_TY", "TO_F", "TO_S", "CU_SLOT", "CU_SUB", "CU_UNIT", "TRN", "LREAD", "LB_CAP", "LDELTA", "LKEY", "LVAL", "LVALN", "LHEAD", "LATEST", "LATESTN", "ILINE", "ILINEN", "SLINE", "SLINEN", "LCTL", "IF_COUNT", "IF_FIRST", "IF_KIND", "IF_INST", "SINST", "P1_RNAMES", "P1_RNTOP", "RNC", "IF_AT", "IF_TOTAL", "IF_NODES", "IF_OPS", "IF_VAL", "IF_KEYS", "IF_TRUE", "N_IFOP", "IF_OPKEYS", "IF_OPKINDS", "N_FEAT", "IF_FEATKEYS", "IF_FEATVALS",
    "THEADS", "DELTA", "DEPTH", "CLOSE", "CLOSE_AT", "CLOSE_TOTAL", "CLOSE_KEYS", "CSORT_KB", "CSORT_VA", "CSORT_VB", "CSORT_HIST",
    "CLOSE_BITS", "CEND", "EHEADS", "REACH", "STATE", "OFLAG", "OID", "OTOTAL", "LISTS", "SLOTS", "CTL", "QCTL", "XFLAG", "XAT",
    "XTOTAL", "XPOS", "XCOVER", "V", "OBASE", "VTOTAL", "OUT", "TOKENS", "ARGS", "WORDS", "SCRATCH", "LIST_CAP", "OUT_CAP", "Q_CAP",
];

fn slot(name: &str) -> usize {
    SLOTS.iter().position(|s| *s == name).unwrap_or_else(|| panic!("no arena slot {name}"))
}

/// A run's layout: each slot's store offset in the arena, or its word.
pub struct Lay(Vec<u32>, Vec<usize>);

impl Lay {
    pub fn new() -> Lay {
        Lay(vec![0; SLOTS.len()], vec![0; SLOTS.len()])
    }

    /// Slot `name` at `b`'s offset.
    pub fn at(&mut self, name: &str, b: &Buf) -> &mut Lay {
        self.0[slot(name)] = u32::try_from(b.at).expect("an arena past 4 GiB");
        self.1[slot(name)] = b.len;
        self
    }

    /// The `k` largest stores the layout names, by bytes (`RERAIL_PP_ARENA_LOG`'s report).
    pub fn largest(&self, k: usize) -> Vec<(&'static str, usize)> {
        let mut v: Vec<(&'static str, usize)> = SLOTS.iter().copied().zip(self.1.iter().copied()).filter(|s| s.1 > 0).collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.1));
        v.truncate(k);
        v
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
    fn bound(&self) -> (&Buffer, u64) {
        let l = self.leased();
        (&l.buffer, l.base as u64)
    }
}

#[cfg(test)]
mod tests;
