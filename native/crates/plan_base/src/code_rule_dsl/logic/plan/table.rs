//! Decision 0209 §3: the planner's statistics as flat arrays.
//!
//! A relation is numbered once in the process (`rel_id`), by its name. A statistic of relations (`RelMap`) is an
//! array by that number. A statistic of columns (`ColMap`) gives each relation number its columns' shared vector: a
//! column is found by two array reads once its relation's number is known, and a strand's model shares a relation's
//! columns by a count (decision 0210 §9). Readers that hold numbers (`Model`'s prepared atoms) read by number; readers
//! by name pay one lookup of the name, not a string built per key.

use rerail_blocks::maps::BlockHash;
use rerail_blocks::vector::Vector;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// A relation's number in this process.
pub type RelId = u32;

#[derive(Default)]
struct Names {
    ids: FxHashMap<Arc<str>, RelId>,
    names: Vec<Arc<str>>,
}

/// Read on every lookup of a relation by name, on every thread of the pool, and written only for a new name: sharded
/// by reader, so lookups on different threads write no shared lock word (decision 0279 §7), and lent (`with_shared`),
/// so they count no reference either. The shared object's (decision 0442): a number is its own object's.
struct RelNames(rerail_accel::sharded::Sharded<Names>);

impl Default for RelNames {
    fn default() -> Self {
        RelNames(rerail_accel::sharded::Sharded::new(Names::default()))
    }
}

fn with_names<R>(f: impl FnOnce(&rerail_accel::sharded::Sharded<Names>) -> R) -> R {
    crate::code_rule_dsl::logic::profile::with_shared::<RelNames, _>(|n| f(&n.0))
}

/// `name`'s number, given it on first sight.
pub fn rel_id(name: &str) -> RelId {
    with_names(|names| {
        if let Some(&id) = names.read().ids.get(name) {
            return id;
        }
        let mut n = names.write();
        if let Some(&id) = n.ids.get(name) {
            return id;
        }
        let id = n.names.len() as RelId;
        let name: Arc<str> = name.into();
        n.names.push(Arc::clone(&name));
        n.ids.insert(name, id);
        id
    })
}

/// `name`'s number, where it has one.
pub fn find(name: &str) -> Option<RelId> {
    with_names(|names| names.read().ids.get(name).copied())
}

/// The name numbered `id`.
pub fn rel_name(id: RelId) -> Arc<str> {
    with_names(|names| Arc::clone(&names.read().names[id as usize]))
}

/// A statistic's value as an overlay layer holds it (decision 0628 §1): a number in the layer's block vector, or a
/// handle to what lies in blocks of its own (a heavy column's values, a column's keys or counts, a relation's mark).
pub trait Stat: Clone {
    type Store: Default;
    /// `v` put in `s`: its slot.
    fn put(s: &mut Self::Store, v: Self) -> u32;
    fn at(s: &Self::Store, i: u32) -> &Self;
    fn at_mut(s: &mut Self::Store, i: u32) -> &mut Self;
}

impl Stat for f64 {
    type Store = Vector<f64>;
    fn put(s: &mut Vector<f64>, v: f64) -> u32 {
        s.push(v);
        (s.len() - 1) as u32
    }
    fn at(s: &Vector<f64>, i: u32) -> &f64 {
        &s[i as usize]
    }
    fn at_mut(s: &mut Vector<f64>, i: u32) -> &mut f64 {
        &mut s[i as usize]
    }
}

/// A handle's slots: the handles that keep the blocks their values lie in (not block data).
macro_rules! handle_stat {
    ($($t:ty),* $(,)?) => {$(
        impl Stat for $t {
            type Store = Vec<$t>;
            fn put(s: &mut Vec<$t>, v: $t) -> u32 {
                s.push(v);
                (s.len() - 1) as u32
            }
            fn at(s: &Vec<$t>, i: u32) -> &$t {
                &s[i as usize]
            }
            fn at_mut(s: &mut Vec<$t>, i: u32) -> &mut $t {
                &mut s[i as usize]
            }
        }
    )*};
}
handle_stat!(Arc<rerail_section::Section<super::joins::Note>>, Arc<rerail_section::Section<Vec<super::joins::Proposal>>>, Arc<BlockHash<u64, [i64; 2]>>, Arc<dyn super::Deferred>, Arc<HeavyCol>, (Arc<crate::code_rule_dsl::logic::native::component::RelMark>, Arc<HeavyCol>), Arc<Freq>, Arc<crate::code_rule_dsl::logic::join_size::JoinCounts>, Arc<crate::code_rule_dsl::logic::native::component::RelMark>, crate::code_rule_dsl::logic::bloom::Keys);

/// A statistic's key: a relation's number, or (relation, column).
pub trait StatKey: rerail_blocks::maps::Key + Ord {
    fn rel(&self) -> RelId;
    /// Its column, for a statistic of columns.
    fn col(&self) -> Option<u32>;
}
impl StatKey for u32 {
    fn rel(&self) -> RelId {
        *self
    }
    fn col(&self) -> Option<u32> {
        None
    }
}
/// A key of no relation (the join memo's, `joins::Memo`): never seen through a fork's relations.
impl StatKey for u64 {
    fn rel(&self) -> RelId {
        RelId::MAX
    }
    fn col(&self) -> Option<u32> {
        None
    }
}
impl StatKey for [u32; 2] {
    fn rel(&self) -> RelId {
        self[0]
    }
    fn col(&self) -> Option<u32> {
        Some(self[1])
    }
}

/// A key's slot that says it is gone: a removal over a layer below that holds it.
const GONE: u32 = u32::MAX;
/// Layers deeper than this are folded into one as a write comes (as a version folds, decision 0241).
const FOLD_DEPTH: u32 = 8;

/// One layer of a statistic: its own entries (a slot of `vals`, or `GONE`), over the layer below it, of which it
/// shows the relations `shown` names (each below its width), or every one.
struct Layer<K: StatKey, T: Stat> {
    at: BlockHash<K, u32>,
    vals: T::Store,
    below: Option<(Arc<Layer<K, T>>, Option<Arc<BlockHash<RelId, u32>>>)>,
    depth: u32,
}

impl<K: StatKey, T: Stat> Layer<K, T> {
    fn empty() -> Self {
        Layer { at: BlockHash::default(), vals: T::Store::default(), below: None, depth: 0 }
    }
}

/// Whether `shown` shows `k`: its relation, and its column below the relation's width.
fn shows<K: StatKey>(shown: &BlockHash<RelId, u32>, k: &K) -> bool {
    shown.get(&k.rel()).is_some_and(|&w| k.col().is_none_or(|c| c < w))
}

/// A statistic in blocks (decision 0628 §1): its top layer written, over the frozen layers its forks share. A fork
/// (`shown_of`) is a layer over its parent's top, which the parent's next write leaves for a layer of its own: a
/// fork costs what it writes.
pub struct Overlay<K: StatKey, T: Stat> {
    top: Arc<Layer<K, T>>,
}

impl<K: StatKey, T: Stat> Default for Overlay<K, T> {
    fn default() -> Self {
        Overlay { top: Arc::new(Layer::empty()) }
    }
}

/// A clone shares every layer: its next write, and the original's, each go to a layer of its own.
impl<K: StatKey, T: Stat> Clone for Overlay<K, T> {
    fn clone(&self) -> Self {
        Overlay { top: Arc::clone(&self.top) }
    }
}

impl<K: StatKey, T: Stat> Overlay<K, T> {
    /// A fork of `from` that shows the relations `shown` names, each below its width.
    pub(super) fn shown_of(from: &Overlay<K, T>, shown: &Arc<BlockHash<RelId, u32>>) -> Self {
        let depth = from.top.depth + 1;
        Overlay { top: Arc::new(Layer { below: Some((Arc::clone(&from.top), Some(Arc::clone(shown)))), depth, ..Layer::empty() }) }
    }

    pub(super) fn get(&self, k: &K) -> Option<&T> {
        let mut layer = &*self.top;
        loop {
            if let Some(&s) = layer.at.get(k) {
                return (s != GONE).then(|| T::at(&layer.vals, s));
            }
            let (below, shown) = layer.below.as_ref()?;
            if shown.as_ref().is_some_and(|sh| !shows(sh, k)) {
                return None;
            }
            layer = below;
        }
    }

    /// Whether a layer below the top holds `k`, as the top shows it.
    fn below_holds(&self, k: &K) -> bool {
        let Some((below, shown)) = &self.top.below else { return false };
        shown.as_ref().is_none_or(|sh| shows(sh, k)) && (Overlay { top: Arc::clone(below) }).get(k).is_some()
    }

    /// The top layer, this statistic's alone: a new one over the shared top, or every layer folded into one past
    /// `FOLD_DEPTH`.
    fn writable(&mut self) -> &mut Layer<K, T> {
        if Arc::get_mut(&mut self.top).is_none() {
            let depth = self.top.depth + 1;
            if depth > FOLD_DEPTH {
                // copy: derived: an overlay's layers folded into one, as a version folds (0241)
                let mut flat = Layer::empty();
                for k in self.keys() {
                    let v = self.get(&k).expect("listed").clone();
                    let s = T::put(&mut flat.vals, v);
                    flat.at.insert(k, s);
                }
                self.top = Arc::new(flat);
            } else {
                self.top = Arc::new(Layer { below: Some((Arc::clone(&self.top), None)), depth, ..Layer::empty() });
            }
        }
        Arc::get_mut(&mut self.top).expect("just made this statistic's own")
    }

    pub(super) fn insert(&mut self, k: K, v: T) -> Option<T> {
        let old = self.get(&k).cloned();
        let top = self.writable();
        match top.at.get(&k) {
            Some(&s) if s != GONE => *T::at_mut(&mut top.vals, s) = v,
            _ => {
                let s = T::put(&mut top.vals, v);
                top.at.insert(k, s);
            }
        }
        old
    }

    pub(super) fn get_mut(&mut self, k: &K) -> Option<&mut T> {
        let v = self.get(k)?.clone();
        let top = self.writable();
        let s = match top.at.get(k) {
            Some(&s) if s != GONE => s,
            _ => {
                let s = T::put(&mut top.vals, v);
                top.at.insert(*k, s);
                s
            }
        };
        Some(T::at_mut(&mut top.vals, s))
    }

    pub(super) fn remove(&mut self, k: &K) -> Option<T> {
        let old = self.get(k).cloned()?;
        // Whether a layer under the one written holds it, once that one is this statistic's own.
        self.writable();
        let below = self.below_holds(k);
        let top = Arc::get_mut(&mut self.top).expect("made this statistic's own");
        if below {
            top.at.insert(*k, GONE);
        } else {
            top.at.remove(k);
        }
        Some(old)
    }

    /// The keys with a value, ascending.
    pub(super) fn keys(&self) -> Vector<K> {
        let mut seen: BlockHash<K, ()> = BlockHash::default();
        let mut out: Vector<K> = Vector::new();
        // The relations each layer is seen through: every `shown` above it.
        let mut through: Vec<&BlockHash<RelId, u32>> = Vec::new();
        let mut layer = Some(&*self.top);
        while let Some(l) = layer {
            for (k, s) in l.at.iter() {
                if through.iter().all(|sh| shows(sh, &k)) && seen.put(k) && s != GONE {
                    out.push(k);
                }
            }
            layer = l.below.as_ref().map(|(below, shown)| {
                if let Some(sh) = shown {
                    through.push(sh);
                }
                &**below
            });
        }
        out.sort_unstable();
        out
    }

    fn clear(&mut self) {
        self.top = Arc::new(Layer::empty());
    }

    /// Whether any key has a value: the first one found ends the search.
    pub(super) fn any(&self) -> bool {
        let mut layer = Some(&*self.top);
        while let Some(l) = layer {
            if l.at.iter().any(|(k, _)| self.get(&k).is_some()) {
                return true;
            }
            layer = l.below.as_ref().map(|(below, _)| &**below);
        }
        false
    }
}

/// A statistic of relations: a value per relation number.
pub struct RelMap<T: Stat>(Overlay<RelId, T>);

impl<T: Stat> Default for RelMap<T> {
    fn default() -> Self {
        RelMap(Overlay::default())
    }
}
impl<T: Stat> Clone for RelMap<T> {
    fn clone(&self) -> Self {
        RelMap(self.0.clone())
    }
}
impl<T: Stat + std::fmt::Debug> std::fmt::Debug for RelMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T: Stat> RelMap<T> {
    pub fn new() -> Self {
        Self::default()
    }
    /// A fork's: `from`'s relations `shown` names, shared.
    pub fn shown_of(from: &RelMap<T>, shown: &Arc<BlockHash<RelId, u32>>) -> Self {
        RelMap(Overlay::shown_of(&from.0, shown))
    }
    #[inline]
    pub fn at(&self, id: RelId) -> Option<&T> {
        self.0.get(&id)
    }
    #[inline]
    pub fn get<S: AsRef<str> + ?Sized>(&self, k: &S) -> Option<&T> {
        find(k.as_ref()).and_then(|id| self.at(id))
    }
    pub fn get_mut<S: AsRef<str> + ?Sized>(&mut self, k: &S) -> Option<&mut T> {
        let id = find(k.as_ref())?;
        self.0.get_mut(&id)
    }
    pub fn insert_at(&mut self, id: RelId, v: T) -> Option<T> {
        self.0.insert(id, v)
    }
    pub fn insert<S: AsRef<str>>(&mut self, k: S, v: T) -> Option<T> {
        self.insert_at(rel_id(k.as_ref()), v)
    }
    pub fn remove<S: AsRef<str> + ?Sized>(&mut self, k: &S) -> Option<T> {
        let id = find(k.as_ref())?;
        self.0.remove(&id)
    }
    pub fn contains_key<S: AsRef<str> + ?Sized>(&self, k: &S) -> bool {
        self.get(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.0.keys().len()
    }
    pub fn is_empty(&self) -> bool {
        !self.0.any()
    }
    /// The relations with a value, by name, in their numbers' order.
    pub fn iter(&self) -> impl Iterator<Item = (Arc<str>, &T)> {
        let keys = self.0.keys();
        (0..keys.len()).map(move |i| (rel_name(keys[i]), self.0.get(&keys[i]).expect("listed")))
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.iter().map(|(_, v)| v)
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    /// The value of `k`, made with `f` where it has none.
    pub fn entry_or_insert_with<S: AsRef<str>>(&mut self, k: S, f: impl FnOnce() -> T) -> &mut T {
        let id = rel_id(k.as_ref());
        if self.0.get(&id).is_none() {
            self.0.insert(id, f());
        }
        self.0.get_mut(&id).expect("just set")
    }
}

impl<T: Stat, S: AsRef<str>> FromIterator<(S, T)> for RelMap<T> {
    fn from_iter<I: IntoIterator<Item = (S, T)>>(it: I) -> Self {
        let mut m = RelMap::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl<T: Stat, S: AsRef<str>> Extend<(S, T)> for RelMap<T> {
    fn extend<I: IntoIterator<Item = (S, T)>>(&mut self, it: I) {
        for (k, v) in it {
            self.insert(k, v);
        }
    }
}

/// A statistic of columns (`Model::ndv`, `Stats::fan`, `dfreq`, `heavy`): one overlay keyed by (relation number,
/// column). A strand's model (`Model::fork`) is a layer over its parent's that shows its relations' columns below
/// their widths (decision 0628 §1, which replaced 0210 §9's shared vectors).
pub struct ColMap<T: Stat>(Overlay<[u32; 2], T>);

impl<T: Stat> Default for ColMap<T> {
    fn default() -> Self {
        ColMap(Overlay::default())
    }
}
impl<T: Stat> Clone for ColMap<T> {
    fn clone(&self) -> Self {
        ColMap(self.0.clone())
    }
}
impl<T: Stat + std::fmt::Debug> std::fmt::Debug for ColMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T: Stat> ColMap<T> {
    pub fn new() -> Self {
        Self::default()
    }
    /// A fork's: `from`'s columns of the relations `shown` names, below their widths, shared.
    pub fn shown_of(from: &ColMap<T>, shown: &Arc<BlockHash<RelId, u32>>) -> Self {
        ColMap(Overlay::shown_of(&from.0, shown))
    }
    #[inline]
    pub fn at_rel(&self, rel: RelId, c: usize) -> Option<&T> {
        self.0.get(&[rel, c as u32])
    }
    /// Column `c` of `rel`, by name.
    #[inline]
    pub fn at(&self, rel: &str, c: usize) -> Option<&T> {
        find(rel).and_then(|r| self.at_rel(r, c))
    }
    #[inline]
    pub fn get<S: AsRef<str>>(&self, k: &(S, usize)) -> Option<&T> {
        self.at(k.0.as_ref(), k.1)
    }
    pub fn get_mut<S: AsRef<str>>(&mut self, k: &(S, usize)) -> Option<&mut T> {
        let r = find(k.0.as_ref())?;
        self.0.get_mut(&[r, k.1 as u32])
    }
    pub fn insert_at_rel(&mut self, rel: RelId, c: usize, v: T) -> Option<T> {
        self.0.insert([rel, c as u32], v)
    }
    pub fn insert<S: AsRef<str>>(&mut self, k: (S, usize), v: T) -> Option<T> {
        self.insert_at_rel(rel_id(k.0.as_ref()), k.1, v)
    }
    /// The value at `k`, `v` put there first where there is none (as `entry(…).or_insert(v)`).
    pub fn or_insert<S: AsRef<str>>(&mut self, k: (S, usize), v: T) -> &mut T {
        self.entry_or_insert_with(k, || v)
    }
    /// The value of `k`, made with `f` where it has none.
    pub fn entry_or_insert_with<S: AsRef<str>>(&mut self, k: (S, usize), f: impl FnOnce() -> T) -> &mut T {
        let key = [rel_id(k.0.as_ref()), k.1 as u32];
        if self.0.get(&key).is_none() {
            self.0.insert(key, f());
        }
        self.0.get_mut(&key).expect("just set")
    }
    pub fn remove<S: AsRef<str>>(&mut self, k: &(S, usize)) -> Option<T> {
        let r = find(k.0.as_ref())?;
        self.0.remove(&[r, k.1 as u32])
    }
    pub fn contains_key<S: AsRef<str>>(&self, k: &(S, usize)) -> bool {
        self.get(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.0.keys().len()
    }
    pub fn is_empty(&self) -> bool {
        !self.0.any()
    }
    /// The columns with a value: (relation, column), value, in (number, column) order.
    pub fn iter(&self) -> impl Iterator<Item = ((Arc<str>, usize), &T)> {
        let keys = self.0.keys();
        (0..keys.len()).map(move |i| {
            let [r, c] = keys[i];
            ((rel_name(r), c as usize), self.0.get(&keys[i]).expect("listed"))
        })
    }
    /// Relation `rel`'s columns with a value. The iterator borrows `self` only, not `rel` (`use<'a, T>`; Relax's
    /// checker refused the capture of `rel`'s lifetime that edition 2024 implies, E0310).
    pub fn of_rel<'a>(&'a self, rel: &str) -> impl Iterator<Item = (usize, &'a T)> + use<'a, T> {
        let r = find(rel);
        let keys: Vector<[u32; 2]> = r.map_or_else(Vector::new, |r| self.0.keys().iter().copied().filter(|k| k[0] == r).collect());
        (0..keys.len()).map(move |i| (keys[i][1] as usize, self.0.get(&keys[i]).expect("listed")))
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    /// Keeps the columns `keep` says to.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, usize, &mut T) -> bool) {
        for k in self.0.keys().iter() {
            let name = rel_name(k[0]);
            let kept = keep(&name, k[1] as usize, self.0.get_mut(k).expect("listed"));
            if !kept {
                self.0.remove(k);
            }
        }
    }
    /// Takes `from`'s columns of `rel` below `width` (a test's fork; `Model::fork` makes its own by `shown_of`).
    pub fn share_from(&mut self, from: &ColMap<T>, rel: &str, width: usize) {
        for c in 0..width {
            if let Some(v) = from.at(rel, c) {
                self.insert((rel, c), v.clone());
            }
        }
    }
    /// Sets `rel`'s columns below `width` to `from`'s (each taken, or none), as one entry each was moved
    /// (`Model::absorb`).
    pub fn take_from(&mut self, from: &mut ColMap<T>, rel: &str, width: usize) {
        for c in 0..width {
            match from.at(rel, c).cloned() {
                Some(v) => {
                    self.insert((rel, c), v);
                }
                None => {
                    self.remove(&(rel, c));
                }
            }
        }
    }
}

impl<T: Stat, S: AsRef<str>> FromIterator<((S, usize), T)> for ColMap<T> {
    fn from_iter<I: IntoIterator<Item = ((S, usize), T)>>(it: I) -> Self {
        let mut m = ColMap::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl<T: Stat, S: AsRef<str>> Extend<((S, usize), T)> for ColMap<T> {
    fn extend<I: IntoIterator<Item = ((S, usize), T)>>(&mut self, it: I) {
        for (k, v) in it {
            self.insert(k, v);
        }
    }
}

impl<T: Stat> std::ops::Index<&str> for RelMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &str) -> &T {
        self.get(k).unwrap_or_else(|| panic!("no statistic of {k}"))
    }
}

impl<T: Stat> std::ops::Index<&String> for RelMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &String) -> &T {
        &self[k.as_str()]
    }
}

impl<T: Stat, S: AsRef<str>> std::ops::Index<&(S, usize)> for ColMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &(S, usize)) -> &T {
        self.get(k).unwrap_or_else(|| panic!("no statistic of {}.{}", k.0.as_ref(), k.1))
    }
}

/// A statistic of named columns: (relation, column name), each numbered by `rel_id` (decision 0628 §4). A fork
/// shows a relation's every column, whatever its width (its key has no column index).
impl StatKey for [u64; 2] {
    fn rel(&self) -> RelId {
        self[0] as RelId
    }
    fn col(&self) -> Option<u32> {
        None
    }
}

/// A statistic of a relation's columns by name (`Stats::col`, `Stats::memo`): one overlay keyed by (relation number,
/// name number).
pub struct NamedMap<T: Stat>(Overlay<[u64; 2], T>);

impl<T: Stat> Default for NamedMap<T> {
    fn default() -> Self {
        NamedMap(Overlay::default())
    }
}
impl<T: Stat> Clone for NamedMap<T> {
    fn clone(&self) -> Self {
        NamedMap(self.0.clone())
    }
}
impl<T: Stat + std::fmt::Debug> std::fmt::Debug for NamedMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

fn named_key(rel: &str, name: &str) -> Option<[u64; 2]> {
    Some([u64::from(find(rel)?), u64::from(find(name)?)])
}

impl<T: Stat> NamedMap<T> {
    /// A fork's: `from`'s columns of the relations `shown` names, shared.
    pub fn shown_of(from: &NamedMap<T>, shown: &Arc<BlockHash<RelId, u32>>) -> Self {
        NamedMap(Overlay::shown_of(&from.0, shown))
    }
    /// `rel`'s column `name`'s.
    pub fn get(&self, rel: &str, name: &str) -> Option<&T> {
        self.0.get(&named_key(rel, name)?)
    }
    pub fn get_mut(&mut self, rel: &str, name: &str) -> Option<&mut T> {
        let k = named_key(rel, name)?;
        self.0.get_mut(&k)
    }
    pub fn contains(&self, rel: &str, name: &str) -> bool {
        self.get(rel, name).is_some()
    }
    pub fn insert(&mut self, rel: &str, name: &str, v: T) -> Option<T> {
        self.0.insert([u64::from(rel_id(rel)), u64::from(rel_id(name))], v)
    }
    /// The value of `rel`'s `name`, made with `f` where it has none.
    pub fn entry_or_insert_with(&mut self, rel: &str, name: &str, f: impl FnOnce() -> T) -> &mut T {
        let k = [u64::from(rel_id(rel)), u64::from(rel_id(name))];
        if self.0.get(&k).is_none() {
            self.0.insert(k, f());
        }
        self.0.get_mut(&k).expect("just set")
    }
    pub fn remove(&mut self, rel: &str, name: &str) -> Option<T> {
        let k = named_key(rel, name)?;
        self.0.remove(&k)
    }
    pub fn len(&self) -> usize {
        self.0.keys().len()
    }
    pub fn is_empty(&self) -> bool {
        !self.0.any()
    }
    /// Whether any column of `rel` has a value.
    pub fn has(&self, rel: &str) -> bool {
        self.of_rel(rel).next().is_some()
    }
    /// Each (relation, column name) with a value, in their numbers' order.
    pub fn iter(&self) -> impl Iterator<Item = ((Arc<str>, Arc<str>), &T)> {
        let keys = self.0.keys();
        (0..keys.len()).map(move |i| {
            let [r, n] = keys[i];
            ((rel_name(r as RelId), rel_name(n as RelId)), self.0.get(&keys[i]).expect("listed"))
        })
    }
    /// `rel`'s columns with a value, by name.
    pub fn of_rel<'a>(&'a self, rel: &str) -> impl Iterator<Item = (Arc<str>, &'a T)> + use<'a, T> {
        let r = find(rel).map(u64::from);
        let keys: Vector<[u64; 2]> = r.map_or_else(Vector::new, |r| self.0.keys().iter().copied().filter(|k| k[0] == r).collect());
        (0..keys.len()).map(move |i| (rel_name(keys[i][1] as RelId), self.0.get(&keys[i]).expect("listed")))
    }
}

impl<T: Stat, R: AsRef<str>, N: AsRef<str>> FromIterator<((R, N), T)> for NamedMap<T> {
    fn from_iter<I: IntoIterator<Item = ((R, N), T)>>(it: I) -> Self {
        let mut m = NamedMap::default();
        for ((r, n), v) in it {
            m.insert(r.as_ref(), n.as_ref(), v);
        }
        m
    }
}

impl NamedMap<Arc<Freq>> {
    /// `rel`'s column `name`'s postings with `more`'s, which replace its own of the same values.
    pub fn extend(&mut self, rel: &str, name: &str, more: impl IntoIterator<Item = (super::Constant, i64)>) {
        let f = Arc::make_mut(self.entry_or_insert_with(rel, name, || Arc::new(Freq::default())));
        for (k, n) in more {
            f.insert(k, n);
        }
    }
}

/// A statistic's number in the layer's block vector.
macro_rules! vector_stat {
    ($($t:ty),* $(,)?) => {$(
        impl Stat for $t {
            type Store = Vector<$t>;
            fn put(s: &mut Vector<$t>, v: $t) -> u32 {
                s.push(v);
                (s.len() - 1) as u32
            }
            fn at(s: &Vector<$t>, i: u32) -> &$t {
                &s[i as usize]
            }
            fn at_mut(s: &mut Vector<$t>, i: u32) -> &mut $t {
                &mut s[i as usize]
            }
        }
    )*};
}
vector_stat!([i64; 4], u32, u64);

/// A column's few values' postings (`Stats::dfreq`), in blocks: an integer's by its value, a string's by its text.
#[derive(Clone, Default)]
pub struct Freq {
    ints: BlockHash<i64, i64>,
    /// The strings' texts (`Constant::Str`), and each one's postings by its index.
    strs: Option<rerail_blocks::maps::BlockInterner>,
    str_n: Vector<i64>,
}

impl std::fmt::Debug for Freq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl Freq {
    pub fn get(&self, k: &super::Constant) -> Option<&i64> {
        match k {
            super::Constant::Int(i) => self.ints.get(i),
            super::Constant::Str(s) => self.strs.as_ref()?.find(s).map(|i| &self.str_n[i]),
        }
    }
    pub fn insert(&mut self, k: super::Constant, n: i64) {
        match k {
            super::Constant::Int(i) => {
                self.ints.insert(i, n);
            }
            super::Constant::Str(s) => {
                let strs = self.strs.get_or_insert_with(|| rerail_blocks::maps::BlockInterner::new(rerail_blocks::maps::default_room()));
                let i = strs.intern(&s);
                if i == self.str_n.len() {
                    self.str_n.push(n);
                } else {
                    self.str_n[i] = n;
                }
            }
        }
    }
    pub fn len(&self) -> usize {
        self.ints.len() + self.str_n.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Each value and its postings: the integers, then the strings.
    pub fn iter(&self) -> impl Iterator<Item = (super::Constant, i64)> + '_ {
        let strs = self.strs.iter().flat_map(|s| s.iter().enumerate()).map(|(i, s)| (super::Constant::Str(s.to_owned()), self.str_n[i]));
        self.ints.iter().map(|(v, n)| (super::Constant::Int(v), n)).chain(strs)
    }
    pub fn keys(&self) -> impl Iterator<Item = super::Constant> + '_ {
        self.iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = i64> + '_ {
        self.iter().map(|(_, n)| n)
    }
}

impl PartialEq for Freq {
    fn eq(&self, other: &Freq) -> bool {
        self.len() == other.len() && self.iter().all(|(k, n)| other.get(&k) == Some(&n))
    }
}

impl FromIterator<(super::Constant, i64)> for Freq {
    fn from_iter<I: IntoIterator<Item = (super::Constant, i64)>>(it: I) -> Self {
        let mut f = Freq::default();
        for (k, n) in it {
            f.insert(k, n);
        }
        f
    }
}

/// Decision 0209 §4, level 1: how many of a column's values are reported by their counts.
pub const HEAVY: usize = 64;
/// The counters a column keeps: twice what it reports, so a value that grows into the top is seen as a delta is
/// merged (a Misra–Gries summary's 2k counters).
pub const KEPT: usize = 2 * HEAVY;

/// A heavy value and its rows (`HeavyCol::top`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeavyVal(pub i64, pub f64);
// SAFETY: `repr(C)`, two 8-byte numbers, no padding.
unsafe impl rerail_blocks::Pod for HeavyVal {}

/// A column's heavy values: its most common values with their exact rows, and what is left (the tail), made over
/// `rows` rows (decision 0209 §4).
#[derive(Clone, Debug, PartialEq)]
pub struct HeavyCol {
    /// The values and their rows, by value (for lookups), at most `KEPT`.
    pub top: Vector<HeavyVal>,
    /// How many rows a value's count may miss: 0 when made by a full pass; after merges (`merge`), the most a value
    /// left out of `top` held, which a value that came in since may have had before it came.
    pub err: f64,
    pub rows: f64,
    pub ndv: f64,
    /// Rows and distinct values outside `top`.
    pub tail_rows: f64,
    pub tail_ndv: f64,
    /// The sums of counts squared and cubed (the column's second and third moments).
    pub sq: f64,
    pub cube: f64,
}

/// A column as `heavy_fan` prices it: its heavy values, and its keys where the engine keeps them (decision 0488: the
/// relation's column's own filter or few values, `logic::bloom::Keys`, read where it lies; no filter is made for the
/// statistics). The keys say whether a value outside `top` is held at all, and how many values two columns share.
#[derive(Clone, Copy, Debug)]
pub struct PricedCol<'a> {
    pub heavy: &'a HeavyCol,
    pub keys: Option<&'a crate::code_rule_dsl::logic::bloom::Keys>,
}

impl<'a> PricedCol<'a> {
    pub fn new(heavy: &'a HeavyCol, keys: Option<&'a crate::code_rule_dsl::logic::bloom::Keys>) -> PricedCol<'a> {
        PricedCol { heavy, keys }
    }

    /// A value's rows: its own where it is heavy, none where the column's keys refuse it, else the tail's mean.
    #[inline]
    pub fn rows_of(&self, v: i64) -> f64 {
        self.heavy.count(v).unwrap_or_else(|| if self.keys.is_some_and(|k| !k.may_hold(v)) { 0.0 } else { self.heavy.tail_mean() })
    }
}

impl HeavyCol {
    /// From a column's exact counts.
    pub fn of_counts(counts: impl IntoIterator<Item = (i64, u64)>) -> HeavyCol {
        let mut all: Vector<[i64; 2]> = counts.into_iter().filter(|c| c.1 > 0).map(|(v, n)| [v, n as i64]).collect();
        let (mut rows, mut sq, mut cube) = (0.0, 0.0, 0.0);
        for &[_, n] in all.iter() {
            let n = n as f64;
            rows += n;
            sq += n * n;
            cube += n * n * n;
        }
        let ndv = all.len() as f64;
        if all.len() > KEPT {
            all.select_nth_unstable_by(KEPT - 1, |a, b| b[1].cmp(&a[1]).then(a[0].cmp(&b[0])));
            all.truncate(KEPT);
        }
        let mut top: Vector<HeavyVal> = all.iter().map(|&[v, n]| HeavyVal(v, n as f64)).collect();
        top.sort_unstable_by_key(|t| t.0);
        let held: f64 = top.iter().map(|t| t.1).sum();
        HeavyCol { tail_rows: rows - held, tail_ndv: ndv - top.len() as f64, top, err: 0.0, rows, ndv, sq, cube }
    }
    /// Decision 0209 §4's merge (O172): the column after a change of `added` and `removed` rows a value, its
    /// distinct values now `ndv` (the sketch's). A kept value's count moves by its change, exact if it was. A value
    /// not kept that the change adds comes in at what it added, and the kept stay the `KEPT` largest; the rest are
    /// the tail. sq and cube move by each value's change, a tail value's taken at the tail's mean.
    pub fn merge(&self, added: &FxHashMap<i64, u64>, removed: &FxHashMap<i64, u64>, ndv: f64) -> HeavyCol {
        let mean = self.tail_mean();
        let mut counts: BlockHash<i64, f64> = self.top.iter().map(|t| (t.0, t.1)).collect();
        let (mut rows, mut sq, mut cube) = (self.rows, self.sq, self.cube);
        let mut tail_rows = self.tail_rows;
        let mut moved = |was: f64, now: f64| {
            sq += now * now - was * was;
            cube += now * now * now - was * was * was;
        };
        for (&v, &a) in added {
            let a = a as f64;
            rows += a;
            match counts.get_mut(&v) {
                Some(n) => { moved(*n, *n + a); *n += a; }
                None => { moved(mean, mean + a); counts.insert(v, a); }
            }
        }
        for (&v, &r) in removed {
            let r = r as f64;
            rows -= r;
            match counts.get_mut(&v) {
                Some(n) => { let now = (*n - r).max(0.0); moved(*n, now); *n = now; }
                None => { tail_rows -= r; moved(mean, (mean - r).max(0.0)); }
            }
        }
        let mut all: Vector<HeavyVal> = counts.iter().filter(|c| c.1 > 0.0).map(|(v, n)| HeavyVal(v, n)).collect();
        let mut err = self.err;
        if all.len() > KEPT {
            all.select_nth_unstable_by(KEPT - 1, |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            err = err.max(all[KEPT..].iter().map(|c| c.1).fold(0.0, f64::max));
            all.truncate(KEPT);
        }
        all.sort_unstable_by_key(|t| t.0);
        let held: f64 = all.iter().map(|t| t.1).sum();
        let ndv = ndv.max(all.len() as f64);
        let _ = tail_rows;
        HeavyCol { tail_rows: (rows - held).max(0.0), tail_ndv: ndv - all.len() as f64, top: all, err, rows, ndv, sq: sq.max(rows), cube: cube.max(rows) }
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
}

/// Decision 0209 §4's price of a probe: the rows column `here` holds per binding of its variable, where the binding
/// was drawn through the columns `by` (the atoms placed before that hold it). A value's weight is the product of its
/// rows in each of `by`, so a value common in all is drawn more often: value by value over the heavy values of any,
/// a value heavy in one only taken at the others' tail mean (none where a column's filter refuses it), and the tails'
/// values at their means: those `by` share drawn, those `here` shares too met. `None` where no value is drawn.
pub fn heavy_fan(here: PricedCol<'_>, by: &[PricedCol<'_>]) -> Option<f64> {
    let mut vals: Vector<i64> = here.heavy.top.iter().map(|t| t.0).chain(by.iter().flat_map(|h| h.heavy.top.iter().map(|t| t.0))).collect();
    vals.sort_unstable();
    vals.dedup();
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for &v in &vals {
        let w: f64 = by.iter().map(|h| h.rows_of(v)).product();
        num += w * here.rows_of(v);
        den += w;
    }
    // The values heavy in none: each column's tail less the values of `vals` it holds in its tail. A binding is
    // drawn from a value every column of `by` holds (`drawn`), and finds rows where `here` holds it too (`met`); a
    // drawn value `here` lacks is a binding of no rows. Both are counted by containment (as many as the fewest any
    // has left) or, where two columns' filters tell values apart (decision 0488), as many as those two share, less
    // the heavy values both hold.
    let left = |h: &PricedCol<'_>| (h.heavy.tail_ndv - (vals.len() - h.heavy.top.len()) as f64).max(0.0);
    let shared = |x: &PricedCol<'_>, y: &PricedCol<'_>| -> Option<f64> {
        let (a, b) = (x.keys?, y.keys?);
        // The same keys (a self-join, or one key set): containment's count is exact there, the estimate not.
        if a.same(b) {
            return None;
        }
        let both = a.shared(b)?;
        let heavy_both = vals.iter().filter(|&&v| x.rows_of(v) > 0.0 && y.rows_of(v) > 0.0).count() as f64;
        Some((both - heavy_both).max(0.0))
    };
    let mut drawn = by.iter().map(&left).fold(f64::INFINITY, f64::min);
    for (i, x) in by.iter().enumerate() {
        for y in &by[i + 1..] {
            drawn = shared(x, y).map_or(drawn, |s| drawn.min(s));
        }
    }
    if !drawn.is_finite() {
        drawn = left(&here);
    }
    let mut met = drawn.min(left(&here));
    for h in by {
        met = shared(&here, h).map_or(met, |s| met.min(s));
    }
    let w: f64 = by.iter().map(|h| h.heavy.tail_mean()).product();
    num += met * w * here.heavy.tail_mean();
    den += drawn * w;
    (den > 0.0).then(|| num / den)
}

/// Decision 0209 §6: each relation's requested columns' heavy values, in one batch: on the device in one command
/// buffer (`heavy_of_device`) where the round's rows make a launch pay (`gpu_pays`), else on the CPU, one pass over
/// each relation's rows (its source, where it has one), the relations spread over the pool. A request names the
/// relation's source and, where the engine holds it, the relation itself: the device reads the columns of one it
/// holds in place (decision 0261).
pub fn heavy_batch(reqs: Vec<HeavyReq>) -> Vec<Option<Vec<HeavyCol>>> {
    use rayon::prelude::*;
    use crate::code_rule_dsl::logic::device::StatColumn;
    let rows: usize = reqs.iter().map(|(s, _, cols)| s.as_ref().map_or(0, |s| s.len() * cols.len())).sum();
    if gpu_pays(rows) {
        // Values gathered only for a source the engine does not hold as a relation.
        let values: Vec<Option<Vec<Vector<i64>>>> = rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
            reqs
                .par_iter()
                .map(|(src, rel, cols)| match (src, rel) {
                    (Some(src), None) => Some(cols.iter().map(|&c| (0..src.len()).map(|row| src.value(row, c)).collect()).collect()),
                    _ => None,
                })
                .collect()
        });
        let mut cols: Vec<StatColumn<'_>> = Vec::new();
        for ((src, rel, cs), v) in reqs.iter().zip(&values) {
            match (src, rel, v) {
                (Some(_), Some(rel), _) => cols.extend(cs.iter().map(|&c| StatColumn::Rel(rel, c))),
                (_, _, Some(v)) => cols.extend(v.iter().map(|v| StatColumn::Values(v))),
                _ => {}
            }
        }
        if let Some(mut made) = heavy_of_device(&cols) {
            rerail_prof::code_rule_dsl::logic::profile::local::<Batches>().0[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut made = made.drain(..);
            return reqs.iter().map(|(src, _, cs)| src.as_ref().map(|_| cs.iter().map(|_| made.next().expect("one a column")).collect())).collect();
        }
    }
    rerail_prof::code_rule_dsl::logic::profile::local::<Batches>().0[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Each relation counted by morsels of its rows across the pool, the morsels' counts merged as a tree. One pass of
    // a relation on one thread left a run's CPU cross part waiting 77 s of its 90 s on three relations' counts, at a
    // parallelism of 1.3 (t50, 2026-11-23: `rf_names`, `rf_fn_follows`, `rf_target`).
    const MORSEL: usize = 1 << 15;
    rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
        reqs.into_par_iter()
            .map(|(src, _, cols)| {
                let src = src?;
                let n = src.len();
                let empty = || -> Vec<FxHashMap<i64, u64>> { cols.iter().map(|_| FxHashMap::default()).collect() };
                let counts = (0..n.div_ceil(MORSEL))
                    .into_par_iter()
                    .fold(empty, |mut counts, m| {
                        for row in m * MORSEL..((m + 1) * MORSEL).min(n) {
                            for (k, &c) in cols.iter().enumerate() {
                                *counts[k].entry(src.value(row, c)).or_default() += 1;
                            }
                        }
                        counts
                    })
                    .reduce(empty, |mut a, b| {
                        for (x, mut y) in a.iter_mut().zip(b) {
                            // The smaller merged into the larger.
                            if y.len() > x.len() {
                                std::mem::swap(x, &mut y);
                            }
                            for (v, k) in y {
                                *x.entry(v).or_default() += k;
                            }
                        }
                        a
                    });
                Some(counts.into_iter().map(HeavyCol::of_counts).collect())
            })
            .collect()
    })
}

/// A request of `heavy_batch`: a relation's source, the engine's relation where it holds one, and the columns.
pub type HeavyReq = (Option<std::sync::Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, Option<std::sync::Arc<crate::code_rule_dsl::logic::native::component::InRel>>, Vector<usize>);

/// The batches made, on the CPU and on the GPU.
#[derive(Default)]
struct Batches([std::sync::atomic::AtomicU64; 2]);

pub fn batches() -> [u64; 2] {
    rerail_prof::code_rule_dsl::logic::profile::local::<Batches>().0.each_ref().map(|b| b.load(std::sync::atomic::Ordering::Relaxed))
}

/// Whether a round of `rows` rows (each column's rows, summed) is made on the GPU: `RERAIL_STATS_GPU` on or off,
/// else from `GPU_ROWS`, where one launch and the columns' upload cost less than the CPU's hashing. Never where the
/// device's kernels run on the CPU (`RERAIL_GPU=cpu`): there its radix sorts ran at a parallelism of about one, and a
/// CPU run's cross part waited 77 s of its 90 s on three relations' rounds (t50, 2026-11-23).
fn gpu_pays(rows: usize) -> bool {
    const GPU_ROWS: usize = 1 << 20;
    rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_STATS_GPU").as_deref() {
        Ok("on") => Some(true),
        Ok("off") => Some(false),
        _ => None,
    }).unwrap_or(rows >= GPU_ROWS && !crate::code_rule_dsl::logic::device::solver().is_some_and(|s| s.kernels_on_cpu()))
}

/// Columns' heavy values made on the registered device (`SolverDevice::column_stats`), exact; `None` without one.
/// The columns read in place are counted (`in_place`).
pub fn heavy_of_device(cols: &[crate::code_rule_dsl::logic::device::StatColumn<'_>]) -> Option<Vec<HeavyCol>> {
    let (made, read) = crate::code_rule_dsl::logic::device::solver()?.column_stats(cols, KEPT).ok()?;
    rerail_prof::code_rule_dsl::logic::profile::local::<InPlace>().0.fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
    Some(made.into_iter().map(|m| {
        let mut top: Vector<HeavyVal> = m.top.iter().map(|&(v, n)| HeavyVal(v, n as f64)).collect();
        top.sort_unstable_by_key(|t| t.0);
        let held: f64 = top.iter().map(|t| t.1).sum();
        let (rows, ndv) = (m.rows as f64, m.ndv as f64);
        HeavyCol { tail_rows: rows - held, tail_ndv: ndv - top.len() as f64, top, err: 0.0, rows, ndv, sq: m.sq as f64, cube: m.cube as f64 }
    }).collect())
}

/// Columns the device read in place (`heavy_of_device`).
#[derive(Default)]
struct InPlace(std::sync::atomic::AtomicU64);

pub fn read_in_place() -> u64 {
    rerail_prof::code_rule_dsl::logic::profile::local::<InPlace>().0.load(std::sync::atomic::Ordering::Relaxed)
}

/// Columns' heavy values made on the device, from host values (`heavy_of_device`).
pub fn heavy_of_values_gpu(cols: &[&[i64]]) -> Option<Vec<HeavyCol>> {
    let cols: Vec<_> = cols.iter().map(|c| crate::code_rule_dsl::logic::device::StatColumn::Values(c)).collect();
    heavy_of_device(&cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relation's heavy values counted by morsels of its rows across the pool are the counts of one pass: over a
    /// source of several morsels and a ragged last one, a column of many values and a column of a few heavy ones.
    #[test]
    fn heavy_values_counted_by_morsels_are_one_pass() {
        struct Rows(usize);
        impl crate::code_rule_dsl::facts::vtab::Source for Rows {
            fn columns(&self) -> Vec<String> {
                vec!["a".into(), "b".into()]
            }
            fn len(&self) -> usize {
                self.0
            }
            fn estimated_matches(&self, _: usize, _: bool) -> usize {
                1
            }
            fn indexed(&self, _: usize) -> bool {
                false
            }
            fn lookup(&self, _: usize, _: &[i64]) -> Option<Vector<usize>> {
                None
            }
            fn value(&self, row: usize, column: usize) -> i64 {
                if column == 0 { (row % 997) as i64 } else { (row * row % 5) as i64 }
            }
        }
        let n = 3 * (1 << 15) + 17;
        let src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = Arc::new(Rows(n));
        let got = heavy_batch(vec![(Some(Arc::clone(&src)), None, rerail_blocks::vector![0, 1])]).pop().flatten().expect("counted");
        for (c, h) in got.iter().enumerate() {
            let mut counts: FxHashMap<i64, u64> = FxHashMap::default();
            (0..n).for_each(|r| *counts.entry(src.value(r, c)).or_default() += 1);
            let want = HeavyCol::of_counts(counts);
            assert_eq!((h.rows, h.ndv, h.sq, h.cube, &h.top), (want.rows, want.ndv, want.sq, want.cube, &want.top), "column {c}");
        }
    }

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
        // Past the relation's columns: its vector grows, with its values.
        fan.insert(("t_rel_a", 9), 9.5);
        assert_eq!((fan.get(&("t_rel_a", 1)), fan.at("t_rel_a", 9), fan.get(&("t_rel_b", 0)), fan.get(&("t_rel_a", 2))), (Some(&1.5), Some(&9.5), Some(&2.5), None));
        assert_eq!(fan.len(), 3);
        let a = find("t_rel_a").unwrap();
        assert_eq!(fan.at_rel(a, 9), Some(&9.5));
        // A fork shares a relation's columns by a count; a write copies only that relation's.
        let mut fork: ColMap<f64> = ColMap::new();
        fork.share_from(&fan, "t_rel_a", 16);
        fork.insert(("t_rel_a", 9), 0.5);
        assert_eq!((fan.at("t_rel_a", 9), fork.at("t_rel_a", 9), fork.at("t_rel_a", 1)), (Some(&9.5), Some(&0.5), Some(&1.5)));
        assert_eq!(fan.of_rel("t_rel_a").collect::<Vec<_>>(), vec![(1, &1.5), (9, &9.5)]);
        fan.retain(|_, c, _| c != 1);
        assert_eq!((fan.len(), fan.get(&("t_rel_a", 1))), (2, None));
    }
    #[test]
    fn heavy_values_price_a_skewed_self_join_value_by_value() {
        // 200 values of 1 row and one of 300: a three-way self-join on the column.
        let counts: Vec<(i64, u64)> = (0..200).map(|v| (v, 1)).chain([(1000, 300)]).collect();
        let true3: f64 = counts.iter().map(|&(_, n)| (n as f64).powi(3)).sum();
        let true2: f64 = counts.iter().map(|&(_, n)| (n as f64).powi(2)).sum();
        let h = HeavyCol::of_counts(counts.clone());
        assert_eq!((h.rows, h.ndv, h.top.len(), h.count(1000), h.tail_ndv), (500.0, 201.0, KEPT, Some(300.0), 73.0));
        // The second atom's rows a binding are sq / rows; the third's, cube / sq.
        assert!((heavy_fan(PricedCol::new(&h, None), &[PricedCol::new(&h, None)]).unwrap() - true2 / 500.0).abs() < 1e-9);
        assert!((heavy_fan(PricedCol::new(&h, None), &[PricedCol::new(&h, None), PricedCol::new(&h, None)]).unwrap() - true3 / true2).abs() < 1e-9);
        assert_eq!((h.sq, h.cube), (true2, true3));
    }
    /// Two columns whose tails share 50 of 2,000 keys (decision 0488): containment would price a probe as if all
    /// 2,000 met; the columns' filters price it by about the 50 that do.
    #[test]
    fn the_columns_filters_count_the_tail_keys_two_columns_share() {
        let a: Vec<(i64, u64)> = (0..2000).map(|v| (v, 2)).chain([(1_000_000, 500)]).collect();
        let b: Vec<(i64, u64)> = (1950..3950).map(|v| (v, 3)).chain([(1_000_000, 400)]).collect();
        let (ha, hb) = (HeavyCol::of_counts(a.clone()), HeavyCol::of_counts(b.clone()));
        // Each column's keys as the engine keeps them: a filter (past a few values) and its sample.
        use crate::code_rule_dsl::logic::bloom::Keys;
        let (ka, kb) = (Keys::of(a.iter().map(|t| t.0)), Keys::of(b.iter().map(|t| t.0)));
        assert!(matches!((&ka, &kb), (Keys::Filter(_), Keys::Filter(_))), "2,000 keys are kept in a filter");
        // The exact fan: rows of b per row of a, a row of a meeting none a binding of no rows.
        let bm: FxHashMap<i64, u64> = b.iter().copied().collect();
        let num: f64 = a.iter().map(|&(v, n)| n as f64 * bm.get(&v).map_or(0.0, |&m| m as f64)).sum();
        let exact = num / a.iter().map(|&(_, n)| n as f64).sum::<f64>();
        let est = heavy_fan(PricedCol::new(&hb, Some(&kb)), &[PricedCol::new(&ha, Some(&ka))]).unwrap();
        // Without the keys: every tail key a has left met as if b held it.
        let without = heavy_fan(PricedCol::new(&hb, None), &[PricedCol::new(&ha, None)]).unwrap();
        assert!((est / exact - 1.0).abs() < 0.15, "with the filters {est}, exact {exact}");
        assert!((without / exact - 1.0).abs() > (est / exact - 1.0).abs(), "the filters closer than containment: {without} against {est}, exact {exact}");
    }

    #[test]
    fn a_change_merged_into_heavy_values_is_its_full_count() {
        // 300 values of 2 rows and one of 100, then a change: 50 rows more of the heavy value, 40 of a new one, 30
        // of a tail value, and the heavy value's 10 rows less.
        let base: Vec<(i64, u64)> = (0..300).map(|v| (v, 2)).chain([(9_999, 100)]).collect();
        let h = HeavyCol::of_counts(base.clone());
        let added: FxHashMap<i64, u64> = [(9_999, 50), (77_777, 40), (250, 30)].into_iter().collect();
        let removed: FxHashMap<i64, u64> = [(9_999, 10)].into_iter().collect();
        let mut now: FxHashMap<i64, u64> = base.into_iter().collect();
        for (v, a) in &added { *now.entry(*v).or_default() += a; }
        for (v, r) in &removed { *now.get_mut(v).unwrap() -= r; }
        let full = HeavyCol::of_counts(now.clone());
        let merged = h.merge(&added, &removed, full.ndv);
        // The heavy value and the new one are counted exactly; the rows, exactly.
        assert_eq!((merged.count(9_999), merged.count(77_777), merged.rows, merged.ndv), (Some(140.0), Some(40.0), full.rows, full.ndv));
        // Its price of a three-way self-join is the full count's within 2%.
        let (m3, f3) = (heavy_fan(PricedCol::new(&merged, None), &[PricedCol::new(&merged, None); 2]).unwrap(), heavy_fan(PricedCol::new(&full, None), &[PricedCol::new(&full, None); 2]).unwrap());
        assert!((m3 / f3 - 1.0).abs() < 0.02, "{m3} against {f3}");
        assert!(merged.err <= 2.0 + 30.0, "a value left out held at most its rows: {}", merged.err);
    }
}
