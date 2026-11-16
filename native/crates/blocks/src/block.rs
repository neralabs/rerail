//! A block: one mapping, reserved large and sparse, bump-allocated from the front, frozen once.
//!
//! The header is at offset 0. Its state and length are written last at freeze. A block read back from a file is
//! valid only when its magic and kind match, its state is frozen, and its length is the file's. No checksum:
//! corruption is the storage's and ECC's to handle.

use crate::structure;
use std::fs::{File, OpenOptions};
use std::io;
use std::ops::Deref;
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
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), reserved, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE, -1, 0)
        };
        assert!(p != libc::MAP_FAILED, "mmap of {reserved} bytes failed");
        let b = Block::make(p.cast(), reserved, 0, None, true, None);
        b.init(kind);
        b
    }

    /// A block backed by the file at `path`: sparse, and written back by the OS (no `fsync`).
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
        let (base, reserved) = (backing.base(), backing.len());
        assert!(reserved >= PAGE && base as usize % 64 == 0, "a backing of at least a page, 64-byte aligned");
        unsafe { std::ptr::write_bytes(base, 0, size_of::<Header>()) };
        let b = Block::make(base, reserved, 0, None, true, Some(backing));
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
        assert!(self.writable, "alloc in a frozen block");
        let mut cur = self.top.load(Relaxed);
        loop {
            let start = round_up(cur, align);
            let end = start + size;
            if end > self.reserved {
                return None;
            }
            match self.top.compare_exchange_weak(cur, end, AcqRel, Relaxed) {
                Ok(_) => {
                    if self.dirty {
                        unsafe { std::ptr::write_bytes(self.base.add(start), 0, size) };
                    }
                    return Some(start);
                }
                Err(c) => cur = c,
            }
        }
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
            let r = unsafe { libc::mprotect(self.base.cast(), self.reserved, libc::PROT_READ) };
            assert_eq!(r, 0, "mprotect");
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
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error().to_string());
        }
        let b = Block::make(p.cast(), len, len, Some(f), false, None);
        let h = b.header();
        if h.magic != MAGIC || h.kind != kind {
            return Err(format!("{}: not a block of kind {kind}", path.display()));
        }
        if h.state != FROZEN {
            return Err(format!("{}: not frozen (state {})", path.display(), h.state));
        }
        if h.length as usize != len {
            return Err(format!("{}: header says {} bytes, the file has {len}", path.display(), h.length));
        }
        Ok(Frozen(Arc::new(b)))
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        if self.backing.is_none() {
            unsafe { libc::munmap(self.base.cast(), self.reserved) };
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
