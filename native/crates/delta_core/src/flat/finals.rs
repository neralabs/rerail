//! The final forms (*Flat Views and Lifecycles* §3, §5): shaped for reading where they are kept, each settled exactly
//! from a working form ([`Settle`]), every reference inside its room.
//!
//! - [`IntTable`]: integers in the fewest bits (MLDB's `RawMappedIntTable`): a 16-byte descriptor, its encoding's tag
//!   and a `Bit32` span of its body, which holds the encoding's parameters and then `IntCol`'s words. Its encodings
//!   and reads are `IntCol`'s (`crate::col`), and its encoder `flat::encode`.
//! - [`Strings`]: byte strings, an `IntTable` of their ends over one run of bytes (MLDB's `MappedStringTable`).
//! - [`Sorted`]: keys sorted with their values beside (MLDB's `MappedSortedKeyValueArray`), from a `Map`.
//! - [`Csr`]: lists in one run, each list's start and then its items in order (a relation's index), from a `Chain`.
//! - `Bloom` (the floor's filter, `crate::bloom`): its words, from keys.

use super::arena::{Arena, Full};
use super::encode::{self as enc, Choice, RUNS_FOR_READS, Serial, U32s, Vals};
use super::map::{Map, Word};
use super::pool::{ChainRead, Link};
use super::settle::{Need, Room, Settle};
use super::{BitSpan, RelSpan};
use crate::Pod;
use crate::bloom::{self, Bloom};
use crate::col::{CONST, DICT, Dec, Fields, GROUPS, LINE, RUNS, words};
use crate::refs::Span;

impl Need {
    /// And `o`.
    pub const fn plus(self, o: Need) -> Need {
        Need { bytes: self.bytes + o.bytes }
    }
}

crate::structure! {
    /// Integers in the fewest bits: `n` of them, as `IntCol`'s encoding `tag` holds them in its body.
    pub struct IntTable {
        /// `IntCol`'s tag, its value bits and its second bits (`rbits`).
        pub tag: u8,
        pub bits: u8,
        pub rbits: u8,
        /// Words of the encoding's parameters at the body's front: its base; `LINE`'s slope and residuals' minimum;
        /// `RUNS`', `GROUPS`' or `DICT`'s count.
        pub params: u8,
        pub n: u32,
        /// Its body, 64-bit words: the parameters, the data words, then the second words.
        pub body: BitSpan,
    }
}

/// Parameter words of an encoding.
fn params_of(tag: u32) -> usize {
    match tag {
        LINE => 3,
        RUNS | GROUPS | DICT => 2,
        _ => 1,
    }
}

impl IntTable {
    /// A table of nothing yet (its body set by a settle).
    pub fn empty() -> IntTable {
        IntTable { tag: CONST as u8, bits: 0, rbits: 0, params: 1, n: 0, body: BitSpan::empty() }
    }

    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Its body's words, where they lie.
    pub fn body(&self) -> &[u64] {
        let Some(at) = self.body.target_bits() else { return &[] };
        debug_assert!(at.is_multiple_of(64));
        // SAFETY: settled inside its room (`fill`), 8-byte aligned, `bits / 64` words, read while the table lives.
        unsafe { core::slice::from_raw_parts(core::ptr::with_exposed_provenance::<u64>((at / 8) as usize), (self.body.bits() / 64) as usize) }
    }

    /// Resolved for reads: `IntCol`'s reads over its words.
    pub fn dec(&self) -> Dec<'_> {
        let b = self.body();
        let (tag, bits, rbits, p) = (self.tag as u32, self.bits as u32, self.rbits as u32, self.params as usize);
        let nruns = if p >= 2 && tag != LINE { b[1] as usize } else { 0 };
        let rw = match tag {
            RUNS => words(nruns, rbits),
            GROUPS => words(nruns, bits) + words(nruns, rbits),
            DICT => words(nruns, bits),
            _ => 0,
        };
        let dw = b.len() - p - rw;
        let (slope, rmin) = if tag == LINE { (b[1] as i64, b[2] as i64) } else { (0, 0) };
        let f = Fields { tag, bits, rbits, n: self.n as usize, nruns, base: b[0] as i64, slope, rmin };
        Dec::of(f, &b[p..p + dw], &b[p + dw..])
    }

    /// Value `i`.
    pub fn get(&self, i: usize) -> i64 {
        assert!(i < self.len(), "value {i} of a table of {}", self.len());
        self.dec().get(i)
    }
}

/// Integers to settle as an `IntTable`: their values (read where they lie) and the encoding chosen for them.
pub struct IntFrom<'w, V: Vals> {
    v: V,
    c: Choice,
    /// `DICT`'s distinct values, sorted, in the work arena.
    dict: &'w [i64],
}

impl<'w, V: Vals> IntFrom<'w, V> {
    /// `v`'s smallest encoding for reads at random (`RUNS` only with few runs), `DICT` among them where a work arena
    /// is given to count the distinct values in.
    pub fn plan(v: V, work: Option<&'w Arena<'_>>, part: u32) -> Result<IntFrom<'w, V>, Full> {
        let (mut found, mut err): (Option<&'w [i64]>, Option<Full>) = (None, None);
        let c = match work {
            None => enc::choose(&v, RUNS_FOR_READS, &Serial, None),
            Some(w) => {
                let mut count = |limit: usize| match enc::distinct_in(&v, limit, w, part) {
                    Ok(Some(d)) => {
                        let k = d.len();
                        found = Some(d);
                        Some(k)
                    }
                    Ok(None) => None,
                    Err(f) => {
                        err = Some(f);
                        None
                    }
                };
                enc::choose(&v, RUNS_FOR_READS, &Serial, Some(&mut count))
            }
        };
        if let Some(f) = err {
            return Err(f);
        }
        let dict: &'w [i64] = if c.tag == DICT { found.expect("DICT's distinct values") } else { &[] };
        Ok(IntFrom { v, c, dict })
    }

    pub fn choice(&self) -> &Choice {
        &self.c
    }

    /// `DICT`'s distinct values, sorted (empty for any other encoding).
    pub fn dict(&self) -> &'w [i64] {
        self.dict
    }

    /// Its values, read where they lie.
    pub fn vals(&self) -> &V {
        &self.v
    }

    fn body_words(&self) -> usize {
        params_of(self.c.tag) + if self.c.tag == CONST { 0 } else { self.c.data_words + self.c.runs_words }
    }

    /// Its body's bytes, without the descriptor.
    pub fn need_body(&self) -> Need {
        Need::none().and::<u64>(self.body_words())
    }

    /// Writes its body into `room` and its fields into `t` (in the same room).
    pub fn fill<'d, 't>(&self, t: &'t mut IntTable, room: &mut Room<'d>) -> &'t IntTable {
        let c = &self.c;
        let p = params_of(c.tag);
        let body = room.take_with::<u64>(self.body_words(), 0);
        body[0] = c.base as u64;
        match c.tag {
            LINE => (body[1], body[2]) = (c.slope as u64, c.rmin as u64),
            RUNS | GROUPS | DICT => body[1] = c.nruns as u64,
            _ => {}
        }
        if c.tag != CONST {
            let (data, runs) = body[p..].split_at_mut(c.data_words);
            enc::write(c, &self.v, data, runs, self.dict, &Serial);
        }
        let nbits = body.len() as u64 * 64;
        assert!(nbits <= u32::MAX as u64 && c.n <= u32::MAX as usize, "an integer table past 2^32 bits");
        t.body.set_bits(room.region(), body.as_ptr().addr() as u64 * 8, nbits).expect("a table's body beside its descriptor");
        (t.tag, t.bits, t.rbits, t.params, t.n) = (c.tag as u8, c.bits as u8, c.rbits as u8, p as u8, c.n as u32);
        t
    }
}

impl<V: Vals> Settle for IntFrom<'_, V> {
    type Final = IntTable;

    fn need(&self) -> Need {
        Need::none().and::<IntTable>(1).plus(self.need_body())
    }

    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d IntTable {
        let t = room.take::<IntTable>(1)[0].write(IntTable::empty());
        self.fill(t, room)
    }
}

crate::structure! {
    /// Byte strings: string `i` is `bytes[ends[i - 1]..ends[i]]` (`ends[-1]` is 0).
    pub struct Strings {
        pub ends: IntTable,
        pub bytes: RelSpan<u8>,
    }
}

impl Strings {
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// String `i`, where it lies.
    pub fn get(&self, i: usize) -> &[u8] {
        let d = self.ends.dec();
        let (from, to) = (if i == 0 { 0 } else { d.get(i - 1) as usize }, d.get(i) as usize);
        &self.bytes.as_slice()[from..to]
    }
}

/// Strings to settle: their bytes, one run, and each one's end (a `Grow<u8>` and a `Grow<u32>`'s items).
pub struct StringsFrom<'s> {
    bytes: &'s [u8],
    ends: IntFrom<'s, U32s<'s>>,
}

impl<'s> StringsFrom<'s> {
    pub fn plan(bytes: &'s [u8], ends: &'s [u32]) -> StringsFrom<'s> {
        assert!(ends.last().is_none_or(|&e| e as usize == bytes.len()) && ends.is_sorted(), "a string table's ends, rising to its bytes' end");
        StringsFrom { bytes, ends: IntFrom::plan(U32s(ends), None, 0).expect("a plan with no work arena cannot run out") }
    }
}

impl Settle for StringsFrom<'_> {
    type Final = Strings;

    fn need(&self) -> Need {
        Need::none().and::<Strings>(1).plus(self.ends.need_body()).and::<u8>(self.bytes.len())
    }

    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d Strings {
        let s = room.take::<Strings>(1)[0].write(Strings { ends: IntTable::empty(), bytes: RelSpan::empty() });
        self.ends.fill(&mut s.ends, room);
        let b = room.take::<u8>(self.bytes.len());
        for (x, &v) in b.iter_mut().zip(self.bytes) {
            x.write(v);
        }
        s.bytes.set(room.region(), b.as_ptr().cast(), b.len()).expect("a string table's bytes beside it");
        s
    }
}

/// Keys sorted, their values beside: a search of the keys, then the value at the same place.
#[repr(C)]
pub struct Sorted<K, V> {
    keys: RelSpan<K>,
    vals: RelSpan<V>,
}

// SAFETY: two spans, no padding.
unsafe impl<K: 'static, V: 'static> Pod for Sorted<K, V> {}

impl<K: Pod + Ord + Copy, V: Pod + Copy> Sorted<K, V> {
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn keys(&self) -> &[K] {
        self.keys.as_slice()
    }

    pub fn vals(&self) -> &[V] {
        self.vals.as_slice()
    }

    /// The first place whose key is not below `k`.
    pub fn lower_bound(&self, k: K) -> usize {
        self.keys().partition_point(|&x| x < k)
    }

    /// `k`'s value, or none.
    pub fn get(&self, k: K) -> Option<V> {
        self.keys().binary_search(&k).ok().map(|i| self.vals()[i])
    }
}

/// A map's entries to settle as a `Sorted`.
pub struct SortedFrom<'m, K, V>(pub &'m Map<K, V>);

impl<K: Word + Pod + Ord, V: Word + Pod> Settle for SortedFrom<'_, K, V> {
    type Final = Sorted<K, V>;

    fn need(&self) -> Need {
        let n = self.0.len();
        Need::none().and::<Sorted<K, V>>(1).and::<K>(n).and::<V>(n)
    }

    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d Sorted<K, V> {
        let n = self.0.len();
        let s = room.take::<Sorted<K, V>>(1)[0].write(Sorted { keys: RelSpan::empty(), vals: RelSpan::empty() });
        let (ks, vs) = (room.take::<K>(n), room.take::<V>(n));
        let mut i = 0;
        for (k, v) in self.0.iter() {
            ks[i].write(k);
            vs[i].write(v);
            i += 1;
        }
        assert_eq!(i, n, "a map settled while it changed");
        // SAFETY: every item written.
        let (ks, vs) = unsafe { (&mut *(core::ptr::from_mut(ks) as *mut [K]), &mut *(core::ptr::from_mut(vs) as *mut [V])) };
        sort_pairs(ks, vs);
        s.keys.set(room.region(), ks.as_ptr(), n).expect("keys beside their table");
        s.vals.set(room.region(), vs.as_ptr(), n).expect("values beside their table");
        s
    }
}

/// Sorts `k` and moves `v` with it, in place (a heap sort: no room past the two).
fn sort_pairs<K: Ord + Copy, V: Copy>(k: &mut [K], v: &mut [V]) {
    let n = k.len();
    let sift = |k: &mut [K], v: &mut [V], mut i: usize, end: usize| loop {
        let mut c = 2 * i + 1;
        if c >= end {
            break;
        }
        if c + 1 < end && k[c + 1] > k[c] {
            c += 1;
        }
        if k[i] >= k[c] {
            break;
        }
        k.swap(i, c);
        v.swap(i, c);
        i = c;
    };
    for i in (0..n / 2).rev() {
        sift(k, v, i, n);
    }
    for end in (1..n).rev() {
        k.swap(0, end);
        v.swap(0, end);
        sift(k, v, 0, end);
    }
}

/// Lists in one run: list `k`'s items are `items[starts[k]..starts[k + 1]]`.
#[repr(C)]
pub struct Csr<T> {
    starts: RelSpan<u32>,
    items: RelSpan<T>,
}

// SAFETY: two spans, no padding.
unsafe impl<T: 'static> Pod for Csr<T> {}

impl<T: Pod> Csr<T> {
    /// Its lists.
    pub fn groups(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    /// List `k`'s items, where they lie.
    pub fn group(&self, k: usize) -> &[T] {
        let s = self.starts.as_slice();
        &self.items.as_slice()[s[k] as usize..s[k + 1] as usize]
    }

    pub fn items(&self) -> &[T] {
        self.items.as_slice()
    }
}

/// A chain pool's lists, by their heads, to settle as a `Csr`.
pub struct CsrFrom<'r, 'c, T> {
    pub lists: &'r ChainRead<'c, T>,
    pub heads: &'r [Link],
}

impl<T: Pod + Copy> CsrFrom<'_, '_, T> {
    fn total(&self) -> usize {
        self.heads.iter().map(|&h| self.lists.walk(h).count()).sum()
    }
}

impl<T: Pod + Copy> Settle for CsrFrom<'_, '_, T> {
    type Final = Csr<T>;

    fn need(&self) -> Need {
        Need::none().and::<Csr<T>>(1).and::<u32>(self.heads.len() + 1).and::<T>(self.total())
    }

    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d Csr<T> {
        let total = self.total();
        assert!(total <= u32::MAX as usize, "a CSR past 2^32 items");
        let c = room.take::<Csr<T>>(1)[0].write(Csr { starts: RelSpan::empty(), items: RelSpan::empty() });
        let starts = room.take::<u32>(self.heads.len() + 1);
        let items = room.take::<T>(total);
        let mut at = 0usize;
        for (k, &h) in self.heads.iter().enumerate() {
            starts[k].write(at as u32);
            for &x in self.lists.walk(h) {
                items[at].write(x);
                at += 1;
            }
        }
        starts[self.heads.len()].write(at as u32);
        c.starts.set(room.region(), starts.as_ptr().cast(), starts.len()).expect("starts beside their lists");
        c.items.set(room.region(), items.as_ptr().cast(), total).expect("items beside their lists");
        c
    }
}

/// Keys to settle as a Bloom filter, `bits_per_key` bits each (rounded up to a power of two of words).
pub struct BloomFrom<'k> {
    pub keys: &'k [i64],
    pub bits_per_key: usize,
}

impl BloomFrom<'_> {
    fn words(&self) -> usize {
        (self.keys.len() * self.bits_per_key).div_ceil(64).max(1).next_power_of_two()
    }
}

impl Settle for BloomFrom<'_> {
    type Final = Bloom;

    fn need(&self) -> Need {
        Need::none().and::<Bloom>(1).and::<u64>(self.words())
    }

    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d Bloom {
        let d = room.take::<Bloom>(1);
        let w = room.take_with::<u64>(self.words(), 0);
        for &k in self.keys {
            let (i, m) = bloom::place(bloom::hash(k), w.len());
            w[i] |= m;
        }
        let b = d[0].as_mut_ptr();
        // SAFETY: the descriptor's one field, set to its words beside it in the room: every byte of it written.
        unsafe {
            Span::set(&raw const (*b).words, w.as_ptr(), w.len());
            &*b
        }
    }
}
