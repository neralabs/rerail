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

use crate::Block;
use crate::ledger::Fence;
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct Access {
    reads: Vec<(u64, &'static Location<'static>)>,
    write: Option<&'static Location<'static>>,
    gpu_reads: Vec<(u64, &'static str)>,
    gpu_writes: Vec<(u64, &'static str, std::sync::Weak<Fence>)>,
    next: u64,
    /// The version the host's bytes hold (the block's), and the device's copy's when it has its own (`Mirror`):
    /// `None` before its first upload. With unified memory there is one copy, and both are the latest once the
    /// GPU's writes complete.
    host: u64,
    device: Option<u64>,
}

impl Access {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn says(&self) -> String {
        let mut v = Vec::new();
        if let Some(w) = self.write {
            v.push(format!("a CPU write from {w}"));
        }
        v.extend(self.reads.iter().map(|(_, l)| format!("a CPU read from {l}")));
        v.extend(self.gpu_reads.iter().map(|(_, op)| format!("a GPU read by {op}")));
        v.extend(self.gpu_writes.iter().map(|(_, op, _)| format!("a GPU write by {op}")));
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
    mirror: std::sync::OnceLock<Box<dyn Mirror>>,
}

fn fail(r: &Region, what: String) -> ! {
    panic!("region '{}' ({} bytes at {} of its block): {what}", r.top().name, r.len, r.at);
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
            mirror: std::sync::OnceLock::new(),
        })
    }

    /// Places it on a device with memory of its own: `mirror` holds the device's copy, uploaded before a kernel reads
    /// a version it lacks, downloaded by `sync_host` after one writes it. A region's, not a range's; once.
    pub fn mirror_on(&self, mirror: Box<dyn Mirror>) {
        if self.root.is_some() || self.top().mirror.set(mirror).is_err() {
            fail(self, "a second device copy, or one of a range (place the region it is part of)".into());
        }
    }

    /// Brings the host's bytes up to its latest version: waits for the GPU's queued writes, then (with a device
    /// copy) transfers it back. A queued write whose work was never committed fails: the wait would never end.
    pub fn sync_host(&self) {
        let writers: Vec<_> = self.lock().gpu_writes.iter().map(|(_, op, f)| (*op, f.clone())).collect();
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
            let m = top.mirror.get().expect("a host behind its latest version has a device copy");
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
        if let Some((_, op, _)) = a.gpu_writes.first() {
            Some(format!("a GPU write by {op} is queued (sync_host first)"))
        } else if a.host < latest {
            Some(format!("its host bytes hold version {} of {latest} (sync_host first)", a.host))
        } else {
            None
        }
    }

    #[track_caller]
    fn begin_read(&self) -> u64 {
        let mut a = self.lock();
        if a.write.is_some() {
            let says = a.says();
            drop(a);
            fail(self, format!("a CPU read from {} while {says}", Location::caller()));
        }
        if let Some(why) = self.host_behind(&a) {
            drop(a);
            fail(self, format!("a CPU read from {}: {why}", Location::caller()));
        }
        let id = a.id();
        a.reads.push((id, Location::caller()));
        id
    }

    fn end_read(&self, id: u64) {
        self.lock().reads.retain(|(i, _)| *i != id);
    }

    #[track_caller]
    fn begin_write(&self) {
        let mut a = self.lock();
        if a.write.is_some() || !a.reads.is_empty() || !a.gpu_reads.is_empty() || !a.gpu_writes.is_empty() {
            let says = a.says();
            drop(a);
            fail(self, format!("a CPU write from {} while {says}", Location::caller()));
        }
        if let Some(why) = self.host_behind(&a) {
            drop(a);
            fail(self, format!("a CPU write from {}: {why}", Location::caller()));
        }
        a.write = Some(Location::caller());
        // SAFETY of the ledger's check too: the chunk's bytes are not the GPU's now (R4 of 0487).
        self.block.ledger().cpu_write(self.at, self.len);
    }

    fn end_write(&self) {
        let mut a = self.lock();
        a.write = None;
        // The host's bytes are the latest; a device copy is behind until its next upload.
        a.host = self.top().version.fetch_add(1, AcqRel) + 1;
    }

    /// A GPU use of it by `op`, pinned on `fence` until it completes (the region kept alive with it).
    fn pin(self: &Arc<Region>, fence: &Arc<Fence>, op: &'static str, write: bool) -> usize {
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
            if let Some(b) = bad {
                drop(a);
                fail(self, b);
            }
            let id = a.id();
            let top = self.top();
            let latest = top.version.load(Acquire);
            if let Some(m) = top.mirror.get()
                && a.device.is_none_or(|d| d < latest)
            {
                // The device's copy is behind: the transfer runs first on the same command buffer. A queued GPU
                // write of it would be ahead of the host (its version is the device's), so there is none here.
                // SAFETY: no CPU write is live (checked above); the bytes are the region's.
                m.upload(unsafe { std::slice::from_raw_parts(top.bytes(), top.len) }, fence);
                a.device = Some(latest);
            }
            if write {
                a.gpu_writes.push((id, op, Arc::downgrade(fence)));
            } else {
                a.gpu_reads.push((id, op));
            }
            // Its block's ledger holds the chunk too (0487 R4, one place): a writer by raw offset is checked there.
            let bytes = self.len.max(1);
            if write {
                fence.writes_chunk(&self.block, self.at, bytes);
            } else {
                fence.reads_chunk(&self.block, self.at, bytes);
            }
            let held = self.clone();
            fence.on_complete(Box::new(move || {
                let mut a = held.lock();
                if write {
                    a.gpu_writes.retain(|(i, _, _)| *i != id);
                    let v = held.top().version.fetch_add(1, AcqRel) + 1;
                    // With a device copy only the device has it; with unified memory there is one copy.
                    if held.top().mirror.get().is_some() {
                        a.device = Some(v);
                    } else {
                        a.host = v;
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
        let bytes = (n * size_of::<T>()).max(1);
        let align = align_of::<T>().max(8);
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
            mirror: std::sync::OnceLock::new(),
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
            mirror: std::sync::OnceLock::new(),
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
}
