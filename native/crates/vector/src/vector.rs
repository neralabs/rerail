//! A growable array in a block (`Vec`'s place for plain values): its `T`s in one region its room gives
//! (`Room`; the run's own, `default_room`, unless one is given), moved to a bigger region as it grows, the old one
//! given back to its block's free list. Read and written as a slice; the host, the JIT and the kernels read it where
//! it lies (`region`, and `rerail_blocks::maps::Placed` for its block).

use crate::{Pod, Region, Room, default_room};
use std::marker::PhantomData;
use std::sync::Arc;

/// Its region may be another holder's, borrowed (`borrowed`: a relation's rows its directory owns): its room then
/// below its length, so that any write moves its `T`s to a region of its own first (`own`), and a clone borrows them
/// again. A region with a guard has each CPU read and write checked by it (decision 0487 R4).
pub struct Vector<T: Pod> {
    /// Its region (none while it has no room) and the room it came from.
    mem: Option<Region>,
    room: Option<Arc<dyn Room>>,
    ptr: *mut T,
    len: usize,
    cap: usize,
    _t: PhantomData<T>,
}

// SAFETY: its `T`s lie in the region it holds, written only through `&mut self`.
unsafe impl<T: Pod + Send> Send for Vector<T> {}
unsafe impl<T: Pod + Sync> Sync for Vector<T> {}
// A panic leaves its `T`s plain values: nothing of it is half-updated past its length.
impl<T: Pod> std::panic::UnwindSafe for Vector<T> {}
impl<T: Pod> std::panic::RefUnwindSafe for Vector<T> {}

impl<T: Pod> Vector<T> {
    /// Empty, with no region: the run's room gives one at the first push.
    pub const fn new() -> Self {
        Vector { mem: None, room: None, ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0, cap: 0, _t: PhantomData }
    }

    /// Empty, its regions from `room`.
    pub fn new_in(room: Arc<dyn Room>) -> Self {
        Vector { room: Some(room), ..Vector::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        let mut v = Vector::new();
        v.reserve(n);
        v
    }

    pub fn with_capacity_in(room: Arc<dyn Room>, n: usize) -> Self {
        let mut v = Vector::new_in(room);
        v.reserve(n);
        v
    }

    /// The `T`s of `items`, its regions from `room`.
    pub fn from_iter_in(room: Arc<dyn Room>, items: impl IntoIterator<Item = T>) -> Self {
        let items = items.into_iter();
        let mut v = Vector::with_capacity_in(room, items.size_hint().0);
        v.extend(items);
        v
    }

    /// The `len` `T`s of `region`, its own (a chunk handed back: `into_region`), its growth from `room`.
    ///
    /// # Safety
    /// `region` holds `len` written `T`s, aligned for `T`.
    pub unsafe fn from_region(mut region: Region, len: usize, room: Option<Arc<dyn Room>>) -> Self {
        let cap = if size_of::<T>() == 0 { usize::MAX } else { region.len() / size_of::<T>() };
        debug_assert!(len <= cap);
        let ptr = region.as_mut_ptr().cast::<T>();
        Vector { mem: Some(region), room, ptr, len, cap, _t: PhantomData }
    }

    /// The `len` `T`s of `region`, borrowed: read where they lie, never written there (a write moves them to a region
    /// of `room`'s first), cloned as another borrow where `region` is shared (`Region::shared_by`).
    ///
    /// # Safety
    /// `region` holds `len` written `T`s, aligned for `T`, which its owner keeps while it lives.
    pub unsafe fn borrowed(mut region: Region, len: usize, room: Option<Arc<dyn Room>>) -> Self {
        if len == 0 {
            return Vector { room, ..Vector::new() };
        }
        let ptr = region.as_mut_ptr().cast::<T>();
        Vector { mem: Some(region), room, ptr, len, cap: 0, _t: PhantomData }
    }

    /// Whether its `T`s are borrowed (`borrowed`), not yet moved by a write.
    #[inline]
    pub fn is_borrowed(&self) -> bool {
        self.cap < self.len
    }

    /// Its region and length, handed on whole (a chunk to its relation's directory), where it is its own: else
    /// itself (borrowed, or with no region).
    pub fn into_region(mut self) -> Result<(Region, usize), Self> {
        if self.is_borrowed() || self.mem.is_none() {
            return Err(self);
        }
        let len = self.len;
        let mut region = self.mem.take().expect("a region");
        region.tag.handed();
        Ok((region, len))
    }

    /// Its room: where its regions come from (none until its first, for the run's).
    pub fn room(&self) -> Option<&Arc<dyn Room>> {
        self.room.as_ref()
    }

    /// The CPU reads (or writes) `n` `T`s from its `from`th: its region's guard checks them (decision 0487 R4). With
    /// none, or none of the device's uses of its block queued, no more than a load: every read of a value through
    /// `Deref` (each `v[i]`) passes here, and a call to the guard at each was a sixth of a cold PostHog run's
    /// instructions (0.58 T of 3.70 T, decision 0684).
    #[inline]
    #[track_caller]
    fn check(&self, from: usize, n: usize, write: bool) {
        if let Some(m) = &self.mem
            && m.device_uses()
            && let Some((g, at)) = m.guard()
        {
            let (at, len) = (at + from * size_of::<T>(), (n * size_of::<T>()).max(1));
            if write { g.cpu_write(at, len) } else { g.cpu_read(at, len) }
        }
    }

    /// Its borrowed `T`s moved to a region of its own, before a write.
    #[cold]
    #[inline(never)]
    fn own(&mut self) {
        self.reserve(0);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    /// Its region, while it has one: where its `T`s lie.
    pub fn region(&self) -> Option<&Region> {
        self.mem.as_ref()
    }

    /// Room for `extra` more `T`s: a region of twice its room (at least), its `T`s moved there.
    pub fn reserve(&mut self, extra: usize) {
        let need = self.len + extra;
        if need <= self.cap || size_of::<T>() == 0 {
            if size_of::<T>() == 0 {
                self.cap = usize::MAX;
            }
            return;
        }
        let cap = need.max(2 * self.cap).max(4);
        let room = self.room.get_or_insert_with(default_room).clone();
        let mut region = room.region(cap * size_of::<T>(), align_of::<T>(), std::any::type_name::<T>());
        let ptr = region.as_mut_ptr().cast::<T>();
        if self.len > 0 {
            self.check(0, self.len, false);
            // SAFETY: the old region holds `len` written `T`s; the new one is this vector's alone.
            // copy: growth: an array that outgrew its region, moved once, as a vector's growth moves it
            unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.len) };
        }
        if let Some(old) = self.mem.as_mut() {
            region.tag.grown(&mut old.tag);
        }
        (self.mem, self.ptr, self.cap) = (Some(region), ptr, cap);
    }

    /// Room for exactly `extra` more `T`s past its length, where it has less.
    pub fn reserve_exact(&mut self, extra: usize) {
        let need = self.len + extra;
        if need <= self.cap || size_of::<T>() == 0 {
            return self.reserve(extra);
        }
        let room = self.room.get_or_insert_with(default_room).clone();
        let mut region = room.region(need * size_of::<T>(), align_of::<T>(), std::any::type_name::<T>());
        let ptr = region.as_mut_ptr().cast::<T>();
        if self.len > 0 {
            self.check(0, self.len, false);
            // SAFETY: as `reserve`'s.
            // copy: growth: an array that outgrew its region, moved once, as a vector's growth moves it
            unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.len) };
        }
        if let Some(old) = self.mem.as_mut() {
            region.tag.grown(&mut old.tag);
        }
        (self.mem, self.ptr, self.cap) = (Some(region), ptr, need);
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        // At its room, or borrowed (its room below its length): a region of its own, room to spare.
        if self.len >= self.cap {
            self.reserve(1);
        }
        self.check(self.len, 1, true);
        // SAFETY: `len` is below its room.
        unsafe { self.ptr.add(self.len).write(v) };
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T>
    where
        T: Copy,
    {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: a written `T`.
        Some(unsafe { self.ptr.add(self.len).read() })
    }

    // copy: derived: an array's values written once where they are computed
    pub fn extend_from_slice(&mut self, src: &[T])
    where
        T: Copy,
    {
        self.reserve(src.len());
        self.check(self.len, src.len(), true);
        // SAFETY: room for `src` past `len`, this vector's alone, apart from `src`.
        // copy: derived: the values `extend_from_slice` writes
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), self.ptr.add(self.len), src.len()) };
        self.len += src.len();
    }

    /// `n` `T`s, those past its length `v`.
    pub fn resize(&mut self, n: usize, v: T)
    where
        T: Copy,
    {
        if n > self.len {
            self.reserve(n - self.len);
            self.check(self.len, n - self.len, true);
            for i in self.len..n {
                // SAFETY: below its room.
                unsafe { self.ptr.add(i).write(v) };
            }
        }
        self.len = n;
    }

    pub fn truncate(&mut self, n: usize) {
        self.len = self.len.min(n);
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn insert(&mut self, i: usize, v: T)
    where
        T: Copy,
    {
        assert!(i <= self.len, "insert at {i} of {}", self.len);
        self.push(v);
        self[i..].rotate_right(1);
    }

    pub fn remove(&mut self, i: usize) -> T
    where
        T: Copy,
    {
        let v = self[i];
        self[i..].rotate_left(1);
        self.len -= 1;
        v
    }

    pub fn swap_remove(&mut self, i: usize) -> T
    where
        T: Copy,
    {
        let last = self.len - 1;
        self.swap(i, last);
        self.pop().expect("an item")
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool)
    where
        T: Copy,
    {
        let mut w = 0;
        for r in 0..self.len {
            let v = self[r];
            if keep(&v) {
                self[w] = v;
                w += 1;
            }
        }
        self.len = w;
    }

    pub fn retain_mut(&mut self, mut keep: impl FnMut(&mut T) -> bool)
    where
        T: Copy,
    {
        let mut w = 0;
        for r in 0..self.len {
            let mut v = self[r];
            if keep(&mut v) {
                self[w] = v;
                w += 1;
            }
        }
        self.len = w;
    }

    pub fn dedup(&mut self)
    where
        T: Copy + PartialEq,
    {
        self.dedup_by_key(|v| *v);
    }

    pub fn dedup_by_key<K: PartialEq>(&mut self, mut key: impl FnMut(&T) -> K)
    where
        T: Copy,
    {
        self.dedup_by(|a, b| key(a) == key(b));
    }

    /// Runs of `T`s `same` says are one kept once: `same(later, kept)`, as `Vec::dedup_by`.
    pub fn dedup_by(&mut self, mut same: impl FnMut(&mut T, &mut T) -> bool)
    where
        T: Copy,
    {
        if self.len < 2 {
            return;
        }
        let mut w = 1;
        for r in 1..self.len {
            let (mut a, mut b) = (self[r], self[w - 1]);
            if !same(&mut a, &mut b) {
                self[w] = self[r];
                w += 1;
            }
        }
        self.len = w;
    }

    pub fn split_off(&mut self, at: usize) -> Vector<T>
    where
        T: Copy,
    {
        let mut tail = Vector { room: self.room.clone(), ..Vector::new() };
        tail.extend_from_slice(&self[at..]);
        self.len = at;
        tail
    }

    pub fn drain(&mut self, range: impl std::ops::RangeBounds<usize>) -> IntoIter<T>
    where
        T: Copy,
    {
        use std::ops::Bound::*;
        let a = match range.start_bound() {
            Included(&a) => a,
            Excluded(&a) => a + 1,
            Unbounded => 0,
        };
        let b = match range.end_bound() {
            Included(&b) => b + 1,
            Excluded(&b) => b,
            Unbounded => self.len,
        };
        let mut out = Vector { room: self.room.clone(), ..Vector::new() };
        out.extend_from_slice(&self[a..b]);
        self[a..].rotate_left(b - a);
        self.len -= b - a;
        out.into_iter()
    }

    pub fn append(&mut self, other: &mut Vector<T>)
    where
        T: Copy,
    {
        self.extend_from_slice(other);
        other.clear();
    }

    pub fn as_slice(&self) -> &[T] {
        self
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self
    }

    /// Its unused room, as `Vec::spare_capacity_mut`'s place: written, then counted by `set_len`.
    ///
    /// # Safety
    /// `n` is at most its capacity, and its first `n` `T`s are written.
    pub unsafe fn set_len(&mut self, n: usize) {
        debug_assert!(n <= self.cap);
        self.len = n;
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        if self.is_borrowed() {
            self.own();
        }
        self.ptr
    }

    /// Itself, frozen in use (a `Box<[T]>`'s place: `Vec::into_boxed_slice`).
    pub fn into_boxed_slice(self) -> Self {
        self
    }

    /// Room no more than its length (`Vec::shrink_to_fit`): kept as it is.
    pub fn shrink_to_fit(&mut self) {}

    pub fn first_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().first_mut()
    }

    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().last_mut()
    }
}

impl<T: Pod> std::ops::Deref for Vector<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.check(0, self.len, false);
        // SAFETY: `len` written `T`s at `ptr` (dangling and aligned when empty).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T: Pod> std::ops::DerefMut for Vector<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        if self.is_borrowed() {
            self.own();
        }
        self.check(0, self.len, true);
        // SAFETY: as `deref`, through `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl<T: Pod> Default for Vector<T> {
    fn default() -> Self {
        Vector::new()
    }
}

impl<T: Pod + Copy> Clone for Vector<T> {
    fn clone(&self) -> Self {
        // Borrowed: another borrow of the same `T`s, where they lie.
        if self.is_borrowed()
            && let Some(region) = self.mem.as_ref().and_then(Region::share)
        {
            return Vector { mem: Some(region), room: self.room.clone(), ptr: self.ptr, len: self.len, cap: 0, _t: PhantomData };
        }
        let mut v = Vector { room: self.room.clone(), ..Vector::new() };
        // copy: derived: an array written again for a holder of its own (`Clone`), as a `Vec`'s clone is
        v.extend_from_slice(self);
        v
    }
}

impl<T: Pod> Extend<T> for Vector<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        let items = items.into_iter();
        self.reserve(items.size_hint().0);
        for v in items {
            self.push(v);
        }
    }
}

impl<'a, T: Pod + Copy> Extend<&'a T> for Vector<T> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, items: I) {
        self.extend(items.into_iter().copied());
    }
}

impl<T: Pod> FromIterator<T> for Vector<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut v = Vector::new();
        v.extend(items);
        v
    }
}

impl<'a, T: Pod + Copy> FromIterator<&'a T> for Vector<T> {
    fn from_iter<I: IntoIterator<Item = &'a T>>(items: I) -> Self {
        items.into_iter().copied().collect()
    }
}

impl<'a, T: Pod> IntoIterator for &'a Vector<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: Pod> IntoIterator for &'a mut Vector<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T: Pod + Copy> IntoIterator for Vector<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        let end = self.len;
        IntoIter { v: self, at: 0, end }
    }
}

/// A `Vector`'s `T`s by value, its region kept until the last.
pub struct IntoIter<T: Pod> {
    v: Vector<T>,
    at: usize,
    end: usize,
}

impl<T: Pod + Copy> Iterator for IntoIter<T> {
    type Item = T;
    #[inline]
    fn next(&mut self) -> Option<T> {
        (self.at < self.end).then(|| {
            self.at += 1;
            self.v[self.at - 1]
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.end - self.at, Some(self.end - self.at))
    }
}

impl<T: Pod + Copy> ExactSizeIterator for IntoIter<T> {}

impl<T: Pod + Copy> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        (self.end > self.at).then(|| {
            self.end -= 1;
            self.v[self.end]
        })
    }
}

impl<T: Pod + PartialEq> PartialEq for Vector<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Pod + PartialEq> PartialEq<[T]> for Vector<T> {
    fn eq(&self, other: &[T]) -> bool {
        **self == *other
    }
}

impl<T: Pod + PartialEq, const N: usize> PartialEq<[T; N]> for Vector<T> {
    fn eq(&self, other: &[T; N]) -> bool {
        **self == other[..]
    }
}

impl<T: Pod + PartialEq> PartialEq<Vec<T>> for Vector<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        **self == other[..]
    }
}

impl<T: Pod + PartialEq> PartialEq<&[T]> for Vector<T> {
    fn eq(&self, other: &&[T]) -> bool {
        **self == **other
    }
}

impl<T: Pod + PartialEq> PartialEq<Vector<T>> for [T] {
    fn eq(&self, other: &Vector<T>) -> bool {
        *self == **other
    }
}

impl<T: Pod + PartialEq> PartialEq<Vector<T>> for &[T] {
    fn eq(&self, other: &Vector<T>) -> bool {
        **self == **other
    }
}

impl<T: Pod + PartialEq> PartialEq<Vector<T>> for Vec<T> {
    fn eq(&self, other: &Vector<T>) -> bool {
        self[..] == **other
    }
}

impl<T: Pod + Eq> Eq for Vector<T> {}

impl<T: Pod, I: std::slice::SliceIndex<[T]>> std::ops::Index<I> for Vector<T> {
    type Output = I::Output;
    fn index(&self, i: I) -> &I::Output {
        &(**self)[i]
    }
}

impl<T: Pod, I: std::slice::SliceIndex<[T]>> std::ops::IndexMut<I> for Vector<T> {
    fn index_mut(&mut self, i: I) -> &mut I::Output {
        &mut (**self)[i]
    }
}

impl<T: Pod + std::hash::Hash> std::hash::Hash for Vector<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (**self).hash(h)
    }
}

impl<T: Pod + PartialOrd> PartialOrd for Vector<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<T: Pod + Ord> Ord for Vector<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl<T: Pod + std::fmt::Debug> std::fmt::Debug for Vector<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Pod + Copy> From<&[T]> for Vector<T> {
    fn from(items: &[T]) -> Self {
        let mut v = Vector::new();
        v.extend_from_slice(items);
        v
    }
}

impl<T: Pod + Copy> From<&mut [T]> for Vector<T> {
    fn from(items: &mut [T]) -> Self {
        Vector::from(&*items)
    }
}

impl<T: Pod + Copy, const N: usize> From<[T; N]> for Vector<T> {
    fn from(items: [T; N]) -> Self {
        Vector::from(&items[..])
    }
}

impl<T: Pod + Copy> From<&Vector<T>> for Vector<T> {
    fn from(items: &Vector<T>) -> Self {
        items.clone()
    }
}

impl<T: Pod + Copy> std::borrow::Borrow<[T]> for Vector<T> {
    fn borrow(&self) -> &[T] {
        self
    }
}

impl<T: Pod> AsRef<[T]> for Vector<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T: Pod> AsMut<[T]> for Vector<T> {
    fn as_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T: Pod + serde::Serialize> serde::Serialize for Vector<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de, T: Pod + serde::Deserialize<'de>> serde::Deserialize<'de> for Vector<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Seq<T>(PhantomData<T>);
        impl<'de, T: Pod + serde::Deserialize<'de>> serde::de::Visitor<'de> for Seq<T> {
            type Value = Vector<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a sequence")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vector<T>, A::Error> {
                let mut v = Vector::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(x) = seq.next_element()? {
                    v.push(x);
                }
                Ok(v)
            }
        }
        d.deserialize_seq(Seq(PhantomData))
    }
}

impl<T: Pod + Copy + Send> rayon::iter::FromParallelIterator<T> for Vector<T> {
    /// Each thread's run, then the runs in order (as rayon's `Vec` collects).
    fn from_par_iter<I: rayon::iter::IntoParallelIterator<Item = T>>(items: I) -> Self {
        use rayon::iter::ParallelIterator;
        // The caller's parallel loop, in the caller's pool (its `on_cores`): a collect starts no work of its own.
        let runs: std::collections::LinkedList<Vector<T>> = rayon::iter::IntoParallelIterator::into_par_iter(items)
            .fold(Vector::new, |mut v, x| {
                v.push(x);
                v
            })
            .map(|v| std::collections::LinkedList::from([v]))
            .reduce(std::collections::LinkedList::new, |mut a, mut b| {
                a.append(&mut b);
                a
            });
        let mut out = Vector::with_capacity(runs.iter().map(|r| r.len()).sum());
        for r in &runs {
            // copy: derived: each thread's run of a parallel collect, written once in order (as rayon's `Vec`'s)
            out.extend_from_slice(r);
        }
        out
    }
}

impl<T: Pod + Copy + Send> rayon::iter::ParallelExtend<T> for Vector<T> {
    fn par_extend<I: rayon::iter::IntoParallelIterator<Item = T>>(&mut self, items: I) {
        let more: Vector<T> = rayon::iter::FromParallelIterator::from_par_iter(items);
        self.extend_from_slice(&more);
    }
}

impl std::io::Write for Vector<u8> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A `Vector` of the items given, as `vec!` makes a `Vec` (`vector![v; n]`: `n` copies of `v`).
#[macro_export]
macro_rules! vector {
    () => { $crate::vector::Vector::new() };
    ($v:expr; $n:expr) => {{
        let mut v = $crate::vector::Vector::new();
        v.resize($n, $v);
        v
    }};
    ($($v:expr),+ $(,)?) => { $crate::vector::Vector::from([$($v),+]) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_holds_what_a_vec_holds() {
        let mut v: Vector<u64> = Vector::new();
        let mut w: Vec<u64> = Vec::new();
        for i in 0..10_000u64 {
            v.push(i * 3);
            w.push(i * 3);
            if i % 7 == 0 {
                assert_eq!(v.pop(), w.pop());
            }
        }
        v.retain(|x| x % 2 == 0);
        w.retain(|x| x % 2 == 0);
        v.insert(3, 99);
        w.insert(3, 99);
        assert_eq!(v.remove(10), w.remove(10));
        assert_eq!(&v[..], &w[..]);
        let back: Vec<u64> = v.clone().into_iter().rev().collect();
        assert_eq!(back, w.iter().rev().copied().collect::<Vec<_>>());
        let d: Vec<u64> = v.drain(5..20).collect();
        assert_eq!(d, w.drain(5..20).collect::<Vec<_>>());
        assert_eq!(&v[..], &w[..]);
    }
}
