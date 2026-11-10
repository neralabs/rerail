//! memory_region.h, in Rust.
//!
//! Jeremy Barnes, 27 March 2016. This file is part of MLDB. Copyright 2016 mldb.ai inc. Apache-2.0 (see
//! `../LICENSE`).
//!
//! Basic primitives around memory regions. Once frozen, these are the representation that covers CPU memory, device
//! memory and remote memory and implements the primitives that allow data to be made available and brought to the
//! compute resources required.
//!
//! In Rust a region is an aligned allocation shared by `Arc`, with an offset and a length. A frozen region is
//! immutable and can be wrapped by a Metal buffer without a copy when page-aligned (the gpu lane's rule). The parts
//! of the header that need MLDB facilities not brought over are not carried: `mapFile` (MLDB's `Url` and VFS),
//! `getStream` and the structured serializer and reconstituter (MLDB's `filter_ostream`, `Path` and
//! `ValueDescription`), and the `ValueDescription` of handles; the forest's training and prediction call none of
//! them. `memory_region.cc` (not copied) held the out-of-line bodies written here.

use std::alloc::{Layout, alloc, alloc_zeroed, dealloc};
use std::any::TypeId;
use std::sync::{Arc, Weak};

/// A plain value a region can be viewed as: any bit pattern is a value, no padding read as data, no pointers.
///
/// # Safety
/// Implementors must be `Copy`, without padding that is read, and valid for every bit pattern.
pub unsafe trait Pod: Copy + 'static {}
macro_rules! pod {
    ($($t:ty),*) => {$( unsafe impl Pod for $t {} )*};
}
pod!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

/// Handle to a memory pool, which is a set of memory attached by some kind of a bus to one or more devices.
#[derive(Clone, Copy, Debug, Default)]
pub struct AddressSpace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryRegionState {
    /// Newly created, does not yet have a size etc.
    New,
    /// Memory is allocated and can be written from one place.
    Allocated,
    /// Memory is frozen and is now immutable, cacheable.
    Frozen,
    /// Memory has been deleted but is still referenced.
    Zombie,
    /// Memory has been disposed of.
    Deleted,
}

/// A handle to an abstract memory region. It can be used to refer to the memory region and to perform operations
/// on it. In general, operations need to be queued on a device's work queue in order to run; they cannot be run
/// imperatively. The device's runtime keeps its own state beside it (`MemoryRegionHandleInfo` is upcast by context
/// in C++; here the runtime holds the info inside its own type).
#[derive(Debug, Default)]
pub struct MemoryRegionHandleInfo {
    /// Non-CV qualified type in the array.
    pub type_: Option<(TypeId, &'static str)>,
    /// Is the referred to memory constant or mutable?
    pub is_const: bool,
    pub length_in_bytes: isize,
    /// Parent region (if we're part of a subrange).
    pub parent: Option<Weak<MemoryRegionHandleInfo>>,
    /// Offset in our parent range.
    pub owner_offset: isize,
    /// Mnemonic name.
    pub name: String,
    /// Version we refer to (-1 means latest/unknown).
    pub version: i32,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryRegionHandle {
    /// Opaque; upcast by context.
    pub handle: Option<Arc<MemoryRegionHandleInfo>>,
}

impl MemoryRegionHandle {
    pub fn new(handle: Arc<MemoryRegionHandleInfo>) -> Self {
        assert!(handle.length_in_bytes >= 0, "memory region handle: negative length");
        MemoryRegionHandle { handle: Some(handle) }
    }

    pub fn check_type_accessible_as<T: 'static>(&self) -> Result<(), String> {
        let Some(h) = &self.handle else { return Ok(()) };
        let Some((id, name)) = h.type_ else { return Ok(()) };
        if id != TypeId::of::<T>() {
            return Err(format!("Attempt to cast to wrong type: from {name} to {}", std::any::type_name::<T>()));
        }
        Ok(())
    }

    pub fn length_in_bytes(&self) -> usize {
        let h = self.handle.as_ref().expect("memory region handle: empty");
        assert!(h.length_in_bytes >= 0);
        h.length_in_bytes as usize
    }
}

/// A handle to a region known to hold `T`s.
#[derive(Debug)]
pub struct MemoryArrayHandleT<T> {
    pub region: MemoryRegionHandle,
    _t: std::marker::PhantomData<fn() -> T>,
}

impl<T> Clone for MemoryArrayHandleT<T> {
    fn clone(&self) -> Self {
        MemoryArrayHandleT { region: self.region.clone(), _t: Default::default() }
    }
}

impl<T> Default for MemoryArrayHandleT<T> {
    fn default() -> Self {
        MemoryArrayHandleT { region: Default::default(), _t: Default::default() }
    }
}

impl<T: 'static> MemoryArrayHandleT<T> {
    pub fn new(handle: Arc<MemoryRegionHandleInfo>) -> Self {
        let region = MemoryRegionHandle::new(handle);
        region.check_type_accessible_as::<T>().unwrap_or_else(|e| panic!("{e}"));
        MemoryArrayHandleT { region, _t: Default::default() }
    }

    /// From a handle of another type (the C++ converting constructors and assignments).
    pub fn from_other<T2>(other: &MemoryArrayHandleT<T2>) -> Self {
        other.region.check_type_accessible_as::<T>().unwrap_or_else(|e| panic!("{e}"));
        MemoryArrayHandleT { region: other.region.clone(), _t: Default::default() }
    }

    pub fn length(&self) -> usize {
        self.region.length_in_bytes() / std::mem::size_of::<T>()
    }
}

/// The memory a region is part of: one aligned allocation.
#[derive(Debug)]
pub struct Allocation {
    ptr: *mut u8,
    layout: Layout,
}

// The allocation is plain memory; who may write it is the regions' business.
unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

impl Allocation {
    fn new(bytes: usize, alignment: usize, zeroed: bool) -> Arc<Allocation> {
        let layout = Layout::from_size_align(bytes.max(1), alignment.max(1)).expect("memory region: bad layout");
        let ptr = unsafe { if zeroed { alloc_zeroed(layout) } else { alloc(layout) } };
        assert!(!ptr.is_null(), "not enough memory to allocate {bytes} bytes");
        Arc::new(Allocation { ptr, layout })
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
}

impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

/// An immutable region of memory.
#[derive(Clone, Debug, Default)]
pub struct FrozenMemoryRegion {
    handle: Option<Arc<Allocation>>,
    offset: usize,
    length: usize,
}

impl FrozenMemoryRegion {
    pub fn new(handle: Arc<Allocation>, offset: usize, length: usize) -> Self {
        assert!(offset + length <= handle.layout.size() || length == 0);
        FrozenMemoryRegion { handle: Some(handle), offset, length }
    }

    pub fn data(&self) -> &[u8] {
        match &self.handle {
            Some(h) => unsafe { std::slice::from_raw_parts(h.ptr.add(self.offset), self.length) },
            None => &[],
        }
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn memusage(&self) -> usize {
        self.length()
    }

    /// Another frozen memory region that points to a subset range of the current one.
    pub fn range(&self, start: usize, end: usize) -> FrozenMemoryRegion {
        assert!(start <= end && end <= self.length, "frozen memory region: range {start}..{end} of {}", self.length);
        FrozenMemoryRegion { handle: self.handle.clone(), offset: self.offset + start, length: end - start }
    }

    /// The given number of bytes at the start.
    pub fn range_at_start(&self, length: usize) -> FrozenMemoryRegion {
        self.range(0, length)
    }

    /// The given number of bytes at the end.
    pub fn range_at_end(&self, length: usize) -> FrozenMemoryRegion {
        self.range(self.length - length, self.length)
    }

    /// Whether the region is initialized (not default constructed).
    pub fn is_some(&self) -> bool {
        self.handle.is_some()
    }

    /// A combined, contiguous memory region. Regions that already lie next to each other in one allocation are
    /// returned as one range; otherwise a new region is allocated and the bytes copied.
    pub fn combined(regions: &[FrozenMemoryRegion]) -> FrozenMemoryRegion {
        let regions: Vec<&FrozenMemoryRegion> = regions.iter().filter(|r| r.length > 0).collect();
        match regions.as_slice() {
            [] => FrozenMemoryRegion::default(),
            [one] => (*one).clone(),
            [first, ..] => {
                let adjacent = regions.windows(2).all(|w| match (&w[0].handle, &w[1].handle) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b) && w[0].offset + w[0].length == w[1].offset,
                    _ => false,
                });
                if adjacent {
                    let total = regions.iter().map(|r| r.length).sum();
                    return FrozenMemoryRegion { handle: first.handle.clone(), offset: first.offset, length: total };
                }
                let total: usize = regions.iter().map(|r| r.length).sum();
                let mut serializer = MemorySerializer;
                let mut out = serializer.allocate_writable(total as u64, 4096);
                let mut at = 0;
                for r in &regions {
                    out.data_mut()[at..at + r.length].copy_from_slice(r.data());
                    at += r.length;
                }
                serializer.freeze(&mut out)
            }
        }
    }

    pub fn combined2(region1: &FrozenMemoryRegion, region2: &FrozenMemoryRegion) -> FrozenMemoryRegion {
        Self::combined(&[region1.clone(), region2.clone()])
    }

    pub fn handle(&self) -> Option<&Arc<Allocation>> {
        self.handle.as_ref()
    }
}

/// A frozen region viewed as `T`s.
#[derive(Clone, Debug)]
pub struct FrozenMemoryRegionT<T: Pod> {
    raw: FrozenMemoryRegion,
    _t: std::marker::PhantomData<T>,
}

impl<T: Pod> Default for FrozenMemoryRegionT<T> {
    fn default() -> Self {
        FrozenMemoryRegionT { raw: Default::default(), _t: Default::default() }
    }
}

impl<T: Pod> FrozenMemoryRegionT<T> {
    pub fn new(raw: FrozenMemoryRegion) -> Self {
        assert_eq!(raw.data().as_ptr() as usize % std::mem::align_of::<T>(), 0, "frozen region not aligned for its type");
        FrozenMemoryRegionT { raw, _t: Default::default() }
    }

    pub fn data(&self) -> &[T] {
        let bytes = self.raw.data();
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const T, bytes.len() / std::mem::size_of::<T>()) }
    }

    pub fn length(&self) -> usize {
        self.raw.length() / std::mem::size_of::<T>()
    }

    pub fn memusage(&self) -> usize {
        self.length() * std::mem::size_of::<T>()
    }

    pub fn raw(&self) -> &FrozenMemoryRegion {
        &self.raw
    }

    pub fn get_const_span(&self, start: usize, length: Option<usize>) -> Result<&[T], String> {
        if start > self.length() {
            return Err("getConstSpan: start is out of range".into());
        }
        let length = length.unwrap_or(self.length() - start);
        if start + length > self.length() {
            return Err("getConstSpan: length is out of range".into());
        }
        Ok(&self.data()[start..start + length])
    }
}

impl<T: Pod> std::ops::Index<usize> for FrozenMemoryRegionT<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        &self.data()[index]
    }
}

/// A writable region of memory. Ranges of one region share its allocation; writers of two ranges must keep to
/// their own bytes, as in C++.
#[derive(Debug, Default)]
pub struct MutableMemoryRegion {
    handle: Option<Arc<Allocation>>,
    offset: usize,
    length: usize,
}

impl MutableMemoryRegion {
    pub fn new(handle: Arc<Allocation>, offset: usize, length: usize) -> Self {
        assert!(offset + length <= handle.layout.size() || length == 0);
        MutableMemoryRegion { handle: Some(handle), offset, length }
    }

    pub fn data(&self) -> &[u8] {
        match &self.handle {
            Some(h) => unsafe { std::slice::from_raw_parts(h.ptr.add(self.offset), self.length) },
            None => &[],
        }
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        match &self.handle {
            Some(h) => unsafe { std::slice::from_raw_parts_mut(h.ptr.add(self.offset), self.length) },
            None => &mut [],
        }
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn range(&self, start_byte: usize, end_byte: usize) -> MutableMemoryRegion {
        assert!(start_byte <= end_byte && end_byte <= self.length, "mutable memory region: range out of bounds");
        MutableMemoryRegion { handle: self.handle.clone(), offset: self.offset + start_byte, length: end_byte - start_byte }
    }

    pub fn handle(&self) -> Option<&Arc<Allocation>> {
        self.handle.as_ref()
    }

    /// Reset the region (stealing the handle) to be null. This is mostly designed to be used by the
    /// `MappedSerializer` and its descendents.
    pub fn reset(&mut self) -> (Option<Arc<Allocation>>, usize, usize) {
        let r = (self.handle.take(), self.offset, self.length);
        self.offset = 0;
        self.length = 0;
        r
    }
}

/// A writable region viewed as `T`s.
#[derive(Debug)]
pub struct MutableMemoryRegionT<T: Pod> {
    raw: MutableMemoryRegion,
    _t: std::marker::PhantomData<T>,
}

impl<T: Pod> Default for MutableMemoryRegionT<T> {
    fn default() -> Self {
        MutableMemoryRegionT { raw: Default::default(), _t: Default::default() }
    }
}

impl<T: Pod> MutableMemoryRegionT<T> {
    pub fn new(raw: MutableMemoryRegion) -> Self {
        assert_eq!(raw.data().as_ptr() as usize % std::mem::align_of::<T>(), 0, "mutable region not aligned for its type");
        MutableMemoryRegionT { raw, _t: Default::default() }
    }

    pub fn data(&self) -> &[T] {
        let bytes = self.raw.data();
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const T, bytes.len() / std::mem::size_of::<T>()) }
    }

    pub fn data_mut(&mut self) -> &mut [T] {
        let bytes = self.raw.data_mut();
        unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut T, bytes.len() / std::mem::size_of::<T>()) }
    }

    pub fn length(&self) -> usize {
        self.raw.length() / std::mem::size_of::<T>()
    }

    pub fn range_bytes(&self, start: usize, end: usize) -> MutableMemoryRegionT<T> {
        MutableMemoryRegionT::new(self.raw.range(start, end))
    }

    pub fn raw(&self) -> &MutableMemoryRegion {
        &self.raw
    }

    pub fn raw_mut(&mut self) -> &mut MutableMemoryRegion {
        &mut self.raw
    }

    pub fn get_span(&mut self, start: usize, length: Option<usize>) -> Result<&mut [T], String> {
        let n = self.length();
        if start > n {
            return Err("getSpan: start is out of range".into());
        }
        let length = length.unwrap_or(n - start);
        if start + length > n {
            return Err("getSpan: length is out of range".into());
        }
        Ok(&mut self.data_mut()[start..start + length])
    }

    pub fn get_const_span(&self, start: usize, length: Option<usize>) -> Result<&[T], String> {
        let n = self.length();
        if start > n {
            return Err("getConstSpan: start is out of range".into());
        }
        let length = length.unwrap_or(n - start);
        if start + length > n {
            return Err("getConstSpan: length is out of range".into());
        }
        Ok(&self.data()[start..start + length])
    }
}

/// A serializer that hands out memory to write, then freezes it.
pub trait MappedSerializer {
    /// Commit all changes; this normally means that the current block is finished writing. It is guaranteed that no
    /// more changes will be made to any allocated blocks after this method is called.
    fn commit(&mut self);

    /// Allocate a writable block of memory with the given size and alignment. The memory in the block can be written
    /// until `freeze` is called. The memory is uninitialized (zeroed here: Rust reads no uninitialized memory).
    fn allocate_writable(&mut self, bytes_required: u64, alignment: usize) -> MutableMemoryRegion;

    fn allocate_writable_t<T: Pod>(&mut self, num_items: usize, alignment: usize) -> MutableMemoryRegionT<T>
    where
        Self: Sized,
    {
        MutableMemoryRegionT::new(self.allocate_writable((num_items * std::mem::size_of::<T>()) as u64, alignment.max(std::mem::align_of::<T>())))
    }

    /// Allocate a writable block of memory with the given size and alignment, guaranteed filled with zeros.
    fn allocate_zero_filled_writable(&mut self, bytes_required: u64, alignment: usize) -> MutableMemoryRegion {
        let mut r = self.allocate_writable(bytes_required, alignment);
        r.data_mut().fill(0);
        r
    }

    fn allocate_zero_filled_writable_t<T: Pod>(&mut self, num_items: usize, alignment: usize) -> MutableMemoryRegionT<T>
    where
        Self: Sized,
    {
        MutableMemoryRegionT::new(self.allocate_zero_filled_writable((num_items * std::mem::size_of::<T>()) as u64, alignment.max(std::mem::align_of::<T>())))
    }

    /// Freeze the given block of writable memory into a fixed, frozen representation of the same data. For memory
    /// that is backed by disk, this may also mean writing it out. All references to the given region become invalid.
    fn freeze(&mut self, region: &mut MutableMemoryRegion) -> FrozenMemoryRegion;

    fn freeze_t<T: Pod>(&mut self, region: &mut MutableMemoryRegionT<T>) -> FrozenMemoryRegionT<T>
    where
        Self: Sized,
    {
        FrozenMemoryRegionT::new(self.freeze(region.raw_mut()))
    }

    /// Copy the given memory region into the current serializer: a new region, the memory copied, frozen.
    fn copy(&mut self, region: &FrozenMemoryRegion) -> FrozenMemoryRegion {
        let mut out = self.allocate_writable(region.length() as u64, 4096);
        out.data_mut().copy_from_slice(region.data());
        self.freeze(&mut out)
    }
}

/// Mapped serializer that puts things in memory.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemorySerializer;

impl MappedSerializer for MemorySerializer {
    fn commit(&mut self) {}

    fn freeze(&mut self, region: &mut MutableMemoryRegion) -> FrozenMemoryRegion {
        match region.reset() {
            (Some(h), offset, length) => FrozenMemoryRegion::new(h, offset, length),
            (None, _, _) => FrozenMemoryRegion::default(),
        }
    }

    fn allocate_writable(&mut self, bytes_required: u64, alignment: usize) -> MutableMemoryRegion {
        let bytes = bytes_required as usize;
        MutableMemoryRegion::new(Allocation::new(bytes, alignment, true), 0, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_region_write_freeze_range() {
        let mut s = MemorySerializer;
        let mut w = s.allocate_writable_t::<u32>(1000, 4096);
        assert_eq!(w.data().as_ptr() as usize % 4096, 0, "page aligned");
        for (i, v) in w.data_mut().iter_mut().enumerate() {
            *v = i as u32 * 3;
        }
        let f = s.freeze_t(&mut w);
        assert_eq!(w.length(), 0, "frozen away");
        assert_eq!(f.length(), 1000);
        assert_eq!(f[999], 2997);
        assert_eq!(f.get_const_span(10, Some(2)).unwrap(), &[30, 33]);
        assert!(f.get_const_span(1001, None).is_err());
        let a = f.raw().range(0, 400);
        let b = f.raw().range(400, 4000);
        let c = FrozenMemoryRegion::combined(&[a, b]);
        assert_eq!(c.data().as_ptr(), f.raw().data().as_ptr(), "adjacent ranges combine without a copy");
        let z = s.allocate_zero_filled_writable_t::<u64>(8, 8);
        assert!(z.data().iter().all(|&x| x == 0));
    }
}
