//! Sections (decision 0628 §4): a tree of plan or analysis values (a rule's join, a component's spec, a program's
//! declarations) encoded once, where it is computed, into one run of bytes in a block, and read where it lies
//! through its archived view. A struct's view is a `repr(C)` record of its fields' views, an enum's a `repr(u32)`
//! tagged record, and a list or a text lies out of line, named by an offset relative to the field that names it:
//! a section reads the same wherever its block maps it, on the host or on a device.
//!
//! `#[derive(Archive)]` writes a type's view (`ArchivedT`) and its encoder; `Section<T>` holds one tree's bytes.

extern crate self as rerail_section;

pub use rerail_section_derive::Archive;

pub mod text;
pub use text::{TextMap, TextSet};
use rerail_vector::Pod;
use rerail_vector::vector::Vector;
use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

/// The largest alignment a view may ask: a block's regions are aligned to at least this.
pub const ALIGN: usize = 8;

/// A value with an archived view in a section.
///
/// # Safety
/// `Archived` is `repr(C)` (or a primitive) and aligned to at most `ALIGN`; `resolve` writes a valid `Archived` at
/// `out`, naming only data `write_out` wrote before it in the same section.
pub unsafe trait Archive {
    type Archived;
    type Resolver;
    /// Writes its out-of-line data (lists, text, boxed values) and says where it put them.
    fn write_out(&self, w: &mut Writer) -> Self::Resolver;
    /// Writes its record at `out`, which lies at `at` in the section, zeroed.
    ///
    /// # Safety
    /// `out` is the writer's bytes at `at`, room for an `Archived`, aligned; `r` is this value's own resolver.
    unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived);
}

/// The run's objects a section names by number (`Bound`): another section, a relation. Not part of the section's
/// words: a plan names them, and whoever holds the section holds them with it.
type Handles = Vec<Arc<dyn Any + Send + Sync>>;

/// A value made again from its view (`#[archive(decode)]`): for a rewrite of a plan into another plan, which is
/// encoded in its turn. Not a way to hold a tree: what is held is the section.
pub trait Decode: Archive + Sized {
    /// `a` made again; `n` names the objects its section's handles name (`Section::names`).
    fn decode(a: &Self::Archived, n: &Names<'_>) -> Self;
}

/// The objects a section names by number (`Section::names`), for a `Decode` of a tree that names them.
#[derive(Clone, Copy)]
pub struct Names<'a>(&'a [Arc<dyn Any + Send + Sync>]);

impl<'a> Names<'a> {
    /// No objects: for a tree that names none.
    pub const NONE: Names<'static> = Names(&[]);

    pub fn get<H: Any + Send + Sync>(&self, h: &Handle<H>) -> Arc<H> {
        let any = self.0[h.n as usize].clone();
        any.downcast::<H>().unwrap_or_else(|_| panic!("a handle of another type"))
    }
}

impl<H: Any + Send + Sync> Decode for Bound<H> {
    fn decode(a: &Handle<H>, n: &Names<'_>) -> Self {
        Bound(n.get(a))
    }
}

macro_rules! decode_plain {
    ($($t:ty),*) => {$(
        impl Decode for $t {
            fn decode(a: &Self::Archived, _: &Names<'_>) -> Self {
                *a as $t
            }
        }
    )*};
}

decode_plain!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, usize, isize);

impl Decode for bool {
    fn decode(a: &bool, _: &Names<'_>) -> Self {
        *a
    }
}

impl Decode for () {
    fn decode(_: &(), _: &Names<'_>) -> Self {}
}

impl Decode for String {
    fn decode(a: &RelStr, _: &Names<'_>) -> Self {
        a.as_str().to_owned()
    }
}

impl Decode for Box<str> {
    fn decode(a: &RelStr, _: &Names<'_>) -> Self {
        a.as_str().into()
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(a: &RelList<T::Archived>, n: &Names<'_>) -> Self {
        a.iter().map(|x| T::decode(x, n)).collect()
    }
}

impl<T: Decode + Pod> Decode for Vector<T> {
    fn decode(a: &RelList<T::Archived>, n: &Names<'_>) -> Self {
        a.iter().map(|x| T::decode(x, n)).collect()
    }
}

impl<T: Decode> Decode for Box<T> {
    fn decode(a: &RelBox<T::Archived>, n: &Names<'_>) -> Self {
        Box::new(T::decode(a, n))
    }
}

impl<T: Decode> Decode for Arc<T> {
    fn decode(a: &RelBox<T::Archived>, n: &Names<'_>) -> Self {
        Arc::new(T::decode(a, n))
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(a: &ArchivedOption<T::Archived>, n: &Names<'_>) -> Self {
        a.as_ref().map(|x| T::decode(x, n))
    }
}

impl<A: Decode, B: Decode> Decode for (A, B) {
    fn decode(a: &Tuple2<A::Archived, B::Archived>, n: &Names<'_>) -> Self {
        (A::decode(&a.0, n), B::decode(&a.1, n))
    }
}

impl<A: Decode, B: Decode, C: Decode> Decode for (A, B, C) {
    fn decode(a: &Tuple3<A::Archived, B::Archived, C::Archived>, n: &Names<'_>) -> Self {
        (A::decode(&a.0, n), B::decode(&a.1, n), C::decode(&a.2, n))
    }
}

impl<T: Decode, const N: usize> Decode for [T; N] {
    fn decode(a: &[T::Archived; N], n: &Names<'_>) -> Self {
        std::array::from_fn(|i| T::decode(&a[i], n))
    }
}

/// A section being written: its bytes in a block (`Vector`), and the handles it names (`Bound`).
#[derive(Default)]
pub struct Writer {
    bytes: Vector<u8>,
    // copy: not block data: handles to the objects the section names by number, not their data.
    handles: Handles,
}

impl Writer {
    pub fn new() -> Self {
        Writer { bytes: Vector::new(), handles: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        Writer { bytes: Vector::with_capacity(n), handles: Vec::new() }
    }

    /// Names `h` by a number in this section (the same number for the same object).
    pub fn bind(&mut self, h: Arc<dyn Any + Send + Sync>) -> u32 {
        if let Some(i) = self.handles.iter().position(|x| Arc::ptr_eq(x, &h)) {
            return i as u32;
        }
        self.handles.push(h);
        u32::try_from(self.handles.len() - 1).expect("4G handles")
    }

    pub fn pos(&self) -> usize {
        self.bytes.len()
    }

    /// Zeroes up to the next multiple of `align`.
    pub fn pad(&mut self, align: usize) {
        let n = self.bytes.len().next_multiple_of(align);
        self.bytes.resize(n, 0);
    }

    /// Room for `n` zeroed `T`s, aligned for `T`: where they start.
    pub fn room<T>(&mut self, n: usize) -> usize {
        debug_assert!(align_of::<T>() <= ALIGN, "a view aligned past a block's regions");
        self.pad(align_of::<T>());
        let at = self.bytes.len();
        self.bytes.resize(at + n * size_of::<T>(), 0);
        at
    }

    /// The bytes at `at`, as a `T`'s place.
    ///
    /// # Safety
    /// `at` is aligned for `T` and `size_of::<T>()` bytes from it lie in the section.
    pub unsafe fn ptr<T>(&mut self, at: usize) -> *mut T {
        debug_assert!(at + size_of::<T>() <= self.bytes.len());
        unsafe { self.bytes.as_mut_ptr().add(at) as *mut T }
    }

    /// Plain bytes, unaligned: where they start.
    pub fn bytes(&mut self, b: &[u8]) -> usize {
        let at = self.bytes.len();
        self.bytes.extend_from_slice(b);
        at
    }

    /// Plain values, aligned for `T`, as they lie in memory: where they start.
    pub fn plain<T: Pod + Copy>(&mut self, v: &[T]) -> usize {
        let at = self.room::<T>(v.len());
        // SAFETY: room for `v` at `at`, aligned for `T`; `T` is plain data.
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), self.ptr::<T>(at), v.len()) };
        at
    }

    /// Writes `v` (its out-of-line data, then its record): where its record starts.
    pub fn write<T: Archive + ?Sized>(&mut self, v: &T) -> usize {
        let r = v.write_out(self);
        let at = self.room::<T::Archived>(1);
        // SAFETY: zeroed room for one `T::Archived` at `at`, aligned; `r` is `v`'s.
        unsafe { v.resolve(at, r, self.ptr(at)) };
        at
    }

    /// The section's bytes so far.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// The section of `root`: everything written, then `root`, then where `root` starts (8 bytes).
    pub fn finish<T: Archive>(mut self, root: &T) -> Section<T> {
        let at = self.write(root);
        self.pad(ALIGN);
        self.bytes.extend_from_slice(&(at as u64).to_le_bytes());
        Section { bytes: self.bytes, handles: self.handles, _t: PhantomData }
    }
}

/// One tree's section: its bytes in a block, its root last (`Writer::finish`), and the objects it names by number.
pub struct Section<T: Archive> {
    bytes: Vector<u8>,
    handles: Handles,
    _t: PhantomData<fn() -> T>,
}

impl<T: Archive> Section<T> {
    /// `v` encoded.
    pub fn of(v: &T) -> Self {
        Writer::new().finish(v)
    }

    /// A section's bytes as written (`as_bytes`), read back from where they were kept.
    ///
    /// # Safety
    /// `bytes` are a `Section<T>`'s, of this build's `T`.
    pub unsafe fn from_bytes(bytes: Vector<u8>) -> Self {
        Section { bytes, handles: Vec::new(), _t: PhantomData }
    }

    /// The object `h` names (`Bound`).
    pub fn bound<H: Any + Send + Sync>(&self, h: &Handle<H>) -> &Arc<H> {
        let any = &self.handles[h.n as usize];
        // SAFETY: `h` was written for an `Arc<H>` (`Bound<H>`'s `write_out`), its number this section's.
        debug_assert!(any.downcast_ref::<H>().is_some(), "a handle of another type");
        unsafe { &*(any as *const Arc<dyn Any + Send + Sync> as *const Arc<H>) }
    }

    /// The objects its handles name, for a `Decode` of its tree.
    pub fn names(&self) -> Names<'_> {
        Names(&self.handles)
    }

    /// Its tree made again (`Decode`): for a rewrite into another tree.
    pub fn decode(&self) -> T
    where
        T: Decode,
    {
        T::decode(self.get(), &self.names())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vector<u8> {
        self.bytes
    }

    /// Its root's view.
    pub fn get(&self) -> &T::Archived {
        // SAFETY: its bytes are a `Section<T>`'s (`of`, `from_bytes`).
        unsafe { root::<T>(&self.bytes) }
    }
}

impl<T: Archive> std::ops::Deref for Section<T> {
    type Target = T::Archived;
    fn deref(&self) -> &T::Archived {
        self.get()
    }
}

impl<T: Archive> Clone for Section<T> {
    fn clone(&self) -> Self {
        // copy: derived: a second holder of one tree's section; a plan held twice is held through an `Arc`.
        Section { bytes: self.bytes.clone(), handles: self.handles.clone(), _t: PhantomData }
    }
}

impl<T: Archive> PartialEq for Section<T>
where
    T::Archived: PartialEq,
{
    fn eq(&self, o: &Self) -> bool {
        self.get() == o.get()
    }
}

impl<T: Archive> Eq for Section<T> where T::Archived: Eq {}

impl<T: Archive> Hash for Section<T>
where
    T::Archived: Hash,
{
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.get().hash(h)
    }
}

impl<T: Archive> fmt::Debug for Section<T>
where
    T::Archived: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// The root of a section's bytes (`Writer::finish`).
///
/// # Safety
/// `bytes` are a `Section<T>`'s, aligned to `ALIGN`.
pub unsafe fn root<T: Archive>(bytes: &[u8]) -> &T::Archived {
    let n = bytes.len();
    let at = u64::from_le_bytes(bytes[n - 8..].try_into().expect("a section's root")) as usize;
    debug_assert!(at + size_of::<T::Archived>() <= n - 8);
    debug_assert_eq!(bytes.as_ptr() as usize % align_of::<T::Archived>(), 0);
    unsafe { &*(bytes.as_ptr().add(at) as *const T::Archived) }
}

/// The offset from `at` to `pos`, as a relative field holds it.
fn rel(at: usize, pos: usize) -> i32 {
    i32::try_from(pos as i64 - at as i64).expect("a section past 2 GB")
}

/// The address `off` bytes from `field`.
///
/// # Safety
/// `field` lies in a section and `off` was written for it.
unsafe fn from<F, T>(field: &F, off: i32) -> *const T {
    unsafe { (field as *const F as *const u8).offset(off as isize) as *const T }
}

/// A list of views out of line: its first's offset from this field, and its length.
#[repr(C)]
pub struct RelList<T> {
    off: i32,
    len: u32,
    _t: PhantomData<T>,
}

impl<T> RelList<T> {
    pub fn as_slice(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: `off` names `len` `T`s written for this field (`resolve_list`).
        unsafe { std::slice::from_raw_parts(from(self, self.off), self.len as usize) }
    }

    /// Writes a list's field at `out` (at `at`): its `len` items at `pos`.
    ///
    /// # Safety
    /// `out` is a `RelList<T>`'s place at `at`.
    pub unsafe fn resolve(at: usize, pos: usize, len: usize, out: *mut Self) {
        let len = u32::try_from(len).expect("a list of 4G items");
        let off = if len == 0 { 0 } else { rel(at, pos) };
        unsafe { out.write(RelList { off, len, _t: PhantomData }) };
    }
}

impl<T> std::ops::Deref for RelList<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<'a, T> IntoIterator for &'a RelList<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<T: PartialEq> PartialEq for RelList<T> {
    fn eq(&self, o: &Self) -> bool {
        self.as_slice() == o.as_slice()
    }
}

impl<T: Eq> Eq for RelList<T> {}

impl<T: Hash> Hash for RelList<T> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.as_slice().hash(h)
    }
}

impl<T: fmt::Debug> fmt::Debug for RelList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

/// A text out of line: its bytes' offset from this field, and its length.
#[repr(C)]
pub struct RelStr {
    off: i32,
    len: u32,
}

impl RelStr {
    pub fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        // SAFETY: `off` names `len` bytes of a `str` written for this field.
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(from(self, self.off), self.len as usize)) }
    }

    /// # Safety
    /// `out` is a `RelStr`'s place at `at`.
    pub unsafe fn resolve(at: usize, pos: usize, len: usize, out: *mut Self) {
        let len = u32::try_from(len).expect("a text of 4 GB");
        let off = if len == 0 { 0 } else { rel(at, pos) };
        unsafe { out.write(RelStr { off, len }) };
    }
}

impl std::ops::Deref for RelStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for RelStr {
    fn eq(&self, o: &Self) -> bool {
        self.as_str() == o.as_str()
    }
}

impl PartialEq<str> for RelStr {
    fn eq(&self, o: &str) -> bool {
        self.as_str() == o
    }
}

impl PartialEq<&str> for RelStr {
    fn eq(&self, o: &&str) -> bool {
        self.as_str() == *o
    }
}

impl Eq for RelStr {}

impl Hash for RelStr {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.as_str().hash(h)
    }
}

impl fmt::Debug for RelStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

impl fmt::Display for RelStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

/// One view out of line (a `Box`'s or an `Arc`'s value): its offset from this field.
#[repr(C)]
pub struct RelBox<T> {
    off: i32,
    _t: PhantomData<T>,
}

impl<T> RelBox<T> {
    pub fn get(&self) -> &T {
        // SAFETY: `off` names a `T` written for this field.
        unsafe { &*from(self, self.off) }
    }
}

impl<T> std::ops::Deref for RelBox<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T: PartialEq> PartialEq for RelBox<T> {
    fn eq(&self, o: &Self) -> bool {
        self.get() == o.get()
    }
}

impl<T: Eq> Eq for RelBox<T> {}

impl<T: Hash> Hash for RelBox<T> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.get().hash(h)
    }
}

impl<T: fmt::Debug> fmt::Debug for RelBox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// An `Option`'s view: its tag, then its value.
#[repr(u32)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum ArchivedOption<T> {
    None,
    Some(T),
}

impl<T> ArchivedOption<T> {
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            ArchivedOption::None => None,
            ArchivedOption::Some(v) => Some(v),
        }
    }

    pub fn is_some(&self) -> bool {
        matches!(self, ArchivedOption::Some(_))
    }

    pub fn is_none(&self) -> bool {
        !self.is_some()
    }
}

impl<T: Copy> ArchivedOption<T> {
    pub fn get(&self) -> Option<T> {
        self.as_ref().copied()
    }

    /// `Option::map`, of the value read.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Option<U> {
        self.get().map(f)
    }

    /// `Option::map_or`, of the value read.
    pub fn map_or<U>(self, default: U, f: impl FnOnce(T) -> U) -> U {
        self.get().map_or(default, f)
    }

    /// `Option::is_some_and`, of the value read.
    pub fn is_some_and(self, f: impl FnOnce(T) -> bool) -> bool {
        self.get().is_some_and(f)
    }

    /// `Option::iter`: the value, if any.
    pub fn iter(&self) -> std::option::IntoIter<&T> {
        self.as_ref().into_iter()
    }
}

impl<'a, T> IntoIterator for &'a ArchivedOption<T> {
    type Item = &'a T;
    type IntoIter = std::option::IntoIter<&'a T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_ref().into_iter()
    }
}

/// A tuple's view.
#[repr(C)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Tuple2<A, B>(pub A, pub B);

#[repr(C)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Tuple3<A, B, C>(pub A, pub B, pub C);

macro_rules! plain {
    ($($t:ty => $a:ty),* $(,)?) => {$(
        // SAFETY: a primitive is its own record.
        unsafe impl Archive for $t {
            type Archived = $a;
            type Resolver = ();
            #[inline]
            fn write_out(&self, _: &mut Writer) {}
            #[inline]
            unsafe fn resolve(&self, _: usize, _: (), out: *mut $a) {
                unsafe { out.write(*self as $a) }
            }
        }
    )*};
}

plain!(u8 => u8, u16 => u16, u32 => u32, u64 => u64, i8 => i8, i16 => i16, i32 => i32, i64 => i64,
    f32 => f32, f64 => f64, usize => u64, isize => i64);

// SAFETY: `bool` is its own record (written 0 or 1).
unsafe impl Archive for bool {
    type Archived = bool;
    type Resolver = ();
    fn write_out(&self, _: &mut Writer) {}
    unsafe fn resolve(&self, _: usize, _: (), out: *mut bool) {
        unsafe { out.write(*self) }
    }
}

// SAFETY: a `char` is its scalar value.
unsafe impl Archive for char {
    type Archived = u32;
    type Resolver = ();
    fn write_out(&self, _: &mut Writer) {}
    unsafe fn resolve(&self, _: usize, _: (), out: *mut u32) {
        unsafe { out.write(*self as u32) }
    }
}

// SAFETY: nothing to write.
unsafe impl Archive for () {
    type Archived = ();
    type Resolver = ();
    fn write_out(&self, _: &mut Writer) {}
    unsafe fn resolve(&self, _: usize, _: (), _: *mut ()) {}
}

// SAFETY: a text's bytes out of line, then its `RelStr`.
unsafe impl Archive for str {
    type Archived = RelStr;
    type Resolver = usize;
    fn write_out(&self, w: &mut Writer) -> usize {
        w.bytes(self.as_bytes())
    }
    unsafe fn resolve(&self, at: usize, pos: usize, out: *mut RelStr) {
        unsafe { RelStr::resolve(at, pos, self.len(), out) }
    }
}

macro_rules! text {
    ($($t:ty),*) => {$(
        // SAFETY: as `str`'s.
        unsafe impl Archive for $t {
            type Archived = RelStr;
            type Resolver = usize;
            fn write_out(&self, w: &mut Writer) -> usize {
                <str as Archive>::write_out(self, w)
            }
            unsafe fn resolve(&self, at: usize, pos: usize, out: *mut RelStr) {
                unsafe { <str as Archive>::resolve(self, at, pos, out) }
            }
        }
    )*};
}

text!(String, Box<str>, std::sync::Arc<str>);

/// Writes `items`' out-of-line data, then their records: where the records start.
pub fn write_list<T: Archive>(items: &[T], w: &mut Writer) -> usize {
    if items.is_empty() {
        return 0;
    }
    // copy: not block data: the encoder's resolvers, gone once the list's records are written.
    let rs: Vec<T::Resolver> = items.iter().map(|x| x.write_out(w)).collect();
    let pos = w.room::<T::Archived>(items.len());
    for (i, (x, r)) in items.iter().zip(rs).enumerate() {
        let at = pos + i * size_of::<T::Archived>();
        // SAFETY: zeroed room for the list's `i`th record at `at`.
        unsafe { x.resolve(at, r, w.ptr(at)) };
    }
    pos
}

// SAFETY: its items' records out of line (`write_list`), then its `RelList`.
unsafe impl<T: Archive> Archive for [T] {
    type Archived = RelList<T::Archived>;
    type Resolver = usize;
    fn write_out(&self, w: &mut Writer) -> usize {
        write_list(self, w)
    }
    unsafe fn resolve(&self, at: usize, pos: usize, out: *mut Self::Archived) {
        unsafe { RelList::resolve(at, pos, self.len(), out) }
    }
}

macro_rules! list {
    ($($t:ty where $($b:path),*);* $(;)?) => {$(
        // SAFETY: as `[T]`'s.
        unsafe impl<T: Archive $(+ $b)*> Archive for $t {
            type Archived = RelList<T::Archived>;
            type Resolver = usize;
            fn write_out(&self, w: &mut Writer) -> usize {
                write_list(self, w)
            }
            unsafe fn resolve(&self, at: usize, pos: usize, out: *mut Self::Archived) {
                unsafe { RelList::resolve(at, pos, self.len(), out) }
            }
        }
    )*};
}

list!(Vec<T> where; Box<[T]> where; std::sync::Arc<[T]> where; Vector<T> where Pod);

// SAFETY: its items' records in place.
unsafe impl<T: Archive, const N: usize> Archive for [T; N] {
    type Archived = [T::Archived; N];
    type Resolver = [T::Resolver; N];
    fn write_out(&self, w: &mut Writer) -> Self::Resolver {
        std::array::from_fn(|i| self[i].write_out(w))
    }
    unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived) {
        for (i, r) in r.into_iter().enumerate() {
            let size = size_of::<T::Archived>();
            unsafe { self[i].resolve(at + i * size, r, (out as *mut T::Archived).add(i)) };
        }
    }
}

macro_rules! boxed {
    ($($t:ident),*) => {$(
        // SAFETY: its value's record out of line, then its `RelBox`.
        unsafe impl<T: Archive> Archive for $t<T> {
            type Archived = RelBox<T::Archived>;
            type Resolver = usize;
            fn write_out(&self, w: &mut Writer) -> usize {
                w.write::<T>(self)
            }
            unsafe fn resolve(&self, at: usize, pos: usize, out: *mut Self::Archived) {
                unsafe { out.write(RelBox { off: rel(at, pos), _t: PhantomData }) }
            }
        }
    )*};
}

boxed!(Box, Arc, Rc);

/// A run's object a section names by number: in the section, its `Handle`; the object is held with the section
/// (`Section::bound`). Compared and hashed as the object's value is.
pub struct Bound<H>(pub Arc<H>);

impl<H> Bound<H> {
    pub fn new(h: H) -> Self {
        Bound(Arc::new(h))
    }
}

impl<H> Clone for Bound<H> {
    fn clone(&self) -> Self {
        Bound(Arc::clone(&self.0))
    }
}

impl<H> std::ops::Deref for Bound<H> {
    type Target = Arc<H>;
    fn deref(&self) -> &Arc<H> {
        &self.0
    }
}

impl<H: PartialEq> PartialEq for Bound<H> {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0) || *self.0 == *o.0
    }
}

impl<H: Eq> Eq for Bound<H> {}

impl<H: Hash> Hash for Bound<H> {
    fn hash<S: Hasher>(&self, h: &mut S) {
        self.0.hash(h)
    }
}

impl<H: fmt::Debug> fmt::Debug for Bound<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A `Bound`'s number in its section.
#[repr(C)]
pub struct Handle<H> {
    n: u32,
    _t: PhantomData<fn() -> H>,
}

impl<H> Handle<H> {
    pub fn index(&self) -> u32 {
        self.n
    }
}

impl<H> PartialEq for Handle<H> {
    fn eq(&self, o: &Self) -> bool {
        self.n == o.n
    }
}

impl<H> Eq for Handle<H> {}

impl<H> Hash for Handle<H> {
    fn hash<S: Hasher>(&self, h: &mut S) {
        self.n.hash(h)
    }
}

impl<H> fmt::Debug for Handle<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.n)
    }
}

// SAFETY: a number.
unsafe impl<H: Any + Send + Sync> Archive for Bound<H> {
    type Archived = Handle<H>;
    type Resolver = u32;
    fn write_out(&self, w: &mut Writer) -> u32 {
        w.bind(self.0.clone())
    }
    unsafe fn resolve(&self, _: usize, n: u32, out: *mut Handle<H>) {
        unsafe { out.write(Handle { n, _t: PhantomData }) }
    }
}

#[repr(C)]
struct OptionSome<T> {
    tag: u32,
    v: T,
}

// SAFETY: `ArchivedOption` is `repr(u32)`: its tag, then its value as `OptionSome` lays it.
unsafe impl<T: Archive> Archive for Option<T> {
    type Archived = ArchivedOption<T::Archived>;
    type Resolver = Option<T::Resolver>;
    fn write_out(&self, w: &mut Writer) -> Self::Resolver {
        self.as_ref().map(|v| v.write_out(w))
    }
    unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived) {
        unsafe {
            match (self, r) {
                (Some(v), Some(r)) => {
                    let out = out as *mut OptionSome<T::Archived>;
                    std::ptr::addr_of_mut!((*out).tag).write(1);
                    let p = std::ptr::addr_of_mut!((*out).v);
                    v.resolve(at + (p as usize - out as usize), r, p);
                }
                _ => (out as *mut u32).write(0),
            }
        }
    }
}

macro_rules! tuple {
    ($view:ident: $($t:ident $i:tt),*) => {
        // SAFETY: a `repr(C)` record of its items' records.
        unsafe impl<$($t: Archive),*> Archive for ($($t,)*) {
            type Archived = $view<$($t::Archived),*>;
            type Resolver = ($($t::Resolver,)*);
            fn write_out(&self, w: &mut Writer) -> Self::Resolver {
                ($(self.$i.write_out(w),)*)
            }
            unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived) {
                unsafe {
                    $(
                        let p = std::ptr::addr_of_mut!((*out).$i);
                        self.$i.resolve(at + (p as usize - out as usize), r.$i, p);
                    )*
                }
            }
        }
    };
}

tuple!(Tuple2: A 0, B 1);
tuple!(Tuple3: A 0, B 1, C 2);

/// A value that is its own record: copied in as it lies in memory, read in place as itself (a slot number, a key, a
/// step's kind). It names nothing (no pointer, no reference, no handle), so it reads the same wherever its section
/// lies in the process that wrote it. A device reads records of its own layout (`repr(C)`), not these.
///
/// # Safety
/// No pointer or reference lies in it, it is aligned to at most `ALIGN`, and every value of it is valid to copy.
pub unsafe trait Plain: Copy + 'static {}

macro_rules! plain_types {
    ($($t:ty),*) => {$(
        // SAFETY: a primitive names nothing.
        unsafe impl Plain for $t {}
    )*};
}

plain_types!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, usize, isize, bool, char, ());

// SAFETY: as its parts'.
unsafe impl<T: Plain> Plain for Option<T> {}
unsafe impl<A: Plain, B: Plain> Plain for (A, B) {}
unsafe impl<A: Plain, B: Plain, C: Plain> Plain for (A, B, C) {}
unsafe impl<T: Plain, const N: usize> Plain for [T; N] {}

/// `Archive` for a `Plain` type of this crate: its record is itself.
#[macro_export]
macro_rules! plain_archive {
    ($($t:ty),* $(,)?) => {$(
        // SAFETY: a `Plain` value is its own record.
        unsafe impl $crate::Plain for $t {}
        // SAFETY: as `Plain`'s.
        unsafe impl $crate::Archive for $t {
            type Archived = $t;
            type Resolver = ();
            #[inline]
            fn write_out(&self, _: &mut $crate::Writer) {}
            #[inline]
            unsafe fn resolve(&self, _: usize, _: (), out: *mut $t) {
                unsafe { out.write(*self) }
            }
        }
        impl $crate::Decode for $t {
            fn decode(a: &$t, _: &$crate::Names<'_>) -> Self {
                *a
            }
        }
    )*};
}

/// A field archived flat (`#[archive(flat)]`): a `Plain` value as itself, a list of them as a `RelList` of
/// themselves, their bytes copied in as they lie.
///
/// # Safety
/// As `Archive`'s.
pub unsafe trait Flat {
    type Archived;
    type Resolver;
    fn write_out(&self, w: &mut Writer) -> Self::Resolver;
    /// # Safety
    /// As `Archive::resolve`'s.
    unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived);
}

// SAFETY: a `Plain` value is its own record.
unsafe impl<T: Plain> Flat for T {
    type Archived = T;
    type Resolver = ();
    #[inline]
    fn write_out(&self, _: &mut Writer) {}
    #[inline]
    unsafe fn resolve(&self, _: usize, _: (), out: *mut T) {
        unsafe { out.write(*self) }
    }
}

impl Writer {
    /// `Plain` values, aligned for `T`, as they lie in memory: where they start.
    pub fn plain_items<T: Plain>(&mut self, v: &[T]) -> usize {
        if v.is_empty() {
            return 0;
        }
        let at = self.room::<T>(v.len());
        // SAFETY: room for `v` at `at`, aligned for `T`; `T` names nothing.
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), self.ptr::<T>(at), v.len()) };
        at
    }
}

macro_rules! flat_list {
    ($($t:ty where $($b:path),*);* $(;)?) => {$(
        // SAFETY: its items copied in (`plain_items`), then its `RelList`.
        unsafe impl<T: Plain $(+ $b)*> Flat for $t {
            type Archived = RelList<T>;
            type Resolver = usize;
            fn write_out(&self, w: &mut Writer) -> usize {
                w.plain_items(self)
            }
            unsafe fn resolve(&self, at: usize, pos: usize, out: *mut RelList<T>) {
                unsafe { RelList::resolve(at, pos, self.len(), out) }
            }
        }
    )*};
}

flat_list!(Vec<T> where; Vector<T> where Pod);

/// A flat field made again from its view (`#[archive(flat)]`, `Decode`).
pub trait FlatDecode: Flat + Sized {
    fn decode(a: &Self::Archived) -> Self;
}

impl<T: Plain> FlatDecode for T {
    fn decode(a: &T) -> T {
        *a
    }
}

impl<T: Plain> FlatDecode for Vec<T> {
    fn decode(a: &RelList<T>) -> Self {
        a.to_vec()
    }
}

impl<T: Plain + Pod> FlatDecode for Vector<T> {
    fn decode(a: &RelList<T>) -> Self {
        Vector::from(a.as_slice())
    }
}

// SAFETY: as its value's.
unsafe impl<T: Archive + ?Sized> Archive for &T {
    type Archived = T::Archived;
    type Resolver = T::Resolver;
    fn write_out(&self, w: &mut Writer) -> Self::Resolver {
        (**self).write_out(w)
    }
    unsafe fn resolve(&self, at: usize, r: Self::Resolver, out: *mut Self::Archived) {
        unsafe { (**self).resolve(at, r, out) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Archive, Debug, PartialEq)]
    #[archive(derive(Debug, PartialEq), decode)]
    enum Tok {
        Wild,
        Slot(usize),
        Sym(String),
        Pair { a: Box<Tok>, b: Option<(u32, bool)> },
    }

    #[derive(Archive)]
    #[archive(derive(Debug))]
    struct Spec {
        name: String,
        toks: Vec<Tok>,
        from: Vector<u32>,
        est: f64,
    }

    #[test]
    fn a_tree_reads_in_place() {
        let spec = Spec {
            name: "rule".into(),
            toks: vec![
                Tok::Wild,
                Tok::Slot(7),
                Tok::Sym("x.y".into()),
                Tok::Pair { a: Box::new(Tok::Slot(3)), b: Some((9, true)) },
            ],
            from: Vector::from(&[1u32, 2, 3][..]),
            est: 2.5,
        };
        let s = Section::of(&spec);
        let v = s.get();
        assert_eq!(v.name.as_str(), "rule");
        assert_eq!(v.from.as_slice(), &[1, 2, 3]);
        assert_eq!(v.est, 2.5);
        assert_eq!(v.toks.len(), 4);
        assert!(matches!(v.toks[0], ArchivedTok::Wild));
        assert!(matches!(v.toks[1], ArchivedTok::Slot(7)));
        match &v.toks[2] {
            ArchivedTok::Sym(s) => assert_eq!(s.as_str(), "x.y"),
            t => panic!("{t:?}"),
        }
        match &v.toks[3] {
            ArchivedTok::Pair { a, b } => {
                assert!(matches!(**a, ArchivedTok::Slot(3)));
                assert_eq!(b.get(), Some(Tuple2(9, true)));
            }
            t => panic!("{t:?}"),
        }
        // A section reads the same wherever its bytes lie.
        let moved = Vector::from(s.as_bytes());
        let m = unsafe { Section::<Spec>::from_bytes(moved) };
        assert_eq!(m.toks[3], v.toks[3]);
        // A rewrite starts from the view made a value again.
        assert_eq!(Tok::decode(&v.toks[3], &Names::NONE), spec.toks[3]);
        assert_eq!(Tok::decode(&v.toks[2], &Names::NONE), Tok::Sym("x.y".into()));
    }

    #[test]
    fn a_handle_names_its_object() {
        #[derive(Archive)]
        struct Line {
            head: u32,
            join: Bound<Section<Spec2>>,
        }
        #[derive(Archive)]
        #[archive(derive(Debug, PartialEq))]
        struct Spec2 {
            name: String,
        }
        let j = Bound::new(Section::of(&Spec2 { name: "j".into() }));
        let lines = vec![Line { head: 1, join: j.clone() }, Line { head: 2, join: j.clone() }];
        let s = Section::of(&lines);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].join, s[1].join, "one object, one number");
        assert_eq!(s.bound(&s[1].join).name.as_str(), "j");
    }
}
