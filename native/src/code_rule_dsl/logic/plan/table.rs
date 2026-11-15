//! Decision 0209 §3: the planner's statistics as flat arrays.
//!
//! A relation is numbered once in the process (`rel_id`), by its name. A statistic of relations (`RelMap`) is an
//! array by that number. A statistic of columns (`ColMap`) gives each relation a block of slots, one a column, in one
//! array: a column is a slot (`ColId`), found by two array reads once its relation's number is known. Readers that
//! hold numbers (`Model`'s prepared atoms) read slots; readers by name pay one lookup of the name, not a string built
//! per key.

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::{Arc, LazyLock};

/// A relation's number in this process.
pub type RelId = u32;
/// A column's slot in a `ColMap`.
pub type ColId = u32;

#[derive(Default)]
struct Names {
    ids: FxHashMap<Arc<str>, RelId>,
    names: Vec<Arc<str>>,
}

static NAMES: LazyLock<RwLock<Names>> = LazyLock::new(Default::default);

/// `name`'s number, given it on first sight.
pub fn rel_id(name: &str) -> RelId {
    if let Some(&id) = NAMES.read().ids.get(name) {
        return id;
    }
    let mut n = NAMES.write();
    if let Some(&id) = n.ids.get(name) {
        return id;
    }
    let id = n.names.len() as RelId;
    let name: Arc<str> = name.into();
    n.names.push(Arc::clone(&name));
    n.ids.insert(name, id);
    id
}

/// `name`'s number, where it has one.
pub fn find(name: &str) -> Option<RelId> {
    NAMES.read().ids.get(name).copied()
}

/// The name numbered `id`.
pub fn rel_name(id: RelId) -> Arc<str> {
    Arc::clone(&NAMES.read().names[id as usize])
}

/// A statistic of relations: a value per relation number.
#[derive(Clone, Debug)]
pub struct RelMap<T> {
    vals: Vec<Option<T>>,
    n: usize,
}

impl<T> Default for RelMap<T> {
    fn default() -> Self {
        RelMap { vals: Vec::new(), n: 0 }
    }
}

impl<T> RelMap<T> {
    pub fn new() -> Self {
        Self::default()
    }
    #[inline]
    pub fn at(&self, id: RelId) -> Option<&T> {
        self.vals.get(id as usize).and_then(Option::as_ref)
    }
    #[inline]
    pub fn get<S: AsRef<str> + ?Sized>(&self, k: &S) -> Option<&T> {
        find(k.as_ref()).and_then(|id| self.at(id))
    }
    pub fn get_mut<S: AsRef<str> + ?Sized>(&mut self, k: &S) -> Option<&mut T> {
        let id = find(k.as_ref())?;
        self.vals.get_mut(id as usize).and_then(Option::as_mut)
    }
    pub fn insert_at(&mut self, id: RelId, v: T) -> Option<T> {
        let i = id as usize;
        if self.vals.len() <= i {
            self.vals.resize_with(i + 1, || None);
        }
        let old = self.vals[i].replace(v);
        if old.is_none() {
            self.n += 1;
        }
        old
    }
    pub fn insert<S: AsRef<str>>(&mut self, k: S, v: T) -> Option<T> {
        self.insert_at(rel_id(k.as_ref()), v)
    }
    pub fn remove<S: AsRef<str> + ?Sized>(&mut self, k: &S) -> Option<T> {
        let id = find(k.as_ref())?;
        let old = self.vals.get_mut(id as usize).and_then(Option::take);
        if old.is_some() {
            self.n -= 1;
        }
        old
    }
    pub fn contains_key<S: AsRef<str> + ?Sized>(&self, k: &S) -> bool {
        self.get(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// The relations with a value, by name.
    pub fn iter(&self) -> impl Iterator<Item = (Arc<str>, &T)> {
        self.vals.iter().enumerate().filter_map(|(i, v)| v.as_ref().map(|v| (rel_name(i as RelId), v)))
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.vals.iter().filter_map(Option::as_ref)
    }
    pub fn clear(&mut self) {
        self.vals.clear();
        self.n = 0;
    }
    /// The value of `k`, made with `f` where it has none.
    pub fn entry_or_insert_with<S: AsRef<str>>(&mut self, k: S, f: impl FnOnce() -> T) -> &mut T {
        let i = rel_id(k.as_ref()) as usize;
        if self.vals.len() <= i {
            self.vals.resize_with(i + 1, || None);
        }
        if self.vals[i].is_none() {
            self.vals[i] = Some(f());
            self.n += 1;
        }
        self.vals[i].as_mut().expect("just set")
    }
}

impl<T, S: AsRef<str>> FromIterator<(S, T)> for RelMap<T> {
    fn from_iter<I: IntoIterator<Item = (S, T)>>(it: I) -> Self {
        let mut m = RelMap::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl<T, S: AsRef<str>> Extend<(S, T)> for RelMap<T> {
    fn extend<I: IntoIterator<Item = (S, T)>>(&mut self, it: I) {
        for (k, v) in it {
            self.insert(k, v);
        }
    }
}

/// A statistic of columns: each relation's columns a block of slots in one array.
#[derive(Clone, Debug)]
pub struct ColMap<T> {
    /// Per relation number, its block's first slot and its width (`u32::MAX`: none yet).
    base: Vec<(ColId, u32)>,
    vals: Vec<Option<T>>,
    n: usize,
}

impl<T> Default for ColMap<T> {
    fn default() -> Self {
        ColMap { base: Vec::new(), vals: Vec::new(), n: 0 }
    }
}

/// A block's least width: most relations are this narrow, so a block seldom moves.
const WIDTH: u32 = 4;

impl<T> ColMap<T> {
    pub fn new() -> Self {
        Self::default()
    }
    /// The slot of column `c` of relation `rel`, where its block has one.
    #[inline]
    pub fn col_id(&self, rel: RelId, c: usize) -> Option<ColId> {
        let &(b, w) = self.base.get(rel as usize)?;
        (b != u32::MAX && (c as u32) < w).then(|| b + c as u32)
    }
    #[inline]
    pub fn at(&self, id: ColId) -> Option<&T> {
        self.vals.get(id as usize).and_then(Option::as_ref)
    }
    #[inline]
    pub fn at_rel(&self, rel: RelId, c: usize) -> Option<&T> {
        self.col_id(rel, c).and_then(|id| self.at(id))
    }
    #[inline]
    pub fn get_at(&self, rel: &str, c: usize) -> Option<&T> {
        find(rel).and_then(|r| self.at_rel(r, c))
    }
    #[inline]
    pub fn get<S: AsRef<str>>(&self, k: &(S, usize)) -> Option<&T> {
        self.get_at(k.0.as_ref(), k.1)
    }
    pub fn get_mut<S: AsRef<str>>(&mut self, k: &(S, usize)) -> Option<&mut T> {
        let id = self.col_id(find(k.0.as_ref())?, k.1)?;
        self.vals.get_mut(id as usize).and_then(Option::as_mut)
    }
    /// The slot of column `c` of `rel`, its block made or widened for it.
    fn slot(&mut self, rel: RelId, c: usize) -> usize {
        let r = rel as usize;
        if self.base.len() <= r {
            self.base.resize(r + 1, (u32::MAX, 0));
        }
        let (b, w) = self.base[r];
        if b != u32::MAX && (c as u32) < w {
            return (b + c as u32) as usize;
        }
        // A new block at the end, wide enough; a block outgrown moves there with its values.
        let width = (c as u32 + 1).max(WIDTH).max(w * 2);
        let at = self.vals.len() as u32;
        self.vals.resize_with(self.vals.len() + width as usize, || None);
        if b != u32::MAX {
            for i in 0..w {
                let v = self.vals[(b + i) as usize].take();
                self.vals[(at + i) as usize] = v;
            }
        }
        self.base[r] = (at, width);
        (at + c as u32) as usize
    }
    pub fn insert_at_rel(&mut self, rel: RelId, c: usize, v: T) -> Option<T> {
        let i = self.slot(rel, c);
        let old = self.vals[i].replace(v);
        if old.is_none() {
            self.n += 1;
        }
        old
    }
    pub fn insert<S: AsRef<str>>(&mut self, k: (S, usize), v: T) -> Option<T> {
        self.insert_at_rel(rel_id(k.0.as_ref()), k.1, v)
    }
    pub fn remove<S: AsRef<str>>(&mut self, k: &(S, usize)) -> Option<T> {
        let id = self.col_id(find(k.0.as_ref())?, k.1)?;
        let old = self.vals[id as usize].take();
        if old.is_some() {
            self.n -= 1;
        }
        old
    }
    pub fn contains_key<S: AsRef<str>>(&self, k: &(S, usize)) -> bool {
        self.get(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// The columns with a value: (relation, column), value.
    pub fn iter(&self) -> impl Iterator<Item = ((Arc<str>, usize), &T)> {
        self.base.iter().enumerate().filter(|(_, (b, _))| *b != u32::MAX).flat_map(move |(r, &(b, w))| {
            (0..w).filter_map(move |c| self.vals[(b + c) as usize].as_ref().map(|v| ((rel_name(r as RelId), c as usize), v)))
        })
    }
    /// Relation `rel`'s columns with a value. The iterator borrows `self` only, not `rel` (`use<'a, T>`; Relax's
    /// checker refused the capture of `rel`'s lifetime that edition 2024 implies, E0310).
    pub fn of_rel<'a>(&'a self, rel: &str) -> impl Iterator<Item = (usize, &'a T)> + use<'a, T> {
        let block = find(rel).and_then(|r| self.base.get(r as usize).copied()).filter(|(b, _)| *b != u32::MAX);
        block.into_iter().flat_map(move |(b, w)| (0..w).filter_map(move |c| self.vals[(b + c) as usize].as_ref().map(|v| (c as usize, v))))
    }
    pub fn clear(&mut self) {
        self.base.clear();
        self.vals.clear();
        self.n = 0;
    }
    /// The value of `k`, made with `f` where it has none.
    pub fn entry_or_insert_with<S: AsRef<str>>(&mut self, k: (S, usize), f: impl FnOnce() -> T) -> &mut T {
        let i = self.slot(rel_id(k.0.as_ref()), k.1);
        if self.vals[i].is_none() {
            self.vals[i] = Some(f());
            self.n += 1;
        }
        self.vals[i].as_mut().expect("just set")
    }
    /// Keeps the columns `keep` says to.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, usize, &mut T) -> bool) {
        for (r, &(b, w)) in self.base.iter().enumerate() {
            if b == u32::MAX {
                continue;
            }
            let name = rel_name(r as RelId);
            for c in 0..w {
                let slot = &mut self.vals[(b + c) as usize];
                if let Some(v) = slot.as_mut()
                    && !keep(&name, c as usize, v)
                {
                    *slot = None;
                    self.n -= 1;
                }
            }
        }
    }
}

impl<T, S: AsRef<str>> FromIterator<((S, usize), T)> for ColMap<T> {
    fn from_iter<I: IntoIterator<Item = ((S, usize), T)>>(it: I) -> Self {
        let mut m = ColMap::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl<T, S: AsRef<str>> Extend<((S, usize), T)> for ColMap<T> {
    fn extend<I: IntoIterator<Item = ((S, usize), T)>>(&mut self, it: I) {
        for (k, v) in it {
            self.insert(k, v);
        }
    }
}

impl<T> std::ops::Index<&str> for RelMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &str) -> &T {
        self.get(k).unwrap_or_else(|| panic!("no statistic of {k}"))
    }
}

impl<T> std::ops::Index<&String> for RelMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &String) -> &T {
        &self[k.as_str()]
    }
}

impl<T, S: AsRef<str>> std::ops::Index<&(S, usize)> for ColMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &(S, usize)) -> &T {
        self.get(k).unwrap_or_else(|| panic!("no statistic of {}.{}", k.0.as_ref(), k.1))
    }
}

/// Decision 0209 §4, level 1: how many of a column's values are kept by their counts.
pub const HEAVY: usize = 64;

/// A column's heavy values: its most common values with their exact rows, and what is left (the tail), made over
/// `rows` rows (decision 0209 §4).
#[derive(Clone, Debug, PartialEq)]
pub struct HeavyCol {
    /// The values and their rows, by value (for lookups), at most `HEAVY`.
    pub top: Vec<(i64, f64)>,
    pub rows: f64,
    pub ndv: f64,
    /// Rows and distinct values outside `top`.
    pub tail_rows: f64,
    pub tail_ndv: f64,
    /// The sums of counts squared and cubed (the column's second and third moments).
    pub sq: f64,
    pub cube: f64,
}

impl HeavyCol {
    /// From a column's exact counts.
    pub fn of_counts(counts: impl IntoIterator<Item = (i64, u64)>) -> HeavyCol {
        let mut all: Vec<(i64, u64)> = counts.into_iter().filter(|c| c.1 > 0).collect();
        let (mut rows, mut sq, mut cube) = (0.0, 0.0, 0.0);
        for &(_, n) in &all {
            let n = n as f64;
            rows += n;
            sq += n * n;
            cube += n * n * n;
        }
        let ndv = all.len() as f64;
        if all.len() > HEAVY {
            all.select_nth_unstable_by(HEAVY - 1, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            all.truncate(HEAVY);
        }
        let mut top: Vec<(i64, f64)> = all.into_iter().map(|(v, n)| (v, n as f64)).collect();
        top.sort_unstable_by_key(|t| t.0);
        let held: f64 = top.iter().map(|t| t.1).sum();
        HeavyCol { tail_rows: rows - held, tail_ndv: ndv - top.len() as f64, top, rows, ndv, sq, cube }
    }
    /// Value `v`'s rows, where it is heavy.
    #[inline]
    pub fn count(&self, v: i64) -> Option<f64> {
        self.top.binary_search_by_key(&v, |t| t.0).ok().map(|i| self.top[i].1)
    }
    /// The rows of a value of the tail, on average.
    #[inline]
    pub fn tail_mean(&self) -> f64 {
        if self.tail_ndv >= 1.0 { self.tail_rows / self.tail_ndv } else { 0.0 }
    }
    /// A value's rows: its own where it is heavy, else the tail's mean.
    #[inline]
    pub fn rows_of(&self, v: i64) -> f64 {
        self.count(v).unwrap_or_else(|| self.tail_mean())
    }
}

/// Decision 0209 §4's price of a probe: the rows column `here` holds per binding of its variable, where the binding
/// was drawn through the columns `by` (the atoms placed before that hold it). A value's weight is the product of its
/// rows in each of `by`, so a value common in all is drawn more often: value by value over the heavy values of any,
/// a value heavy in one only taken at the others' tail mean, and the tails' shared values (containment: as many as
/// the fewest any has left) at their means. `None` where no value is drawn.
pub fn heavy_fan(here: &HeavyCol, by: &[&HeavyCol]) -> Option<f64> {
    let mut vals: Vec<i64> = here.top.iter().map(|t| t.0).chain(by.iter().flat_map(|h| h.top.iter().map(|t| t.0))).collect();
    vals.sort_unstable();
    vals.dedup();
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for &v in &vals {
        let w: f64 = by.iter().map(|h| h.rows_of(v)).product();
        num += w * here.rows_of(v);
        den += w;
    }
    // The values heavy in none: each column's tail less the values of `vals` it holds in its tail.
    let left = |h: &HeavyCol| (h.tail_ndv - (vals.len() - h.top.len()) as f64).max(0.0);
    let shared = by.iter().map(|h| left(h)).fold(left(here), f64::min);
    let w: f64 = by.iter().map(|h| h.tail_mean()).product();
    num += shared * w * here.tail_mean();
    den += shared * w;
    (den > 0.0).then(|| num / den)
}

/// Decision 0209 §6, the CPU batch: each relation's requested columns' heavy values, in one pass over its rows (its
/// source, where it has one), the relations spread over the pool.
pub fn heavy_batch(reqs: Vec<(Option<std::sync::Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, Vec<usize>)>) -> Vec<Option<Vec<HeavyCol>>> {
    use rayon::prelude::*;
    reqs.into_par_iter()
        .map(|(src, cols)| {
            let src = src?;
            let mut counts: Vec<FxHashMap<i64, u64>> = cols.iter().map(|_| FxHashMap::default()).collect();
            for row in 0..src.len() {
                for (k, &c) in cols.iter().enumerate() {
                    *counts[k].entry(src.value(row, c)).or_default() += 1;
                }
            }
            Some(counts.into_iter().map(HeavyCol::of_counts).collect())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_maps_read_back_what_they_hold_as_blocks_grow() {
        let mut card: RelMap<f64> = RelMap::new();
        card.insert("t_rel_a", 3.0);
        card.insert(String::from("t_rel_b"), 4.0);
        assert_eq!((card.get("t_rel_a"), card.get(&String::from("t_rel_b")), card.get("t_rel_none")), (Some(&3.0), Some(&4.0), None));
        assert_eq!(card.remove("t_rel_a"), Some(3.0));
        assert_eq!(card.len(), 1);
        let mut fan: ColMap<f64> = ColMap::new();
        fan.insert(("t_rel_a", 1), 1.5);
        fan.insert(("t_rel_b".to_owned(), 0), 2.5);
        // Past the block's width: the block moves, with its values.
        fan.insert(("t_rel_a", 9), 9.5);
        assert_eq!((fan.get(&("t_rel_a", 1)), fan.get_at("t_rel_a", 9), fan.get(&("t_rel_b", 0)), fan.get(&("t_rel_a", 2))), (Some(&1.5), Some(&9.5), Some(&2.5), None));
        assert_eq!(fan.len(), 3);
        let a = find("t_rel_a").unwrap();
        let id = fan.col_id(a, 9).unwrap();
        assert_eq!(fan.at(id), Some(&9.5));
        assert_eq!(fan.of_rel("t_rel_a").collect::<Vec<_>>(), vec![(1, &1.5), (9, &9.5)]);
        fan.retain(|_, c, _| c != 1);
        assert_eq!((fan.len(), fan.get(&("t_rel_a", 1))), (2, None));
    }
    #[test]
    fn heavy_values_price_a_skewed_self_join_value_by_value() {
        // 100 values of 1 row and one of 300: a three-way self-join on the column.
        let counts: Vec<(i64, u64)> = (0..100).map(|v| (v, 1)).chain([(1000, 300)]).collect();
        let true3: f64 = counts.iter().map(|&(_, n)| (n as f64).powi(3)).sum();
        let true2: f64 = counts.iter().map(|&(_, n)| (n as f64).powi(2)).sum();
        let h = HeavyCol::of_counts(counts.clone());
        assert_eq!((h.rows, h.ndv, h.top.len(), h.count(1000), h.tail_ndv), (400.0, 101.0, 64, Some(300.0), 37.0));
        // The second atom's rows a binding are sq / rows; the third's, cube / sq.
        assert!((heavy_fan(&h, &[&h]).unwrap() - true2 / 400.0).abs() < 1e-9);
        assert!((heavy_fan(&h, &[&h, &h]).unwrap() - true3 / true2).abs() < 1e-9);
        assert_eq!((h.sq, h.cube), (true2, true3));
    }
}
