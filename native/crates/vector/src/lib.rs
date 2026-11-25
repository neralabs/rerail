//! Plain values, the rooms structures take their regions from, and the growable array over them (decision 0628 §3).
//!
//! Below the tape format, so that its writer's columns, like every crate's arrays, lie in the blocks the run's room
//! gives (`rerail_blocks` implements `Room` over its blocks; the facts layer installs the run's, `rows::run_room`, the
//! run's open relation block, with `install_default_room`). A crate's own tests, run without it, take their regions
//! from the heap.

pub mod vector;

use std::any::Any;
use std::sync::Arc;

/// A type whose bytes are its value: no pointers, no padding the compiler chose (`rerail_blocks::structure!` checks).
///
/// # Safety
/// Implementors are `#[repr(C)]` with every byte a named field, and hold no address.
pub unsafe trait Pod: 'static {}

macro_rules! pod {
    ($($t:ty),*) => {$(
        unsafe impl Pod for $t {}
    )*};
}
pod!(u8, u16, u32, u64, u128, i16, i32, i64, i128, f32, f64, usize, ());
// A field that changes while its block is open (a counter, a chunk's offset, a span set at freeze) is an atomic:
// written through a shared reference, which Rust allows only by interior mutability.
pod!(std::sync::atomic::AtomicU32, std::sync::atomic::AtomicU64, std::sync::atomic::AtomicI64);
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}

/// What keeps a region's block, beside the region.
pub type Keep = Option<Arc<dyn Any + Send + Sync>>;

/// A region a room gave: its first byte, its bytes, and what holds it (a block's region, given back when this goes).
/// A region a device may use carries its block's guard (the check of each CPU read and write against the device's
/// queued uses, decision 0487 R4); one another holder owns, the way to borrow it again (`share`).
pub struct Region {
    ptr: *mut u8,
    len: usize,
    owner: Box<dyn Any + Send + Sync>,
    guard: Option<(Arc<dyn Guard>, usize)>,
    /// The guard's count of the device's queued uses of its block (`Guard::uses`), read where it lies: the guard's
    /// check is called only while one is queued. It lives as long as the guard, which the region holds.
    uses: *const std::sync::atomic::AtomicUsize,
    share: Option<fn(&Region) -> Region>,
}

/// The check of the CPU's reads and writes of a block's bytes against the device's queued uses of them (decision 0487
/// R4: `rerail_blocks::ledger::Ledger`): `at` and `len` in the block's bytes.
pub trait Guard: Send + Sync {
    fn cpu_read(&self, at: usize, len: usize);
    fn cpu_write(&self, at: usize, len: usize);
    /// How many of the device's uses of the block are queued: none, and every CPU read and write passes (the check's
    /// first test, made by the region where it reads, decision 0684). Held by the guard for as long as it lives.
    fn uses(&self) -> &std::sync::atomic::AtomicUsize;
}

/// No guard's count of uses: none queued.
static NO_USES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// SAFETY: the region's bytes are its holder's alone, written through `&mut self` or as atomics.
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

impl Region {
    /// The region of `len` bytes at `ptr`, held by `owner` for as long as this lives.
    ///
    /// # Safety
    /// `ptr..ptr + len` is writable, aligned as asked, and this region's alone while `owner` lives.
    pub unsafe fn new(ptr: *mut u8, len: usize, owner: Box<dyn Any + Send + Sync>) -> Region {
        Region { ptr, len, owner, guard: None, uses: &NO_USES, share: None }
    }

    /// It, each CPU read and write checked by `guard`, its first byte at `at` of the guard's block.
    pub fn guarded(mut self, guard: Arc<dyn Guard>, at: usize) -> Region {
        self.uses = guard.uses();
        self.guard = Some((guard, at));
        self
    }

    /// It, borrowed again by `share` (a borrowed vector's clone: another borrow of the same bytes).
    pub fn shared_by(mut self, share: fn(&Region) -> Region) -> Region {
        self.share = Some(share);
        self
    }

    /// Whether any of the device's uses of its guard's block is queued: only then does a CPU read or write call the
    /// guard (decision 0487 R4). Relaxed, as the guard's own first test (`Ledger::cpu`).
    #[inline]
    pub fn device_uses(&self) -> bool {
        // SAFETY: `uses` is `NO_USES` or the count of the guard this region holds.
        unsafe { (*self.uses).load(std::sync::atomic::Ordering::Relaxed) != 0 }
    }

    /// Its guard and its first byte's offset in the guard's block.
    #[inline]
    pub fn guard(&self) -> Option<&(Arc<dyn Guard>, usize)> {
        self.guard.as_ref()
    }

    /// Another borrow of its bytes, where it is borrowed (`shared_by`).
    pub fn share(&self) -> Option<Region> {
        self.share.map(|f| f(self))
    }

    /// What holds it, taken: a room's region handed on whole (its chunk to a relation's directory).
    pub fn into_owner(self) -> Box<dyn Any + Send + Sync> {
        self.owner
    }
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// What holds it: a block's region, for its room to read back where it lies.
    pub fn owner(&self) -> &(dyn Any + Send + Sync) {
        &*self.owner
    }
}

/// Where a structure lives: a region of `bytes` bytes in a block of the right life, its first byte at a multiple of
/// `align` (at least 8).
pub trait Room: Send + Sync {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> Region;
}

/// The heap's regions: a crate's own tests', run without the blocks' room.
pub struct HeapRoom;

struct HeapBytes {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}
// SAFETY: the bytes are freed once, by their one holder.
unsafe impl Send for HeapBytes {}
unsafe impl Sync for HeapBytes {}
impl Drop for HeapBytes {
    fn drop(&mut self) {
        // SAFETY: allocated with this layout (`HeapRoom::region`).
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

impl Room for HeapRoom {
    fn region(&self, bytes: usize, align: usize, _name: &'static str) -> Region {
        let layout = std::alloc::Layout::from_size_align(bytes.max(1), align.max(8)).expect("a region's layout");
        // SAFETY: a layout of at least one byte.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        // SAFETY: the bytes just allocated, held by the owner until it goes.
        unsafe { Region::new(ptr, bytes, Box::new(HeapBytes { ptr, layout })) }
    }
}

/// The run's room, as the facts layer gives it (`rows::run_room`, `install_default_room`): registered once, at load.
static DEFAULT_ROOM: rerail_registry::Slot<fn() -> Arc<dyn Room>> = rerail_registry::Slot::new("the run's room every structure made with no room takes (rerail_vector::DEFAULT_ROOM)");

/// Installs the run's room: what a structure made with no room (`Vector::new`) takes its regions from. Once, at load
/// (a second registration is a bug in the program's composition: `rerail_registry::Slot`).
pub fn install_default_room(provider: &'static str, room: fn() -> Arc<dyn Room>) {
    DEFAULT_ROOM.register(provider, room);
}

/// The room of the run under way (the facts layer's, once installed), else the heap's.
pub fn default_room() -> Arc<dyn Room> {
    match DEFAULT_ROOM.try_get() {
        Some(room) => room(),
        None => Arc::new(HeapRoom),
    }
}
