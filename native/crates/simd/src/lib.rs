//! The part of `std::simd`'s interface the engine's SIMD kernels use, on stable Rust.
//!
//! The names, paths, signatures and semantics are `std::simd`'s (`core::simd`, nightly `portable_simd`): `Simd`,
//! `Mask`, `LaneCount`/`SupportedLaneCount`, `SimdElement`/`MaskElement`, `prelude`, and the traits of `num`, `cmp` and
//! `ptr`. Code written against this crate is code written against `std::simd`: once that is stable, `use
//! rerail_simd::...` becomes `use std::simd::...` and this crate is removed. So nothing is added here that `std::simd`
//! does not have, and what it has is added only as a kernel needs it.
//!
//! Each operation is a loop over the lanes of an array, `#[inline(always)]`, which LLVM turns into the target's
//! vector instructions (NEON, SSE, AVX2, AVX-512) at a lane count the target holds, and splits across registers above
//! it. Integer arithmetic wraps; a shift's amount is taken modulo the lane's bits (`wrapping_shl`), as `std::simd`'s.
#![no_std]
#![allow(non_camel_case_types)]

use core::ops::{Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div, Mul, MulAssign, Neg, Not, Rem, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign};

#[doc(hidden)]
pub mod chunks;
use chunks::Chunks;

/// `[T; N]` as a vector operation `OP` with `b`, a chunk at a time.
#[inline(always)]
fn bin<T: Chunks, const OP: u8, const N: usize>(a: &[T; N], b: &[T; N]) -> [T; N] {
    let mut o = core::mem::MaybeUninit::<[T; N]>::uninit();
    let p = o.as_mut_ptr().cast::<T>();
    // SAFETY: every lane of `o` written, a chunk or a lane at a time.
    unsafe {
        if T::L > 1 && N.is_multiple_of(T::L) {
            let mut i = 0;
            while i < N {
                T::bin::<OP>(a.as_ptr().add(i), b.as_ptr().add(i), p.add(i));
                i += T::L;
            }
        } else {
            for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
                p.add(i).write(T::bin1::<OP>(x, y));
            }
        }
        o.assume_init()
    }
}
/// `[T; N]` as a vector operation `OP`, a chunk at a time.
#[inline(always)]
fn un<T: Chunks, const OP: u8, const N: usize>(a: &[T; N]) -> [T; N] {
    let mut o = core::mem::MaybeUninit::<[T; N]>::uninit();
    let p = o.as_mut_ptr().cast::<T>();
    // SAFETY: every lane of `o` written.
    unsafe {
        if T::L > 1 && N.is_multiple_of(T::L) {
            let mut i = 0;
            while i < N {
                T::un::<OP>(a.as_ptr().add(i), p.add(i));
                i += T::L;
            }
        } else {
            for (i, &x) in a.iter().enumerate() {
                p.add(i).write(T::un1::<OP>(x));
            }
        }
        o.assume_init()
    }
}
/// Whether any (`ALL`: every) lane is non-zero.
#[inline(always)]
fn test<T: Chunks + PartialEq + Default, const ALL: bool, const N: usize>(a: &[T; N]) -> bool {
    // SAFETY: each chunk in `a`.
    unsafe {
        if T::L > 1 && N.is_multiple_of(T::L) {
            let mut r = ALL;
            let mut i = 0;
            while i < N {
                let c = if ALL { T::all(a.as_ptr().add(i)) } else { T::any(a.as_ptr().add(i)) };
                r = if ALL { r & c } else { r | c };
                i += T::L;
            }
            r
        } else if ALL {
            a.iter().fold(true, |r, x| r & (*x != T::default()))
        } else {
            a.iter().fold(false, |r, x| r | (*x != T::default()))
        }
    }
}
/// The bits of `a` as `B` (of `A`'s size).
#[inline(always)]
fn bits<A: Copy, B: Copy>(a: A) -> B {
    const { assert!(size_of::<A>() == size_of::<B>()) };
    // SAFETY: of one size, both plain data.
    unsafe { core::mem::transmute_copy(&a) }
}

mod sealed {
    pub trait Sealed {}
}
use sealed::Sealed;

/// A lane count `std::simd` supports.
pub struct LaneCount<const N: usize>;
pub trait SupportedLaneCount: Sealed {
    /// Writes `N` copies of `v` at `out`, as stores LLVM keeps in a register: `[v; N]` is a loop, which LLVM's loop
    /// idioms turn into a call of `memset_pattern16`, and a vector behind that call stays on the stack.
    #[doc(hidden)]
    unsafe fn __splat<T: Copy>(v: T, out: *mut T);
}
macro_rules! lane_counts {
    ($($n:literal)*) => {$(
        impl Sealed for LaneCount<$n> {}
        impl SupportedLaneCount for LaneCount<$n> {
            #[inline(always)]
            unsafe fn __splat<T: Copy>(v: T, out: *mut T) {
                let x1 = [v];
                let x2 = [v, v];
                let x4 = [x2, x2];
                let x8 = [x4, x4];
                let x16 = [x8, x8];
                let x32 = [x16, x16];
                let x64 = [x32, x32];
                let out = out.cast::<[T; $n]>();
                // SAFETY: the copy of `$n` lanes, `[T; $n]` exactly (the arm of this `$n` is the one taken).
                unsafe {
                    match $n {
                        1 => out.write(core::mem::transmute_copy(&x1)),
                        2 => out.write(core::mem::transmute_copy(&x2)),
                        4 => out.write(core::mem::transmute_copy(&x4)),
                        8 => out.write(core::mem::transmute_copy(&x8)),
                        16 => out.write(core::mem::transmute_copy(&x16)),
                        32 => out.write(core::mem::transmute_copy(&x32)),
                        _ => out.write(core::mem::transmute_copy(&x64)),
                    }
                }
            }
        }
    )*};
}
lane_counts!(1 2 4 8 16 32 64);

/// A type a lane holds.
///
/// # Safety
/// As `std::simd::SimdElement`: implemented only here, for the primitive integers and pointers.
pub unsafe trait SimdElement: Sealed + Copy {
    /// The mask element of a lane this wide.
    type Mask: MaskElement;
}

/// A mask lane's integer: all ones for true, zero for false.
///
/// # Safety
/// As `std::simd::MaskElement`: implemented only here, for the signed integers.
pub unsafe trait MaskElement: SimdElement<Mask = Self> + Sealed + PartialEq + Default + Chunks {
    #[doc(hidden)]
    const TRUE: Self;
    #[doc(hidden)]
    const FALSE: Self;
    #[doc(hidden)]
    fn __test(self) -> bool;
}

/// A vector of `N` lanes of `T`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Simd<T, const N: usize>([T; N])
where
    LaneCount<N>: SupportedLaneCount,
    T: SimdElement;

/// A mask of `N` lanes, each true or false, for lanes of `T`'s width.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Mask<T, const N: usize>([T; N])
where
    LaneCount<N>: SupportedLaneCount,
    T: MaskElement;

// `std::simd`'s `len` has no `is_empty`: a vector is never empty.
#[allow(clippy::len_without_is_empty)]
impl<T: SimdElement, const N: usize> Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    pub const LEN: usize = N;

    #[inline(always)]
    pub const fn len(&self) -> usize {
        N
    }
    #[inline(always)]
    pub fn splat(value: T) -> Self {
        let mut a = core::mem::MaybeUninit::<[T; N]>::uninit();
        // SAFETY: `__splat` writes all `N` lanes.
        unsafe {
            LaneCount::<N>::__splat(value, a.as_mut_ptr().cast::<T>());
            Simd(a.assume_init())
        }
    }
    #[inline(always)]
    pub const fn from_array(array: [T; N]) -> Self {
        Simd(array)
    }
    #[inline(always)]
    pub const fn to_array(self) -> [T; N] {
        self.0
    }
    #[inline(always)]
    pub const fn as_array(&self) -> &[T; N] {
        &self.0
    }
    #[inline(always)]
    pub fn as_mut_array(&mut self) -> &mut [T; N] {
        &mut self.0
    }
    /// The first `N` elements of `slice`; panics if it is shorter.
    #[inline(always)]
    pub fn from_slice(slice: &[T]) -> Self {
        assert!(slice.len() >= N, "slice length must be at least the number of elements");
        Simd(core::array::from_fn(|i| slice[i]))
    }
    /// Writes the lanes to the first `N` elements of `slice`; panics if it is shorter.
    #[inline(always)]
    pub fn copy_to_slice(self, slice: &mut [T]) {
        assert!(slice.len() >= N, "slice length must be at least the number of elements");
        slice[..N].copy_from_slice(&self.0);
    }
    /// The elements of `slice` in the first lanes, the rest the default.
    #[inline(always)]
    pub fn load_or_default(slice: &[T]) -> Self
    where
        T: Default,
    {
        Self::load_or(slice, Self::splat(T::default()))
    }
    /// The elements of `slice` in the first lanes, the rest `or`'s.
    #[inline(always)]
    pub fn load_or(slice: &[T], or: Self) -> Self {
        Simd(core::array::from_fn(|i| if i < slice.len() { slice[i] } else { or.0[i] }))
    }
    /// Lane `i` from `slice[idxs[i]]` where `enable` is set and the index is in bounds, else `or`'s.
    #[inline(always)]
    pub fn gather_select(slice: &[T], enable: Mask<isize, N>, idxs: Simd<usize, N>, or: Self) -> Self {
        Simd(core::array::from_fn(|i| if enable.0[i].__test() && idxs.0[i] < slice.len() { slice[idxs.0[i]] } else { or.0[i] }))
    }
    /// Lane `i` from `slice[idxs[i]]`, or `or`'s where the index is out of bounds.
    #[inline(always)]
    pub fn gather_or(slice: &[T], idxs: Simd<usize, N>, or: Self) -> Self {
        Self::gather_select(slice, Mask::splat(true), idxs, or)
    }
    /// Lane `i` read from `source[i]`.
    ///
    /// # Safety
    /// Each pointer is valid for a read of `T`, as `std::simd::Simd::gather_ptr`.
    #[inline(always)]
    pub unsafe fn gather_ptr(source: Simd<*const T, N>) -> Self {
        Simd(core::array::from_fn(|i| unsafe { source.0[i].read() }))
    }
    /// Lane `i` read from `source[i]` where `enable` is set, else `or`'s.
    ///
    /// # Safety
    /// Each enabled lane's pointer is valid for a read of `T`.
    #[inline(always)]
    pub unsafe fn gather_select_ptr(source: Simd<*const T, N>, enable: Mask<isize, N>, or: Self) -> Self {
        Simd(core::array::from_fn(|i| if enable.0[i].__test() { unsafe { source.0[i].read() } } else { or.0[i] }))
    }
    /// Writes lane `i` to `dest[i]` where `enable` is set, in lane order.
    ///
    /// # Safety
    /// Each enabled lane's pointer is valid for a write of `T`.
    #[inline(always)]
    pub unsafe fn scatter_select_ptr(self, dest: Simd<*mut T, N>, enable: Mask<isize, N>) {
        for i in 0..N {
            if enable.0[i].__test() {
                unsafe { dest.0[i].write(self.0[i]) };
            }
        }
    }
    /// Writes lane `i` to `dest[i]`, in lane order.
    ///
    /// # Safety
    /// Each pointer is valid for a write of `T`.
    #[inline(always)]
    pub unsafe fn scatter_ptr(self, dest: Simd<*mut T, N>) {
        for i in 0..N {
            unsafe { dest.0[i].write(self.0[i]) };
        }
    }
}

// `cast` of numbers in a block of its own, as `std::simd`'s: a vector of pointers has none, and its `cast` is
// `SimdConstPtr`'s.
impl<T: SimdCast, const N: usize> Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    /// Each lane converted as `as` converts it.
    #[inline(always)]
    pub fn cast<U: SimdCast>(self) -> Simd<U, N> {
        let mut o = core::mem::MaybeUninit::<[U; N]>::uninit();
        // SAFETY: `chunks::cast` writes every lane, or none and says so.
        unsafe {
            if chunks::cast::<T, U, N>(self.0.as_ptr(), o.as_mut_ptr().cast::<U>()) {
                return Simd(o.assume_init());
            }
        }
        Simd(core::array::from_fn(|i| U::__from_i128(self.0[i].__to_i128())))
    }
}

impl<T: SimdElement, const N: usize> Default for Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
    T: Default,
{
    #[inline(always)]
    fn default() -> Self {
        Self::splat(T::default())
    }
}
impl<T: SimdElement, const N: usize> From<[T; N]> for Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    fn from(a: [T; N]) -> Self {
        Simd(a)
    }
}
impl<T: SimdElement, const N: usize> From<Simd<T, N>> for [T; N]
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    fn from(v: Simd<T, N>) -> Self {
        v.0
    }
}
impl<T: SimdElement, const N: usize> core::ops::Index<usize> for Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.0[i]
    }
}
impl<T: SimdElement, const N: usize> core::ops::IndexMut<usize> for Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.0[i]
    }
}

/// A lane type `Simd::cast` converts from and to, as `as` does.
pub trait SimdCast: Sealed + SimdElement + Chunks {
    #[doc(hidden)]
    fn __to_i128(self) -> i128;
    #[doc(hidden)]
    fn __from_i128(v: i128) -> Self;
}

macro_rules! elements {
    ($($t:ty => $m:ty;)*) => {$(
        impl Sealed for $t {}
        unsafe impl SimdElement for $t {
            type Mask = $m;
        }
        impl SimdCast for $t {
            #[inline(always)]
            fn __to_i128(self) -> i128 {
                self as i128
            }
            #[inline(always)]
            fn __from_i128(v: i128) -> Self {
                v as $t
            }
        }
    )*};
}
elements! {
    u8 => i8; u16 => i16; u32 => i32; u64 => i64; usize => isize;
    i8 => i8; i16 => i16; i32 => i32; i64 => i64; isize => isize;
}
macro_rules! mask_elements {
    ($($t:ty)*) => {$(
        unsafe impl MaskElement for $t {
            const TRUE: Self = -1;
            const FALSE: Self = 0;
            #[inline(always)]
            fn __test(self) -> bool {
                self != 0
            }

        }
    )*};
}
mask_elements!(i8 i16 i32 i64 isize);

impl<T> Sealed for *const T {}
impl<T> Sealed for *mut T {}
unsafe impl<T> SimdElement for *const T {
    type Mask = isize;
}
unsafe impl<T> SimdElement for *mut T {
    type Mask = isize;
}

// ---- Masks. ----

impl<T: MaskElement, const N: usize> Mask<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    pub fn splat(value: bool) -> Self {
        Mask(Simd::<T, N>::splat(if value { T::TRUE } else { T::FALSE }).0)
    }
    #[inline(always)]
    pub fn from_array(array: [bool; N]) -> Self {
        Mask(core::array::from_fn(|i| if array[i] { T::TRUE } else { T::FALSE }))
    }
    #[inline(always)]
    pub fn to_array(self) -> [bool; N] {
        core::array::from_fn(|i| self.0[i].__test())
    }
    /// The mask of the lanes of `value` that are all ones; panics where a lane is neither all ones nor zero.
    #[inline(always)]
    pub fn from_int(value: Simd<T, N>) -> Self {
        assert!(value.0.iter().all(|&x| x == T::TRUE || x == T::FALSE), "all values must be either 0 or -1");
        Mask(value.0)
    }
    /// # Safety
    /// Each lane of `value` is all ones or zero.
    #[inline(always)]
    pub unsafe fn from_int_unchecked(value: Simd<T, N>) -> Self {
        Mask(value.0)
    }
    /// Each lane all ones where set, zero where not.
    #[inline(always)]
    pub fn to_int(self) -> Simd<T, N> {
        Simd(self.0)
    }
    /// The mask for lanes of another width.
    #[inline(always)]
    pub fn cast<U: MaskElement>(self) -> Mask<U, N> {
        let mut o = core::mem::MaybeUninit::<[U; N]>::uninit();
        // SAFETY: `chunks::cast` writes every lane (a mask lane sign-extended or truncated is one), or none and says so.
        unsafe {
            if chunks::cast::<T, U, N>(self.0.as_ptr(), o.as_mut_ptr().cast::<U>()) {
                return Mask(o.assume_init());
            }
        }
        Mask(core::array::from_fn(|i| if self.0[i].__test() { U::TRUE } else { U::FALSE }))
    }
    #[inline(always)]
    pub fn test(&self, index: usize) -> bool {
        self.0[index].__test()
    }
    #[inline(always)]
    pub fn set(&mut self, index: usize, value: bool) {
        self.0[index] = if value { T::TRUE } else { T::FALSE };
    }
    /// Whether any lane is set.
    #[inline(always)]
    pub fn any(self) -> bool {
        test::<T, false, N>(&self.0)
    }
    /// Whether every lane is set.
    #[inline(always)]
    pub fn all(self) -> bool {
        test::<T, true, N>(&self.0)
    }
    /// Lane `i` of `true_values` where set, of `false_values` where not.
    #[inline(always)]
    pub fn select<U>(self, true_values: Simd<U, N>, false_values: Simd<U, N>) -> Simd<U, N>
    where
        U: SimdElement<Mask = T>,
    {
        // The lanes' bits as the mask's integers: `(m & t) | (!m & f)`, one `bsl`.
        let (t, f): ([T; N], [T; N]) = (bits(true_values.0), bits(false_values.0));
        let m = &self.0;
        Simd(bits(bin::<T, { chunks::OR }, N>(&bin::<T, { chunks::AND }, N>(m, &t), &bin::<T, { chunks::AND }, N>(&un::<T, { chunks::NOT }, N>(m), &f))))
    }
    /// Bit `i` set where lane `i` is.
    #[inline(always)]
    pub fn to_bitmask(self) -> u64 {
        let mut b = 0u64;
        for i in 0..N {
            b |= (self.0[i].__test() as u64) << i;
        }
        b
    }
    #[inline(always)]
    pub fn from_bitmask(bitmask: u64) -> Self {
        Mask(core::array::from_fn(|i| if bitmask >> i & 1 != 0 { T::TRUE } else { T::FALSE }))
    }
    /// The first lane set, if any.
    #[inline(always)]
    pub fn first_set(self) -> Option<usize> {
        let b = self.to_bitmask();
        (b != 0).then(|| b.trailing_zeros() as usize)
    }
}

impl<T: MaskElement, const N: usize> Default for Mask<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    fn default() -> Self {
        Self::splat(false)
    }
}

macro_rules! mask_ops {
    ($($tr:ident $f:ident $atr:ident $af:ident $m:ident;)*) => {$(
        impl<T: MaskElement, const N: usize> $tr for Mask<T, N>
        where
            LaneCount<N>: SupportedLaneCount,
        {
            type Output = Self;
            #[inline(always)]
            fn $f(self, o: Self) -> Self {
                Mask(bin::<T, { chunks::$m }, N>(&self.0, &o.0))
            }
        }
        impl<T: MaskElement, const N: usize> $tr<bool> for Mask<T, N>
        where
            LaneCount<N>: SupportedLaneCount,
        {
            type Output = Self;
            #[inline(always)]
            fn $f(self, o: bool) -> Self {
                $tr::$f(self, Self::splat(o))
            }
        }
        impl<T: MaskElement, const N: usize> $atr for Mask<T, N>
        where
            LaneCount<N>: SupportedLaneCount,
        {
            #[inline(always)]
            fn $af(&mut self, o: Self) {
                *self = $tr::$f(*self, o);
            }
        }
    )*};
}
mask_ops! {
    BitAnd bitand BitAndAssign bitand_assign AND;
    BitOr bitor BitOrAssign bitor_assign OR;
    BitXor bitxor BitXorAssign bitxor_assign XOR;
}
impl<T: MaskElement, const N: usize> Not for Mask<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    type Output = Self;
    #[inline(always)]
    fn not(self) -> Self {
        Mask(un::<T, { chunks::NOT }, N>(&self.0))
    }
}

// ---- Integer lanes' operators: wrapping, a shift's amount modulo the lane's bits. ----

macro_rules! int_ops {
    ($($t:ty)*) => {$(
        int_ops!(@op $t, Add add AddAssign add_assign ADD);
        int_ops!(@op $t, Sub sub SubAssign sub_assign SUB);
        int_ops!(@op $t, Mul mul MulAssign mul_assign MUL);
        int_ops!(@op $t, Shl shl ShlAssign shl_assign SHL);
        int_ops!(@op $t, Shr shr ShrAssign shr_assign SHR);
        int_ops!(@bit $t, BitAnd bitand BitAndAssign bitand_assign AND);
        int_ops!(@bit $t, BitOr bitor BitOrAssign bitor_assign OR);
        int_ops!(@bit $t, BitXor bitxor BitXorAssign bitxor_assign XOR);
        impl<const N: usize> Div for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            /// Panics on a zero divisor (and on `MIN / -1`), as `std::simd`'s.
            #[inline(always)]
            fn div(self, o: Self) -> Self {
                Simd(core::array::from_fn(|i| self.0[i] / o.0[i]))
            }
        }
        impl<const N: usize> Rem for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            #[inline(always)]
            fn rem(self, o: Self) -> Self {
                Simd(core::array::from_fn(|i| self.0[i] % o.0[i]))
            }
        }
        impl<const N: usize> Not for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            #[inline(always)]
            fn not(self) -> Self {
                Simd(un::<$t, { chunks::NOT }, N>(&self.0))
            }
        }
    )*};
    (@op $t:ty, $tr:ident $f:ident $atr:ident $af:ident $w:ident) => {
        impl<const N: usize> $tr for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            #[inline(always)]
            fn $f(self, o: Self) -> Self {
                Simd(bin::<$t, { chunks::$w }, N>(&self.0, &o.0))
            }
        }
        impl<const N: usize> $atr for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            #[inline(always)]
            fn $af(&mut self, o: Self) {
                *self = $tr::$f(*self, o);
            }
        }
    };
    (@bit $t:ty, $tr:ident $f:ident $atr:ident $af:ident $op:ident) => {
        impl<const N: usize> $tr for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            #[inline(always)]
            fn $f(self, o: Self) -> Self {
                Simd(bin::<$t, { chunks::$op }, N>(&self.0, &o.0))
            }
        }
        impl<const N: usize> $atr for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            #[inline(always)]
            fn $af(&mut self, o: Self) {
                *self = $tr::$f(*self, o);
            }
        }
    };
}

int_ops!(u8 u16 u32 u64 usize i8 i16 i32 i64 isize);

macro_rules! neg_ops {
    ($($t:ty)*) => {$(
        impl<const N: usize> Neg for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
            type Output = Self;
            #[inline(always)]
            fn neg(self) -> Self {
                Simd(core::array::from_fn(|i| self.0[i].wrapping_neg()))
            }
        }
    )*};
}
neg_ops!(i8 i16 i32 i64 isize);

// ---- The traits of `std::simd::{num, cmp, ptr}`. ----

pub mod cmp {
    use super::*;

    /// Lane-wise `==` and `!=`.
    pub trait SimdPartialEq {
        type Mask;
        fn simd_eq(self, other: Self) -> Self::Mask;
        fn simd_ne(self, other: Self) -> Self::Mask;
    }
    /// Lane-wise ordering.
    pub trait SimdPartialOrd: SimdPartialEq {
        fn simd_lt(self, other: Self) -> Self::Mask;
        fn simd_le(self, other: Self) -> Self::Mask;
        fn simd_gt(self, other: Self) -> Self::Mask;
        fn simd_ge(self, other: Self) -> Self::Mask;
    }
    /// Lane-wise maximum, minimum and clamp.
    pub trait SimdOrd: SimdPartialOrd {
        fn simd_max(self, other: Self) -> Self;
        fn simd_min(self, other: Self) -> Self;
        fn simd_clamp(self, min: Self, max: Self) -> Self;
    }

    macro_rules! cmps {
        ($($t:ty)*) => {$(
            impl<const N: usize> SimdPartialEq for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
                type Mask = Mask<<$t as SimdElement>::Mask, N>;
                #[inline(always)]
                fn simd_eq(self, o: Self) -> Self::Mask {
                    Mask(bits(bin::<$t, { chunks::EQ }, N>(&self.0, &o.0)))
                }
                #[inline(always)]
                fn simd_ne(self, o: Self) -> Self::Mask {
                    !self.simd_eq(o)
                }
            }
            impl<const N: usize> SimdPartialOrd for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
                #[inline(always)]
                fn simd_lt(self, o: Self) -> Self::Mask {
                    Mask(bits(bin::<$t, { chunks::LT }, N>(&self.0, &o.0)))
                }
                #[inline(always)]
                fn simd_le(self, o: Self) -> Self::Mask {
                    Mask(bits(bin::<$t, { chunks::LE }, N>(&self.0, &o.0)))
                }
                #[inline(always)]
                fn simd_gt(self, o: Self) -> Self::Mask {
                    o.simd_lt(self)
                }
                #[inline(always)]
                fn simd_ge(self, o: Self) -> Self::Mask {
                    o.simd_le(self)
                }
            }
            impl<const N: usize> SimdOrd for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
                #[inline(always)]
                fn simd_max(self, o: Self) -> Self {
                    Simd(bin::<$t, { chunks::MAX }, N>(&self.0, &o.0))
                }
                #[inline(always)]
                fn simd_min(self, o: Self) -> Self {
                    Simd(bin::<$t, { chunks::MIN }, N>(&self.0, &o.0))
                }
                #[inline(always)]
                fn simd_clamp(self, min: Self, max: Self) -> Self {
                    self.simd_max(min).simd_min(max)
                }
            }
        )*};
    }
    cmps!(u8 u16 u32 u64 usize i8 i16 i32 i64 isize);

    macro_rules! mask_cmps {
        ($($t:ty)*) => {$(
            impl<const N: usize> SimdPartialEq for Mask<$t, N> where LaneCount<N>: SupportedLaneCount {
                type Mask = Self;
                #[inline(always)]
                fn simd_eq(self, o: Self) -> Self {
                    !(self ^ o)
                }
                #[inline(always)]
                fn simd_ne(self, o: Self) -> Self {
                    self ^ o
                }
            }
        )*};
    }
    mask_cmps!(i8 i16 i32 i64 isize);
}

pub mod num {
    use super::*;

    /// Operations of unsigned integer lanes.
    pub trait SimdUint: Copy + Sealed {
        type Scalar;
        type Cast<T: SimdElement>;
        fn cast<T: SimdCast>(self) -> Self::Cast<T>;
        fn wrapping_neg(self) -> Self;
        fn saturating_add(self, second: Self) -> Self;
        fn saturating_sub(self, second: Self) -> Self;
        fn abs_diff(self, second: Self) -> Self;
        fn reduce_sum(self) -> Self::Scalar;
        fn reduce_product(self) -> Self::Scalar;
        fn reduce_max(self) -> Self::Scalar;
        fn reduce_min(self) -> Self::Scalar;
        fn reduce_and(self) -> Self::Scalar;
        fn reduce_or(self) -> Self::Scalar;
        fn reduce_xor(self) -> Self::Scalar;
        fn swap_bytes(self) -> Self;
        fn reverse_bits(self) -> Self;
        fn count_ones(self) -> Self;
        fn count_zeros(self) -> Self;
        fn leading_zeros(self) -> Self;
        fn trailing_zeros(self) -> Self;
        fn leading_ones(self) -> Self;
        fn trailing_ones(self) -> Self;
    }
    /// Operations of signed integer lanes.
    pub trait SimdInt: Copy + Sealed {
        type Mask;
        type Scalar;
        type Unsigned;
        type Cast<T: SimdElement>;
        fn cast<T: SimdCast>(self) -> Self::Cast<T>;
        fn saturating_add(self, second: Self) -> Self;
        fn saturating_sub(self, second: Self) -> Self;
        fn abs(self) -> Self;
        fn saturating_abs(self) -> Self;
        fn saturating_neg(self) -> Self;
        fn is_positive(self) -> Self::Mask;
        fn is_negative(self) -> Self::Mask;
        fn signum(self) -> Self;
        fn reduce_sum(self) -> Self::Scalar;
        fn reduce_product(self) -> Self::Scalar;
        fn reduce_max(self) -> Self::Scalar;
        fn reduce_min(self) -> Self::Scalar;
        fn reduce_and(self) -> Self::Scalar;
        fn reduce_or(self) -> Self::Scalar;
        fn reduce_xor(self) -> Self::Scalar;
        fn swap_bytes(self) -> Self;
        fn reverse_bits(self) -> Self;
        fn count_ones(self) -> Self::Unsigned;
        fn count_zeros(self) -> Self::Unsigned;
        fn leading_zeros(self) -> Self::Unsigned;
        fn trailing_zeros(self) -> Self::Unsigned;
        fn leading_ones(self) -> Self::Unsigned;
        fn trailing_ones(self) -> Self::Unsigned;
    }

    impl<T: SimdElement, const N: usize> Sealed for Simd<T, N> where LaneCount<N>: SupportedLaneCount {}

    macro_rules! map {
        ($s:expr, |$x:ident| $e:expr) => {
            Simd(core::array::from_fn(|i| {
                let $x = $s.0[i];
                $e
            }))
        };
    }

    macro_rules! uints {
        ($($t:ty)*) => {$(
            impl<const N: usize> SimdUint for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
                type Scalar = $t;
                type Cast<T: SimdElement> = Simd<T, N>;
                #[inline(always)]
                fn cast<T: SimdCast>(self) -> Simd<T, N> { Simd::cast(self) }
                #[inline(always)]
                fn wrapping_neg(self) -> Self { map!(self, |x| x.wrapping_neg()) }
                #[inline(always)]
                fn saturating_add(self, o: Self) -> Self { Simd(core::array::from_fn(|i| self.0[i].saturating_add(o.0[i]))) }
                #[inline(always)]
                fn saturating_sub(self, o: Self) -> Self { Simd(core::array::from_fn(|i| self.0[i].saturating_sub(o.0[i]))) }
                #[inline(always)]
                fn abs_diff(self, o: Self) -> Self { Simd(core::array::from_fn(|i| self.0[i].abs_diff(o.0[i]))) }
                #[inline(always)]
                fn reduce_sum(self) -> $t { self.0.iter().fold(0, |a: $t, &x| a.wrapping_add(x)) }
                #[inline(always)]
                fn reduce_product(self) -> $t { self.0.iter().fold(1, |a: $t, &x| a.wrapping_mul(x)) }
                #[inline(always)]
                fn reduce_max(self) -> $t { self.0.iter().fold(<$t>::MIN, |a, &x| a.max(x)) }
                #[inline(always)]
                fn reduce_min(self) -> $t { self.0.iter().fold(<$t>::MAX, |a, &x| a.min(x)) }
                #[inline(always)]
                fn reduce_and(self) -> $t { self.0.iter().fold(!0, |a, &x| a & x) }
                #[inline(always)]
                fn reduce_or(self) -> $t { self.0.iter().fold(0, |a, &x| a | x) }
                #[inline(always)]
                fn reduce_xor(self) -> $t { self.0.iter().fold(0, |a, &x| a ^ x) }
                #[inline(always)]
                fn swap_bytes(self) -> Self { map!(self, |x| x.swap_bytes()) }
                #[inline(always)]
                fn reverse_bits(self) -> Self { map!(self, |x| x.reverse_bits()) }
                #[inline(always)]
                fn count_ones(self) -> Self { map!(self, |x| x.count_ones() as $t) }
                #[inline(always)]
                fn count_zeros(self) -> Self { map!(self, |x| x.count_zeros() as $t) }
                #[inline(always)]
                fn leading_zeros(self) -> Self { Simd(un::<$t, { chunks::CLZ }, N>(&self.0)) }
                #[inline(always)]
                fn trailing_zeros(self) -> Self { map!(self, |x| x.trailing_zeros() as $t) }
                #[inline(always)]
                fn leading_ones(self) -> Self { map!(self, |x| x.leading_ones() as $t) }
                #[inline(always)]
                fn trailing_ones(self) -> Self { map!(self, |x| x.trailing_ones() as $t) }
            }
        )*};
    }
    uints!(u8 u16 u32 u64 usize);

    macro_rules! ints {
        ($($t:ty => $u:ty)*) => {$(
            impl<const N: usize> SimdInt for Simd<$t, N> where LaneCount<N>: SupportedLaneCount {
                type Mask = Mask<$t, N>;
                type Scalar = $t;
                type Unsigned = Simd<$u, N>;
                type Cast<T: SimdElement> = Simd<T, N>;
                #[inline(always)]
                fn cast<T: SimdCast>(self) -> Simd<T, N> { Simd::cast(self) }
                #[inline(always)]
                fn saturating_add(self, o: Self) -> Self { Simd(core::array::from_fn(|i| self.0[i].saturating_add(o.0[i]))) }
                #[inline(always)]
                fn saturating_sub(self, o: Self) -> Self { Simd(core::array::from_fn(|i| self.0[i].saturating_sub(o.0[i]))) }
                #[inline(always)]
                fn abs(self) -> Self { map!(self, |x| x.wrapping_abs()) }
                #[inline(always)]
                fn saturating_abs(self) -> Self { map!(self, |x| x.saturating_abs()) }
                #[inline(always)]
                fn saturating_neg(self) -> Self { map!(self, |x| x.saturating_neg()) }
                #[inline(always)]
                fn is_positive(self) -> Mask<$t, N> { Mask(core::array::from_fn(|i| if self.0[i] > 0 { -1 } else { 0 })) }
                #[inline(always)]
                fn is_negative(self) -> Mask<$t, N> { Mask(core::array::from_fn(|i| if self.0[i] < 0 { -1 } else { 0 })) }
                #[inline(always)]
                fn signum(self) -> Self { map!(self, |x| x.signum()) }
                #[inline(always)]
                fn reduce_sum(self) -> $t { self.0.iter().fold(0, |a: $t, &x| a.wrapping_add(x)) }
                #[inline(always)]
                fn reduce_product(self) -> $t { self.0.iter().fold(1, |a: $t, &x| a.wrapping_mul(x)) }
                #[inline(always)]
                fn reduce_max(self) -> $t { self.0.iter().fold(<$t>::MIN, |a, &x| a.max(x)) }
                #[inline(always)]
                fn reduce_min(self) -> $t { self.0.iter().fold(<$t>::MAX, |a, &x| a.min(x)) }
                #[inline(always)]
                fn reduce_and(self) -> $t { self.0.iter().fold(!0, |a, &x| a & x) }
                #[inline(always)]
                fn reduce_or(self) -> $t { self.0.iter().fold(0, |a, &x| a | x) }
                #[inline(always)]
                fn reduce_xor(self) -> $t { self.0.iter().fold(0, |a, &x| a ^ x) }
                #[inline(always)]
                fn swap_bytes(self) -> Self { map!(self, |x| x.swap_bytes()) }
                #[inline(always)]
                fn reverse_bits(self) -> Self { map!(self, |x| x.reverse_bits()) }
                #[inline(always)]
                fn count_ones(self) -> Simd<$u, N> { map!(self, |x| x.count_ones() as $u) }
                #[inline(always)]
                fn count_zeros(self) -> Simd<$u, N> { map!(self, |x| x.count_zeros() as $u) }
                #[inline(always)]
                fn leading_zeros(self) -> Simd<$u, N> { map!(self, |x| x.leading_zeros() as $u) }
                #[inline(always)]
                fn trailing_zeros(self) -> Simd<$u, N> { map!(self, |x| x.trailing_zeros() as $u) }
                #[inline(always)]
                fn leading_ones(self) -> Simd<$u, N> { map!(self, |x| x.leading_ones() as $u) }
                #[inline(always)]
                fn trailing_ones(self) -> Simd<$u, N> { map!(self, |x| x.trailing_ones() as $u) }
            }
        )*};
    }
    ints!(i8 => u8 i16 => u16 i32 => u32 i64 => u64 isize => usize);
}

pub mod ptr {
    use super::*;

    /// Operations of lanes of constant pointers.
    pub trait SimdConstPtr: Copy + Sealed {
        type Usize;
        type Isize;
        type Mask;
        type CastPtr<T>;
        fn is_null(self) -> Self::Mask;
        fn cast<T>(self) -> Self::CastPtr<T>;
        fn addr(self) -> Self::Usize;
        fn expose_provenance(self) -> Self::Usize;
        fn with_exposed_provenance(addr: Self::Usize) -> Self;
        fn wrapping_offset(self, offset: Self::Isize) -> Self;
        fn wrapping_add(self, count: Self::Usize) -> Self;
        fn wrapping_sub(self, count: Self::Usize) -> Self;
    }
    /// Operations of lanes of mutable pointers.
    pub trait SimdMutPtr: Copy + Sealed {
        type Usize;
        type Isize;
        type Mask;
        type CastPtr<T>;
        fn is_null(self) -> Self::Mask;
        fn cast<T>(self) -> Self::CastPtr<T>;
        fn addr(self) -> Self::Usize;
        fn expose_provenance(self) -> Self::Usize;
        fn with_exposed_provenance(addr: Self::Usize) -> Self;
        fn wrapping_offset(self, offset: Self::Isize) -> Self;
        fn wrapping_add(self, count: Self::Usize) -> Self;
        fn wrapping_sub(self, count: Self::Usize) -> Self;
    }

    macro_rules! ptrs {
        ($tr:ident, $p:ident, $from:ident) => {
            impl<T, const N: usize> $tr for Simd<*$p T, N>
            where
                LaneCount<N>: SupportedLaneCount,
            {
                type Usize = Simd<usize, N>;
                type Isize = Simd<isize, N>;
                type Mask = Mask<isize, N>;
                type CastPtr<U> = Simd<*$p U, N>;
                #[inline(always)]
                fn is_null(self) -> Mask<isize, N> {
                    Mask(core::array::from_fn(|i| if self.0[i].is_null() { -1 } else { 0 }))
                }
                #[inline(always)]
                fn cast<U>(self) -> Simd<*$p U, N> {
                    Simd(core::array::from_fn(|i| self.0[i].cast::<U>()))
                }
                #[inline(always)]
                fn addr(self) -> Simd<usize, N> {
                    Simd(core::array::from_fn(|i| self.0[i].addr()))
                }
                #[inline(always)]
                fn expose_provenance(self) -> Simd<usize, N> {
                    Simd(core::array::from_fn(|i| self.0[i].expose_provenance()))
                }
                #[inline(always)]
                fn with_exposed_provenance(addr: Simd<usize, N>) -> Self {
                    Simd(core::array::from_fn(|i| core::ptr::$from(addr.0[i])))
                }
                #[inline(always)]
                fn wrapping_offset(self, offset: Simd<isize, N>) -> Self {
                    // As addresses, `offset` in `T`s.
                    let a: [isize; N] = bits(self.0);
                    let k = Simd::<isize, N>::splat(size_of::<T>() as isize);
                    Simd(bits(bin::<isize, { chunks::ADD }, N>(&a, &(offset * k).0)))
                }
                #[inline(always)]
                fn wrapping_add(self, count: Simd<usize, N>) -> Self {
                    let a: [usize; N] = bits(self.0);
                    let k = Simd::<usize, N>::splat(size_of::<T>());
                    Simd(bits(bin::<usize, { chunks::ADD }, N>(&a, &(count * k).0)))
                }
                #[inline(always)]
                fn wrapping_sub(self, count: Simd<usize, N>) -> Self {
                    let a: [usize; N] = bits(self.0);
                    let k = Simd::<usize, N>::splat(size_of::<T>());
                    Simd(bits(bin::<usize, { chunks::SUB }, N>(&a, &(count * k).0)))
                }
            }
        };
    }
    ptrs!(SimdConstPtr, const, with_exposed_provenance);
    ptrs!(SimdMutPtr, mut, with_exposed_provenance_mut);
}

/// The types and traits a SIMD kernel uses, as `std::simd::prelude`.
pub mod prelude {
    pub use super::cmp::{SimdOrd, SimdPartialEq, SimdPartialOrd};
    pub use super::num::{SimdInt, SimdUint};
    pub use super::ptr::{SimdConstPtr, SimdMutPtr};
    pub use super::{Mask, Simd};
    pub use super::{
        i8x16, i16x8, i32x4, i32x8, i32x16, i64x2, i64x4, i64x8, mask8x16, mask32x4, mask32x8, mask32x16, mask64x2, mask64x4, mask64x8, u8x16, u8x32, u16x8, u32x4, u32x8, u32x16,
        u64x2, u64x4, u64x8, usizex4, usizex8,
    };
}

macro_rules! aliases {
    ($($name:ident = $t:ty, $n:literal;)*) => {$(
        pub type $name = Simd<$t, $n>;
    )*};
}
aliases! {
    u8x16 = u8, 16; u8x32 = u8, 32; u16x8 = u16, 8; u32x4 = u32, 4; u32x8 = u32, 8; u32x16 = u32, 16;
    u64x2 = u64, 2; u64x4 = u64, 4; u64x8 = u64, 8; usizex4 = usize, 4; usizex8 = usize, 8;
    i8x16 = i8, 16; i16x8 = i16, 8; i32x4 = i32, 4; i32x8 = i32, 8; i32x16 = i32, 16;
    i64x2 = i64, 2; i64x4 = i64, 4; i64x8 = i64, 8;
}
macro_rules! mask_aliases {
    ($($name:ident = $t:ty, $n:literal;)*) => {$(
        pub type $name = Mask<$t, $n>;
    )*};
}
mask_aliases! {
    mask8x16 = i8, 16; mask32x4 = i32, 4; mask32x8 = i32, 8; mask32x16 = i32, 16;
    mask64x2 = i64, 2; mask64x4 = i64, 4; mask64x8 = i64, 8;
}

#[cfg(test)]
mod tests;
