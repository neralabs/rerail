//! The one Metal manager for relational work (decision 0100 §2): what every relational submission on the GPU shares,
//! once per process.
//!
//! - **The device and one command queue.** The executor's GPU lane and the engine's sorts and probes
//!   (`metal_sort`, `metal_rows`) commit to the same queue, so their command buffers are ordered with each other.
//! - **Pipelines** by a key the caller chooses (a kernel's name and its variant): compiled once, found without
//!   hashing their source.
//! - **A pool of shared buffers** by size class: powers of two from 16 KiB to 1 MiB, then four classes a doubling
//!   (1, 1¼, 1½, 1¾ of a power of two), so a large lease wastes at most a quarter. A buffer a command has used is
//!   resident already, so a pooled one costs neither its creation nor its first mapping: about 6 µs against 0.1 µs
//!   for a small one (0095), and about 20 ms a GiB for a large one (0266 §12), which is why large buffers are kept.
//! - **One budget for all GPU memory** (0266 §16). The Metal budget (`crate::budget::metal`) holds what
//!   `rerail-metal`'s ledger reserves (`set_other_reserved`), every live lease (relational work's, the device
//!   blocks of `rerail_metal::pool`, the tape residency's slabs) and every idle buffer. Idle buffers fill what the
//!   rest leaves, no more: past it, the least recently returned are released first. `set_slab_idle_limit` caps idle
//!   slabs further, when set.
//! - **One eviction order.** Room for a buffer (`make_room`) is made by releasing idle buffers, least recently
//!   returned first, then by retiring what the evictor holds (`set_evictor`: the residency's least recently used
//!   slab), whose memory comes back as idle buffers once its last reader lets go.
//! - **Returning.** A lease goes back when it is dropped. Its holder drops it once every command buffer that reads
//!   it has completed (the host's next write into it is not ordered after a GPU read), or hands it to a command
//!   buffer (`Pooled::after`), which returns it when the command buffer completes.
//! - **Blocks in slabs** (`lease_block`): a block (`rerail_blocks`) whose memory is a pooled buffer, bound to the
//!   device by that buffer: pinned once, recycled run after run. A block that outgrows its slab moves to a larger
//!   one by one copy (`grow_block`); its references are self-relative, so the copy is the move.
//! - **The guard.** A function the engine sets to its launch budget. Every submission checks it first, on the
//!   submitting thread, so relational work is refused by the same budget as the rest of the engine's GPU work.
//!
//! Everything here is safe to share between threads: Metal's device, queues and pipelines are, and the pool is
//! behind a lock.

use metal::{Buffer, BufferRef, CommandQueue, ComputePipelineState, Device, MTLResourceOptions};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The smallest size class.
const SMALLEST: usize = 16 << 10;
/// The smallest buffer counted as a slab: kept idle under the slab allowance.
pub const SLAB_MIN: usize = 16 << 20;
/// The device's memory and pipelines on this machine (decision 0442: the machine's): the cap on idle slabs, the
/// bytes leased, the pipelines made, the buffer pool.
#[derive(Default)]
pub(crate) struct Memory {
    /// The cap on idle slabs when set (`set_slab_idle_limit`); 0: none beside the budget.
    slab_idle: std::sync::atomic::AtomicUsize,
    /// The bytes leased now (and not yet dropped): what relational work holds on the device, for the profile.
    live: std::sync::atomic::AtomicUsize,
    pipelines: Mutex<HashMap<String, Slot>>,
    pool: Mutex<Pool>,
}

fn dev() -> std::sync::Arc<Memory> {
    rerail_context::machine::<Memory>()
}

/// Sets how many bytes of idle slabs the pool keeps.
pub fn set_slab_idle_limit(bytes: usize) {
    dev().slab_idle.store(bytes.max(1), std::sync::atomic::Ordering::Relaxed);
}

impl Memory {
    fn slab_idle_limit(&self) -> usize {
        match self.slab_idle.load(std::sync::atomic::Ordering::Relaxed) {
            0 => usize::MAX,
            b => b,
        }
    }

    /// The bytes idle buffers may hold, with `need` more about to be leased: what the budget leaves past the ledger
    /// and the live leases.
    fn idle_room(&self, budgets: &crate::budget::Budgets, need: usize) -> usize {
        budgets.metal().saturating_sub(other() + self.live.load(std::sync::atomic::Ordering::Relaxed) + need)
    }

    /// Releases idle buffers until they fit what `budgets` leave with `need` more leased (`Pool::trim`).
    fn trim(&self, budgets: &crate::budget::Budgets, need: usize) {
        let room = self.idle_room(budgets, need);
        self.pool.lock().expect("the pool").trim(room, self.slab_idle_limit());
    }
}

/// What the ledger of `rerail-metal` reserves now.
fn other() -> usize {
    OTHER_RESERVED.get().map_or(0, |f| f())
}


/// A profiler's hook (`set_lease_hook`): told each lease's buffer address and length when it is leased, and its address
/// with length 0 when it is dropped.
static LEASE_HOOK: OnceLock<fn(usize, usize)> = OnceLock::new();

/// Sets the hook told of every lease and its drop (the engine's heap sampler, which names the span that leased it).
pub fn set_lease_hook(hook: fn(usize, usize)) {
    let _ = LEASE_HOOK.set(hook);
}

fn leased(b: &Buffer) {
    dev().live.fetch_add(b.length() as usize, std::sync::atomic::Ordering::Relaxed);
    if let Some(hook) = LEASE_HOOK.get() {
        hook(b.contents() as usize, b.length() as usize);
    }
}

/// The bytes leased now, and the bytes the pool keeps idle: relational work's share of the device's memory.
pub fn held_bytes() -> (usize, usize) {
    let st_ = dev();
    let idle = st_.pool.lock().map_or(0, |p| p.held);
    (dev().live.load(std::sync::atomic::Ordering::Relaxed), idle)
}

/// The device and its one queue.
pub struct Shared {
    pub device: Device,
    pub queue: CommandQueue,
    /// The host's page size (a column wrapped without a copy must be page-aligned).
    pub page: usize,
    pub max_buffer: u64,
}

/// The process's device and queue: `None` without a Metal device.
pub fn get() -> Option<&'static Shared> {
    static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            // The pool's blocks are handed out by the one policy (decision 0342).
            crate::blocks::install();
            objc::rc::autoreleasepool(|| {
                // The forest's (the lowest crate that uses Metal): one device and one queue for the process.
                let one = rerail_forest::gpu::shared::get()?;
                let (device, queue): (Device, CommandQueue) = (one.device.clone(), one.queue.clone());
                let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as usize;
                let max_buffer = device.max_buffer_length();
                Some(Shared { device, queue, page, max_buffer })
            })
        })
        .as_ref()
}

/// A pipeline's slot: made once. A thread that asks while another makes it waits for that one (the prewarm's
/// compile is not done twice by a job that needs it first).
type Slot = std::sync::Arc<Mutex<Option<ComputePipelineState>>>;

/// The pipeline kept under `key`, made by `make` the first time. A caller that finds another thread making it
/// waits, scored as the phase `pipeline wait` (a cold job, like the compile itself).
pub fn pipeline<E>(key: &str, make: impl FnOnce() -> Result<ComputePipelineState, E>) -> Result<ComputePipelineState, E> {
    let slot = std::sync::Arc::clone(dev().pipelines.lock().expect("the pipelines").entry(key.to_owned()).or_default());
    let mut s = match slot.try_lock() {
        Ok(s) => s,
        Err(std::sync::TryLockError::WouldBlock) => crate::score::phase("pipeline wait", || slot.lock().unwrap_or_else(|p| p.into_inner())),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
    };
    if let Some(p) = &*s {
        return Ok(p.clone());
    }
    let p = crate::score::phase("pipeline compile", make)?;
    *s = Some(p.clone());
    Ok(p)
}

#[derive(Default)]
struct Pool {
    /// Idle buffers by class, each with when it came back (a count), the oldest first.
    idle: HashMap<usize, Vec<(u64, Buffer)>>,
    /// Bytes idle: all, and in slabs.
    held: usize,
    held_slabs: usize,
    returned: u64,
    /// Leases made and served from the pool, for the profile.
    made: u64,
    reused: u64,
    /// Idle deferrable files (archived segments) by directory and class, the oldest first, and how many in all.
    files: HashMap<(std::path::PathBuf, usize), Vec<(u64, std::path::PathBuf)>>,
    files_idle: usize,
}

fn class(bytes: usize) -> usize {
    let bytes = bytes.max(SMALLEST);
    if bytes <= 1 << 20 {
        return bytes.next_power_of_two();
    }
    // Four classes a doubling: p, 1¼p, 1½p, 1¾p (p a power of two), each a multiple of 16 KiB.
    let p = 1usize << (usize::BITS - 1 - bytes.leading_zeros());
    (4..=8).map(|q| p / 4 * q).find(|&c| c >= bytes).expect("2p covers it")
}

impl Pool {
    /// Releases the least recently returned idle buffers until they fit `room` (what the budget leaves: `Memory::
    /// idle_room`), and the slabs `slab_limit` (the slab cap, when set).
    fn trim(&mut self, room: usize, slab_limit: usize) {
        while self.held > room || self.held_slabs > slab_limit {
            let slabs_only = self.held <= room;
            let oldest = self
                .idle
                .iter()
                .filter(|(c, v)| !v.is_empty() && (!slabs_only || **c >= SLAB_MIN))
                .min_by_key(|(_, v)| v[0].0)
                .map(|(c, _)| *c);
            let Some(c) = oldest else { break };
            self.idle.get_mut(&c).unwrap().remove(0);
            self.held -= c;
            if c >= SLAB_MIN {
                self.held_slabs -= c;
            }
        }
    }

    fn take(&mut self, class: usize) -> Option<Buffer> {
        let b = self.idle.get_mut(&class).and_then(Vec::pop)?.1;
        self.held -= class;
        if class >= SLAB_MIN {
            self.held_slabs -= class;
        }
        Some(b)
    }
}

/// A shared buffer of the pool, returned to it when dropped.
pub struct Pooled {
    /// The machine's device it was leased from, and goes back to, and that machine's budgets, which its return trims
    /// the pool to: on whichever thread drops it, in a run or none.
    on: std::sync::Arc<Memory>,
    budgets: std::sync::Arc<crate::budget::Budgets>,
    buffer: Option<Buffer>,
    /// Whether it goes back to the pool (a buffer past the largest class is made to size and released).
    keep: bool,
    /// Made for this lease (zero, never written), else recycled (another lease's bytes).
    fresh: bool,
}

impl Pooled {
    pub fn buffer(&self) -> &Buffer {
        self.buffer.as_ref().expect("a leased buffer")
    }

    /// Releases the buffer to Metal instead of the pool: a command buffer still encoding or running holds it until
    /// it completes, and no later lease can be handed it meanwhile.
    pub fn release(mut self) {
        self.keep = false;
    }

    /// Whether the buffer was made for this lease, so it reads zero; else it holds an earlier lease's bytes.
    pub fn fresh(&self) -> bool {
        self.fresh
    }

    /// The buffer's host address.
    pub fn host(&self) -> *mut u8 {
        self.buffer().contents().cast()
    }

    /// Hands the lease to `cmd`: it goes back to the pool when `cmd` completes, not before (a GPU read is never
    /// overtaken by the next lease's host writes).
    pub fn after(self, cmd: &metal::CommandBufferRef) {
        let held = Mutex::new(Some(self));
        let done = block::ConcreteBlock::new(move |_: &metal::CommandBufferRef| drop(held.lock().map(|mut h| h.take())));
        cmd.add_completed_handler(&done.copy());
    }

    /// A lease of at least `bytes` holding this one's first `used` bytes: the move of something that outgrew its
    /// buffer. This lease goes back to the pool; its holder made sure no command reads it still.
    pub fn grow(self, bytes: usize, used: usize) -> Result<Pooled, String> {
        assert!(used <= self.length() as usize && used <= bytes);
        let next = lease(bytes)?;
        // SAFETY: both leases are the caller's, each at least `used` bytes long, and no command uses either.
        unsafe { std::ptr::copy_nonoverlapping(self.host(), next.host(), used) };
        Ok(next)
    }
}

/// A pooled buffer as a block's memory.
impl rerail_blocks::block::Backing for Pooled {
    fn base(&self) -> *mut u8 {
        self.host()
    }

    fn len(&self) -> usize {
        self.length() as usize
    }

    fn buffer(&self) -> Option<Buffer> {
        self.buffer.clone()
    }
}

/// A block of `kind` in a pooled slab of at least `bytes`: bound to the device by the slab's own buffer, which a
/// command has used before when it is recycled, so no dispatch pays to map it.
pub fn lease_block(kind: u32, bytes: usize) -> Result<rerail_blocks::Block, String> {
    let slab = lease(bytes.max(SLAB_MIN))?;
    Ok(rerail_blocks::Block::over(kind, Box::new(slab)))
}

/// `lease_block`, kept within the Metal budget (`lease_within`).
pub fn lease_block_within(kind: u32, bytes: usize) -> Result<rerail_blocks::Block, String> {
    let slab = lease_within(bytes.max(SLAB_MIN))?;
    Ok(rerail_blocks::Block::over(kind, Box::new(slab)))
}

/// A block for one of a device join's own relations (decision 0325 step 5: a view it makes, a fixpoint's total): a
/// pooled buffer of the relation's size, leased as a join's buffer is (`lease_for_join`: bounded by the budget inside
/// `within_budget`, where a refusal runs the work on the CPU), and back in the pool when the relation goes. No slab's
/// least size: shared 16 MB blocks, kept by any relation in them, refused small views their room.
pub fn lease_block_for_join(kind: u32, bytes: usize) -> Result<rerail_blocks::Block, String> {
    let slab = lease_for_join(bytes.max(4 * rerail_blocks::block::PAGE), false)?;
    Ok(rerail_blocks::Block::over(kind, Box::new(slab)))
}

/// `lease_block`, only when the Metal budget holds it with no more than idle buffers released and `keep` bytes of it
/// still free after: nothing the evictor holds (the tape residency's slabs) is retired for it, and the device's own
/// buffers keep their room. For a block that has a home without the pool (anonymous memory, which the device reads
/// through a no-copy wrap): the FACTS blocks of a run's relations (decision 0325). Their leases retiring the
/// residency's tapes made a run admit them again (20-35 s runs on posthog/hogql); taking the budget's last room
/// left the device's joins 6-48 MB, and its resident relations were dropped 28 times a run.
pub fn lease_block_spare(kind: u32, bytes: usize, keep: usize) -> Result<rerail_blocks::Block, String> {
    let need = class(bytes.max(SLAB_MIN));
    dev().trim(&crate::budget::budgets(), need);
    if other() + dev().live.load(std::sync::atomic::Ordering::Relaxed) + need + keep > crate::budget::metal() {
        return Err(format!("device budget: no spare room for a block of {} MB", need >> 20));
    }
    let slab = lease(bytes.max(SLAB_MIN))?;
    Ok(rerail_blocks::Block::over(kind, Box::new(slab)))
}

/// A scratch region (`rerail_blocks::scratch`) in a pooled slab of at least `bytes`: freeze's and pack's
/// temporaries counted in the one Metal budget, and readable by a kernel. Install it on a thread
/// (`rerail_blocks::scratch::install`) for that thread's freezes; it goes back to the pool when dropped.
pub fn lease_scratch(bytes: usize) -> Result<rerail_blocks::scratch::Scratch, String> {
    let slab = lease_within(bytes.max(SLAB_MIN))?;
    Ok(rerail_blocks::scratch::Scratch::over(Box::new(slab)))
}

/// Moves an open block into a pooled slab of at least `bytes`: one copy of what it holds. No command may be
/// reading the block (it is moved between passes, as its chunks are made).
pub fn grow_block(b: rerail_blocks::Block, bytes: usize) -> Result<rerail_blocks::Block, String> {
    let slab = lease(bytes.max(SLAB_MIN))?;
    Ok(b.relocate(Box::new(slab)))
}

/// Idle deferrable files the pool keeps at most, all classes (each beyond is removed): they hold disk, not memory.
const IDLE_FILES: usize = 16;

/// A deferrable lease (decision 0295 §1): a block of `kind` in a file-backed shared mapping of at least `bytes` at
/// `path`, wrapped for the device once. It counts in the one budget as a live lease until it is sealed
/// (`Deferrable::seal`, at the wave's end) or dropped; then its pages are the OS's. A file the writer gave back
/// (`return_deferrable`) in the same directory and of the same class is reused, renamed to `path`: its pages may
/// still be cached, and the block zeroes what it allocates in it.
pub fn lease_deferrable(kind: u32, bytes: usize, path: &std::path::Path) -> Result<rerail_blocks::deferrable::Deferrable, String> {
    lease_file(kind, bytes, path, true)
}

/// `lease_deferrable` for the host's writes alone (the store's writer): no wrap for the device, whose wire on the
/// pages would hold their writeback some 200 ms (§7).
pub fn lease_deferrable_host(kind: u32, bytes: usize, path: &std::path::Path) -> Result<rerail_blocks::deferrable::Deferrable, String> {
    lease_file(kind, bytes, path, false)
}

fn lease_file(kind: u32, bytes: usize, path: &std::path::Path, for_device: bool) -> Result<rerail_blocks::deferrable::Deferrable, String> {
    let class = class(bytes.max(SLAB_MIN));
    let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let reused = {
        let st_ = dev();
        let mut p = st_.pool.lock().expect("the pool");
        let found = p.files.get_mut(&(dir, class)).and_then(Vec::pop).map(|(_, f)| f);
        if found.is_some() {
            p.files_idle -= 1;
        }
        found
    };
    let fresh = match reused {
        Some(old) => std::fs::rename(&old, path).is_err(),
        None => true,
    };
    let device = if for_device { get().map(|s| s.device.as_ref()) } else { None };
    let mut d = rerail_blocks::deferrable::Deferrable::create(path, kind, class, fresh, device).map_err(|e| format!("{}: {e}", path.display()))?;
    let on = dev();
    on.live.fetch_add(class, std::sync::atomic::Ordering::Relaxed);
    d.on_release(move || {
        on.live.fetch_sub(class, std::sync::atomic::Ordering::Relaxed);
    });
    let st_ = dev();
    let mut p = st_.pool.lock().expect("the pool");
    if fresh {
        p.made += 1;
    } else {
        p.reused += 1;
    }
    Ok(d)
}

/// `lease_deferrable`, kept within the Metal budget (`lease_within`).
pub fn lease_deferrable_within(kind: u32, bytes: usize, path: &std::path::Path) -> Result<rerail_blocks::deferrable::Deferrable, String> {
    let need = class(bytes.max(SLAB_MIN));
    if !make_room(need) {
        return Err(format!("device budget: a deferrable lease of {} MB past the Metal budget of {} MB", need >> 20, crate::budget::metal() >> 20));
    }
    lease_deferrable(kind, bytes, path)
}

/// Gives the pool back a deferrable segment's file once the writer has archived it (decision 0295 §4), with the
/// slab's length (`Deferrable::slab_len`, `Sealed::slab_len`): the next lease of its class in its directory reuses
/// it. Past `IDLE_FILES`, the oldest is removed.
pub fn return_deferrable(path: std::path::PathBuf, slab_len: usize) {
    let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let st_ = dev();
    let mut p = st_.pool.lock().expect("the pool");
    p.returned += 1;
    let at = p.returned;
    p.files.entry((dir, slab_len)).or_default().push((at, path));
    p.files_idle += 1;
    while p.files_idle > IDLE_FILES {
        // The least recently returned: each list is oldest first.
        let Some(k) = p.files.iter().filter(|(_, v)| !v.is_empty()).min_by_key(|(_, v)| v[0].0).map(|(k, _)| k.clone()) else { break };
        let (_, old) = p.files.get_mut(&k).unwrap().remove(0);
        p.files_idle -= 1;
        let _ = std::fs::remove_file(old);
    }
}

impl std::ops::Deref for Pooled {
    type Target = BufferRef;
    fn deref(&self) -> &BufferRef {
        self.buffer()
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        let Some(b) = self.buffer.take() else { return };
        self.on.live.fetch_sub(b.length() as usize, std::sync::atomic::Ordering::Relaxed);
        if let Some(hook) = LEASE_HOOK.get() {
            hook(b.contents() as usize, 0);
        }
        if !self.keep {
            return;
        }
        let class = b.length() as usize;
        let mut p = self.on.pool.lock().expect("the pool");
        p.returned += 1;
        let at = p.returned;
        p.held += class;
        if class >= SLAB_MIN {
            p.held_slabs += class;
        }
        p.idle.entry(class).or_default().push((at, b));
        let room = self.on.idle_room(&self.budgets, 0);
        p.trim(room, self.on.slab_idle_limit());
    }
}

/// A buffer of at least `bytes`. Its contents are a previous lease's: every reader writes it first.
pub fn lease(bytes: usize) -> Result<Pooled, String> {
    let shared = get().ok_or("no Metal device")?;
    let class = class(bytes);
    if class as u64 > shared.max_buffer {
        if bytes as u64 > shared.max_buffer {
            return Err(format!("a buffer of {bytes} bytes is past the device's largest ({})", shared.max_buffer));
        }
        let b = objc::rc::autoreleasepool(|| shared.device.new_buffer(bytes as u64, MTLResourceOptions::StorageModeShared));
        leased(&b);
        return Ok(Pooled { on: dev(), budgets: crate::budget::budgets(), buffer: Some(b), keep: false, fresh: true });
    }
    let st_ = dev();
    let mut p = st_.pool.lock().expect("the pool");
    if let Some(b) = p.take(class) {
        p.reused += 1;
        leased(&b);
        return Ok(Pooled { on: dev(), budgets: crate::budget::budgets(), buffer: Some(b), keep: true, fresh: false });
    }
    p.made += 1;
    // A new buffer: idle ones give way to it first, so the budget holds both.
    p.trim(st_.idle_room(&crate::budget::budgets(), class), st_.slab_idle_limit());
    drop(p);
    let b = crate::score::phase("new buffer", || objc::rc::autoreleasepool(|| shared.device.new_buffer(class as u64, MTLResourceOptions::StorageModeShared)));
    leased(&b);
    Ok(Pooled { on: dev(), budgets: crate::budget::budgets(), buffer: Some(b), keep: true, fresh: true })
}

/// What else holds the device's budget beside relational work (the buffer pool's reservations, `rerail-metal`'s
/// ledger): set by its owner, which this crate cannot depend on.
static OTHER_RESERVED: OnceLock<fn() -> usize> = OnceLock::new();

/// Sets the reader of what else holds the Metal budget (`lease_within`).
pub fn set_other_reserved(reader: fn() -> usize) {
    let _ = OTHER_RESERVED.set(reader);
}

/// What gives way when the budget is short, after the idle buffers (`set_evictor`): it retires one thing it holds
/// (the residency's least recently used slab) and says whether it did. It must not wait for a lock (it is called
/// from inside allocations): a holder it cannot reach now is skipped.
static EVICTOR: OnceLock<fn() -> bool> = OnceLock::new();

/// Sets the evictor (once per process; a second call is ignored).
pub fn set_evictor(f: fn() -> bool) {
    let _ = EVICTOR.set(f);
}

/// Makes room in the Metal budget for `need` more bytes: idle buffers released, least recently returned first, then
/// the evictor's holdings retired one at a time while it has any. Whether the ledger's reservations, the live leases
/// and `need` fit the budget now. A retired holding reached by a batch still running comes back later.
pub fn make_room(need: usize) -> bool {
    let fits = || other() + dev().live.load(std::sync::atomic::Ordering::Relaxed) + need <= crate::budget::metal();
    loop {
        dev().trim(&crate::budget::budgets(), need);
        if fits() {
            return true;
        }
        if !EVICTOR.get().is_some_and(|f| f()) {
            return false;
        }
    }
}

/// A lease that keeps within the Metal budget (`crate::budget::metal`, decision 0126): refused, as a capacity
/// refusal its caller runs on the CPU (or splits its batch), when the leases and the ledger's reservations would
/// pass it after room is made (`make_room`). The device joins' buffers are leased so: without it, their waves and
/// the strata run apart from them held up to 600 MB of a 512 MB budget.
pub fn lease_within(bytes: usize) -> Result<Pooled, String> {
    let need = class(bytes);
    if !make_room(need) {
        let (live, idle) = held_bytes();
        return Err(format!(
            "device budget: a lease of {} MB past the Metal budget of {} MB ({} MB leased, {} MB idle, {} MB reserved by the buffer pool)",
            need >> 20, crate::budget::metal() >> 20, live >> 20, idle >> 20, other() >> 20
        ));
    }
    lease(bytes)
}

/// Releases idle buffers until they fit what the budget leaves: for a reservation made outside the pool (the
/// ledger's), after it is made.
pub fn trim_idle() {
    dev().trim(&crate::budget::budgets(), 0);
}

thread_local! {
    /// Whether this thread's leases keep within the budget (`within_budget`).
    static BOUNDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with this thread's device-join leases (`crate::gpu::join::Held`) bounded by the Metal budget
/// (`lease_within`): for a caller that runs its work on the CPU when a lease is refused (the logic engine's device
/// joins). Elsewhere (the executor's GPU lane, which has no such fallback) a lease is never refused.
pub fn within_budget<R>(f: impl FnOnce() -> R) -> R {
    let was = BOUNDED.with(|b| b.replace(true));
    let out = f();
    BOUNDED.with(|b| b.set(was));
    out
}

/// A lease for a device join's buffer: bounded inside `within_budget`, else as `lease`.
pub fn lease_for_join(bytes: usize, zeroed: bool) -> Result<Pooled, String> {
    match (BOUNDED.try_with(std::cell::Cell::get).unwrap_or(false), zeroed) {
        (true, false) => lease_within(bytes),
        (true, true) => lease_zeroed_within(bytes),
        (false, false) => lease(bytes),
        (false, true) => lease_zeroed(bytes),
    }
}

/// `lease_within`, its first `bytes` zero.
pub fn lease_zeroed_within(bytes: usize) -> Result<Pooled, String> {
    let b = lease_within(bytes)?;
    // SAFETY: the lease is this caller's alone, its buffer at least `bytes` long and not in use by any command.
    unsafe { std::ptr::write_bytes(b.host(), 0, bytes) };
    Ok(b)
}

/// A buffer of at least `bytes`, its first `bytes` zero (a new buffer is zero already).
pub fn lease_zeroed(bytes: usize) -> Result<Pooled, String> {
    let b = lease(bytes)?;
    // SAFETY: the lease is this caller's alone, its buffer at least `bytes` long and not in use by any command.
    unsafe { std::ptr::write_bytes(b.host(), 0, bytes) };
    Ok(b)
}

/// The pool's counts: bytes idle, leases made, leases served from the pool.
pub fn pool_counts() -> (usize, u64, u64) {
    let st_ = dev();
    let p = st_.pool.lock().expect("the pool");
    (p.held, p.made, p.reused)
}

/// Releases every idle buffer and every pipeline (the engine's drop after its last execution, decision 0032).
pub fn drop_idle() {
    let st_ = dev();
    let mut p = st_.pool.lock().expect("the pool");
    p.idle.clear();
    p.held = 0;
    p.held_slabs = 0;
    drop(p);
    dev().pipelines.lock().expect("the pipelines").clear();
}

/// Releases every idle buffer, keeping the pipelines: what a lane gives back to the device's budget after a refusal
/// for room.
pub fn drop_free() {
    let st_ = dev();
    let mut p = st_.pool.lock().expect("the pool");
    p.idle.clear();
    p.held = 0;
    p.held_slabs = 0;
}

static GUARD: OnceLock<fn() -> Result<(), String>> = OnceLock::new();

/// Sets the check every submission makes first (once per process; a second call is ignored).
pub fn set_guard(f: fn() -> Result<(), String>) {
    let _ = GUARD.set(f);
}

/// The submission check: the engine's launch budget when it set one.
pub fn guard() -> Result<(), String> {
    GUARD.get().map_or(Ok(()), |f| f())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by each test that leases tens of MB at once: the budget test counts every live lease in the process.
    fn big_leases() -> std::sync::MutexGuard<'static, ()> {
        static BIG: Mutex<()> = Mutex::new(());
        BIG.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A bounded lease is refused once relational work's leases would pass the Metal budget, and fits again once they
    /// are dropped.
    #[test]
    fn a_bounded_lease_keeps_within_the_metal_budget() {
        if get().is_none() {
            return;
        }
        let _big = big_leases();
        let (live, _) = held_bytes();
        crate::budget::METAL_TEST.with(|m| m.set(live + (96 << 20)));
        let first = lease_within(64 << 20).expect("64 MB fits a budget of 96 MB more than is leased");
        let refused = lease_within(64 << 20);
        assert!(refused.as_ref().is_err_and(|e| e.contains("device budget")), "a second 64 MB passes the budget");
        drop(first);
        drop(refused);
        assert!(lease_within(64 << 20).is_ok(), "fits again once the first is dropped");
        crate::budget::METAL_TEST.with(|m| m.set(0));
    }

    #[test]
    fn leases_return_to_the_pool_by_size_class() {
        if get().is_none() {
            return;
        }
        // Other tests lease from the same pool at the same time, so only the sizes and the zeroing are this test's.
        let a = lease(20_000).unwrap();
        assert_eq!(a.length(), 32 << 10);
        // SAFETY: the lease is this test's, 32 KiB long.
        unsafe { std::ptr::write_bytes(a.host(), 7, 32 << 10) };
        drop(a);
        let b = lease_zeroed(30_000).unwrap();
        assert_eq!(b.length(), 32 << 10);
        // SAFETY: the lease is this test's, 32 KiB long.
        assert!(unsafe { std::slice::from_raw_parts(b.host(), 30_000) }.iter().all(|&x| x == 0));
        let p = pipeline::<String>("test no-op", || Err("not made".into()));
        assert!(p.is_err());
    }

    #[test]
    fn large_classes_are_quarter_steps() {
        assert_eq!(class(1 << 20), 1 << 20);
        assert_eq!(class((1 << 20) + 1), 5 << 18);
        assert_eq!(class(5 << 20), 5 << 20);
        assert_eq!(class((5 << 20) + 1), 6 << 20);
        assert_eq!(class(9 << 20), 10 << 20);
        assert_eq!(class(15 << 20), 16 << 20);
        for b in [3 << 20, 7 << 20, 33 << 20, 300 << 20] {
            let c = class(b);
            assert!(c >= b && c * 4 <= b * 5 + (4 << 20), "{b}: class {c} wastes at most a quarter");
            assert_eq!(c % SMALLEST, 0);
        }
    }

    /// A block in a slab: recycled with its device buffer, and its memory zeroed as each part is made, though the
    /// slab holds the last block's bytes.
    #[test]
    fn a_block_in_a_slab_is_recycled_and_reads_fresh() {
        use rerail_blocks::block::kind;
        use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
        if get().is_none() {
            return;
        }
        let _big = big_leases();
        set_slab_idle_limit(1 << 30);
        let spec = Spec { widths: vec![4, 4], order: vec![0, 1], indexes: vec![vec![1]], min_log: 10 };
        let lock = Mutex::new(());
        // 23 MiB: a class (24 MiB) no other test leases.
        let b = lease_block(kind::FACTS, 23 << 20).unwrap();
        let first = b.gpu_buffer(&get().unwrap().device).gpu_address();
        let r = Relation::create(&b, &spec, None);
        RelW { block: &b, rel: r, lock: &lock }.extend(&(0..20_000).flat_map(|i| [i % 300, i % 7]).collect::<Vec<_>>());
        relation::freeze(&b, r, &spec, None);
        assert_eq!(View::full(r).live_len(), 2_100);
        drop(b);
        let b = lease_block(kind::FACTS, 23 << 20).unwrap();
        assert_eq!(b.gpu_buffer(&get().unwrap().device).gpu_address(), first, "the slab came back, with its buffer");
        let r = Relation::create(&b, &spec, None);
        assert_eq!(r.rows.len(), 0);
        assert!(!r.is_frozen(), "a new relation in a recycled slab reads fresh");
        let at = b.alloc(1 << 20, 64);
        // SAFETY: the block's own allocation, 1 MiB long.
        assert!(unsafe { std::slice::from_raw_parts(b.ptr::<u8>(at), 1 << 20) }.iter().all(|&x| x == 0));
    }

    /// A freeze whose temporaries come from a pooled scratch slab: the same relation as from the thread's own, and
    /// the slab back in the pool after.
    #[test]
    fn a_freeze_takes_its_scratch_from_a_pooled_slab() {
        use rerail_blocks::block::kind;
        use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
        if get().is_none() {
            return;
        }
        let _big = big_leases();
        set_slab_idle_limit(1 << 30);
        let spec = Spec { widths: vec![4, 4], order: vec![0, 1], indexes: vec![vec![1]], min_log: 10 };
        let rows: Vec<i64> = (0..200_000).flat_map(|i| [i % 3_000, i % 17]).collect();
        let lock = Mutex::new(());
        let digest = |scratch: Option<rerail_blocks::scratch::Scratch>| {
            let was = scratch.map(rerail_blocks::scratch::install);
            let b = rerail_blocks::Block::anon(kind::FACTS, 64 << 20);
            let r = Relation::create(&b, &spec, None);
            RelW { block: &b, rel: r, lock: &lock }.extend(&rows);
            relation::freeze(&b, r, &spec, None);
            if let Some(was) = was {
                // Put the thread's own back; the pooled one drops here.
                drop(was.map(rerail_blocks::scratch::install));
            }
            View::full(r).digest()
        };
        let own = digest(None);
        // 29 MiB: a class (32 MiB) no other test leases.
        let s = lease_scratch(29 << 20).unwrap();
        assert_eq!(digest(Some(s)), own, "the same relation");
        let back = dev().pool.lock().unwrap().idle.get(&class(29 << 20)).is_some_and(|v| !v.is_empty());
        assert!(back, "the scratch slab went back to the pool");
    }

    /// A lease handed to a command buffer goes back to the pool when it completes.
    #[test]
    fn a_lease_after_a_command_returns_when_it_completes() {
        let Some(sh) = get() else { return };
        let _big = big_leases();
        set_slab_idle_limit(1 << 30);
        // 27 MiB: a class (28 MiB) no other test leases.
        let l = lease(27 << 20).unwrap();
        let addr = l.gpu_address();
        let cmd = sh.queue.new_command_buffer();
        l.after(cmd);
        cmd.commit();
        cmd.wait_until_completed();
        let t = std::time::Instant::now();
        loop {
            let back = dev().pool.lock().unwrap().idle.get(&class(27 << 20)).is_some_and(|v| v.iter().any(|(_, b)| b.gpu_address() == addr));
            if back {
                break;
            }
            assert!(t.elapsed().as_secs() < 2, "the lease came back when its command buffer completed");
            std::thread::yield_now();
        }
    }

    /// A block that outgrows its slab moves into a larger one by one copy, and reads the same.
    #[test]
    fn a_block_grows_into_a_larger_slab() {
        use rerail_blocks::block::kind;
        use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
        if get().is_none() {
            return;
        }
        let _big = big_leases();
        let spec = Spec { widths: vec![4, 4], order: vec![0, 1], indexes: vec![vec![1]], min_log: 10 };
        let lock = Mutex::new(());
        let b = lease_block(kind::FACTS, 17 << 20).unwrap();
        let off = {
            let r = Relation::create(&b, &spec, None);
            RelW { block: &b, rel: r, lock: &lock }.extend(&(0..100_000).flat_map(|i| [i % 1000, i % 13]).collect::<Vec<_>>());
            b.offset_of(r)
        };
        let b = grow_block(b, 41 << 20).unwrap();
        assert!(b.reserved() >= 41 << 20);
        let r: &Relation = unsafe { &*b.ptr::<Relation>(off) };
        RelW { block: &b, rel: r, lock: &lock }.extend(&(0..1_000_000).flat_map(|i| [1000 + i % 500, i % 11]).collect::<Vec<_>>());
        relation::freeze(&b, r, &spec, None);
        // (i % 1000, i % 13) repeats every 13,000 rows; (1000 + i % 500, i % 11) every 5,500.
        assert_eq!(View::full(r).live_len(), 13_000 + 5_500);
    }

    /// A journal in a deferrable lease (decision 0295 §1): counted live until sealed, read in place once sealed,
    /// written back and frozen by the writer, and its file reused by the next lease of its class.
    #[test]
    fn a_deferrable_journal_is_handed_off_and_its_file_reused() {
        use rerail_blocks::block::kind;
        use rerail_blocks::journal;
        if get().is_none() {
            return;
        }
        let _big = big_leases();
        let dir = std::env::temp_dir().join(format!("rerail-deferrable-pool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let live = || dev().live.load(std::sync::atomic::Ordering::Relaxed);
        let before = live();
        let d = lease_deferrable(kind::JOURNAL, 1 << 20, &dir.join("seg-1")).unwrap();
        assert_eq!(live(), before + class(SLAB_MIN), "a deferrable lease counts in the budget while open");
        journal::init(d.block(), 8);
        let src = rerail_blocks::Block::anon(kind::SCRATCH, 1 << 20);
        let start = src.alloc(4096, 64);
        unsafe { std::ptr::write_bytes(src.ptr::<u8>(start), 0x5a, 4096) };
        journal::add(d.block(), &src, start, start + 4096, start, 7, 0xfeed);
        let slab_len = d.slab_len();
        let sealed = d.seal();
        assert_eq!(live(), before, "the seal takes it out of the budget");
        assert_eq!(journal::entries(&sealed.reader())[0].digest, 0xfeed, "read in place once sealed");
        let back = sealed.write_back().unwrap();
        let e = &journal::entries(&back)[0];
        assert_eq!((e.key, e.digest), (7, 0xfeed));
        assert_eq!(back.bytes(e.start as usize, e.end as usize), &[0x5a; 4096][..]);
        drop(back);
        assert!(journal::open(&dir.join("seg-1")).is_ok(), "frozen on disk");

        let (_, _, reused) = pool_counts();
        return_deferrable(dir.join("seg-1"), slab_len);
        let again = lease_deferrable(kind::JOURNAL, 1 << 20, &dir.join("seg-2")).unwrap();
        assert_eq!(pool_counts().2, reused + 1, "the archived file is reused");
        assert!(!dir.join("seg-1").exists() && dir.join("seg-2").exists(), "renamed to the new segment");
        let off = again.block().alloc(4096, 64);
        assert!(again.block().bytes(off, off + 4096).iter().all(|&b| b == 0), "a reused file reads zero where allocated");
        drop(again);
        assert_eq!(live(), before, "dropped unsealed, it leaves the budget too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
