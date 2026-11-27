//! Plain values and the typed views over a block's bytes (moved from `rerail_vector` and `rerail_blocks::gens`).
//!
//! A block's columns are fixed-width integers read where they lie: `words` and `words_mut` give a run of a `Pod` type
//! at a byte offset, checked for bounds and alignment.

use core::sync::atomic::{AtomicI16, AtomicI32, AtomicI64, AtomicU16, AtomicU32, AtomicU64};

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
pod!(AtomicU16, AtomicU32, AtomicU64, AtomicI16, AtomicI32, AtomicI64);
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}
// A run of them: a final form that is its bytes alone (a sketch's kept bytes, `flat::Settle`).
unsafe impl<T: Pod> Pod for [T] {}

/// A run of `n` `T`s at byte `at` of `bytes`, read in place. Panics, naming the run, past the bytes or misaligned.
#[inline]
pub fn words<T: Pod>(bytes: &[u8], at: usize, n: usize) -> &[T] {
    let len = n * size_of::<T>();
    assert!(at + len <= bytes.len(), "a run of {n} words at {at} past a block's {} bytes", bytes.len());
    let p = bytes[at..].as_ptr();
    assert!((p as usize).is_multiple_of(align_of::<T>()), "a run of words at {at}, not aligned to {}", align_of::<T>());
    // SAFETY: in bounds and aligned (checked); `T: Pod` has no invalid bit patterns and holds no address.
    unsafe { core::slice::from_raw_parts(p as *const T, n) }
}

/// `words`, to write.
#[inline]
pub fn words_mut<T: Pod>(bytes: &mut [u8], at: usize, n: usize) -> &mut [T] {
    let len = n * size_of::<T>();
    assert!(at + len <= bytes.len(), "a run of {n} words at {at} past a block's {} bytes", bytes.len());
    let p = bytes[at..].as_mut_ptr();
    assert!((p as usize).is_multiple_of(align_of::<T>()), "a run of words at {at}, not aligned to {}", align_of::<T>());
    // SAFETY: as `words`, and the borrow is exclusive.
    unsafe { core::slice::from_raw_parts_mut(p as *mut T, n) }
}

/// One `T` at byte `at`.
#[inline]
pub fn word<T: Pod + Copy>(bytes: &[u8], at: usize) -> T {
    words::<T>(bytes, at, 1)[0]
}

/// Writes one `T` at byte `at`.
#[inline]
pub fn set_word<T: Pod>(bytes: &mut [u8], at: usize, v: T) {
    words_mut::<T>(bytes, at, 1)[0] = v;
}
