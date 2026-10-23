//! Values inside the dataflow: SQLite's storage classes packed into 8-byte `Copy` words, rows as
//! compact inline-or-boxed slices, convertible to and from the IR executors' `sql::value::V`.
//!
//! Text is a symbol of the process's one string table: the v0 store's interner, owned by the
//! world `install`ed by the driver (the test vectors' rows carry the store's symbols as they are).
use crate::derive::World;
use rerail_facts::sql::value::{self as sv, V};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

/// A stored value in one 32-bit word (x2-compact): the low 2 bits a tag, the rest the payload.
/// NULL is 0; an INTEGER in [-2^29, 2^29) is inline; TEXT is a symbol of the one interner; any
/// other INTEGER and every REAL (by its bits) is an index into the process's number table. Every
/// value has exactly one encoding, so the derived equality and hash are the representational ones
/// the dataflow keys on. The derived order is representational too and carries no SQL meaning: no
/// operator reads it beyond grouping equal values (SQL comparison goes through `V`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Val(u32);

/// A value unpacked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Null,
    Int(i64),
    /// The `f64` bits.
    Real(u64),
    Text(u32),
}

const TAG_INT: u32 = 1;
const TAG_TEXT: u32 = 2;
const TAG_NUM: u32 = 3;
const INLINE_MIN: i64 = -(1 << 29);
const INLINE_MAX: i64 = (1 << 29) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Num {
    Int(i64),
    Real(u64),
}

/// The number table: boxed numbers by id, with a free list (ids of numbers no record holds,
/// reclaimed by `sweep`).
#[derive(Default)]
struct Nums {
    vals: Vec<Num>,
    index: FxHashMap<Num, u32>,
    free: Vec<u32>,
    pinned: usize,
}

thread_local! {
    static NUMS: RefCell<Nums> = RefCell::new(Nums::default());
    static WORLD: RefCell<Option<Rc<RefCell<World>>>> = const { RefCell::new(None) };
}

fn boxed(n: Num) -> Val {
    let i = NUMS.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(&i) = t.index.get(&n) {
            return i;
        }
        let i = match t.free.pop() {
            Some(i) => {
                t.vals[i as usize] = n;
                i
            }
            None => {
                t.vals.push(n);
                (t.vals.len() - 1) as u32
            }
        };
        t.index.insert(n, i);
        i
    });
    Val((i << 2) | TAG_NUM)
}

impl Val {
    pub const NULL: Val = Val(0);

    pub fn int(i: i64) -> Val {
        if (INLINE_MIN..=INLINE_MAX).contains(&i) {
            Val(((i as i32 as u32) << 2) | TAG_INT)
        } else {
            boxed(Num::Int(i))
        }
    }

    pub fn real(bits: u64) -> Val {
        boxed(Num::Real(bits))
    }

    pub fn text(sym: u32) -> Val {
        assert!(sym < 1 << 30, "symbol {sym} does not fit a 32-bit value word");
        Val((sym << 2) | TAG_TEXT)
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    pub fn kind(self) -> Kind {
        match self.0 & 3 {
            0 => Kind::Null,
            TAG_INT => Kind::Int(((self.0 as i32) >> 2) as i64),
            TAG_TEXT => Kind::Text(self.0 >> 2),
            _ => match NUMS.with(|t| t.borrow().vals[(self.0 >> 2) as usize]) {
                Num::Int(i) => Kind::Int(i),
                Num::Real(b) => Kind::Real(b),
            },
        }
    }
}

/// Number table: (live numbers, bytes).
pub fn nums_footprint() -> (usize, usize) {
    NUMS.with(|t| {
        let t = t.borrow();
        (t.vals.len() - t.free.len(), t.vals.capacity() * std::mem::size_of::<Num>() + t.index.capacity() * 24 + t.free.capacity() * 4)
    })
}

/// Every number boxed so far is kept forever (plan constants, the store's facts).
pub fn pin_nums() {
    NUMS.with(|t| {
        let mut t = t.borrow_mut();
        t.pinned = t.vals.len();
    })
}

/// Frees every unpinned number whose id `live` does not hold; returns how many were freed.
pub fn sweep_nums(live: &[u64]) -> usize {
    NUMS.with(|t| {
        let mut t = t.borrow_mut();
        let mut free = vec![0u64; 0];
        for &i in &t.free {
            set_bit(&mut free, i);
        }
        let mut freed = 0;
        for i in t.pinned..t.vals.len() {
            let i = i as u32;
            if bit(&free, i) || bit(live, i) {
                continue;
            }
            let n = t.vals[i as usize];
            t.index.remove(&n);
            t.free.push(i);
            freed += 1;
        }
        freed
    })
}

pub fn bit(bits: &[u64], i: u32) -> bool {
    bits.get((i >> 6) as usize).is_some_and(|w| (w >> (i & 63)) & 1 == 1)
}

pub fn set_bit(bits: &mut Vec<u64>, i: u32) {
    let w = (i >> 6) as usize;
    if bits.len() <= w {
        bits.resize(w + 1, 0);
    }
    bits[w] |= 1 << (i & 63);
}

/// Marks the symbols and numbers `vals` hold.
pub fn mark(vals: &[Val], syms: &mut Vec<u64>, nums: &mut Vec<u64>) {
    for v in vals {
        match v.0 & 3 {
            TAG_TEXT => set_bit(syms, v.0 >> 2),
            TAG_NUM => set_bit(nums, v.0 >> 2),
            _ => {}
        }
    }
}

impl fmt::Debug for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.kind(), f)
    }
}

/// Inline capacity of a row (values) outside arrangements (collections in flight, outputs): 4
/// 32-bit values fit the 24-byte row; arranged rows are columnar (`gen::col`), never a `Row`.
const INLINE: usize = 4;

const _: () = assert!(std::mem::size_of::<Val>() == 4 && std::mem::size_of::<Row>() == 24);

/// A row: up to `INLINE` values inline, longer rows boxed (no capacity word). Equality, order and
/// hash are the slice's, as `Vec<Val>`'s were.
#[derive(Clone)]
pub struct Row(Repr);

#[derive(Clone)]
enum Repr {
    Inline(u8, [Val; INLINE]),
    Heap(Box<[Val]>),
}

impl Row {
    pub fn new(vals: &[Val]) -> Row {
        if vals.len() <= INLINE {
            let mut a = [Val::NULL; INLINE];
            a[..vals.len()].copy_from_slice(vals);
            Row(Repr::Inline(vals.len() as u8, a))
        } else {
            Row(Repr::Heap(vals.into()))
        }
    }

    /// `a` followed by `b` (a join's output row).
    pub fn concat(a: &[Val], b: &[Val]) -> Row {
        a.iter().chain(b).copied().collect()
    }

    /// `a` followed by `v`.
    pub fn with(a: &[Val], v: Val) -> Row {
        a.iter().copied().chain([v]).collect()
    }

    /// Bytes held outside the row itself.
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            Repr::Inline(..) => 0,
            Repr::Heap(b) => b.len() * std::mem::size_of::<Val>(),
        }
    }
}

impl Default for Row {
    fn default() -> Row {
        Row(Repr::Inline(0, [Val::NULL; INLINE]))
    }
}

impl Deref for Row {
    type Target = [Val];
    fn deref(&self) -> &[Val] {
        match &self.0 {
            Repr::Inline(n, a) => &a[..*n as usize],
            Repr::Heap(b) => b,
        }
    }
}

impl PartialEq for Row {
    fn eq(&self, o: &Row) -> bool {
        **self == **o
    }
}

impl Eq for Row {}

impl PartialOrd for Row {
    fn partial_cmp(&self, o: &Row) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Row {
    fn cmp(&self, o: &Row) -> std::cmp::Ordering {
        (**self).cmp(&**o)
    }
}

impl Hash for Row {
    fn hash<H: Hasher>(&self, h: &mut H) {
        (**self).hash(h)
    }
}

impl fmt::Debug for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl FromIterator<Val> for Row {
    fn from_iter<I: IntoIterator<Item = Val>>(it: I) -> Row {
        let mut a = [Val::NULL; INLINE];
        let mut n = 0;
        let mut it = it.into_iter();
        for v in it.by_ref() {
            if n == INLINE {
                let mut spill = Vec::with_capacity(INLINE + 1 + it.size_hint().0);
                spill.extend_from_slice(&a);
                spill.push(v);
                spill.extend(it);
                return Row(Repr::Heap(spill.into_boxed_slice()));
            }
            a[n] = v;
            n += 1;
        }
        Row(Repr::Inline(n as u8, a))
    }
}

impl<const K: usize> From<[Val; K]> for Row {
    fn from(a: [Val; K]) -> Row {
        Row::new(&a)
    }
}

impl From<Vec<Val>> for Row {
    fn from(v: Vec<Val>) -> Row {
        if v.len() <= INLINE {
            Row::new(&v)
        } else {
            Row(Repr::Heap(v.into_boxed_slice()))
        }
    }
}

impl Serialize for Row {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Row, D::Error> {
        Vec::<Val>::deserialize(d).map(Row::from)
    }
}

/// Makes `w`'s store interner the string table of every text value (before any is made).
pub fn install(w: Rc<RefCell<World>>) {
    WORLD.with(|x| *x.borrow_mut() = Some(w));
}

fn world<X>(f: impl FnOnce(&RefCell<World>) -> X) -> X {
    WORLD.with(|x| f(x.borrow().as_ref().expect("string table not installed (gen::val::install)")))
}

/// A text value's symbol: computed strings are evictable (`store::Interner::intern_computed`);
/// a string the store pinned keeps its symbol.
pub fn sym(s: &str) -> u32 {
    world(|w| w.borrow_mut().store.syms.intern_computed(s))
}

/// Pins every symbol and number made so far (plans' constants, the store's facts).
pub fn pin_all() {
    world(|w| w.borrow_mut().store.syms.pin_all());
    pin_nums();
}

/// Symbols created since the last sweep.
pub fn syms_created() -> usize {
    world(|w| w.borrow().store.syms.created)
}

/// Frees every unpinned symbol and number the marks do not hold: (symbols, numbers) freed.
pub fn sweep(syms: &[u64], nums: &[u64]) -> (usize, usize) {
    (world(|w| w.borrow_mut().store.syms.sweep(syms)), sweep_nums(nums))
}

fn string(i: u32) -> Rc<str> {
    world(|w| Rc::from(w.borrow().store.syms.str(i)))
}

/// The string table: (strings, bytes).
pub fn syms_footprint() -> (usize, usize) {
    world(|w| {
        let w = w.borrow();
        (w.store.syms.len(), w.store.syms.bytes())
    })
}

pub fn to_v(v: Val) -> V {
    match v.kind() {
        Kind::Null => V::Null,
        Kind::Int(i) => V::Int(i),
        Kind::Real(b) => V::Real(f64::from_bits(b)),
        Kind::Text(s) => V::Text(string(s)),
    }
}

pub fn from_v(v: &V) -> Val {
    match v {
        V::Null => Val::NULL,
        V::Int(i) => Val::int(*i),
        V::Real(f) => Val::real(f.to_bits()),
        V::Text(s) => Val::text(sym(s)),
    }
}

pub fn row_to_v(r: &[Val]) -> Vec<V> {
    r.iter().map(|v| to_v(*v)).collect()
}

/// Equality key of a value under SQL `=` after comparison affinity: NULL has none (never equal),
/// an integral REAL equals the INTEGER (`value::key`).
pub fn eq_key(v: &V) -> Option<Val> {
    sv::key(v).map(|k| match k {
        sv::Key::Int(i) => Val::int(i),
        sv::Key::Real(b) => Val::real(b),
        sv::Key::Text(s) => Val::text(sym(&s)),
    })
}

/// Grouping key (DISTINCT, UNION, GROUP BY, PARTITION BY): NULLs equal each other.
pub fn group_key(v: &V) -> Val {
    eq_key(v).unwrap_or(Val::NULL)
}
