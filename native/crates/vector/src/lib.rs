//! Plain values, the rooms structures take their regions from, and the growable array over them (decision 0628 §3).
//!
//! Below the tape format, so that its writer's columns, like every crate's arrays, lie in the blocks the run's room
//! gives (`rerail_blocks` implements `Room` over its blocks and installs the run's, `install_default_room`). A crate's
//! own tests, run without it, take their regions from the heap.

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
pub struct Region {
    ptr: *mut u8,
    len: usize,
    owner: Box<dyn Any + Send + Sync>,
}

// SAFETY: the region's bytes are its holder's alone, written through `&mut self` or as atomics.
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

impl Region {
    /// The region of `len` bytes at `ptr`, held by `owner` for as long as this lives.
    ///
    /// # Safety
    /// `ptr..ptr + len` is writable, aligned as asked, and this region's alone while `owner` lives.
    pub unsafe fn new(ptr: *mut u8, len: usize, owner: Box<dyn Any + Send + Sync>) -> Region {
        Region { ptr, len, owner }
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

/// The run's room, as the blocks crate gives it (`install_default_room`): registered once, at load.
static DEFAULT_ROOM: std::sync::OnceLock<fn() -> Arc<dyn Room>> = std::sync::OnceLock::new();

/// Installs the run's room: what a structure made with no room (`Vector::new`) takes its regions from.
pub fn install_default_room(room: fn() -> Arc<dyn Room>) {
    let _ = DEFAULT_ROOM.set(room);
}

/// The room of the run under way (the blocks', once installed), else the heap's.
pub fn default_room() -> Arc<dyn Room> {
    match DEFAULT_ROOM.get() {
        Some(room) => room(),
        None => Arc::new(HeapRoom),
    }
}
