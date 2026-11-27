//! References inside a block (moved from `rerail_blocks::refs`): each one relative to its own address (Boost's
//! `offset_ptr`, MLDB's `MappedPtr`). A range that holds both a reference and its target can be moved by `memcpy` with
//! nothing to fix up.

use crate::Pod;
use crate::kernel::{Decls, KernelField};
use core::fmt::{self, Write};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering::Relaxed};

/// One `T`, or none (offset 0: a reference never points at itself).
#[repr(C)]
pub struct Ptr<T> {
    pub off: AtomicI64,
    _t: PhantomData<fn() -> T>,
}

unsafe impl<T: 'static> Pod for Ptr<T> {}

impl<T> Ptr<T> {
    #[inline]
    pub fn is_null(&self) -> bool {
        self.off.load(Relaxed) == 0
    }

    #[inline]
    pub fn get(&self) -> Option<&T> {
        let off = self.off.load(Relaxed);
        if off == 0 {
            return None;
        }
        Some(unsafe { &*((self as *const Self as *const u8).offset(off as isize) as *const T) })
    }

    /// # Safety
    /// `this` and `target` are in the same mapping.
    pub unsafe fn set(this: *const Self, target: *const T) {
        unsafe { (*this).off.store(target as isize as i64 - this as isize as i64, Relaxed) };
    }
}

impl<T: KernelField> KernelField for Ptr<T> {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        out.write_str("Ptr")
    }
    fn deps(out: &mut dyn Decls) -> fmt::Result {
        T::deps(out)?;
        out.declare("Ptr", &|s: &mut dyn Write| s.write_str("typedef struct Ptr {\n    int64_t off;\n} Ptr;\nUKL_STATIC_ASSERT(sizeof(Ptr) == 8, \"Ptr: size\");\n"))
    }
}

/// `len` items of `T`, starting `off` bytes from the span's own address.
#[repr(C)]
pub struct Span<T> {
    pub off: AtomicI64,
    pub len: AtomicU64,
    _t: PhantomData<fn() -> T>,
}

unsafe impl<T: 'static> Pod for Span<T> {}

impl<T> Span<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.len.load(Relaxed) as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        unsafe { (self as *const Self as *const u8).offset(self.off.load(Relaxed) as isize) as *const T }
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        let n = self.len();
        if n == 0 {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.as_ptr(), n) }
    }

    /// The items, writable: for the one writer of an open block.
    ///
    /// # Safety
    /// The block is open, and no other thread writes these items.
    #[allow(clippy::mut_from_ref)]
    #[inline]
    pub unsafe fn as_mut_slice(&self) -> &mut [T] {
        let n = self.len();
        if n == 0 {
            return &mut [];
        }
        unsafe { core::slice::from_raw_parts_mut(self.as_ptr() as *mut T, n) }
    }

    /// # Safety
    /// `this` and `target` are in the same mapping.
    pub unsafe fn set(this: *const Self, target: *const T, len: usize) {
        unsafe {
            (*this).off.store(target as isize as i64 - this as isize as i64, Relaxed);
            (*this).len.store(len as u64, Relaxed);
        }
    }
}

impl<T: KernelField> KernelField for Span<T> {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        out.write_str("Span")
    }
    fn deps(out: &mut dyn Decls) -> fmt::Result {
        T::deps(out)?;
        out.declare("Span", &|s: &mut dyn Write| s.write_str("typedef struct Span {\n    int64_t off;\n    uint64_t len;\n} Span;\nUKL_STATIC_ASSERT(sizeof(Span) == 16, \"Span: size\");\n"))
    }
}
