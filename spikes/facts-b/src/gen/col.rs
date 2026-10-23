//! Columnar arrangements (x2-compact, disposable): every arranged key and value is a run of
//! 32-bit words in one flat column, never a per-record `Row`/`Vec`. Rows of one arrangement have
//! one width (a node's schema, a key's arity), so a column is `width` words per item with no
//! offsets; a column whose items differ in width falls back to one `u32` offset per item. Tags
//! are their own `u32` column; times and diffs stay differential's (one offset per value when a
//! value's single update repeats the previous one).
//!
//! The registry keeps a handle on every arrangement of the measured dataflow (compaction never
//! held back: its frontiers are empty) so the interner's sweep can mark the symbols and numbers
//! the arranged records hold, and the run can report the columns' exact bytes per record.
use super::tags::W;
use super::val::{self, Row, Val};
use differential_dataflow::operators::arrange::TraceAgent;
use differential_dataflow::trace::implementations::ord_neu::{OrdValBatch, OrdValBatcher, OrdValBuilder};
use differential_dataflow::trace::implementations::spine_fueled::Spine;
use differential_dataflow::trace::implementations::{BatchContainer, OffsetList};
use differential_dataflow::trace::rc_blanket_impls::RcBuilder;
use differential_dataflow::trace::TraceReader;
use std::cell::RefCell;
use std::rc::Rc;
use timely::container::PushInto;

pub type R = i32;
/// Fixed-width rows in one flat column (offsets only for ragged columns).
pub struct RowCol {
    vals: Vec<Val>,
    /// `u32::MAX` until the first push.
    width: u32,
    len: usize,
    /// Items to reserve for at the first push (its width known then).
    hint: usize,
    /// Item boundaries once two widths were seen (`offs[i]..offs[i + 1]`).
    offs: Option<Vec<u32>>,
}

impl RowCol {
    fn push_slice(&mut self, r: &[Val]) {
        if self.offs.is_none() {
            if self.width == u32::MAX {
                self.width = r.len() as u32;
                let want = self.hint * r.len();
                if self.vals.capacity() < want {
                    self.vals.reserve_exact(want - self.vals.len());
                }
            } else if self.width as usize != r.len() {
                let w = self.width as usize;
                self.offs = Some((0..=self.len).map(|i| (i * w) as u32).collect());
            }
        }
        self.vals.extend_from_slice(r);
        if let Some(o) = self.offs.as_mut() {
            o.push(self.vals.len() as u32);
        }
        self.len += 1;
    }
    fn get(&self, i: usize) -> &[Val] {
        match &self.offs {
            None => {
                let w = self.width as usize;
                &self.vals[i * w..(i + 1) * w]
            }
            Some(o) => &self.vals[o[i] as usize..o[i + 1] as usize],
        }
    }
    pub fn bytes(&self) -> usize {
        self.vals.capacity() * 4 + self.offs.as_ref().map_or(0, |o| o.capacity() * 4)
    }
    pub fn words(&self) -> &[Val] {
        &self.vals
    }
}

impl BatchContainer for RowCol {
    type Owned = Row;
    type ReadItem<'a> = &'a [Val];

    fn into_owned<'a>(item: &'a [Val]) -> Row {
        Row::new(item)
    }
    fn push_ref(&mut self, item: &[Val]) {
        self.push_slice(item)
    }
    fn push_own(&mut self, item: &Row) {
        self.push_slice(item)
    }
    fn clear(&mut self) {
        self.vals.clear();
        self.width = u32::MAX;
        self.len = 0;
        self.offs = None;
    }
    fn with_capacity(size: usize) -> Self {
        // Widths are unknown until the first push, which reserves `size` items of its width.
        RowCol { vals: Vec::new(), width: u32::MAX, len: 0, hint: size, offs: None }
    }
    fn merge_capacity(a: &Self, b: &Self) -> Self {
        RowCol { vals: Vec::with_capacity(a.vals.len() + b.vals.len()), width: u32::MAX, len: 0, hint: 0, offs: None }
    }
    fn reborrow<'b, 'a: 'b>(item: &'a [Val]) -> &'b [Val] {
        item
    }
    fn index(&self, i: usize) -> &[Val] {
        self.get(i)
    }
    fn len(&self) -> usize {
        self.len
    }
}

impl PushInto<Row> for RowCol {
    fn push_into(&mut self, item: Row) {
        self.push_slice(&item)
    }
}

/// `&[Val] == &Row` (the builder compares a column's last key with the next update's).
impl PartialEq<Row> for [Val] {
    fn eq(&self, o: &Row) -> bool {
        *self == **o
    }
}

/// A tag in 16 bits: tags below 2^15, the refusal bit (`lower::REFUSED`) as bit 15.
fn pack_tag(w: W) -> u16 {
    const REFUSED: W = 1 << 31;
    let t = w & !REFUSED;
    assert!(t < 1 << 15, "tag {t} does not fit 15 bits");
    t as u16 | if w & REFUSED != 0 { 1 << 15 } else { 0 }
}

fn unpack_tag(x: u16) -> W {
    (x as W & 0x7fff) | if x & (1 << 15) != 0 { 1 << 31 } else { 0 }
}

/// A tagged row read from a column: (tag, row), ordered as `(W, Row)` is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TR<'a>(pub W, pub &'a [Val]);

impl<'a> PartialEq<&'a (W, Row)> for TR<'a> {
    fn eq(&self, o: &&'a (W, Row)) -> bool {
        self.0 == o.0 && *self.1 == *o.1
    }
}

/// Tagged rows: a 16-bit tag column and a row column.
pub struct TagRowCol {
    tags: Vec<u16>,
    rows: RowCol,
}

impl TagRowCol {
    pub fn words(&self) -> &[Val] {
        self.rows.words()
    }
}

impl BatchContainer for TagRowCol {
    type Owned = (W, Row);
    type ReadItem<'a> = TR<'a>;

    fn into_owned<'a>(item: TR<'a>) -> (W, Row) {
        (item.0, Row::new(item.1))
    }
    fn push_ref(&mut self, item: TR<'_>) {
        self.tags.push(pack_tag(item.0));
        self.rows.push_slice(item.1);
    }
    fn push_own(&mut self, item: &(W, Row)) {
        self.tags.push(pack_tag(item.0));
        self.rows.push_slice(&item.1);
    }
    fn clear(&mut self) {
        self.tags.clear();
        self.rows.clear();
    }
    fn with_capacity(size: usize) -> Self {
        TagRowCol { tags: Vec::with_capacity(size), rows: RowCol::with_capacity(size) }
    }
    fn merge_capacity(a: &Self, b: &Self) -> Self {
        TagRowCol { tags: Vec::with_capacity(a.tags.len() + b.tags.len()), rows: RowCol::merge_capacity(&a.rows, &b.rows) }
    }
    fn reborrow<'b, 'a: 'b>(item: TR<'a>) -> TR<'b> {
        item
    }
    fn index(&self, i: usize) -> TR<'_> {
        TR(unpack_tag(self.tags[i]), self.rows.get(i))
    }
    fn len(&self) -> usize {
        self.tags.len()
    }
}

impl PushInto<(W, Row)> for TagRowCol {
    fn push_into(&mut self, item: (W, Row)) {
        self.tags.push(pack_tag(item.0));
        self.rows.push_slice(&item.1);
    }
}

/// A presence value: its tag.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TU(pub W);

impl<'a> PartialEq<&'a (W, ())> for TU {
    fn eq(&self, o: &&'a (W, ())) -> bool {
        self.0 == o.0
    }
}

/// Presence values: a 16-bit tag column.
pub struct TagCol {
    tags: Vec<u16>,
}

impl BatchContainer for TagCol {
    type Owned = (W, ());
    type ReadItem<'a> = TU;

    fn into_owned<'a>(item: TU) -> (W, ()) {
        (item.0, ())
    }
    fn push_ref(&mut self, item: TU) {
        self.tags.push(pack_tag(item.0));
    }
    fn push_own(&mut self, item: &(W, ())) {
        self.tags.push(pack_tag(item.0));
    }
    fn clear(&mut self) {
        self.tags.clear();
    }
    fn with_capacity(size: usize) -> Self {
        TagCol { tags: Vec::with_capacity(size) }
    }
    fn merge_capacity(a: &Self, b: &Self) -> Self {
        TagCol { tags: Vec::with_capacity(a.tags.len() + b.tags.len()) }
    }
    fn reborrow<'b, 'a: 'b>(item: TU) -> TU {
        item
    }
    fn index(&self, i: usize) -> TU {
        TU(unpack_tag(self.tags[i]))
    }
    fn len(&self) -> usize {
        self.tags.len()
    }
}

impl PushInto<(W, ())> for TagCol {
    fn push_into(&mut self, item: (W, ())) {
        self.tags.push(pack_tag(item.0));
    }
}

/// A timestamp stored in one word: the outer time as itself, an iteration time `(outer, round)`
/// as one `u64` (round below 2^32), instead of the 16-byte `Product`.
pub trait Packed: timely::progress::Timestamp + differential_dataflow::lattice::Lattice + Copy {
    type Word: Copy + 'static;
    fn pack(self) -> Self::Word;
    fn unpack(w: Self::Word) -> Self;
}

impl Packed for u32 {
    type Word = u32;
    fn pack(self) -> u32 {
        self
    }
    fn unpack(w: u32) -> u32 {
        w
    }
}

impl Packed for timely::order::Product<u32, u64> {
    type Word = u64;
    fn pack(self) -> u64 {
        assert!(self.inner < 1 << 32, "iteration {} does not fit 32 bits", self.inner);
        ((self.outer as u64) << 32) | self.inner
    }
    fn unpack(w: u64) -> Self {
        timely::order::Product::new((w >> 32) as u32, w & 0xffff_ffff)
    }
}

/// Update times, packed (`Packed`), read by value.
pub struct Times<Tm: Packed>(Vec<Tm::Word>);

impl<Tm: Packed> BatchContainer for Times<Tm> {
    type Owned = Tm;
    type ReadItem<'a> = Tm;

    fn into_owned<'a>(item: Tm) -> Tm {
        item
    }
    fn push_ref(&mut self, item: Tm) {
        self.0.push(item.pack())
    }
    fn push_own(&mut self, item: &Tm) {
        self.0.push(item.pack())
    }
    fn clear(&mut self) {
        self.0.clear()
    }
    fn with_capacity(_size: usize) -> Self {
        Times(Vec::new())
    }
    fn merge_capacity(_a: &Self, _b: &Self) -> Self {
        Times(Vec::new())
    }
    fn reborrow<'b, 'a: 'b>(item: Tm) -> Tm {
        item
    }
    fn index(&self, i: usize) -> Tm {
        Tm::unpack(self.0[i])
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Times or diffs: a `Vec` that reserves nothing up front. Differential's builder reserves one
/// slot per update, but its singleton optimisation stores almost none of them (one update per
/// value at the batch's time, repeating the previous one), so the reservation was dead weight.
pub struct Lean<T>(Vec<T>);

impl<T: Ord + Clone + 'static> BatchContainer for Lean<T> {
    type Owned = T;
    type ReadItem<'a> = &'a T;

    fn into_owned<'a>(item: &'a T) -> T {
        item.clone()
    }
    fn push_ref(&mut self, item: &T) {
        self.0.push(item.clone())
    }
    fn push_own(&mut self, item: &T) {
        self.0.push(item.clone())
    }
    fn clear(&mut self) {
        self.0.clear()
    }
    fn with_capacity(_size: usize) -> Self {
        Lean(Vec::new())
    }
    fn merge_capacity(_a: &Self, _b: &Self) -> Self {
        Lean(Vec::new())
    }
    fn reborrow<'b, 'a: 'b>(item: &'a T) -> &'b T {
        item
    }
    fn index(&self, i: usize) -> &T {
        &self.0[i]
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Offsets (non-decreasing) as one bit per item while every step is 0 or 1 (an update list per
/// value is almost always one update, or the previous one repeated: differential's singleton
/// optimisation), with a rank directory every 512 items; any longer step switches the column to
/// `OffsetList`'s explicit offsets.
pub struct Offs {
    first: usize,
    len: usize,
    /// Step `i` (from item `i` to item `i + 1`) is bit `i`.
    bits: Vec<u64>,
    /// Ones before each 512-step block.
    rank: Vec<u32>,
    last: usize,
    explicit: Option<OffsetList>,
}

impl Offs {
    fn push(&mut self, o: usize) {
        if let Some(x) = self.explicit.as_mut() {
            x.push(o);
            self.len += 1;
            return;
        }
        if self.len == 0 {
            self.first = o;
            self.last = o;
            self.len = 1;
            return;
        }
        let step = o.wrapping_sub(self.last);
        if step > 1 {
            let mut x = OffsetList::with_capacity(self.len + 1);
            for i in 0..self.len {
                x.push(self.get(i));
            }
            x.push(o);
            self.explicit = Some(x);
            self.bits = Vec::new();
            self.rank = Vec::new();
            self.len += 1;
            return;
        }
        let s = self.len - 1;
        if s % 512 == 0 {
            self.rank.push((self.last - self.first) as u32);
        }
        if s % 64 == 0 {
            self.bits.push(0);
        }
        if step == 1 {
            *self.bits.last_mut().unwrap() |= 1 << (s % 64);
        }
        self.last = o;
        self.len += 1;
    }
    fn get(&self, i: usize) -> usize {
        if let Some(x) = &self.explicit {
            return x.index(i);
        }
        if i + 1 == self.len {
            return self.last;
        }
        // The sum of the first i steps (i < steps: its rank block exists).
        let b = i / 512;
        let mut r = self.rank[b] as usize;
        let w0 = b * 8;
        let full = i / 64;
        for w in w0..full {
            r += self.bits[w].count_ones() as usize;
        }
        if i % 64 != 0 {
            r += (self.bits[full] & ((1u64 << (i % 64)) - 1)).count_ones() as usize;
        }
        self.first + r
    }
    pub fn bytes(&self) -> usize {
        self.bits.capacity() * 8 + self.rank.capacity() * 4 + self.explicit.as_ref().map_or(0, |x| x.smol.capacity() * 4 + x.chonk.capacity() * 8)
    }
}

impl BatchContainer for Offs {
    type Owned = usize;
    type ReadItem<'a> = usize;

    fn into_owned<'a>(item: usize) -> usize {
        item
    }
    fn reborrow<'b, 'a: 'b>(item: usize) -> usize {
        item
    }
    fn push_ref(&mut self, item: usize) {
        self.push(item)
    }
    fn push_own(&mut self, item: &usize) {
        self.push(*item)
    }
    fn clear(&mut self) {
        *self = Self::with_capacity(0);
    }
    fn with_capacity(size: usize) -> Self {
        Offs { first: 0, len: 0, bits: Vec::with_capacity(size.div_ceil(64)), rank: Vec::with_capacity(size.div_ceil(512)), last: 0, explicit: None }
    }
    fn merge_capacity(a: &Self, b: &Self) -> Self {
        Self::with_capacity(a.len + b.len)
    }
    fn index(&self, i: usize) -> usize {
        self.get(i)
    }
    fn len(&self) -> usize {
        self.len
    }
}

/// Keyed tagged rows: key column, (tag, row) columns.
pub type RowLayout<Tm> = (RowCol, TagRowCol, Times<Tm>, Lean<R>, Offs);
/// Presence: key column, a tag per value.
pub type PresLayout<Tm> = (RowCol, TagCol, Times<Tm>, Lean<R>, Offs);

pub type RowSpine<Tm> = Spine<Rc<OrdValBatch<RowLayout<Tm>>>>;
pub type PresSpine<Tm> = Spine<Rc<OrdValBatch<PresLayout<Tm>>>>;
pub type RowBuilder<Tm> = RcBuilder<OrdValBuilder<RowLayout<Tm>, Vec<((Row, (W, Row)), Tm, R)>>>;
pub type PresBuilder<Tm> = RcBuilder<OrdValBuilder<PresLayout<Tm>, Vec<((Row, (W, ())), Tm, R)>>>;
pub type RowBatcher<Tm> = OrdValBatcher<Row, (W, Row), Tm, R>;
pub type PresBatcher<Tm> = OrdValBatcher<Row, (W, ()), Tm, R>;

/// Per registered arrangement: records, keys, values and the columns' bytes by component.
#[derive(Default, Clone, Copy)]
pub struct Census {
    pub records: usize,
    pub keys: usize,
    pub vals: usize,
    pub bytes: usize,
    pub key_bytes: usize,
    pub row_bytes: usize,
    pub tag_bytes: usize,
    pub off_bytes: usize,
    pub upd_bytes: usize,
    pub batches: usize,
    /// Key and value row widths (words) of the last batch seen (`u32::MAX`: ragged or empty).
    pub key_width: u32,
    pub row_width: u32,
}

impl Census {
    fn add(&mut self, records: usize, keys: &RowCol, vals_len: usize, row_bytes: usize, tag_bytes: usize, off_bytes: usize, upd_bytes: usize, row_width: u32) {
        self.records += records;
        self.keys += keys.len;
        self.vals += vals_len;
        self.key_bytes += keys.bytes();
        self.row_bytes += row_bytes;
        self.tag_bytes += tag_bytes;
        self.off_bytes += off_bytes;
        self.upd_bytes += upd_bytes;
        self.bytes += keys.bytes() + row_bytes + tag_bytes + off_bytes + upd_bytes;
        self.batches += 1;
        if keys.len > 0 {
            self.key_width = if keys.offs.is_some() { u32::MAX } else { keys.width };
            self.row_width = row_width;
        }
    }
}

type Visit = Box<dyn FnMut(&mut Vec<u64>, &mut Vec<u64>, bool) -> Census>;

thread_local! {
    static ON: RefCell<bool> = const { RefCell::new(false) };
    static REG: RefCell<Vec<(String, Visit)>> = RefCell::new(Vec::new());
}

/// Registers the arrangements built while `f` runs (the measured dataflow only).
pub fn registering<X>(f: impl FnOnce() -> X) -> X {
    ON.with(|o| *o.borrow_mut() = true);
    let x = f();
    ON.with(|o| *o.borrow_mut() = false);
    x
}

fn on() -> bool {
    ON.with(|o| *o.borrow())
}

fn upd_bytes<Tm: Packed>(u: &differential_dataflow::trace::implementations::ord_neu::Upds<Offs, Times<Tm>, Lean<R>>) -> usize {
    u.times.0.capacity() * std::mem::size_of::<Tm::Word>() + u.diffs.0.capacity() * 4
}

fn offs_bytes(o: &Offs) -> usize {
    o.bytes()
}

pub fn register_rows<Tm: Packed>(name: &str, mut t: TraceAgent<RowSpine<Tm>>) {
    if !on() {
        return;
    }
    t.set_logical_compaction(timely::progress::Antichain::new().borrow());
    t.set_physical_compaction(timely::progress::Antichain::new().borrow());
    let visit: Visit = Box::new(move |syms, nums, marking| {
        let mut c = Census::default();
        t.map_batches(|b| {
            let s = &b.storage;
            let rows = &s.vals.vals.rows;
            let rw = if rows.offs.is_some() { u32::MAX } else { rows.width };
            c.add(b.updates, &s.keys, s.vals.vals.len(), rows.bytes(), s.vals.vals.tags.capacity() * 2, offs_bytes(&s.vals.offs) + offs_bytes(&s.upds.offs), upd_bytes(&s.upds), rw);
            if marking {
                val::mark(s.keys.words(), syms, nums);
                val::mark(s.vals.vals.words(), syms, nums);
            }
        });
        c
    });
    REG.with(|r| r.borrow_mut().push((name.to_owned(), visit)));
}

pub fn register_pres<Tm: Packed>(name: &str, mut t: TraceAgent<PresSpine<Tm>>) {
    if !on() {
        return;
    }
    t.set_logical_compaction(timely::progress::Antichain::new().borrow());
    t.set_physical_compaction(timely::progress::Antichain::new().borrow());
    let visit: Visit = Box::new(move |syms, nums, marking| {
        let mut c = Census::default();
        t.map_batches(|b| {
            let s = &b.storage;
            c.add(b.updates, &s.keys, s.vals.vals.len(), 0, s.vals.vals.tags.capacity() * 2, offs_bytes(&s.vals.offs) + offs_bytes(&s.upds.offs), upd_bytes(&s.upds), 0);
            if marking {
                val::mark(s.keys.words(), syms, nums);
            }
        });
        c
    });
    REG.with(|r| r.borrow_mut().push((name.to_owned(), visit)));
}

/// Every registered arrangement's census; with `marks`, the symbols and numbers its records hold
/// are marked.
pub fn census(mut marks: Option<(&mut Vec<u64>, &mut Vec<u64>)>) -> Vec<(String, Census)> {
    REG.with(|r| {
        let mut r = r.borrow_mut();
        let (mut ds, mut dn) = (Vec::new(), Vec::new());
        r.iter_mut()
            .map(|(n, v)| {
                let c = match marks.as_mut() {
                    Some((s, m)) => v(s, m, true),
                    None => v(&mut ds, &mut dn, false),
                };
                (n.clone(), c)
            })
            .collect()
    })
}

/// A (path, content)'s exported rows of one export, flat: `width` words per row (offsets only
/// when the rows differ in width).
pub struct Flat {
    rows: RowCol,
}

impl Flat {
    pub fn len(&self) -> usize {
        self.rows.len
    }
    pub fn get(&self, i: usize) -> &[Val] {
        self.rows.get(i)
    }
    pub fn bytes(&self) -> usize {
        self.rows.bytes() + std::mem::size_of::<Flat>()
    }
    pub fn words(&self) -> &[Val] {
        self.rows.words()
    }
}

impl From<Vec<Row>> for Flat {
    fn from(v: Vec<Row>) -> Flat {
        let mut rows = RowCol::with_capacity(v.len());
        for r in &v {
            rows.push_slice(r);
        }
        rows.vals.shrink_to_fit();
        if let Some(o) = rows.offs.as_mut() {
            o.shrink_to_fit();
        }
        Flat { rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Offs` reads back every offset it was given, in bit mode and after switching to explicit.
    #[test]
    fn offs_round_trip() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..200 {
            let n = (rnd() % 3000) as usize;
            let jump_at = if case % 3 == 0 { Some((rnd() % (n as u64 + 1)) as usize) } else { None };
            let mut want = Vec::with_capacity(n);
            let mut o = if case % 2 == 0 { 0 } else { (rnd() % 100) as usize };
            let mut c = Offs::with_capacity(n);
            for i in 0..n {
                if i > 0 {
                    o += if Some(i) == jump_at { 2 + (rnd() % 50) as usize } else { (rnd() % 2) as usize };
                }
                want.push(o);
                c.push_ref(o);
            }
            assert_eq!(c.len(), n);
            for (i, w) in want.iter().enumerate() {
                assert_eq!(c.index(i), *w, "case {case} item {i}");
            }
        }
    }

    /// Fixed-width columns read back their rows; a second width switches to offsets.
    #[test]
    fn rowcol_round_trip() {
        let rows: Vec<Row> = (0..100).map(|i| Row::new(&(0..(if i < 60 { 3 } else { 1 + i % 5 })).map(|j| Val::int(i * 10 + j)).collect::<Vec<_>>())).collect();
        let mut c = RowCol::with_capacity(rows.len());
        for r in &rows {
            c.push_own(r);
        }
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(c.index(i), &r[..]);
        }
    }
}
