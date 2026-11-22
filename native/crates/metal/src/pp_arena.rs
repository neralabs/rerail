//! The preprocessor's evaluation arena and its kernels' one source (decisions 0448 §3, 0468):
//! every store an evaluation's passes use is at an offset in one block (`Arena`), leased once a
//! run within the one budget and bound at each store's offset; the layout (`Lay`) names each
//! store's offset by slot (`SLOTS`); and the passes are UKL (`pp_*.uglykl`), launched on the
//! process's device through the lane (`Enc`, `crate::lane`).

use crate::Error;
use crate::lane::{self, Buffer};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

/// A store's alignment in its arena at the least: every binding offset and every type's. A device that binds a
/// region only at a coarser alignment (OpenCL's sub-buffers: 512 on NVIDIA's) has its own (`Device::bind_align`).
const ALIGN: usize = 256;

/// A store's alignment in an arena on the process's device.
fn store_align() -> usize {
    lane::device().map_or(ALIGN, |d| (d.bind_align() as usize).max(ALIGN).next_power_of_two())
}

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
    /// Its stores' alignment (`store_align`), found at the first store.
    align: Cell<usize>,
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

    fn align(&self) -> usize {
        if self.0.align.get() == 0 {
            self.0.align.set(store_align());
        }
        self.0.align.get()
    }

    fn place(&self, len: usize) -> Buf {
        let at = self.0.end.get();
        self.0.end.set(at + len.next_multiple_of(self.align()));
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
        let align = self.align();
        let bytes = self.bytes().max(align);
        let device = lane::device()?;
        let block = rerail_accel::gpu::shared::lease_block_within(rerail_blocks::block::kind::SCRATCH, bytes + 2 * align + 4096)
            .map_err(|why| capacity(&format!("{}:{}: {why}", file!(), line!())))?;
        // Zeroed: a pooled slab may hold an earlier block's bytes.
        let base = block.try_alloc(bytes, align).ok_or_else(|| capacity(concat!(file!(), ":", line!())))?;
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

/// The evaluator's one source, UKL (decision 0468 §5): the tape's view and its reads written once
/// in CLIF (`<ukl/reads/tape.h>`, the view read where the batch holds it), the persistent grid's
/// barrier and primitives (`<ukl/grid.h>`), the arena's slots (`SLOTS`, the source's `S_*`), the front passes (`pp_eval.uglykl`), the searches
/// (`pp_search.uglykl`), the expansion's passes (`pp_expand.uglykl`), the kernels that run them
/// all (`pp_grid.uglykl`) and the tests' probes (`pp_probes.uglykl`). The kernel store
/// preprocesses it for the device's backend.
pub fn source() -> &'static str {
    static SOURCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "#include <ukl/backend.h>\n{PP_GLOBAL}#define CLIF_VIEW_SPACE UKL_GLOBAL\n{}#include <ukl/reads/tape.h>\n#include <ukl/bits.h>\n#include <ukl/grid.h>\n{}{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            rerail_clif::view::ukl(),
            SLOTS.iter().enumerate().map(|(k, s)| format!("#define S_{s} {k}u\n")).collect::<String>(),
            include_str!("pp_eval.uglykl"),
            include_str!("pp_search.uglykl"),
            include_str!("pp_expand.uglykl"),
            include_str!("pp_p1.uglykl"),
            include_str!("pp_if.uglykl"),
            include_str!("pp_loop.uglykl"),
            include_str!("pp_grid.uglykl"),
            include_str!("pp_probes.uglykl")
        )
    });
    &SOURCE
}

/// The passes' space for the stores they write (`PP_GLOBAL`): a phase reads what other groups wrote before the grid's
/// barrier, so on a GPU it is coherent memory (`UKL_COHERENT`, `ukl/grid.h`'s rule). On an M1 Max a plain device read
/// of another group's write came back stale every time once the grid spanned more than 8 groups, and a coherent one
/// never did (Q367). The CPU's caches are coherent, so there it is a plain pointer. CUDA's
/// `UKL_COHERENT` is `volatile`, from which C++ copies no struct, and the passes copy theirs: a plain pointer there
/// too (not run on CUDA yet).
const PP_GLOBAL: &str = "#if defined(UKL_METAL) || defined(UKL_OPENCL)\n#define PP_GLOBAL UKL_COHERENT\n#else\n#define PP_GLOBAL UKL_GLOBAL\n#endif\n";

/// The kernel `name` of the evaluator's source, made once a machine (a failed compile is fatal).
pub fn kernel(name: &str) -> lane::Kernel {
    let made = rerail_context::machine::<Kernels>();
    let mut made = made.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    std::sync::Arc::clone(made.entry(name.to_owned()).or_insert_with(|| lane::kernel_fixed(source(), name, None)))
}

/// The machine's kernels of the evaluator, by name (`kernel`): a machine object (decision 0442), as its device is.
#[derive(Default)]
struct Kernels(std::sync::Mutex<std::collections::HashMap<String, lane::Kernel>>);

/// The threads of a group on a GPU.
pub const TG: u64 = 256;

/// The threads of a group on the process's device: `TG` on a GPU; 32 on the CPU (decision 0527
/// §3), where each of a group's threads is a coroutine that every barrier switches, so a phase
/// costs its group's width in switches (the radix sort's tiles run 8 of their threads a thread).
pub fn tg() -> u64 {
    static T: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *T.get_or_init(|| if matches!(lane::device(), Ok(lane::Device::Cpu(_))) { 32 } else { TG })
}

/// A probe's scalars (`pp_probes.uglykl`'s `PpProbeArgs`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProbeArgs(pub [u32; 4]);
unsafe impl lane::Pod for ProbeArgs {}

/// The launches of one command, on the process's device (`lane::Command`).
pub struct Enc<'a> {
    command: RefCell<lane::Command>,
    /// The slabs each launch makes resident: a unit's documents past the bound one (`pp_eval::Unit`, 0471).
    resident: &'a [std::sync::Arc<crate::slabs::Slab>],
}

impl<'a> Enc<'a> {
    pub fn new(label: &str) -> Enc<'a> {
        Enc::over(label, &[])
    }

    /// An `Enc` whose launches make `resident` resident.
    pub fn over(label: &str, resident: &'a [std::sync::Arc<crate::slabs::Slab>]) -> Enc<'a> {
        Enc { command: RefCell::new(lane::Command::new(label)), resident }
    }

    /// Kernel `name` over `groups` groups of `TG` threads, its buffer rows `rows` (each a row's
    /// name and what it binds), and its `args` where it has them.
    pub fn run(&self, name: &str, rows: &[(&'a str, &'a dyn Bound)], args: Option<ProbeArgs>, groups: u64) {
        let mut command = self.command.borrow_mut();
        let mut launch = command.launch(&kernel(name), name);
        for &(row, b) in rows {
            let (buffer, offset) = b.bound();
            launch = launch.buffer_at(row, buffer, offset);
        }
        for slab in self.resident {
            launch = launch.uses_read(slab.device_buffer());
        }
        if let Some(a) = args {
            launch = launch.value("args", &a);
        }
        launch.groups([groups.max(1), 1, 1], [tg(), 1, 1]);
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
        bits.div_ceil(8).is_multiple_of(2)
    }
}

/// The persistent grid's groups with no core count (decision 0468 §4).
pub const GROUPS_UNKNOWN: u64 = 8;

/// The persistent grid's groups at most: a barrier costs more with every group (decision 0468: 80
/// groups took 168 ms of GPU time over the corpus, 32 took 147 ms). Since the loop (0588) a phase's
/// work is latency's, and more threads hide it: the corpus batch on the GB10 (2026-11-22) took
/// 5.95 ms at 16 groups, 4.78 at 32, 4.23 at 64 and 4.19 at 96.
pub const GROUPS_MOST: u64 = 64;

/// The persistent grid's groups dispatched (decision 0468 §4, 0568): its kernel's resident groups (`residency`), at
/// most `GROUPS_MOST`; 1 on the CPU.
pub fn groups() -> u64 {
    let Ok(device) = lane::device() else { return GROUPS_UNKNOWN };
    // The CPU runs a launch's groups as coroutines on one host thread: past one group, a group adds work to no core,
    // only its threads' switches at every barrier.
    if matches!(device, lane::Device::Cpu(_)) {
        return 1;
    }
    // `RERAIL_PP_GROUPS`: that many at most in place of `GROUPS_MOST`, a measurement aid.
    let most = std::env::var("RERAIL_PP_GROUPS").ok().and_then(|g| g.parse().ok()).unwrap_or(GROUPS_MOST);
    residency().groups.min(most)
}

/// `ukl/grid.h`'s `UKL_GRID_SMALL`: at most this many values, a grid primitive is group 0's alone.
pub const GRID_SMALL: usize = 4096;

/// The groups for `nodes` items of work (a unit's nodes, or its expansion's tokens): one a `GRID_SMALL` of them, at
/// least one, at most `groups()`. Past
/// its work a group only waits at the grid's barriers, and a barrier costs more with every group: on a GB10 under
/// CUDA, an evaluation test's 18 runs took 58 ms of GPU time at 1 group, 173 ms at 8 and 640 ms at 32 (2026-11-21).
pub fn groups_for(nodes: usize) -> u64 {
    (nodes.div_ceil(GRID_SMALL).max(1) as u64).min(groups())
}

/// The grid kernel's resident groups on this machine (decision 0568, `rerail_device::grid`): the backend's answer
/// (CUDA's occupancy, the CPU's rule), else the measure kept for the machine, else the device's bound until a
/// failed barrier measures it. Found once a process, a machine object (decision 0442), and lowered by `halved`.
pub fn residency() -> rerail_device::grid::Residency {
    let held = rerail_context::machine::<Groups>();
    let mut held = held.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *held.get_or_insert_with(|| {
        let Ok(device) = lane::device() else { return rerail_device::grid::Residency { groups: GROUPS_UNKNOWN, known: rerail_device::grid::Known::Bound } };
        let k = kernel("pp_setup");
        rerail_device::grid::residency(device, &k, &grid_key(), tg(), k.threadgroup_bytes())
    })
}

/// The grid kernel's key in the machine's measures: its source and name.
fn grid_key() -> String {
    rerail_device::store::key(&[source().as_bytes(), b"pp_setup"])
}

/// `BackendCapacity`, refused at `at` (`file:line`), which `RERAIL_PP_WHY` prints: a refusal for room or a budget is
/// the caller's to rerun smaller, and the log says which one refused.
pub fn capacity(at: &str) -> Error {
    if std::env::var_os("RERAIL_PP_WHY").is_some() {
        eprintln!("pp: BackendCapacity at {at}");
    }
    Error::BackendCapacity
}

/// A dispatch of `groups` groups past the grid kernel's resident groups is refused (`BackendCapacity`, decision
/// 0568): its groups past them would wait at a barrier for groups that do not run. The join (`ukl_grid_join`) is the
/// floor below this, should a device run fewer than it holds.
pub fn refuse_past(groups: u64) -> Result<(), Error> {
    if groups > residency().groups { Err(capacity(concat!(file!(), ":", line!()))) } else { Ok(()) }
}

/// `BackendFailure`, at `at` (`file:line`), which `RERAIL_PP_WHY` prints: a device run whose answer does not add up
/// (or a barrier that failed), and the log says which check found it.
pub fn failure(at: &str) -> Error {
    if std::env::var_os("RERAIL_PP_WHY").is_some() {
        eprintln!("pp: BackendFailure at {at}");
    }
    Error::BackendFailure
}

/// A grid of `groups` groups failed a barrier: the grid kernel's resident groups are half of it from now on, kept
/// for the machine (`rerail_device::grid::halved`), and the run is the caller's to run again at them. `None` at
/// one group, which no halving helps, and where the backend answered (`Known::Api`): its count is resident, so its
/// failed barrier is the kernel's failure, not a residency to measure.
pub fn halved(groups: u64) -> Option<u64> {
    if groups <= 1 || residency().known == rerail_device::grid::Known::Api {
        return None;
    }
    let held = rerail_context::machine::<Groups>();
    let mut held = held.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let half = match lane::device() {
        Ok(device) => rerail_device::grid::halved(device, &grid_key(), tg(), groups),
        Err(_) => groups / 2,
    };
    *held = Some(rerail_device::grid::Residency { groups: half, known: rerail_device::grid::Known::Measured });
    Some(half)
}

/// The machine's grid residency (`residency`), found once: a machine object (decision 0442), as its device is.
#[derive(Default)]
struct Groups(std::sync::Mutex<Option<rerail_device::grid::Residency>>);

/// The arena's stores the persistent kernels read, by slot: `pp_grid.uglykl`'s `S_*`, made from
/// this list (`source`), and a run's offsets (`Lay`). A word slot holds a value, not an offset.
pub const SLOTS: &[&str] = &[
    "GRID", "SUMS", "LAST", "STOP", "N_FILES", "NODES_FLAG", "FILES", "BASES", "TOTAL", "DIR_BASES", "DIR_TOTAL", "DFLAG", "GFLAG", "AFLAG", "DEV", "DRANK", "GRANK", "ARANK", "NDEPTH", "DEV_TOTAL", "N_GROUPS", "N_ARMS",
    "ARM_KEYS", "ASORT_KB", "ASORT_VA", "ASORT_VB", "ASORT_HIST", "GRP_KEYS", "PSORT_KB", "PSORT_VA", "PSORT_VB", "PSORT_HIST", "G_OUT", "G_ARMS",
    "DIR_GROUPS", "GUARDED", "GUARDS", "BAD", "KEYS", "NODES_OUT", "ROWS", "DIR_KEYS",
    "DEFS", "PCOUNT", "BCOUNT", "PFIRST", "IFIRST", "PTOTAL", "ITOTAL", "PKEYS", "ITEMS", "IROWS", "VAI", "ROOM", "CAP", "VA_LO", "VA_HI", "VO_LO", "VO_HI",
    "INSTS", "N_INSTS", "DSLOT", "DACTIVE", "N_DIRS", "GSLOT", "GACTIVE", "N_GAPS", "N_BUILTIN", "ND", "NG", "OWNER_IN", "VHEADS",
    "OWNER", "TFLAG", "TRANK", "TTOTAL", "TEXT_NODES", "RFLAG", "RAT", "RTOTAL", "SORT_KEYS", "NAME_KEYS", "VALUES", "N_ROWS",
    "SORT_KB", "SORT_VA", "SORT_VB", "SORT_HIST", "GCOUNTS", "GFIRSTS", "GKEYS", "GSORT_KB", "GSORT_VA", "GSORT_VB", "GSORT_HIST",
    "SSLOT", "GAT", "GTOTAL", "LOOKUPS", "TOPNODE", "TROWS", "VBAD", "FLOORS", "N_UNITS",
    "P1_NODES", "P1_NDIRS", "P1_NGAPS", "P1_DFIRST", "P1_GFIRST", "P1_SIZE", "P1_CSIZE", "P1_GC", "P1_GHEADS", "P1_GSHIFT",
    "P1_USIZE", "P1_UTOTAL", "P1_BASE", "DIR_ACTS", "RTYPES", "N_STACK", "STK_MATCH", "STK_SRC", "STK_CHANGED", "STK_ROUNDS",
    "NQ", "P1_GCOUNT", "P1_GRPF", "P1_NGRP", "P1_GA", "P1_TAKEN", "P1_OWN", "P1_PAR", "P1_OWN2", "P1_PAR2", "P1_JCHANGED", "P1_ICHANGED",
    "P1_JUMPS", "P1_ITERS",
    "P1_DEPTHN", "N_FT", "P1_FTKEYS", "P1_FTFILES", "P1_FOUNDBY", "P1_MAXDEPTH", "P1_LIVE",
    "P1_MADE", "P1_MADEF", "P1_ENTER", "P1_CAND", "P1_ONCE", "P1_NEWF", "P1_NEWAT", "P1_NNEW", "P1_REQS", "P1_NREQ", "P1_FLAGS", "P1_NEED", "P1_UFIRST",
    "NC", "DC", "GC", "TC", "P1_UFILE",
    "RSTART", "RUNX", "RUNTOT", "RUN_END", "CNT", "CNT_CAP", "TR_SK", "TR_NK", "TR_TY", "TO_F", "TO_S", "CU_SLOT", "CU_SUB", "CU_UNIT", "TRN", "LREAD", "LB_CAP", "LDELTA", "LKEY", "LVAL", "LVALN", "LHEAD", "LATEST", "LATESTN", "ILINE", "ILINEN", "SLINE", "SLINEN", "LCTL", "IF_COUNT", "IF_FIRST", "IF_KIND", "IF_INST", "SINST", "P1_RNAMES", "P1_RNTOP", "RNC", "RQC", "IF_AT", "IF_TOTAL", "IF_NODES", "IF_OPS", "IF_VAL", "IF_KEYS", "IF_TRUE", "N_IFOP", "IF_OPKEYS", "IF_OPKINDS", "N_FEAT", "IF_FEATKEYS", "IF_FEATVALS",
    "THEADS", "DELTA", "DEPTH", "STATE", "OFLAG", "CTL", "XFLAG", "XAT", "XTOTAL", "XPOS", "V", "OBASE", "LOB", "PLAST", "VTOTAL", "OPV", "OPBASE",
    "OPTOTAL", "OUT", "SC_DEPTH", "SC_CLOSE", "SC_CEND", "SC_REACH", "SC_LIVE", "SC_RANK", "SC_CKEYS", "SC_KB", "SC_VA", "SC_VB", "SC_HIST",
    "SC_CAP", "TOKENS", "WORDS", "SCRATCH", "OUT_CAP",
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
