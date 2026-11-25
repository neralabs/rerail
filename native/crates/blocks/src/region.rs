//! Regions of a block, held by handles (decision 0491, after MLDB's memory regions: `MemoryRegionHandleInfo`,
//! `MutableMemoryRegion::freeze`, `FrozenMemoryRegion::range`, `Manager::pinAccess`).
//!
//! A region is a run of `T`s in a block. It is held only through handles, and goes back to its block when the last
//! one goes, and not before the GPU work that uses it completes. Each handle says what it may do with its type:
//!
//! ```text
//! let mut m = Mutable::<u64>::alloc(&block, n, "edge rows")?;   // allocated: one writer
//! m.write()[0] = 7;                                            // a CPU write: no reader, no GPU use, may be live
//! let s: Shared<u64> = m.share();                              // a const handle: reads only (&[T])
//! m.bind_read(&fence, "probe");                                 // a GPU read, pinned until the fence completes
//! let f: Frozen<u64> = m.freeze();                              // immutable: read without guards, by any number
//! let part = f.range(10, 20);                                   // keeps the whole region alive
//! ```
//!
//! What cannot work fails at once, naming the region and where its other use began: two writers, a write while a
//! read is live (CPU or GPU), a read while a write is, a GPU write of a frozen region, a freeze while it is read or
//! written, a range past its end. A region's version counts its writes (a CPU write ended, a GPU write completed).
//!
//! An operation's slots (decision 0686 B1) are whole blocks, pinned through the block's own region (`whole`) by
//! `pin_block`, under the same checks. A conflict there is refused, not failed: the caller is starting an operation,
//! which then does not start, and its `Err` names the operation and what holds the block.

use crate::Block;
use crate::ledger::Fence;
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::{Arc, Mutex, MutexGuard};

/// Who holds a CPU use: a guard, by the source line that took it, or an operation's slot, by the operation's name.
#[derive(Clone, Copy)]
enum By {
    Line(&'static Location<'static>),
    Op(&'static str),
}

impl std::fmt::Display for By {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            By::Line(l) => write!(f, "from {l}"),
            By::Op(op) => write!(f, "by {op}"),
        }
    }
}

#[derive(Default)]
struct Access {
    reads: Vec<(u64, By)>,
    write: Option<By>,
    gpu_reads: Vec<(u64, &'static str)>,
    /// Each with the domain it writes in (0: the host's memory, which a unified GPU reads in place).
    gpu_writes: Vec<(u64, &'static str, std::sync::Weak<Fence>, u32)>,
    next: u64,
    /// The version the host's bytes hold (the block's), and the device's copy's when it has its own (`Mirror`):
    /// `None` before its first upload. With unified memory there is one copy, and both are the latest once the
    /// GPU's writes complete.
    host: u64,
    /// The version each device domain's copy holds (decision 0440 §7.2): one entry a domain that holds a copy.
    devices: Vec<(u32, u64)>,
}

impl Access {
    fn held_in(&self, domain: u32) -> Option<u64> {
        self.devices.iter().find(|(d, _)| *d == domain).map(|(_, v)| *v)
    }

    fn set_in(&mut self, domain: u32, version: u64) {
        match self.devices.iter_mut().find(|(d, _)| *d == domain) {
            Some(e) => e.1 = version,
            None => self.devices.push((domain, version)),
        }
    }

    /// The domain that holds `latest`: the host's (0) when its bytes do, else the device's whose copy does.
    fn owner(&self, latest: u64) -> u32 {
        if self.host >= latest { 0 } else { self.devices.iter().find(|(_, v)| *v >= latest).map_or(0, |(d, _)| *d) }
    }
}

impl Access {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn says(&self) -> String {
        let mut v = Vec::new();
        if let Some(w) = self.write {
            v.push(format!("a CPU write {w}"));
        }
        v.extend(self.reads.iter().map(|(_, by)| format!("a CPU read {by}")));
        v.extend(self.gpu_reads.iter().map(|(_, op)| format!("a GPU read by {op}")));
        v.extend(self.gpu_writes.iter().map(|(_, op, _, _)| format!("a GPU write by {op}")));
        v.join(", ")
    }
}

/// A device's own copy of a region, on a GPU whose memory is not the host's (a discrete GPU): the backend that placed
/// the region there attaches it (`Region::mirror_on`). Without one the device reads the block's bytes where they are
/// (unified memory), and coherence is only order: the host waits for the GPU's writes.
pub trait Mirror: Send + Sync {
    /// Queues the host's bytes' transfer to the device's copy, ahead of the work encoded after it on `fence`'s
    /// command buffer.
    fn upload(&self, host: &[u8], fence: &Fence);
    /// Transfers the device's copy into the host's bytes, now: the GPU's writes to it have completed.
    fn download(&self, host: &mut [u8]);
}

/// A run of bytes in a block, and what holds and uses it. A range of another (`root`) shares its root's uses,
/// version and state, and keeps it alive.
pub struct Region {
    block: Arc<Block>,
    at: usize,
    len: usize,
    /// The chunk's bytes it gives back to its block when it goes (`try_alloc_chunk_fit`'s); 0 for a range, or a
    /// region in a block without a free list (its bytes go with the block).
    kept: usize,
    root: Option<Arc<Region>>,
    name: &'static str,
    frozen: AtomicBool,
    version: AtomicU64,
    access: Mutex<Access>,
    /// Its device domains' copies (decision 0440 §7.2), each by its domain (1 and up; 0 is the host's memory).
    mirrors: Mutex<Vec<(u32, Arc<dyn Mirror>)>>,
    /// The whole block (`whole`): its pins hold the block, not a chunk of it, whose CPU writers (a relation's rows)
    /// write other chunks meanwhile.
    whole: bool,
}

fn fail(r: &Region, what: String) -> ! {
    panic!("{}", says(r, what));
}

fn says(r: &Region, what: String) -> String {
    format!("region '{}' ({} bytes at {} of its block): {what}", r.top().name, r.len, r.at)
}

impl Region {
    fn top(&self) -> &Region {
        self.root.as_deref().unwrap_or(self)
    }

    fn lock(&self) -> MutexGuard<'_, Access> {
        self.top().access.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn block(&self) -> &Arc<Block> {
        &self.block
    }

    /// Its offset in its block (where the device binds it).
    pub fn at(&self) -> usize {
        self.at
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn name(&self) -> &'static str {
        self.top().name
    }

    pub fn is_frozen(&self) -> bool {
        self.top().frozen.load(Acquire)
    }

    /// Its writes so far: a CPU write ended, or a GPU write completed.
    pub fn version(&self) -> u64 {
        self.top().version.load(Acquire)
    }

    fn range(self: &Arc<Region>, start: usize, end: usize) -> Arc<Region> {
        if start > end || end > self.len {
            fail(self, format!("a range {start}..{end} of {} bytes", self.len));
        }
        let root = self.root.clone().unwrap_or_else(|| self.clone());
        Arc::new(Region {
            block: self.block.clone(),
            at: self.at + start,
            len: end - start,
            kept: 0,
            root: Some(root),
            name: "",
            frozen: AtomicBool::new(false),
            version: AtomicU64::new(0),
            access: Mutex::default(),
            mirrors: Mutex::new(Vec::new()),
            whole: false,
        })
    }

    /// Places it on a device with memory of its own, domain 1: `mirror_in`.
    pub fn mirror_on(&self, mirror: Box<dyn Mirror>) {
        self.mirror_in(1, mirror);
    }

    /// Places it in memory domain `domain` (a discrete GPU's, 1 and up: decision 0440 §7.2): `mirror` holds that
    /// domain's copy, uploaded before a kernel there reads a version it lacks, downloaded by `sync_host` after one
    /// writes it. A region's, not a range's; once a domain.
    pub fn mirror_in(&self, domain: u32, mirror: Box<dyn Mirror>) {
        let mut m = self.top().mirrors.lock().unwrap_or_else(|e| e.into_inner());
        if self.root.is_some() || domain == 0 || m.iter().any(|(d, _)| *d == domain) {
            drop(m);
            fail(self, format!("a second copy in domain {domain}, one in the host's (0), or one of a range (place the region it is part of)"));
        }
        m.push((domain, Arc::from(mirror)));
    }

    fn mirror(&self, domain: u32) -> Option<Arc<dyn Mirror>> {
        self.top().mirrors.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(d, _)| *d == domain).map(|(_, m)| m.clone())
    }

    /// The domain a bind names none for: its one device domain, or the host's memory (0) with none. Fails with two.
    fn default_domain(&self) -> u32 {
        let m = self.top().mirrors.lock().unwrap_or_else(|e| e.into_inner());
        match m.as_slice() {
            [] => 0,
            [(d, _)] => *d,
            _ => {
                let n = m.len();
                drop(m);
                fail(self, format!("a bind naming no domain of a region in {n} (bind_read_on, bind_write_on)"));
            }
        }
    }

    /// How many CPU reads of it are open now (E1's tests: what a refused start left pinned).
    pub fn cpu_reads(&self) -> usize {
        self.lock().reads.len()
    }

    /// The domain whose copy holds its latest version (0: the host's): the one a write may begin in without moving it.
    pub fn owner(&self) -> u32 {
        self.lock().owner(self.top().version.load(Acquire))
    }

    /// Brings the host's bytes up to its latest version: waits for the GPU's queued writes, then (with a device
    /// copy) transfers it back. A queued write whose work was never committed fails: the wait would never end.
    pub fn sync_host(&self) {
        let writers: Vec<_> = self.lock().gpu_writes.iter().map(|(_, op, f, _)| (*op, f.clone())).collect();
        for (op, f) in writers {
            let Some(f) = f.upgrade() else { continue };
            if !f.is_committed() {
                fail(self, format!("a host sync waiting on a GPU write by {op} whose work is not committed"));
            }
            f.wait();
        }
        let top = self.top();
        let mut a = self.lock();
        let latest = top.version.load(Acquire);
        if a.host < latest {
            let owner = a.owner(latest);
            let m = self.mirror(owner).expect("a host behind its latest version has a device copy that holds it");
            // SAFETY: no CPU use is live (the host's bytes are behind, so none could begin) and the GPU's writes
            // completed; the region's bytes are its block's.
            m.download(unsafe { std::slice::from_raw_parts_mut(top.bytes(), top.len) });
            a.host = latest;
        }
    }

    /// The host's bytes are behind its latest version (a GPU write completed on a device copy), or a GPU write is
    /// queued: what a CPU use must wait for (`sync_host`).
    fn host_behind(&self, a: &Access) -> Option<String> {
        let latest = self.top().version.load(Acquire);
        if let Some((_, op, _, _)) = a.gpu_writes.first() {
            Some(format!("a GPU write by {op} is queued (sync_host first)"))
        } else if a.host < latest {
            Some(format!("its host bytes hold version {} of {latest} (sync_host first)", a.host))
        } else {
            None
        }
    }

    #[track_caller]
    fn begin_read(&self) -> u64 {
        self.try_begin_read(By::Line(Location::caller())).unwrap_or_else(|e| fail(self, e))
    }

    fn try_begin_read(&self, by: By) -> Result<u64, String> {
        let mut a = self.lock();
        if a.write.is_some() {
            return Err(format!("a CPU read {by} while {}", a.says()));
        }
        if let Some(why) = self.host_behind(&a) {
            return Err(format!("a CPU read {by}: {why}"));
        }
        let id = a.id();
        a.reads.push((id, by));
        Ok(id)
    }

    fn end_read(&self, id: u64) {
        self.lock().reads.retain(|(i, _)| *i != id);
    }

    #[track_caller]
    fn begin_write(&self) {
        let by = By::Line(Location::caller());
        if let Err(e) = self.try_begin_write(by) {
            fail(self, e);
        }
        // SAFETY of the ledger's check too: the chunk's bytes are not the GPU's now (R4 of 0487).
        self.block.ledger().cpu_write(self.at, self.len);
    }

    fn try_begin_write(&self, by: By) -> Result<(), String> {
        let mut a = self.lock();
        if a.write.is_some() || !a.reads.is_empty() || !a.gpu_reads.is_empty() || !a.gpu_writes.is_empty() {
            return Err(format!("a CPU write {by} while {}", a.says()));
        }
        if let Some(why) = self.host_behind(&a) {
            return Err(format!("a CPU write {by}: {why}"));
        }
        a.write = Some(by);
        Ok(())
    }

    fn end_write(&self) {
        let mut a = self.lock();
        a.write = None;
        // The host's bytes are the latest; a device copy is behind until its next upload.
        a.host = self.top().version.fetch_add(1, AcqRel) + 1;
    }

    /// A GPU use of it by `op`, pinned on `fence` until it completes (the region kept alive with it), in its one
    /// domain (`default_domain`).
    fn pin(self: &Arc<Region>, fence: &Arc<Fence>, op: &'static str, write: bool) -> usize {
        let domain = self.default_domain();
        self.pin_in(fence, op, write, domain)
    }

    /// `pin`, by a kernel in memory domain `domain` (0: the host's memory, read in place). Its copy there is made the
    /// latest first: uploaded from the host, which takes the latest from the domain that holds it when another
    /// does (ownership moves through the host). A write while another domain's write is queued fails: two domains
    /// never write one region.
    fn pin_in(self: &Arc<Region>, fence: &Arc<Fence>, op: &'static str, write: bool, domain: u32) -> usize {
        let mirror = if domain == 0 { None } else { Some(self.mirror(domain).unwrap_or_else(|| fail(self, format!("a bind in domain {domain}, where it has no copy (mirror_in)")))) };
        {
            let mut a = self.lock();
            let bad = if write {
                if self.is_frozen() {
                    Some("a GPU write of a frozen region".to_string())
                } else if a.write.is_some() || !a.reads.is_empty() {
                    Some(format!("a GPU write by {op} while {}", a.says()))
                } else {
                    None
                }
            } else if a.write.is_some() {
                Some(format!("a GPU read by {op} while {}", a.says()))
            } else {
                None
            };
            let elsewhere = a.gpu_writes.iter().find(|w| w.3 != domain).map(|w| (w.1, w.3));
            let bad = bad.or_else(|| elsewhere.map(|(other, d)| format!("a GPU {} by {op} in domain {domain} while {other} writes it in domain {d} (one owner domain: 0440 §7.2)", if write { "write" } else { "read" })));
            if let Some(b) = bad {
                drop(a);
                fail(self, b);
            }
            let id = a.id();
            let top = self.top();
            let latest = top.version.load(Acquire);
            if let Some(m) = &mirror
                && a.held_in(domain).is_none_or(|d| d < latest)
            {
                if a.host < latest {
                    // Another domain owns the latest: it comes to the host first (that domain's writes completed, as
                    // its version moved when they did).
                    let owner = a.owner(latest);
                    let from = self.mirror(owner).expect("the owner domain's copy");
                    // SAFETY: no CPU use is live (the host is behind, so none could begin); the bytes are the region's.
                    from.download(unsafe { std::slice::from_raw_parts_mut(top.bytes(), top.len) });
                    a.host = latest;
                }
                // The domain's copy is behind: the transfer runs first on the same command buffer.
                // SAFETY: no CPU write is live (checked above); the bytes are the region's.
                m.upload(unsafe { std::slice::from_raw_parts(top.bytes(), top.len) }, fence);
                a.set_in(domain, latest);
            }
            if write {
                a.gpu_writes.push((id, op, Arc::downgrade(fence), domain));
            } else {
                a.gpu_reads.push((id, op));
            }
            // Its block's ledger holds the chunk too (0487 R4, one place): a writer by raw offset is checked there. A
            // whole block's pin holds the block (below), as a by-address use does.
            let bytes = self.len.max(1);
            if self.whole {
            } else if write {
                fence.writes_chunk(&self.block, self.at, bytes);
            } else {
                fence.reads_chunk(&self.block, self.at, bytes);
            }
            let held = self.clone();
            fence.on_complete(Box::new(move || {
                let mut a = held.lock();
                if write {
                    a.gpu_writes.retain(|(i, _, _, _)| *i != id);
                    let v = held.top().version.fetch_add(1, AcqRel) + 1;
                    // In a device domain only its copy has it (the owner now); in the host's memory there is one copy.
                    if domain == 0 {
                        a.host = v;
                    } else {
                        a.set_in(domain, v);
                    }
                } else {
                    a.gpu_reads.retain(|(i, _)| *i != id);
                }
            }));
        }
        // Its block's chunks are not freed under it either (0487 R3).
        if write {
            fence.writes(&self.block);
        } else {
            fence.reads(&self.block);
        }
        self.at
    }

    /// The device buffer to bind it with, and its offset in it.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn buffer(&self, device: &rerail_device::device::Device) -> Result<(rerail_device::device::Buffer, u64), String> {
        Ok((self.block.device_buffer(device)?, self.at as u64))
    }

    fn bytes(&self) -> *mut u8 {
        self.block.ptr(self.at)
    }
}

/// The block's own region (decision 0491 step 3), as a const handle: its frozenness, versions, placement and pins, in
/// the same place as its regions'. One a block while any handle holds it; a kernel's bind of the block's buffer,
/// found by address, pins it (`bind_address`). A frozen block's refuses a GPU write.
pub fn whole(block: &Arc<Block>) -> Shared<u8> {
    let mut slot = block.ledger().root.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = slot.upgrade() {
        return Shared { r, _t: PhantomData };
    }
    let r = Arc::new(Region {
        block: block.clone(),
        at: 0,
        len: block.reserved(),
        kept: 0,
        root: None,
        name: "a block",
        frozen: AtomicBool::new(!block.is_writable()),
        version: AtomicU64::new(0),
        access: Mutex::default(),
        mirrors: Mutex::new(Vec::new()),
        whole: true,
    });
    *slot = Arc::downgrade(&r);
    Shared { r, _t: PhantomData }
}

/// A kernel's bind of the bytes at `addr`, until `fence` completes, by `op`, writing them or not (its argument's
/// access, as the kernel declares it): the block or lease holding them is held (`Fence::uses_address`), and a block's
/// own region, while a handle holds it, is pinned (once a fence and access). Whether a ledger knows the address.
///
/// The ledger is found once a fence and address (`Fence::held_at`): a stack's thousands of dispatches each bind its
/// buffers, and each bind was a search of the live blocks (decision 0641).
pub fn bind_address(fence: &Arc<Fence>, addr: usize, write: bool, op: &'static str) -> bool {
    let Some(l) = fence.held_at(addr) else { return false };
    let root = l.root.lock().unwrap_or_else(|e| e.into_inner()).upgrade();
    if let Some(r) = root
        && fence.first_pin(Arc::as_ptr(&r) as usize, write)
    {
        r.pin(fence, op, write);
    }
    true
}

/// One slot of an operation (decision 0686 B1): `block`, whole, pinned for the CPU by `op`, to read or to write,
/// through the block's own region (`whole`) and its ledger. Refused, with nothing left held, when it cannot start: a
/// write of a frozen block, a write while anything else uses the block, a read while something writes it. The `Err`
/// names the block's region, `op`, and what holds it. The pin ends when it goes; a write's end moves the version.
pub fn pin_block(block: &Arc<Block>, op: &'static str, write: bool) -> Result<BlockPin, String> {
    let r = whole(block).r;
    let by = By::Op(op);
    if write {
        if r.is_frozen() {
            return Err(says(&r, format!("a CPU write by {op} of a frozen block")));
        }
        r.try_begin_write(by).map_err(|e| says(&r, e))?;
        if let Err(e) = block.ledger().try_cpu(0, r.len, true) {
            r.end_write();
            return Err(says(&r, format!("a CPU write by {op}: {e}")));
        }
        Ok(BlockPin { r, write: true, id: 0 })
    } else {
        let id = r.try_begin_read(by).map_err(|e| says(&r, e))?;
        if let Err(e) = block.ledger().try_cpu(0, r.len, false) {
            r.end_read(id);
            return Err(says(&r, format!("a CPU read by {op}: {e}")));
        }
        Ok(BlockPin { r, write: false, id })
    }
}

/// A block pinned as an operation's slot (`pin_block`): its bytes, all of them, header first, read in place.
pub struct BlockPin {
    r: Arc<Region>,
    write: bool,
    id: u64,
}

impl BlockPin {
    pub fn block(&self) -> &Arc<Block> {
        &self.r.block
    }

    /// Whether it was pinned to write.
    pub fn writes(&self) -> bool {
        self.write
    }

    /// The block's bytes: no write but this pin's can be live while it is.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the pin's checks hold (`pin_block`): no write but its own is live, and none begins while it lives.
        unsafe { std::slice::from_raw_parts(self.r.bytes(), self.r.len) }
    }

    /// The block's bytes, to write: `None` for a read pin.
    pub fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        if !self.write {
            return None;
        }
        // SAFETY: a write pin is the block's only use while it lives (`pin_block`).
        Some(unsafe { std::slice::from_raw_parts_mut(self.r.bytes(), self.r.len) })
    }

    /// Where the block lies in the host's memory: a CPU descriptor table's address of it (`gens::Desc`).
    pub fn addr(&self) -> usize {
        self.r.bytes() as usize
    }

    /// The bytes it pins: the block's whole reservation.
    pub fn len(&self) -> usize {
        self.r.len
    }

    pub fn is_empty(&self) -> bool {
        self.r.len == 0
    }

    /// The block's version: its writes so far.
    pub fn version(&self) -> u64 {
        self.r.version()
    }

    /// `n` atomic words at byte `at` of a block pinned to write (`None` for a read pin): claims and cells that many
    /// writers fill at once through one pin (host threads, or a GPU's), as a kernel writes them (P§12 item 5).
    pub fn atomics<A: crate::gens::Atomic>(&self, at: usize, n: usize) -> Option<&[A]> {
        if !self.write {
            return None;
        }
        assert!(at + n * size_of::<A>() <= self.r.len, "atomics past the block");
        // SAFETY: a write pin is the block's only use; atomics are written through shared references by design.
        let p = unsafe { self.r.bytes().add(at) };
        assert!((p as usize).is_multiple_of(align_of::<A>()), "atomics at {at}, not aligned");
        Some(unsafe { std::slice::from_raw_parts(p as *const A, n) })
    }
}

impl Drop for BlockPin {
    fn drop(&mut self) {
        if self.write {
            self.r.end_write();
        } else {
            self.r.end_read(self.id);
        }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // Nothing can use it now: its guards borrow a handle, and its GPU pins hold it.
        if self.root.is_none() && self.kept > 0 {
            self.block.free_chunk_fit(self.at, self.kept);
        }
    }
}

macro_rules! common {
    ($h:ident) => {
        impl<T: crate::Pod> $h<T> {
            /// The `T`s it holds.
            pub fn len(&self) -> usize {
                self.r.len / size_of::<T>()
            }

            pub fn is_empty(&self) -> bool {
                self.r.len == 0
            }

            /// What it is: its block, offset, name, version, state.
            pub fn region(&self) -> &Region {
                &self.r
            }

            pub fn version(&self) -> u64 {
                self.r.version()
            }

            /// A GPU read of it by `op`, until `fence` completes: its offset in its block's buffer.
            pub fn bind_read(&self, fence: &Arc<Fence>, op: &'static str) -> usize {
                self.r.pin(fence, op, false)
            }

            /// `bind_read` by a kernel in memory domain `domain` (0: the host's memory).
            pub fn bind_read_on(&self, fence: &Arc<Fence>, op: &'static str, domain: u32) -> usize {
                self.r.pin_in(fence, op, false, domain)
            }

            /// `bind_read`, with the buffer to bind and its offset.
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub fn buffer_read(&self, fence: &Arc<Fence>, op: &'static str, device: &rerail_device::device::Device) -> Result<(rerail_device::device::Buffer, u64), String> {
                self.r.pin(fence, op, false);
                self.r.buffer(device)
            }

            fn elems(&self, start: usize, end: usize) -> (usize, usize) {
                (start * size_of::<T>(), end * size_of::<T>())
            }
        }
    };
}

/// A region one owner may write until it is frozen. Not `Clone`: its writes need `&mut`.
pub struct Mutable<T: crate::Pod> {
    r: Arc<Region>,
    _t: PhantomData<T>,
}

/// A const handle of a region, open or frozen: it reads, under a guard while the region may still be written.
pub struct Shared<T: crate::Pod> {
    r: Arc<Region>,
    _t: PhantomData<T>,
}

/// A frozen region: immutable, read as a slice by any number of holders, without guards.
pub struct Frozen<T: crate::Pod> {
    r: Arc<Region>,
    _t: PhantomData<T>,
}

common!(Mutable);
common!(Shared);
common!(Frozen);

impl<T: crate::Pod> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared { r: self.r.clone(), _t: PhantomData }
    }
}

impl<T: crate::Pod> Clone for Frozen<T> {
    fn clone(&self) -> Self {
        Frozen { r: self.r.clone(), _t: PhantomData }
    }
}

impl<T: crate::Pod> Mutable<T> {
    /// `n` zeroed `T`s in `block`, named `name` for what fails: a chunk of its free list (given back when the last
    /// handle goes), or from its front. `None` when it has no room.
    pub fn alloc(block: &Arc<Block>, n: usize, name: &'static str) -> Option<Mutable<T>> {
        Self::alloc_aligned(block, n, align_of::<T>(), name)
    }

    /// `alloc`, its first byte at a multiple of `align` (at least the `T`'s, and 8): a region that holds a wider type
    /// than its own (a structure's bytes, `maps::Room`).
    pub fn alloc_aligned(block: &Arc<Block>, n: usize, align: usize, name: &'static str) -> Option<Mutable<T>> {
        let bytes = (n * size_of::<T>()).max(1);
        let align = align.max(align_of::<T>()).max(8);
        let (at, kept) = if block.has_free_list() { block.try_alloc_chunk_fit_zeroed(bytes, align)? } else { (block.try_alloc(bytes, align)?, 0) };
        let r = Region {
            block: block.clone(),
            at,
            len: n * size_of::<T>(),
            kept,
            root: None,
            name,
            frozen: AtomicBool::new(false),
            version: AtomicU64::new(0),
            access: Mutex::default(),
            mirrors: Mutex::new(Vec::new()),
            whole: false,
        };
        Some(Mutable { r: Arc::new(r), _t: PhantomData })
    }

    /// The chunk of `kept` bytes at `at` of `block`, which its caller took from the block's free list
    /// (`try_alloc_chunk_fit`), held as `n` `T`s: given back when the last handle goes, as `alloc`'s is. For an owner
    /// that places its chunks itself (a relation's arrays, near their relation).
    ///
    /// # Safety
    /// The chunk is the caller's, taken and not freed, and holds `n` `T`s.
    pub unsafe fn adopt(block: &Arc<Block>, at: usize, n: usize, kept: usize, name: &'static str) -> Mutable<T> {
        debug_assert!(n * size_of::<T>() <= kept.max(1));
        let r = Region {
            block: block.clone(),
            at,
            len: n * size_of::<T>(),
            kept,
            root: None,
            name,
            frozen: AtomicBool::new(false),
            version: AtomicU64::new(0),
            access: Mutex::default(),
            mirrors: Mutex::new(Vec::new()),
            whole: false,
        };
        Mutable { r: Arc::new(r), _t: PhantomData }
    }

    /// Its chunk handed on, to an owner that frees it itself (a relation's directory): (offset, kept bytes). Nothing
    /// else may hold it.
    #[track_caller]
    pub fn into_chunk(mut self) -> (usize, usize) {
        let Some(r) = Arc::get_mut(&mut self.r) else {
            fail(&self.r, format!("a hand-on from {} while another handle, range or GPU use holds it", Location::caller()));
        };
        if r.root.is_some() {
            fail(r, "a hand-on of a range".into());
        }
        let kept = std::mem::take(&mut r.kept);
        (r.at, kept)
    }

    /// Its first `T`, for its owner's own writes through `&mut` (a relation's array's pushes): no guard, the ledger's
    /// check (0487 R4) is the owner's.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.r.block.ptr::<T>(self.r.at)
    }

    /// Writes it: no other write, no read (CPU or GPU) may be live, and none may begin until the guard goes.
    #[track_caller]
    pub fn write(&mut self) -> Write<'_, T> {
        self.r.begin_write();
        Write { r: &self.r, _t: PhantomData }
    }

    /// Reads it: no write may be live.
    #[track_caller]
    pub fn read(&self) -> Read<'_, T> {
        let id = self.r.begin_read();
        Read { r: &self.r, id, _t: PhantomData }
    }

    /// A const handle of it.
    pub fn share(&self) -> Shared<T> {
        Shared { r: self.r.clone(), _t: PhantomData }
    }

    /// The `T`s `start..end` as a region of their own, writable, kept with this one: it and this one never write at
    /// once (their uses are one region's).
    pub fn range(&self, start: usize, end: usize) -> Mutable<T> {
        let (a, b) = self.elems(start, end);
        Mutable { r: self.r.range(a, b), _t: PhantomData }
    }

    /// A GPU write of it by `op`, until `fence` completes: its offset in its block's buffer.
    pub fn bind_write(&self, fence: &Arc<Fence>, op: &'static str) -> usize {
        self.r.pin(fence, op, true)
    }

    /// `bind_write` by a kernel in memory domain `domain` (0: the host's memory): that domain owns it once the write
    /// completes.
    pub fn bind_write_on(&self, fence: &Arc<Fence>, op: &'static str, domain: u32) -> usize {
        self.r.pin_in(fence, op, true, domain)
    }

    /// `bind_write`, with the buffer to bind and its offset.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn buffer_write(&self, fence: &Arc<Fence>, op: &'static str, device: &rerail_device::device::Device) -> Result<(rerail_device::device::Buffer, u64), String> {
        self.r.pin(fence, op, true);
        self.r.buffer(device)
    }

    /// Keeps its first `n` `T`s and gives back the rest: to its block when it was the block's last allocation from
    /// the front. A chunk of a free list keeps its whole chunk (the ledger frees chunks whole, 0487 R2). Nothing else
    /// may hold it: no other handle, range or GPU use.
    #[track_caller]
    pub fn truncate(&mut self, n: usize) {
        let keep = n * size_of::<T>();
        let Some(r) = Arc::get_mut(&mut self.r) else {
            fail(&self.r, format!("a truncate from {} while another handle, range or GPU use holds it", Location::caller()));
        };
        if keep > r.len {
            fail(r, format!("a truncate from {} to {keep} bytes, past its length", Location::caller()));
        }
        if r.kept == 0 {
            // `alloc` took at least a byte from the front.
            r.block.shrink_last(r.at, r.len.max(1), keep);
        }
        r.len = keep;
    }

    /// Freezes it: no CPU read or write may be live, and no GPU write queued; GPU reads may be.
    #[track_caller]
    pub fn freeze(self) -> Frozen<T> {
        {
            let a = self.r.lock();
            if a.write.is_some() || !a.reads.is_empty() || !a.gpu_writes.is_empty() {
                let says = a.says();
                drop(a);
                fail(&self.r, format!("a freeze from {} while {says}", Location::caller()));
            }
            // A frozen region is read on the host without guards: its host bytes must be the latest.
            if let Some(why) = self.r.host_behind(&a) {
                drop(a);
                fail(&self.r, format!("a freeze from {}: {why}", Location::caller()));
            }
        }
        if self.r.root.is_some() {
            fail(&self.r, "a freeze of a range (freeze the region it is part of)".into());
        }
        self.r.frozen.store(true, Release);
        Frozen { r: self.r, _t: PhantomData }
    }
}

impl<T: crate::Pod> Shared<T> {
    /// Reads it: no write may be live.
    #[track_caller]
    pub fn read(&self) -> Read<'_, T> {
        let id = self.r.begin_read();
        Read { r: &self.r, id, _t: PhantomData }
    }

    pub fn range(&self, start: usize, end: usize) -> Shared<T> {
        let (a, b) = self.elems(start, end);
        Shared { r: self.r.range(a, b), _t: PhantomData }
    }
}

impl<T: crate::Pod> Frozen<T> {
    pub fn range(&self, start: usize, end: usize) -> Frozen<T> {
        let (a, b) = self.elems(start, end);
        Frozen { r: self.r.range(a, b), _t: PhantomData }
    }

    /// A const handle of it.
    pub fn share(&self) -> Shared<T> {
        Shared { r: self.r.clone(), _t: PhantomData }
    }
}

impl<T: crate::Pod> std::ops::Deref for Frozen<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: frozen, nothing writes it (its one writer was consumed, and a GPU write of it fails); it lives as
        // long as this handle.
        unsafe { std::slice::from_raw_parts(self.r.bytes().cast::<T>(), self.len()) }
    }
}

/// A live CPU read of a region.
pub struct Read<'a, T: crate::Pod> {
    r: &'a Arc<Region>,
    id: u64,
    _t: PhantomData<T>,
}

impl<T: crate::Pod> Read<'_, T> {
    /// The version it reads.
    pub fn version(&self) -> u64 {
        self.r.version()
    }
}

impl<T: crate::Pod> std::ops::Deref for Read<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: no write is live while this read is (`begin_read`, `begin_write`).
        unsafe { std::slice::from_raw_parts(self.r.bytes().cast::<T>(), self.r.len / size_of::<T>()) }
    }
}

impl<T: crate::Pod> Drop for Read<'_, T> {
    fn drop(&mut self) {
        self.r.end_read(self.id);
    }
}

/// A live CPU write of a region.
pub struct Write<'a, T: crate::Pod> {
    r: &'a Arc<Region>,
    _t: PhantomData<T>,
}

impl<T: crate::Pod> std::ops::Deref for Write<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: this write is the region's only use while it lives.
        unsafe { std::slice::from_raw_parts(self.r.bytes().cast::<T>(), self.r.len / size_of::<T>()) }
    }
}

impl<T: crate::Pod> std::ops::DerefMut for Write<'_, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as `deref`.
        unsafe { std::slice::from_raw_parts_mut(self.r.bytes().cast::<T>(), self.r.len / size_of::<T>()) }
    }
}

impl<T: crate::Pod> Drop for Write<'_, T> {
    fn drop(&mut self) {
        self.r.end_write();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::Ordering::SeqCst;

    fn block() -> Arc<Block> {
        Arc::new(Block::anon(kind::FACTS, 8 << 20).with_free_list())
    }

    fn fails(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    /// A write while a read is live fails, through any handle; once the read goes it passes.
    #[test]
    fn a_write_while_a_read_is_live_fails() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 64, "rows").unwrap();
        let s = m.share();
        {
            let r = s.read();
            assert!(fails(|| drop(m.write())), "a reader is live");
            drop(r);
        }
        m.write()[3] = 7;
        assert_eq!(s.read()[3], 7);
    }

    /// A read while a write is live fails, and so does a second writer (a range of the same region).
    #[test]
    fn a_read_or_a_second_write_while_a_write_is_live_fails() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 64, "rows").unwrap();
        let s = m.share();
        let mut part = m.range(0, 8);
        let w = m.write();
        assert!(fails(|| drop(s.read())), "a write is live");
        assert!(fails(|| drop(part.write())), "two writers");
        drop(w);
        part.write()[0] = 1;
    }

    /// The CPU may not write a region a queued GPU read or write uses, nor read one a GPU write uses; the GPU may not
    /// write one a CPU read is live on. Once the fence completes, they pass.
    #[test]
    fn cpu_and_gpu_uses_of_a_region_conflict_as_readers_and_writers() {
        let b = block();
        let mut m = Mutable::<u32>::alloc(&b, 1024, "index").unwrap();
        let f = Fence::new();
        m.bind_read(&f, "probe");
        let _ = m.read();
        assert!(fails(|| drop(m.write())), "a GPU read is queued");
        f.complete();
        m.write()[0] = 1;
        let g = Fence::new();
        m.bind_write(&g, "build");
        assert!(fails(|| drop(m.read())), "a GPU write is queued");
        g.complete();
        let r = m.read();
        let h = Fence::new();
        assert!(fails(|| { m.bind_write(&h, "build"); }), "a CPU read is live");
        drop(r);
    }

    /// A frozen region is read without guards by any number of holders; a GPU write of it fails, as does a freeze
    /// while it is read or written.
    #[test]
    fn a_frozen_region_reads_without_guards_and_refuses_writes() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 16, "kept").unwrap();
        m.write()[5] = 42;
        let s = m.share();
        let r = s.read();
        // The freeze consumes `m`, so it is tried on one that is read.
        let other = Mutable::<u64>::alloc(&b, 16, "other").unwrap();
        let read = other.share();
        let live = read.read();
        assert!(fails(move || drop(other.freeze())), "a read is live");
        drop(live);
        drop(r);
        let f = m.freeze();
        let g = f.clone();
        assert_eq!((f[5], g[5]), (42, 42));
        let fence = Fence::new();
        assert!(fails(|| { s.r.pin(&fence, "w", true); }), "a GPU write of a frozen region");
        f.bind_read(&fence, "r");
    }

    /// A region goes back to its block when its last handle goes, and not before the GPU work that uses it
    /// completes.
    #[test]
    fn a_region_is_freed_with_its_last_handle_after_its_gpu_work() {
        let b = block();
        let m = Mutable::<u64>::alloc(&b, 512, "rows").unwrap();
        let at = m.region().at();
        let s = m.share();
        drop(m);
        let busy = Mutable::<u64>::alloc(&b, 512, "busy").unwrap();
        assert_ne!(busy.region().at(), at, "a shared handle holds it");
        let f = Fence::new();
        s.bind_read(&f, "probe");
        drop(s);
        let again = Mutable::<u64>::alloc(&b, 512, "again").unwrap();
        assert_ne!(again.region().at(), at, "the GPU's read holds it");
        f.complete();
        let last = Mutable::<u64>::alloc(&b, 512, "last").unwrap();
        assert_eq!(last.region().at(), at, "back in its block once the read completed");
    }

    /// A range keeps its region alive and shares its uses and version; a range past the end fails.
    #[test]
    fn a_range_keeps_its_region_and_shares_its_uses() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 512, "rows").unwrap();
        let at = m.region().at();
        m.write()[100] = 9;
        let f = m.freeze();
        let part = f.range(100, 110);
        drop(f);
        assert_eq!(part[0], 9);
        assert_ne!(Mutable::<u64>::alloc(&b, 512, "x").unwrap().region().at(), at, "the range holds it");
        assert!(fails(|| drop(part.range(5, 11))), "past its end");
        assert_eq!(part.region().name(), "rows");
    }

    /// A region truncated to what was written gives its tail back: from the front of its block, the next allocation
    /// lands in it. A truncate while another handle holds it, or past its length, fails.
    #[test]
    fn a_truncated_region_gives_back_its_tail() {
        let b = Arc::new(Block::anon(kind::FACTS, 8 << 20));
        let mut m = Mutable::<u32>::alloc(&b, 1000, "cells").unwrap();
        let at = m.region().at();
        m.write()[..10].copy_from_slice(&[1; 10]);
        let s = m.share();
        assert!(fails(|| m.truncate(10)), "another handle holds it");
        drop(s);
        assert!(fails(|| m.truncate(1001)), "past its length");
        m.truncate(10);
        assert_eq!(m.len(), 10);
        let next = Mutable::<u32>::alloc(&b, 4, "next").unwrap();
        assert!(next.region().at() < at + 4 * 1000, "the tail went back: {} after {at}", next.region().at());
        let f = m.freeze();
        assert_eq!(&f[..], &[1; 10]);
    }

    /// A region's GPU pin is its block's chunk use too: the ledger's CPU check, which writers by raw offset take, sees
    /// it (0487 R4, one place).
    #[test]
    fn a_regions_gpu_pin_is_its_ledgers_chunk_use() {
        let b = block();
        let m = Mutable::<u64>::alloc(&b, 64, "rows").unwrap();
        let (at, len) = (m.region().at(), m.region().len());
        let f = Fence::new();
        m.bind_write(&f, "kernel");
        assert!(fails(|| b.ledger().cpu_read(at, 8)), "the ledger sees the GPU write");
        f.complete();
        b.ledger().cpu_write(at, len);
        let s = m.share();
        let g = Fence::new();
        s.bind_read(&g, "kernel");
        assert!(fails(|| b.ledger().cpu_write(at + 8, 8)), "the ledger sees the GPU read");
        b.ledger().cpu_read(at, len);
        g.complete();
    }

    /// An adopted chunk goes back to its block's free list with its last handle; one handed on stays taken, and a
    /// hand-on while shared fails.
    #[test]
    fn an_adopted_chunk_is_freed_with_its_region_and_kept_when_handed_on() {
        let b = block();
        let (at, kept) = b.try_alloc_chunk_fit(800, 8).unwrap();
        let free = b.free_bytes();
        // SAFETY: the chunk is ours, 800 bytes.
        let mut m = unsafe { Mutable::<u64>::adopt(&b, at, 100, kept, "index") };
        assert_eq!(m.as_mut_ptr() as usize, b.ptr::<u64>(at) as usize);
        drop(m);
        assert!(b.free_bytes() > free, "freed with its region");
        let (at, kept) = b.try_alloc_chunk_fit(800, 8).unwrap();
        let m = unsafe { Mutable::<u64>::adopt(&b, at, 100, kept, "index") };
        let s = m.share();
        let m = std::panic::catch_unwind(AssertUnwindSafe(move || m.into_chunk()));
        assert!(m.is_err(), "a hand-on while shared");
        drop(s);
        let (at2, kept2) = b.try_alloc_chunk_fit(800, 8).unwrap();
        let m = unsafe { Mutable::<u64>::adopt(&b, at2, 100, kept2, "index") };
        let free = b.free_bytes();
        assert_eq!(m.into_chunk(), (at2, kept2));
        assert_eq!(b.free_bytes(), free, "handed on: still taken");
        b.free_chunk_fit(at2, kept2);
    }

    /// Decision 0491 step 3: a block's own region is one while a handle holds it; a kernel's bind of the block found by
    /// address pins it, by the kernel's access, once a fence: a CPU read through it fails while a GPU write is queued,
    /// and the version moves when the write completes. A chunk of the block is not held by the pin: its CPU writers
    /// write other chunks meanwhile.
    #[test]
    fn a_blocks_bind_by_address_pins_its_own_region() {
        let b = block();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        let root = whole(&b);
        assert!(Arc::ptr_eq(&root.r, &whole(&b).r), "one a block");
        assert!(!root.region().is_frozen());
        let f = Fence::new();
        assert!(bind_address(&f, b.ptr::<u8>(at) as usize + 8, true, "a kernel"));
        assert!(bind_address(&f, b.ptr::<u8>(at) as usize, true, "a kernel"), "a second bind, pinned once");
        assert_eq!(root.r.lock().gpu_writes.len(), 1);
        assert!(fails(|| drop(root.read())), "a CPU read of the block through its region under a GPU write");
        b.ledger().cpu_write(at, 8);
        f.complete();
        assert_eq!(root.region().version(), 1, "the GPU's write moved it");
        drop(root.read());
        assert!(!bind_address(&Fence::new(), 8, false, "a kernel"), "an address no ledger knows");
    }

    /// A frozen block's own region is frozen: a kernel's write bind of it fails, a read bind passes.
    #[test]
    fn a_frozen_blocks_region_refuses_a_gpu_write() {
        let b = Block::anon(kind::FACTS, 1 << 20);
        let at = b.try_alloc(64, 8).unwrap();
        let frozen = b.freeze();
        let arc = frozen.arc().clone();
        let root = whole(&arc);
        assert!(root.region().is_frozen());
        let addr = arc.ptr::<u8>(at) as usize;
        let f = Fence::new();
        assert!(bind_address(&f, addr, false, "a reader"));
        assert!(fails(|| { bind_address(&Fence::new(), addr, true, "a writer"); }));
        f.complete();
    }

    /// A device with memory of its own (a discrete GPU's), simulated: its copy, and what crossed the bus.
    #[derive(Default)]
    struct Discrete {
        mem: Mutex<Vec<u8>>,
        ups: AtomicU64,
        downs: AtomicU64,
    }

    struct OnDiscrete(Arc<Discrete>);

    impl Mirror for OnDiscrete {
        fn upload(&self, host: &[u8], _fence: &Fence) {
            *self.0.mem.lock().unwrap() = host.to_vec();
            self.0.ups.fetch_add(1, SeqCst);
        }
        fn download(&self, host: &mut [u8]) {
            host.copy_from_slice(&self.0.mem.lock().unwrap());
            self.0.downs.fetch_add(1, SeqCst);
        }
    }

    /// On a device with memory of its own, a kernel's bind uploads the region only when the device's copy is behind;
    /// after a GPU write the host's bytes are behind, a CPU read or a freeze fails until `sync_host` brings them back.
    #[test]
    fn a_device_copy_is_uploaded_when_behind_and_synced_back_after_a_write() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 8, "rows").unwrap();
        let dev = Arc::new(Discrete::default());
        m.region().mirror_on(Box::new(OnDiscrete(dev.clone())));
        m.write()[0] = 7;
        let f = Fence::new();
        m.bind_read(&f, "probe");
        m.bind_read(&f, "probe again");
        assert_eq!(dev.ups.load(SeqCst), 1, "uploaded once, when behind");
        assert_eq!(dev.mem.lock().unwrap()[0], 7);
        f.complete();
        let g = Fence::new();
        m.bind_write(&g, "build");
        assert_eq!(dev.ups.load(SeqCst), 1, "the device is current");
        dev.mem.lock().unwrap()[0] = 9; // the kernel's write, on the device's copy
        g.mark_committed();
        g.complete();
        assert_eq!(m.version(), 2);
        assert!(fails(|| drop(m.read())), "the host's bytes are a version behind");
        m.region().sync_host();
        assert_eq!(dev.downs.load(SeqCst), 1);
        assert_eq!(m.read()[0], 9);
        m.region().sync_host();
        assert_eq!(dev.downs.load(SeqCst), 1, "nothing to bring back");
        m.write()[1] = 5;
        let h = Fence::new();
        m.bind_read(&h, "probe");
        assert_eq!(dev.ups.load(SeqCst), 2, "the CPU's write sent before the next read");
        // A freeze needs the host current.
        let k = Fence::new();
        m.bind_write(&k, "build");
        k.mark_committed();
        k.complete();
        assert!(fails(move || drop(m.freeze())), "frozen with its host bytes behind");
    }

    /// Decision 0491 step 4 (0440 §7.2): a region in two device domains besides the host's. A write in one makes it the
    /// owner and the other behind; a read in the other takes the latest through the host (a download from the owner,
    /// an upload to it); a write in one while the other's write is queued fails, and a bind naming no domain fails.
    #[test]
    fn two_domains_hold_one_owner_and_the_other_catches_up_through_the_host() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 8, "rows").unwrap();
        let (one, two) = (Arc::new(Discrete::default()), Arc::new(Discrete::default()));
        m.region().mirror_in(1, Box::new(OnDiscrete(one.clone())));
        m.region().mirror_in(2, Box::new(OnDiscrete(two.clone())));
        assert!(fails(|| m.region().mirror_in(2, Box::new(OnDiscrete(two.clone())))), "a second copy in one domain");
        m.write()[0] = 7;
        assert_eq!(m.region().owner(), 0, "the host's write: the host owns it");
        assert!(fails(|| { m.bind_read(&Fence::new(), "unnamed"); }), "two domains: a bind names one");
        let w = Fence::new();
        m.bind_write_on(&w, "build", 1);
        assert_eq!(one.ups.load(SeqCst), 1, "domain 1 brought up to date before it writes");
        assert!(fails(|| { m.bind_write_on(&Fence::new(), "rival", 2); }), "two domains never write one region");
        assert!(fails(|| { m.bind_read_on(&Fence::new(), "stale", 2); }), "nor read it while another writes it");
        one.mem.lock().unwrap()[0] = 9; // domain 1's kernel writes its copy
        w.mark_committed();
        w.complete();
        assert_eq!(m.region().owner(), 1, "the writer's domain owns the latest");
        let r = Fence::new();
        m.bind_read_on(&r, "probe", 2);
        assert_eq!(one.downs.load(SeqCst), 1, "the owner's copy came to the host");
        assert_eq!(two.ups.load(SeqCst), 1, "and from there to domain 2");
        assert_eq!(two.mem.lock().unwrap()[0], 9);
        r.complete();
        assert_eq!(m.read()[0], 9, "the host holds the latest too");
        let r1 = Fence::new();
        m.bind_read_on(&r1, "probe", 1);
        assert_eq!(one.ups.load(SeqCst), 1, "domain 1 holds the latest: nothing moves");
        r1.complete();
    }

    /// With unified memory there is one copy: a host sync is a wait for the GPU's queued writes, and nothing moves.
    /// A sync on a write whose work was never committed fails, as the wait would never end.
    #[test]
    fn with_unified_memory_a_sync_is_a_wait() {
        let b = block();
        let m = Mutable::<u64>::alloc(&b, 8, "rows").unwrap();
        let g = Fence::new();
        m.bind_write(&g, "build");
        assert!(fails(|| drop(m.read())), "a GPU write is queued");
        assert!(fails(|| m.region().sync_host()), "its work is not committed");
        g.mark_committed();
        let done = g.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            done.complete();
        });
        m.region().sync_host();
        assert_eq!(m.read().version(), 1);
        t.join().unwrap();
    }

    /// A region's version counts its writes: a CPU write ended, a GPU write completed.
    #[test]
    fn a_write_moves_the_version() {
        let b = block();
        let mut m = Mutable::<u64>::alloc(&b, 8, "v").unwrap();
        assert_eq!(m.version(), 0);
        m.write()[0] = 1;
        assert_eq!(m.version(), 1);
        let f = Fence::new();
        m.bind_write(&f, "k");
        assert_eq!(m.version(), 1, "not until it completes");
        f.complete();
        assert_eq!(m.share().read().version(), 2);
    }

    /// An operation's slots (decision 0686 B1): a block pinned by one operation refuses another's conflicting pin,
    /// naming both; reads share; a frozen block refuses a write; a refused pin leaves nothing held.
    #[test]
    fn a_slot_pin_refuses_what_conflicts_and_names_the_operations() {
        let b = block();
        let w = pin_block(&b, "consolidate", true).unwrap();
        let e = pin_block(&b, "step", true).err().expect("a second writer is refused");
        assert!(e.contains("a CPU write by step") && e.contains("a CPU write by consolidate"), "{e}");
        let e = pin_block(&b, "seed", false).err().expect("a reader beside a writer is refused");
        assert!(e.contains("a CPU read by seed") && e.contains("by consolidate"), "{e}");
        drop(w);
        let r1 = pin_block(&b, "seed", false).unwrap();
        let r2 = pin_block(&b, "step", false).unwrap();
        let e = pin_block(&b, "head", true).err().expect("a writer beside readers is refused");
        assert!(e.contains("by seed") && e.contains("by step"), "{e}");
        drop((r1, r2));
        let held = whole(&b);
        let mut w = pin_block(&b, "head", true).expect("nothing is left held by the refused pins");
        let v = held.version();
        w.bytes_mut().unwrap()[64] = 1;
        drop(w);
        assert_eq!(held.version(), v + 1, "a write pin's end moves the version");
        assert!(pin_block(&b, "seed", false).unwrap().bytes_mut().is_none(), "a read pin gives no bytes to write");
        let f = Block::anon(kind::FACTS, 1 << 20).freeze();
        let e = pin_block(f.arc(), "head", true).err().expect("a frozen block refuses a write");
        assert!(e.contains("a CPU write by head of a frozen block"), "{e}");
        assert!(pin_block(f.arc(), "seed", false).is_ok(), "and is read");
    }
}
