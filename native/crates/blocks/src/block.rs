//! A block: one mapping, reserved large and sparse, bump-allocated from the front, frozen once.
//!
//! The header is at offset 0. Its state and length are written last at freeze. A block read back from a file is
//! valid only when its magic and kind match, its state is frozen, and its length is the file's. No checksum:
//! corruption is the storage's and ECC's to handle.

use crate::structure;
use std::fs::File;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io;
use std::ops::Deref;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::*, fence};

/// Apple Silicon's page: chunks of a page or more are page-aligned, so a part never written is never resident.
pub const PAGE: usize = 16384;
pub const MAGIC: u64 = u64::from_le_bytes(*b"RRBLOCK1");
pub const OPEN: u32 = 1;
pub const FROZEN: u32 = 2;

pub mod kind {
    pub const FACTS: u32 = 1;
    pub const CHUNK: u32 = 2;
    pub const PROGRAM: u32 = 3;
    pub const OUTPUTS: u32 = 4;
    pub const JOURNAL: u32 = 5;
    pub const SCRATCH: u32 = 6;
}

structure! {
    /// At offset 0 of every block.
    pub struct Header {
        pub magic: u64,
        pub kind: u32,
        pub format: u32,
        pub state: u32,
        /// 1 when this block is a delta of another (`base`).
        pub delta: u32,
        pub length: u64,
        pub reserved: u64,
        /// The root structure's offset from the block's start.
        pub root: u64,
        /// The full block a delta names (its identity, as the store knows it).
        pub base: u64,
        /// What carrying this delta has cost so far, in bytes (decision 0266 §6).
        pub cost: std::sync::atomic::AtomicU64,
    }
}

/// Memory a block lives in that it does not map itself: a pooled slab, pinned for the GPU and recycled (the
/// engine's pool, `rerail_accel::gpu::shared::lease_block`). It may hold an earlier block's bytes.
pub trait Backing: Send + Sync {
    fn base(&self) -> *mut u8;
    fn len(&self) -> usize;
    /// The device buffer over the whole of it, made once by its owner.
    #[cfg(target_os = "macos")]
    fn buffer(&self) -> Option<metal::Buffer> {
        None
    }
}

pub struct Block {
    base: *mut u8,
    reserved: usize,
    top: AtomicUsize,
    file: Option<File>,
    writable: bool,
    /// A slab's owner, when the block lives in one: dropped with the block, which never unmaps it.
    backing: Option<Box<dyn Backing>>,
    /// Its memory is not fresh (a recycled slab): every allocation is zeroed as it is made.
    dirty: bool,
    /// The device buffer this block is read through, kept: a buffer's first use maps it into the GPU's address
    /// space at about 20 ms a GiB (0266 §12), so it is made once and remade only when the block outgrows it.
    #[cfg(target_os = "macos")]
    gpu: std::sync::Mutex<Option<(metal::Buffer, usize)>>,
}

unsafe impl Send for Block {}
unsafe impl Sync for Block {}

fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

impl Block {
    /// An anonymous block of `reserved` bytes of address space: nothing is resident until written.
    pub fn anon(kind: u32, reserved: usize) -> Block {
        let reserved = round_up(reserved.max(PAGE), PAGE);
        let p = crate::mem::reserve(reserved);
        let b = Block::make(p, reserved, 0, None, true, None);
        b.init(kind);
        b
    }

    /// A block backed by the file at `path`: sparse, and written back by the OS (no `fsync`). Unix only: elsewhere
    /// `Unsupported`.
    #[cfg(not(unix))]
    pub fn file(_path: &Path, _kind: u32, _reserved: usize) -> io::Result<Block> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "a block in a file needs a shared mapping"))
    }

    /// A block backed by the file at `path`: sparse, and written back by the OS (no `fsync`).
    #[cfg(unix)]
    pub fn file(path: &Path, kind: u32, reserved: usize) -> io::Result<Block> {
        let reserved = round_up(reserved.max(PAGE), PAGE);
        let f = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        f.set_len(reserved as u64)?;
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), reserved, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, f.as_raw_fd(), 0)
        };
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let b = Block::make(p.cast(), reserved, 0, Some(f), true, None);
        b.init(kind);
        Ok(b)
    }

    fn make(base: *mut u8, reserved: usize, top: usize, file: Option<File>, writable: bool, backing: Option<Box<dyn Backing>>) -> Block {
        let dirty = backing.is_some();
        Block {
            base,
            reserved,
            top: AtomicUsize::new(top),
            file,
            writable,
            backing,
            dirty,
            #[cfg(target_os = "macos")]
            gpu: std::sync::Mutex::new(None),
        }
    }

    /// A block in memory it does not own: `backing`'s, which may be dirty. Freezing it does not protect it (a
    /// slab's pages are the pool's); its owner gets it back when the block is dropped.
    pub fn over(kind: u32, backing: Box<dyn Backing>) -> Block {
        Block::over_with(kind, backing, true)
    }

    /// `over`, saying whether the backing may hold earlier bytes (`dirty`): a backing known to read zero (a new
    /// file, a new buffer) is not zeroed as it is allocated.
    pub fn over_with(kind: u32, backing: Box<dyn Backing>, dirty: bool) -> Block {
        let (base, reserved) = (backing.base(), backing.len());
        assert!(reserved >= PAGE && base as usize % 64 == 0, "a backing of at least a page, 64-byte aligned");
        unsafe { std::ptr::write_bytes(base, 0, size_of::<Header>()) };
        let mut b = Block::make(base, reserved, 0, None, true, Some(backing));
        b.dirty = dirty;
        b.init(kind);
        b
    }

    /// Moves this open block into `backing`: its bytes copied whole, which is the whole move, since every
    /// reference inside it is self-relative. Nothing may hold a reference into this block, and no command read it.
    pub fn relocate(self, backing: Box<dyn Backing>) -> Block {
        assert!(self.writable, "only an open block moves");
        let used = self.used();
        assert!(backing.len() >= used, "a backing of {} bytes for a block of {used}", backing.len());
        let base = backing.base();
        unsafe { std::ptr::copy_nonoverlapping(self.base, base, used) };
        let b = Block::make(base, backing.len(), used, None, true, Some(backing));
        unsafe { b.header_mut().reserved = b.reserved as u64 };
        b
    }

    /// Lets go of the no-copy wrap `gpu_buffer` kept; the device must be done with it. It does not let the OS write
    /// back a file block's pages a kernel wrote: the driver holds them wired until the range is unmapped (decision
    /// 0295 §1 and §7, measured by `examples/deferrable`).
    #[cfg(target_os = "macos")]
    pub fn release_gpu_buffer(&self) {
        self.gpu.lock().unwrap().take();
    }

    /// The device buffer to bind this block with: its slab's own, or a no-copy wrap of its used extent made once
    /// and kept (remade, twice as long, only when the block has outgrown it).
    #[cfg(target_os = "macos")]
    pub fn gpu_buffer(&self, device: &metal::DeviceRef) -> metal::Buffer {
        if let Some(b) = self.backing.as_ref().and_then(|b| b.buffer()) {
            return b;
        }
        let mut kept = self.gpu.lock().unwrap();
        let used = self.used();
        if let Some((b, len)) = kept.as_ref()
            && *len >= used
        {
            return b.clone();
        }
        let len = kept.as_ref().map_or(1 << 20, |k| k.1 * 2).max(used.div_ceil(1 << 20) << 20).min(self.reserved);
        let b = device.new_buffer_with_bytes_no_copy(
            self.base as *const std::ffi::c_void,
            len as u64,
            metal::MTLResourceOptions::StorageModeShared,
            None,
        );
        *kept = Some((b.clone(), len));
        b
    }

    fn init(&self, kind: u32) {
        self.top.store(round_up(size_of::<Header>(), 64), Relaxed);
        let h = unsafe { &mut *(self.base as *mut Header) };
        h.magic = MAGIC;
        h.kind = kind;
        h.format = 1;
        h.state = OPEN;
        h.reserved = self.reserved as u64;
    }

    pub fn header(&self) -> &Header {
        unsafe { &*(self.base as *const Header) }
    }

    /// # Safety
    /// The block is open, and the caller is its one owner.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn header_mut(&self) -> &mut Header {
        assert!(self.writable);
        unsafe { &mut *(self.base as *mut Header) }
    }

    /// `size` bytes at `align`, from the front: many threads may allocate at once.
    pub fn alloc(&self, size: usize, align: usize) -> usize {
        self.try_alloc(size, align).unwrap_or_else(|| panic!("block full: {} more bytes past {} of {}", size, self.used(), self.reserved))
    }

    /// `size` bytes at `align`, or `None` when the block's reservation cannot hold them.
    pub fn try_alloc(&self, size: usize, align: usize) -> Option<usize> {
        let start = self.try_alloc_raw(size, align)?;
        if self.dirty {
            // SAFETY: `start..start + size` was just allocated to this call alone.
            unsafe { std::ptr::write_bytes(self.base.add(start), 0, size) };
        }
        Some(start)
    }

    /// `try_alloc`, its bytes as the backing left them (a dirty block's earlier bytes): for what every reader finds
    /// written first, as a relation's chunks (a value is read only below `len`, and written before it is claimed).
    /// A device relation's chunks of a pooled block then cost no memset (decision 0325 step 5).
    pub fn try_alloc_raw(&self, size: usize, align: usize) -> Option<usize> {
        assert!(self.writable, "alloc in a frozen block");
        let mut cur = self.top.load(Relaxed);
        loop {
            let start = round_up(cur, align);
            let end = start + size;
            if end > self.reserved {
                return None;
            }
            match self.top.compare_exchange_weak(cur, end, AcqRel, Relaxed) {
                Ok(_) => return Some(start),
                Err(c) => cur = c,
            }
        }
    }

    /// Gives back the tail of the last allocation (`size` bytes at `at`), keeping its first `keep` bytes. When
    /// another allocation came after it, nothing is given back. Whether it was. The tail must be unwritten: an
    /// allocation of a block that is not dirty is zero, as the mapping gave it.
    pub fn shrink_last(&self, at: usize, size: usize, keep: usize) -> bool {
        assert!(keep <= size);
        self.top.compare_exchange(at + size, at + keep, AcqRel, Relaxed).is_ok()
    }

    /// `n` zeroed `T`s, or `None` when the block cannot hold them.
    pub fn try_alloc_t<T>(&self, n: usize) -> Option<*mut T> {
        self.try_alloc(n * size_of::<T>(), align_of::<T>().max(8)).map(|at| self.ptr(at))
    }

    /// `n` zeroed `T`s. Memory never written is zero, as the mapping gives it.
    pub fn alloc_t<T>(&self, n: usize) -> *mut T {
        self.ptr(self.alloc(n * size_of::<T>(), align_of::<T>().max(8)))
    }

    pub fn ptr<T>(&self, off: usize) -> *mut T {
        unsafe { self.base.add(off) as *mut T }
    }

    pub fn offset_of<T>(&self, p: *const T) -> usize {
        let o = p as usize - self.base as usize;
        assert!(o < self.reserved);
        o
    }

    pub fn base_ptr(&self) -> *mut u8 {
        self.base
    }

    pub fn reserved(&self) -> usize {
        self.reserved
    }

    /// Bytes used so far (the frozen length, once frozen).
    pub fn used(&self) -> usize {
        self.top.load(Acquire)
    }

    pub fn set_root<T>(&self, p: *const T) {
        unsafe { self.header_mut().root = self.offset_of(p) as u64 };
    }

    pub fn root<T>(&self) -> &T {
        unsafe { &*(self.base.add(self.header().root as usize) as *const T) }
    }

    pub fn bytes(&self, start: usize, end: usize) -> &[u8] {
        assert!(start <= end && end <= self.used());
        unsafe { std::slice::from_raw_parts(self.base.add(start), end - start) }
    }

    /// The start of where the next allocation at `align` would go: a range's start.
    pub fn mark(&self, align: usize) -> usize {
        self.alloc(0, align)
    }

    /// Copies a range of another block in. It keeps the range's place within a page, so page-aligned chunks stay
    /// page-aligned, and every self-relative reference inside it stays valid. Returns the copy's start.
    pub fn copy_in(&self, src: &[u8], src_start: usize) -> usize {
        let room = self.alloc(src.len() + PAGE, PAGE);
        let dst = room + src_start % PAGE;
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), self.base.add(dst), src.len()) };
        dst
    }

    /// Freezes: the length, then the state, written last. A file block is truncated to its length. An anonymous
    /// block is made read-only, so a write after freeze faults instead of changing what readers see.
    pub fn freeze(self) -> Frozen {
        let len = self.used();
        unsafe {
            let h = self.header_mut();
            h.length = len as u64;
            fence(Release);
            std::ptr::write_volatile(&mut h.state, FROZEN);
        }
        if let Some(f) = &self.file {
            f.set_len(len as u64).expect("truncate a journal block");
        } else if self.backing.is_none() {
            unsafe { crate::mem::protect_read(self.base, self.reserved) };
        }
        let mut b = self;
        b.writable = false;
        Frozen(Arc::new(b))
    }

    /// Seals a block whose state is written later, by someone else (a deferrable slab, decision 0295 §1): its length
    /// is written and it takes no more writes, but its state stays open, and nothing is truncated or protected. Its
    /// readers in the process read it as any frozen block.
    pub fn seal(self) -> Frozen {
        let len = self.used();
        unsafe {
            let h = self.header_mut();
            h.length = len as u64;
            fence(Release);
        }
        let mut b = self;
        b.writable = false;
        Frozen(Arc::new(b))
    }

    /// Maps a block's file read-only, valid only if it was frozen whole: magic, kind, state, and the length its
    /// header says is the file's.
    pub fn open_file(path: &Path, kind: u32) -> Result<Frozen, String> {
        let f = File::open(path).map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len() as usize;
        if len < size_of::<Header>() {
            return Err(format!("{}: {len} bytes, shorter than a header", path.display()));
        }
        #[cfg(unix)]
        let b = {
            let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
            if p == libc::MAP_FAILED {
                return Err(io::Error::last_os_error().to_string());
            }
            Block::make(p.cast(), len, len, Some(f), false, None)
        };
        // No mapping: the file read into a reservation of its own.
        #[cfg(not(unix))]
        let b = {
            let mut f = f;
            let p = crate::mem::reserve(len.div_ceil(PAGE) * PAGE);
            std::io::Read::read_exact(&mut f, unsafe { std::slice::from_raw_parts_mut(p, len) }).map_err(|e| e.to_string())?;
            Block::make(p, len.div_ceil(PAGE) * PAGE, len, None, false, None)
        };
        let at = frozen_at(unsafe { std::slice::from_raw_parts(b.base, len) }, kind).map_err(|e| format!("{}: {e}", path.display()))?;
        if at.length != len {
            return Err(format!("{}: header says {} bytes, the file has {len}", path.display(), at.length));
        }
        Ok(Frozen(Arc::new(b)))
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        if self.backing.is_none() {
            unsafe { crate::mem::release(self.base, self.reserved) };
        }
    }
}

/// A frozen block: read-only, shared by any number of readers.
#[derive(Clone)]
pub struct Frozen(Arc<Block>);

impl Deref for Frozen {
    type Target = Block;
    fn deref(&self) -> &Block {
        &self.0
    }
}

/// A run of `T`s in a shared block, read as a slice by the host: an array the host builds once and the device then
/// reads in the same block (decision 0289 §3). The block lives as long as any of its `Cells`.
pub struct Cells<T: crate::Pod> {
    block: Arc<Block>,
    at: usize,
    len: usize,
    _t: std::marker::PhantomData<T>,
}

impl<T: crate::Pod> Cells<T> {
    /// Up to `capacity` `T`s, zero, filled by `fill`, which returns how many it used: the rest is given back to the
    /// block when nothing was allocated after it. `None` when the block cannot hold `capacity`.
    pub fn build(block: &Arc<Block>, capacity: usize, fill: impl FnOnce(&mut [T]) -> usize) -> Option<Self> {
        let size = capacity * size_of::<T>();
        let at = block.try_alloc(size, align_of::<T>().max(8))?;
        // SAFETY: `at..at + size` was just allocated to this call alone, zero (a dirty block zeroes it), and zero
        // is a `T` (`Pod`).
        let cells = unsafe { std::slice::from_raw_parts_mut(block.ptr::<T>(at), capacity) };
        let len = fill(cells);
        assert!(len <= capacity, "cells filled past their capacity");
        block.shrink_last(at, size, len * size_of::<T>());
        Some(Cells { block: Arc::clone(block), at, len, _t: std::marker::PhantomData })
    }

    /// `src` copied into `block`.
    pub fn copied(block: &Arc<Block>, src: &[T]) -> Option<Self> {
        Self::build(block, src.len(), |cells| {
            // SAFETY: `cells` holds `src.len()` `T`s, apart from `src`.
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), cells.as_mut_ptr(), src.len()) };
            src.len()
        })
    }

    /// The block it is in.
    pub fn block(&self) -> &Arc<Block> {
        &self.block
    }

    pub fn as_ptr(&self) -> *const T {
        self.block.ptr(self.at)
    }
}

impl<T: crate::Pod> Deref for Cells<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `build` filled `len` `T`s at `at`, and nothing writes them after it returns.
        unsafe { std::slice::from_raw_parts(self.block.ptr::<T>(self.at), self.len) }
    }
}

impl<T: crate::Pod + std::fmt::Debug> std::fmt::Debug for Cells<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Where a frozen block's root lies and how long it is, as its header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub root: usize,
    pub length: usize,
}

/// The alignment of a framed block's root and length (`frame`): a block that follows another starts aligned.
pub const FRAME_ALIGN: usize = 64;

/// Checks the frozen block of `kind` at the start of `bytes` (a file of one block, or one of several written one
/// after another): its magic and kind, its state frozen, its length within `bytes`, its root (when set) within its
/// length.
pub fn frozen_at(bytes: &[u8], kind: u32) -> Result<At, String> {
    if bytes.len() < size_of::<Header>() {
        return Err(format!("{} bytes, shorter than a header", bytes.len()));
    }
    // SAFETY: at least a header's bytes; every field is a plain integer, read unaligned.
    let h: Header = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<Header>()) };
    if h.magic != MAGIC || h.kind != kind {
        return Err(format!("not a block of kind {kind}"));
    }
    if h.state != FROZEN {
        return Err(format!("not frozen (state {})", h.state));
    }
    let (root, length) = (h.root as usize, h.length as usize);
    // A root of 0 is none set (a block whose root was never set); any other lies past the header.
    if length > bytes.len() || (root != 0 && root < size_of::<Header>()) || root > length {
        return Err(format!("header says {length} bytes from a root at {root}, of {} bytes", bytes.len()));
    }
    Ok(At { root, length })
}

/// The header of a frozen block of `kind` whose root is `root_len` bytes, framed to be written with them: the
/// header's bytes, padded to where the root starts (`FRAME_ALIGN`), and the zeros that pad the root to the block's
/// length. A block made whole elsewhere (a file written by `write`, not mapped) reads as any frozen block.
pub fn frame(kind: u32, root_len: usize) -> (Vec<u8>, Vec<u8>) {
    let root = round_up(size_of::<Header>(), FRAME_ALIGN);
    let length = root + round_up(root_len, FRAME_ALIGN);
    let h = Header {
        magic: MAGIC,
        kind,
        format: 1,
        state: FROZEN,
        delta: 0,
        length: length as u64,
        reserved: length as u64,
        root: root as u64,
        base: 0,
        cost: std::sync::atomic::AtomicU64::new(0),
    };
    let mut head = vec![0u8; root];
    // SAFETY: `head` holds at least a header; every field is a plain integer.
    unsafe { std::ptr::write_unaligned(head.as_mut_ptr().cast::<Header>(), h) };
    (head, vec![0u8; length - root - root_len])
}
