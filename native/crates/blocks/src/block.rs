//! A block: one mapping, reserved large and sparse, bump-allocated from the front, frozen once.
//!
//! The header is at offset 0. Its state and length are written last at freeze. A block read back from a file is
//! valid only when its magic and kind match, its state is frozen, and its length is the file's. No checksum:
//! corruption is the storage's and ECC's to handle.

use crate::structure;
use std::fs::File;
#[cfg(any(unix, windows))]
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
    /// The store's atoms (decision 0401): strings and their hash table.
    pub const ATOMS: u32 = 7;
    /// A space's kept facts (decision 0401 F1).
    pub const KEPT: u32 = 8;
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
    /// Whether it holds no bytes (a block needs at least its header, so a backing of none holds no block).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The device buffer over the whole of it, made once by its owner.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn buffer(&self) -> Option<rerail_device::device::Buffer> {
        None
    }
}

/// The bytes zeroed as blocks' memory was handed out again (a free list's chunk, a dirty block's allocation).
pub fn zeroed_bytes() -> u64 {
    rerail_context::with_local::<Zeroed, _>(|z| z.0.load(Relaxed))
}

/// The run's count (decision 0442: a measurement of the run's).
#[derive(Default)]
struct Zeroed(std::sync::atomic::AtomicU64);

/// Zeroes `len` bytes at `p`, counted (`zeroed_bytes`).
///
/// # Safety
/// `p..p + len` is writable memory of a block.
unsafe fn zero(p: *mut u8, len: usize) {
    unsafe { std::ptr::write_bytes(p, 0, len) };
    rerail_context::with_local::<Zeroed, _>(|z| z.0.fetch_add(len as u64, Relaxed));
}

/// What a block has used, and holds free (its free list's bytes): shared with a ledger that counts live blocks
/// (`meter`), which reads it while the block lives.
#[derive(Default)]
pub struct Extent {
    pub top: AtomicUsize,
    pub free: AtomicUsize,
}

pub struct Block {
    base: *mut u8,
    reserved: usize,
    ext: Arc<Extent>,
    /// A file block's file: an open block's, for the truncation at its freeze, and closed then (Unix): a mapping needs
    /// no descriptor, so a frozen block holds none.
    file: Option<File>,
    /// Its memory is a file's mapping, not a reservation (`mem::reserve`): counted neither way. A mapping read-only
    /// holds no descriptor (`map_file`), so this is not `file.is_some()`.
    mapped: bool,
    writable: bool,
    /// A slab's owner, when the block lives in one: dropped with the block, which never unmaps it.
    backing: Option<Box<dyn Backing>>,
    /// Its memory is not fresh (a recycled slab): every allocation is zeroed as it is made.
    dirty: bool,
    /// It keeps a free list (decision 0342, its lists in its ledger): false for a bump block.
    listed: bool,
    /// It kept a free list until its freeze: its reservation past its length is released then.
    was_free_listed: bool,
    /// The device buffer this block is read through, kept: a buffer's first use maps it into the GPU's address
    /// space at about 20 ms a GiB (0266 §12), so it is made once and remade only when the block outgrows it.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    gpu: std::sync::Mutex<Option<(rerail_device::device::Buffer, usize)>>,
    /// Its ledger (decision 0487): its live chunks, the GPU's queued uses, and when chunks and memory may go.
    ledger: Arc<crate::ledger::Ledger>,
}

unsafe impl Send for Block {}
unsafe impl Sync for Block {}

fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}


/// How a file-backed block's writes reach the disk (decision 0481).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flush {
    /// Each write-back waits until the pages are on the disk (`msync` with `MS_SYNC`, then a barrier on macOS,
    /// `fdatasync` elsewhere) before the store commits what names them: the store survives a power loss.
    Synced,
    /// The default: nothing waits, and the OS writes the pages back at its own pace. The store survives the
    /// process's end (the pages are the OS's), not a power loss or a crash of the OS before they are written.
    #[default]
    Os,
    /// Nothing is flushed, and the store's SQLite syncs nothing either: tests, whose stores are thrown away.
    None,
}

impl Flush {
    /// Whether a write-back waits for the disk.
    pub fn waits(self) -> bool {
        self == Flush::Synced
    }

    /// The mode `RERAIL_FLUSH` names (`synced`, `os`, `none`), else `default`. An unknown value is refused.
    pub fn from_env_or(default: Flush) -> Result<Flush, String> {
        match std::env::var("RERAIL_FLUSH") {
            Err(_) => Ok(default),
            Ok(v) => match v.as_str() {
                "synced" => Ok(Flush::Synced),
                "os" => Ok(Flush::Os),
                "none" => Ok(Flush::None),
                other => Err(format!("RERAIL_FLUSH={other}: not synced, os or none")),
            },
        }
    }
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

    /// A block backed by the file at `path`: sparse, and written back by the OS (no `fsync`). Unix and Windows (a
    /// shared mapping): elsewhere `Unsupported`.
    #[cfg(not(any(unix, windows)))]
    pub fn file(_path: &Path, _kind: u32, _reserved: usize) -> io::Result<Block> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "a block in a file needs a shared mapping"))
    }

    /// A block backed by the file at `path`: sparse, and written back by the OS (no `fsync`).
    #[cfg(any(unix, windows))]
    pub fn file(path: &Path, kind: u32, reserved: usize) -> io::Result<Block> {
        let reserved = round_up(reserved.max(PAGE), PAGE);
        let f = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        // NTFS allocates a file's length unless it is sparse; a Unix file system leaves a hole.
        #[cfg(windows)]
        crate::windows::sparse(&f)?;
        f.set_len(reserved as u64)?;
        let p = map_shared(&f, reserved, true)?;
        let b = Block::make(p, reserved, 0, Some(f), true, None);
        b.init(kind);
        Ok(b)
    }

    fn make(base: *mut u8, reserved: usize, top: usize, file: Option<File>, writable: bool, backing: Option<Box<dyn Backing>>) -> Block {
        let dirty = backing.is_some();
        let ext = Arc::new(Extent { top: AtomicUsize::new(top), free: AtomicUsize::new(0) });
        // Only an anonymous block's pages go to the OS: a slab's are the device buffer's, and a file's are written back.
        let ledger = crate::ledger::Ledger::new(base, reserved, file.is_none() && backing.is_none(), ext.clone());
        Block {
            base,
            reserved,
            ext,
            ledger,
            mapped: file.is_some(),
            file,
            writable,
            backing,
            dirty,
            listed: false,
            was_free_listed: false,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            gpu: std::sync::Mutex::new(None),
        }
    }

    /// This block, keeping a free list (decision 0342): chunks given back (`free_chunk`) are taken again by the
    /// next of their size (`try_alloc_chunk`). Its lists are its ledger's: a free chunk holds nothing of them.
    pub fn with_free_list(mut self) -> Block {
        if !self.listed {
            self.listed = true;
            self.ledger.track(self.used());
        }
        self
    }

    /// Its ledger (decision 0487): what is live in it, and what the GPU has queued on it.
    pub fn ledger(&self) -> &Arc<crate::ledger::Ledger> {
        &self.ledger
    }

    /// Its extent, for a ledger of live blocks: gone once the block is.
    pub fn meter(&self) -> std::sync::Weak<Extent> {
        Arc::downgrade(&self.ext)
    }

    /// Whether it keeps a free list.
    pub fn has_free_list(&self) -> bool {
        self.listed
    }

    /// The bytes its free list holds.
    pub fn free_bytes(&self) -> usize {
        self.ext.free.load(Relaxed)
    }

    /// A chunk of `bytes` at `align`: in a block with a free list, of `bytes`' class (a power of two of at least
    /// 64), taken from the list when one is there (zeroed); else from the front.
    pub fn try_alloc_chunk(&self, bytes: usize, align: usize) -> Option<usize> {
        self.chunk(bytes, align, true)
    }

    fn chunk(&self, bytes: usize, align: usize, zeroed: bool) -> Option<usize> {
        if !self.listed {
            return self.alloc_filled(bytes, align, zeroed);
        }
        let (class, size) = crate::free::class_of(bytes);
        // The list's chunk, or a larger one split down; the pages the ledger gave back are marked in use as it hands
        // the chunk out.
        if let Some(at) = self.ledger.pop(class) {
            if zeroed {
                unsafe { zero(self.base.add(at), size) };
            }
            return Some(at);
        }
        // From the front, aligned to its size up to `ALIGN_MAX` (decision 0342 as amended by 0494 step 6): the bytes
        // skipped to align it onto the list as pieces, each aligned to its own size.
        let (at, from) = self.bump(size, align.max(size.min(crate::free::ALIGN_MAX)))?;
        if at > from.next_multiple_of(64) {
            self.free_pieces(from.next_multiple_of(64), at - from.next_multiple_of(64));
        }
        if self.dirty && zeroed {
            unsafe { zero(self.base.add(at), size) };
        }
        Some(at)
    }

    /// `try_alloc_chunk` for a chunk written before it is read (a relation's rows): a chunk taken again is left as
    /// it was, and a dirty block's is not zeroed.
    pub fn try_alloc_chunk_any(&self, bytes: usize, align: usize) -> Option<usize> {
        self.chunk(bytes, align, false)
    }

    /// Gives back a chunk `try_alloc_chunk` gave (`bytes` as asked): to the free list, when the block keeps one;
    /// else nothing. Nothing may read it after. In an anonymous block the pages it leaves with no live byte go back to
    /// the OS too (Q76: a dead relation's pages otherwise stayed in the footprint until its chunks were taken again),
    /// and are marked in use again when the list hands a chunk in them out. Returns the bytes given to the OS.
    pub fn free_chunk(&self, at: usize, bytes: usize) -> usize {
        self.release_chunk(at, crate::free::class_of(bytes).1, crate::ledger::Freeing { pages: true })
    }

    /// A live chunk of `len` bytes at `at` freed through the ledger (decision 0487): at once, or when the GPU work
    /// queued on the block completes. The bytes given to the OS now.
    fn release_chunk(&self, at: usize, len: usize, how: crate::ledger::Freeing) -> usize {
        if !self.listed || !self.writable {
            return 0;
        }
        self.ledger.free(at, len, how)
    }

    /// A chunk of `bytes`' class at `at`, never live (the rest of a fitted chunk), onto the free list.
    fn push_free(&self, at: usize, bytes: usize) {
        if !self.listed || !self.writable {
            return;
        }
        self.ledger.push_never_live(at, bytes);
    }

    /// A chunk of at least `bytes` at `align` that keeps little more (`try_alloc_chunk_any`'s, not zeroed): in a block
    /// with a free list, `bytes` rounded up to an eighth of its class, the class's rest handed back to the list as
    /// smaller chunks at once (a class is up to twice what was asked). Its offset and the bytes kept, which
    /// `free_chunk_fit` takes back.
    pub fn try_alloc_chunk_fit(&self, bytes: usize, align: usize) -> Option<(usize, usize)> {
        let at = self.chunk(bytes, align, false)?;
        if !self.listed {
            return Some((at, bytes));
        }
        let (_, size) = crate::free::class_of(bytes);
        let keep = self.fit_kept(bytes);
        self.ledger.untaken_tail(at, size, keep);
        // Never written: the pages they leave with no live byte go to the OS (a chunk taken again from the list may
        // have had them resident).
        self.free_pieces(at + keep, size - keep);
        Some((at, keep))
    }

    /// The bytes `try_alloc_chunk_fit` keeps of a chunk of `bytes`: what `free_chunk_fit` takes back, found again from
    /// the bytes asked by a holder that keeps no more (an index's spans).
    pub fn fit_kept(&self, bytes: usize) -> usize {
        if !self.listed {
            return bytes;
        }
        let (_, size) = crate::free::class_of(bytes);
        let step = (size / 8).max(1 << crate::free::MIN_LOG);
        bytes.next_multiple_of(step).min(size)
    }

    /// `try_alloc_chunk_fit`, zeroed: what `try_alloc` gives, in a chunk the free list takes back.
    pub fn try_alloc_chunk_fit_zeroed(&self, bytes: usize, align: usize) -> Option<(usize, usize)> {
        let (at, kept) = self.try_alloc_chunk_fit(bytes, align)?;
        // SAFETY: the chunk is the caller's, `kept` bytes at `at` in the block.
        unsafe { zero(self.base.add(at), bytes) };
        Some((at, kept))
    }

    /// `free_chunk`, its pages left resident: for a chunk the next of its size takes soon (an index built again), or
    /// whose bytes are few beside its relation's (an index's 4 a row), where giving its pages to the OS costs calls
    /// and buys little.
    pub fn free_chunk_resident(&self, at: usize, bytes: usize) {
        self.release_chunk(at, crate::free::class_of(bytes).1, crate::ledger::Freeing { pages: false });
    }

    /// `free_chunk_fit`, its pages left resident (`free_chunk_resident`).
    pub fn free_chunk_fit_resident(&self, at: usize, kept: usize) {
        self.release_chunk(at, kept, crate::ledger::Freeing { pages: false });
    }

    /// Gives back a chunk `try_alloc_chunk_fit` gave (`kept` bytes at `at`): its pieces, each a class's chunk.
    pub fn free_chunk_fit(&self, at: usize, kept: usize) {
        self.release_chunk(at, kept, crate::ledger::Freeing { pages: true });
    }

    /// `bytes` at `at` (a multiple of the least class), never live, to the free list as chunks of classes, each the
    /// largest that fits and is aligned to its size.
    fn free_pieces(&self, mut at: usize, mut bytes: usize) {
        while bytes > 0 {
            // Each piece aligned to its size, so that it merges with its buddy.
            let piece = crate::free::piece(at, bytes);
            self.push_free(at, piece);
            at += piece;
            bytes -= piece;
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
        assert!(reserved >= PAGE && (base as usize).is_multiple_of(64), "a backing of at least a page, 64-byte aligned");
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
        assert!(self.ledger.quiet(), "block ledger: a block moved with GPU work queued on it (decision 0487 R7)");
        let used = self.used();
        assert!(backing.len() >= used, "a backing of {} bytes for a block of {used}", backing.len());
        let base = backing.base();
        unsafe { std::ptr::copy_nonoverlapping(self.base, base, used) };
        let b = Block::make(base, backing.len(), used, None, true, Some(backing));
        unsafe { b.header_mut().reserved = b.reserved as u64 };
        b
    }

    /// Lets go of the no-copy wrap `device_buffer` kept; the device must be done with it. It does not let the OS write
    /// back a file block's pages a kernel wrote: the driver holds them wired until the range is unmapped (decision
    /// 0295 §1 and §7, measured by `examples/deferrable`).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn release_device_buffer(&self) {
        self.gpu.lock().unwrap().take();
    }

    /// The wrap `device_buffer` keeps as it is now, and its length (none made yet, or a slab's block): what a program
    /// bound before the block grew, for a remap to find again (decision 0603).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn kept_device_buffer(&self) -> Option<(rerail_device::device::Buffer, usize)> {
        if self.backing.as_ref().is_some_and(|b| b.buffer().is_some()) {
            return None;
        }
        self.gpu.lock().unwrap().clone()
    }

    /// The device buffer to bind this block with: its slab's own, or a wrap of its used extent in place
    /// (`rerail_device::device::wrap_host_memory`: no copy, decision 0353) made once and kept (remade, twice as long,
    /// only when the block has outgrown it).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn device_buffer(&self, device: &rerail_device::device::Device) -> Result<rerail_device::device::Buffer, String> {
        if let Some(b) = self.backing.as_ref().and_then(|b| b.buffer()) {
            return Ok(b);
        }
        let mut kept = self.gpu.lock().unwrap();
        let used = self.used();
        if let Some((b, len)) = kept.as_ref()
            && *len >= used
        {
            return Ok(b.clone());
        }
        let len = kept.as_ref().map_or(1 << 20, |k| k.1 * 2).max(used.div_ceil(1 << 20) << 20).min(self.reserved);
        // A measurement aid: `RERAIL_DEVICE_TRACE` prints where a block of 4 MB or more is wrapped for the device.
        if len >= 4 << 20 && std::env::var_os("RERAIL_DEVICE_TRACE").is_some() {
            eprintln!("RERAIL_DEVICE_TRACE a block wrapped, {} MB:\n{}", len >> 20, std::backtrace::Backtrace::force_capture());
        }
        // Its pages committed first, where a reservation's are committed as they are touched (Windows: a driver pins them).
        crate::mem::commit(self.base, len).map_err(|e| format!("committing a block's {len} bytes for the device: {e}"))?;
        // SAFETY: the block's pages outlive the wrap: it is kept by the block, and let go before they are unmapped.
        let b = unsafe { rerail_device::device::wrap_host_memory(device, self.base, len)? };
        *kept = Some((b.clone(), len));
        Ok(b)
    }

    fn init(&self, kind: u32) {
        self.ext.top.store(round_up(size_of::<Header>(), 64), Relaxed);
        let h = unsafe { &mut *(self.base as *mut Header) };
        h.magic = MAGIC;
        h.kind = kind;
        h.format = 1;
        h.state = OPEN;
        h.reserved = self.reserved as u64;
    }

    /// Whether it lies in a slab of a pool's (`rerail_accel::gpu::shared::lease_block`), not memory of its own.
    pub fn in_slab(&self) -> bool {
        self.backing.is_some()
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
        self.alloc_filled(size, align, true)
    }

    /// `try_alloc`; a dirty block's memory is zeroed only when `zeroed` asks it.
    fn alloc_filled(&self, size: usize, align: usize, zeroed: bool) -> Option<usize> {
        let (start, _) = self.bump(size, align)?;
        if self.dirty && zeroed {
            unsafe { zero(self.base.add(start), size) };
        }
        Some(start)
    }

    /// `size` bytes at `align` from the front, handed out (`taken_front`): where, and the front before them.
    fn bump(&self, size: usize, align: usize) -> Option<(usize, usize)> {
        // A read-only block (a frozen file's mapping, `share`) has no room: an index a reader would add goes elsewhere.
        if !self.writable {
            return None;
        }
        let mut cur = self.ext.top.load(Relaxed);
        loop {
            let start = round_up(cur, align);
            let end = start + size;
            if end > self.reserved {
                return None;
            }
            match self.ext.top.compare_exchange_weak(cur, end, AcqRel, Relaxed) {
                Ok(_) => {
                    if self.listed {
                        self.ledger.taken_front(start, size);
                    }
                    return Some((start, cur));
                }
                Err(c) => cur = c,
            }
        }
    }

    /// Gives back the tail of the last allocation (`size` bytes at `at`), keeping its first `keep` bytes. When
    /// another allocation came after it, nothing is given back. Whether it was. The tail must be unwritten: an
    /// allocation of a block that is not dirty is zero, as the mapping gave it.
    pub fn shrink_last(&self, at: usize, size: usize, keep: usize) -> bool {
        assert!(keep <= size);
        let shrunk = self.ext.top.compare_exchange(at + size, at + keep, AcqRel, Relaxed).is_ok();
        if shrunk && self.listed {
            self.ledger.untaken_tail(at, size, keep);
        }
        shrunk
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

    /// Whether `p` lies in this block's reserved range.
    pub fn holds<T>(&self, p: *const T) -> bool {
        (p as usize).wrapping_sub(self.base as usize) < self.reserved
    }

    pub fn offset_of<T>(&self, p: *const T) -> usize {
        let o = p as usize - self.base as usize;
        assert!(o < self.reserved);
        o
    }

    pub fn base_ptr(&self) -> *mut u8 {
        self.base
    }

    /// Whether it may be written: an open block's, not a frozen file's mapping or a shared handle (`share`).
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    pub fn reserved(&self) -> usize {
        self.reserved
    }

    /// Bytes used so far (the frozen length, once frozen).
    #[inline]
    pub fn used(&self) -> usize {
        self.ext.top.load(Acquire)
    }

    pub fn set_root<T>(&self, p: *const T) {
        unsafe { self.header_mut().root = self.offset_of(p) as u64 };
    }

    pub fn root<T>(&self) -> &T {
        unsafe { &*(self.base.add(self.header().root as usize) as *const T) }
    }

    #[inline]
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
    pub fn freeze(mut self) -> Frozen {
        self.drop_free_list();
        let len = self.used();
        unsafe {
            let h = self.header_mut();
            h.length = len as u64;
            fence(Release);
            std::ptr::write_volatile(&mut h.state, FROZEN);
        }
        if let Some(f) = &self.file {
            cut(f, len as u64).expect("truncate a journal block");
        } else if self.backing.is_none() {
            // A free-listed block's reservation past its length goes back: nothing is written there now.
            // Whole MBs, and never under a device buffer made over it (`device_buffer`'s extent).
            #[allow(unused_mut)]
            let mut keep = round_up(len.max(1 << 20), 1 << 20);
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            if let Some((_, l)) = self.gpu.lock().unwrap().as_ref() {
                keep = keep.max(*l);
            }
            if self.was_free_listed && keep < self.reserved {
                unsafe { crate::mem::release(self.base.add(keep), self.reserved - keep) };
                self.reserved = keep;
                self.ledger.resized(keep);
            }
            unsafe { crate::mem::protect_read(self.base, self.reserved) };
        }
        let mut b = self;
        // Unix: nothing is left to truncate, and the mapping outlives the file's descriptor: it is closed.
        #[cfg(unix)]
        {
            b.file = None;
        }
        b.writable = false;
        Frozen(Arc::new(b))
    }

    /// Drops the free list (a freeze's): the pages its chunks leave with no live byte go to the OS, and none is taken
    /// again.
    fn drop_free_list(&mut self) {
        if !self.listed {
            return;
        }
        // The ledger gives back the pages its free chunks leave with no live byte (R6), an anonymous block's.
        self.ledger.untrack_list();
        self.listed = false;
        self.ext.free.store(0, Relaxed);
        self.was_free_listed = true;
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

    /// Maps the block in the file at `path` as it is, open or frozen (a store's block that one process appends to and
    /// others read up to an end they are told, decision 0401): its magic and kind checked, its whole file mapped,
    /// writable or not. Read-only, it keeps no descriptor: the mapping outlives it, and a store's reader holds none of
    /// its files open. Without shared mappings (WASI) the file is read into memory, and a writer writes it back itself
    /// (`bytes`). On Windows a view's length is the file's, which a frozen block's may pass (`cut`).
    pub fn map_file(path: &Path, kind: u32, writable: bool) -> io::Result<Block> {
        let invalid = |what: String| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display()));
        let f = std::fs::OpenOptions::new().read(true).write(writable).open(path)?;
        let len = f.metadata()?.len() as usize;
        if len < size_of::<Header>() {
            return Err(invalid(format!("{len} bytes, shorter than a header")));
        }
        #[cfg(any(unix, windows))]
        let b = {
            let p = map_shared(&f, len, writable)?;
            let mut b = Block::make(p, len, len, Some(f), writable, None);
            if !writable {
                b.file = None;
            }
            b
        };
        #[cfg(not(any(unix, windows)))]
        let b = {
            let mut f = f;
            let reserved = len.div_ceil(PAGE) * PAGE;
            let p = crate::mem::reserve(reserved);
            // The kernel reads and writes a reservation's pages without faulting them in (Windows: `crate::windows`).
            crate::mem::commit(p, len)?;
            std::io::Read::read_exact(&mut f, unsafe { std::slice::from_raw_parts_mut(p, len) })?;
            Block::make(p, reserved, len, None, writable, None)
        };
        let h = b.header();
        if h.magic != MAGIC || h.kind != kind {
            return Err(invalid(format!("not a block of kind {kind}")));
        }
        if h.state != OPEN && h.state != FROZEN {
            return Err(invalid(format!("state {}", h.state)));
        }
        Ok(b)
    }

    /// What is written in a block mapped from its file on the disk, with `Flush::Synced`: its dirty pages written
    /// back, then the file flushed (a barrier on macOS, `fdatasync` elsewhere). Otherwise nothing: the OS writes the
    /// pages back at its own pace (decision 0481). Nothing to do for a block with no file.
    #[cfg(any(unix, windows))]
    pub fn sync_mapped(&self, flush: Flush) -> io::Result<()> {
        if !flush.waits() {
            return Ok(());
        }
        // The pages written: nothing past `used` is (a kept file's reservation is hundreds of MB, its facts a few;
        // asked of the whole, the kernel walked every page of it, Q361).
        let written = self.used().next_multiple_of(PAGE).min(self.reserved);
        // Windows: the view's dirty pages written, then the file's buffers flushed (`FlushViewOfFile` waits for no
        // disk).
        #[cfg(windows)]
        {
            crate::windows::flush_view(self.base, written)?;
            return match &self.file {
                Some(f) => f.sync_data(),
                None => Ok(()),
            };
        }
        #[cfg(unix)]
        {
            // SAFETY: the block's own mapping, its first `written` bytes.
            if unsafe { libc::msync(self.base.cast(), written, libc::MS_SYNC) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let Some(f) = &self.file else { return Ok(()) };
            #[cfg(target_os = "macos")]
            let done = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_BARRIERFSYNC) };
            #[cfg(not(target_os = "macos"))]
            let done = unsafe { libc::fdatasync(f.as_raw_fd()) };
            if done != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    /// The file a block maps, when it maps one.
    pub fn mapped_file(&self) -> Option<&File> {
        self.file.as_ref()
    }

    /// Maps a block's file read-only, valid only if it was frozen whole: magic, kind, state, and the length its
    /// header says is the file's.
    pub fn open_file(path: &Path, kind: u32) -> Result<Frozen, String> {
        let f = File::open(path).map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len() as usize;
        if len < size_of::<Header>() {
            return Err(format!("{}: {len} bytes, shorter than a header", path.display()));
        }
        let b = Block::map_read(f, len)?;
        b.check_frozen(len, kind).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Frozen(Arc::new(b)))
    }

    /// The whole of `f`, `len` bytes, mapped read-only. The mapping keeps the pages, so the file is closed (`f`,
    /// dropped as this returns).
    #[cfg(any(unix, windows))]
    fn map_read(f: File, len: usize) -> Result<Block, String> {
        let p = map_shared(&f, len, false).map_err(|e| e.to_string())?;
        // A mapping (counted as one by `make`), its file let go.
        let mut b = Block::make(p, len, len, Some(f), false, None);
        b.file = None;
        Ok(b)
    }

    /// No mapping (WASI): the whole of `f`, `len` bytes, read into a reservation of its own.
    #[cfg(not(any(unix, windows)))]
    fn map_read(mut f: File, len: usize) -> Result<Block, String> {
        let p = crate::mem::reserve(len.div_ceil(PAGE) * PAGE);
        // The kernel reads and writes a reservation's pages without faulting them in (Windows: `crate::windows`).
        crate::mem::commit(p, len).map_err(|e| e.to_string())?;
        std::io::Read::read_exact(&mut f, unsafe { std::slice::from_raw_parts_mut(p, len) }).map_err(|e| e.to_string())?;
        Ok(Block::make(p, len.div_ceil(PAGE) * PAGE, len, None, false, None))
    }

    /// Whether the block, its file's `len` bytes, was frozen whole as a block of `kind`: magic, kind, state, and the
    /// length its header says is the file's.
    fn check_frozen(&self, len: usize, kind: u32) -> Result<(), String> {
        let at = frozen_at(unsafe { std::slice::from_raw_parts(self.base, len) }, kind)?;
        // Windows cuts no file a view maps (`cut`; `deferrable`'s module doc): a frozen block's file can be longer than
        // the block it holds, its header saying the block's length.
        #[cfg(windows)]
        let len = if at.length <= len { at.length } else { len };
        if at.length != len {
            return Err(format!("header says {} bytes, the file has {len}", at.length));
        }
        Ok(())
    }
}

impl Drop for Block {
    /// Its memory goes through its ledger: at once, or when the GPU work queued on it completes (R5).
    fn drop(&mut self) {
        let backing = self.backing.take();
        let (base, reserved, mapped) = (self.base as usize, self.reserved, self.mapped);
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        let wrap = self.gpu.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        self.ledger.let_go(Box::new(move || {
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            drop(wrap);
            match backing {
                Some(slab) => drop(slab),
                None => {
                    // A file's mapping was never `reserve`'s: it is unmapped without counting.
                    if mapped {
                        crate::mem::uncounted(reserved);
                        unsafe { crate::mem::release_mapping(base as *mut u8, reserved) };
                    } else {
                        unsafe { crate::mem::release(base as *mut u8, reserved) };
                    }
                }
            }
        }));
    }
}

/// `len` bytes of `f` mapped shared: read and written (`writable`), or read.
#[cfg(any(unix, windows))]
fn map_shared(f: &File, len: usize, writable: bool) -> io::Result<*mut u8> {
    #[cfg(unix)]
    {
        let prot = if writable { libc::PROT_READ | libc::PROT_WRITE } else { libc::PROT_READ };
        // SAFETY: the file's own descriptor, open for the call; the system picks the address.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, prot, libc::MAP_SHARED, f.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED { Err(io::Error::last_os_error()) } else { Ok(p.cast()) }
    }
    #[cfg(windows)]
    crate::windows::map_file(f, len, writable)
}

/// `f` cut to `len` bytes; on Windows, where a view maps no file it may cut, only once no view maps it
/// (`windows::cut`).
pub(crate) fn cut(f: &File, len: u64) -> io::Result<()> {
    #[cfg(windows)]
    return crate::windows::cut(f, len);
    #[cfg(not(windows))]
    f.set_len(len)
}

/// A frozen block: read-only, shared by any number of readers.
#[derive(Clone)]
pub struct Frozen(Arc<Block>);

impl Deref for Frozen {
    type Target = Block;
    #[inline]
    fn deref(&self) -> &Block {
        &self.0
    }
}

impl Frozen {
    /// Its block, by handle: what a region of it holds (`region::whole`).
    pub fn arc(&self) -> &Arc<Block> {
        &self.0
    }
}

/// A run of `T`s in a shared block, read as a slice by the host: an array the host builds once and the device then
/// reads in the same block (decision 0289 §3). A frozen region (decision 0491): its block lives, and its chunk stays
/// taken, as long as any of its `Cells`, or a GPU use of it, does.
pub struct Cells<T: crate::Pod>(crate::region::Frozen<T>);

impl<T: crate::Pod> Cells<T> {
    /// Up to `capacity` `T`s, zero, filled by `fill`, which returns how many it used: the rest is given back to the
    /// block when nothing was allocated after it (`region::Mutable::truncate`). `None` when the block cannot hold
    /// `capacity`.
    pub fn build(block: &Arc<Block>, capacity: usize, fill: impl FnOnce(&mut [T]) -> usize) -> Option<Self> {
        let mut m = crate::region::Mutable::<T>::alloc(block, capacity, "cells")?;
        let len = fill(&mut m.write());
        assert!(len <= capacity, "cells filled past their capacity");
        m.truncate(len);
        Some(Cells(m.freeze()))
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
        self.0.region().block()
    }

    /// Its region, as the device binds it.
    pub fn frozen(&self) -> &crate::region::Frozen<T> {
        &self.0
    }

    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }
}

impl<T: crate::Pod> Deref for Cells<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The descriptors this process has open.
    fn open_descriptors() -> usize {
        std::fs::read_dir("/dev/fd").expect("/dev/fd").count()
    }

    /// Frozen file blocks hold no descriptor: a store keeps many segments mapped at once (its caches hold them), and
    /// a descriptor each reached the default limit of 1,024 (`unable to open database file` from SQLite after).
    #[test]
    fn frozen_file_blocks_hold_no_descriptor() {
        const N: usize = 256;
        let dir = std::env::temp_dir().join(format!("rerail-block-descriptors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let before = open_descriptors();
        let mut held = Vec::new();
        for i in 0..N {
            let path = dir.join(format!("{i}.block"));
            let block = Block::file(&path, kind::JOURNAL, PAGE).unwrap();
            block.alloc(64, 64);
            held.push(block.freeze());
            held.push(Block::open_file(&path, kind::JOURNAL).unwrap());
        }
        let grown = open_descriptors().saturating_sub(before);
        assert_eq!(held.iter().map(|b| b.used()).min(), held.iter().map(|b| b.used()).max(), "every block read back whole");
        drop(held);
        std::fs::remove_dir_all(&dir).unwrap();
        // Other tests of this binary open files meanwhile: a bound well under the blocks' count, not an exact one.
        assert!(grown < N / 4, "{} frozen file blocks held {grown} more descriptors open", 2 * N);
    }
}
