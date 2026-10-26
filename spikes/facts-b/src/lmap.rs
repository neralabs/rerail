//! A layered map: frozen layers shared by reference (`Rc`), top first, under one mutable top
//! map — Y1's base → cluster overlay → worktree delta, for any keyed state. An entry in a layer
//! overrides every layer below it; `None` is a tombstone ("not here, whatever lies below").
//! The top holds only entries that differ from the layers below (a mutation that restores the
//! value below drops the entry), so a worktree's state costs its difference.
use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::rc::Rc;

pub type Layer<K, V> = Rc<FxHashMap<K, Option<V>>>;

/// Heap bytes owned by a value (beyond its inline size).
pub trait Heap {
    fn heap(&self) -> usize {
        0
    }
}

impl<T> Heap for Vec<T> {
    fn heap(&self) -> usize {
        self.capacity() * std::mem::size_of::<T>()
    }
}
impl<T> Heap for Box<[T]> {
    fn heap(&self) -> usize {
        self.len() * std::mem::size_of::<T>()
    }
}
macro_rules! no_heap {
    ($($t:ty),*) => { $(impl Heap for $t {})* };
}
no_heap!(u32, u64, i32, i64, bool, (u32, u32), (bool, bool));

pub struct LMap<K, V> {
    below: Vec<Layer<K, V>>,
    top: FxHashMap<K, Option<V>>,
    len: usize,
}

impl<K, V> Default for LMap<K, V> {
    fn default() -> Self {
        LMap { below: Vec::new(), top: FxHashMap::default(), len: 0 }
    }
}

/// Bytes of one hash map of `(K, Option<V>)` entries (capacity, control bytes, values' heap).
pub fn map_bytes<K, V: Heap>(m: &FxHashMap<K, Option<V>>) -> usize {
    m.capacity() * (std::mem::size_of::<(K, Option<V>)>() + 1) + m.values().map(|v| v.as_ref().map_or(0, Heap::heap)).sum::<usize>()
}

impl<K: Hash + Eq + Copy, V: Clone + PartialEq> LMap<K, V> {
    #[inline]
    fn below_get(&self, k: &K) -> Option<&V> {
        for l in &self.below {
            if let Some(v) = l.get(k) {
                return v.as_ref();
            }
        }
        None
    }

    #[inline]
    pub fn get(&self, k: &K) -> Option<&V> {
        match self.top.get(k) {
            Some(v) => v.as_ref(),
            None => self.below_get(k),
        }
    }

    #[inline]
    pub fn contains_key(&self, k: &K) -> bool {
        self.get(k).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Set the top entry of `k` to `v` (None: absent), dropping it when the layers below agree.
    fn set(&mut self, k: K, v: Option<V>) {
        let had = self.get(&k).is_some();
        let has = v.is_some();
        if self.below_get(&k) == v.as_ref() {
            self.top.remove(&k);
        } else {
            self.top.insert(k, v);
        }
        match (had, has) {
            (false, true) => self.len += 1,
            (true, false) => self.len -= 1,
            _ => {}
        }
    }

    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        let old = self.get(&k).cloned();
        self.set(k, Some(v));
        old
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        let old = self.get(k).cloned();
        if old.is_some() {
            self.set(*k, None);
        }
        old
    }

    /// Every present entry, the top's first, then each layer's that no layer above overrides.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let top = self.top.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k, v)));
        let rest = self.below.iter().enumerate().flat_map(move |(i, l)| {
            l.iter().filter_map(move |(k, v)| {
                if self.top.contains_key(k) || self.below[..i].iter().any(|u| u.contains_key(k)) {
                    return None;
                }
                v.as_ref().map(|v| (k, v))
            })
        });
        top.chain(rest)
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// A new map over the same layers with an empty top (the top must be empty: frozen).
    pub fn view(&self) -> Self {
        assert!(self.top.is_empty(), "view of a map with an unfrozen top");
        LMap { below: self.below.clone(), top: FxHashMap::default(), len: self.len }
    }

    /// The top becomes a frozen layer. Over no layers (a base) tombstones are dropped.
    pub fn freeze(&mut self) {
        let mut top = std::mem::take(&mut self.top);
        if self.below.is_empty() {
            top.retain(|_, v| v.is_some());
        }
        top.shrink_to_fit();
        self.below.insert(0, Rc::new(top));
    }

    /// Entries in the top (the worktree's difference).
    pub fn top_len(&self) -> usize {
        self.top.len()
    }

    pub fn layers(&self) -> &[Layer<K, V>] {
        &self.below
    }
}

impl<K: Hash + Eq + Copy, V: Clone + PartialEq + Heap> LMap<K, V> {
    pub fn top_bytes(&self) -> usize {
        map_bytes(&self.top)
    }
    /// Bytes of the frozen layer `i` (0: the top-most frozen layer).
    pub fn layer_bytes(&self, i: usize) -> usize {
        self.below.get(i).map_or(0, |l| map_bytes(l))
    }
}

impl<K: Hash + Eq + Copy, V: Clone + PartialEq> std::ops::Index<&K> for LMap<K, V> {
    type Output = V;
    fn index(&self, k: &K) -> &V {
        self.get(k).expect("LMap: key absent")
    }
}

/// Multimaps: a sorted vector per key (duplicates kept), empty meaning absent.
impl<K: Hash + Eq + Copy, T: Ord + Clone> LMap<K, Vec<T>> {
    #[inline]
    pub fn at(&self, k: &K) -> &[T] {
        self.get(k).map_or(&[], Vec::as_slice)
    }

    /// Edit the vector at `k` (an empty result removes the key).
    pub fn update(&mut self, k: K, f: impl FnOnce(&mut Vec<T>)) {
        let mut v = self.get(&k).cloned().unwrap_or_default();
        f(&mut v);
        self.set(k, if v.is_empty() { None } else { Some(v) });
    }

    pub fn push(&mut self, k: K, x: T) {
        // In place when the top already holds the key (the common case after the first edit).
        if let Some(Some(v)) = self.top.get_mut(&k) {
            let i = v.partition_point(|y| *y <= x);
            v.insert(i, x);
            if self.below_get(&k).is_some_and(|b| b == self.top[&k].as_ref().unwrap()) {
                self.top.remove(&k);
            }
            return;
        }
        self.update(k, |v| {
            let i = v.partition_point(|y| *y <= x);
            v.insert(i, x);
        });
    }

    pub fn pull(&mut self, k: K, x: &T) {
        self.update(k, |v| {
            let i = v.binary_search(x).unwrap_or_else(|_| panic!("LMap::pull: element absent"));
            v.remove(i);
        });
    }
}

/// A layered reverse index (key → ids), append-only like Y1's readers: an id once added stays
/// (a stale entry only costs a re-run that changes nothing). A worktree's top holds the ids no
/// layer below holds for the key.
pub struct Rev<K> {
    below: Vec<Rc<FxHashMap<K, Box<[u32]>>>>,
    top: FxHashMap<K, Vec<u32>>,
}

impl<K> Default for Rev<K> {
    fn default() -> Self {
        Rev { below: Vec::new(), top: FxHashMap::default() }
    }
}

impl<K: Hash + Eq + Copy> Rev<K> {
    pub fn add(&mut self, k: K, id: u32) {
        if self.below.iter().any(|l| l.get(&k).is_some_and(|v| v.binary_search(&id).is_ok())) {
            return;
        }
        let v = self.top.entry(k).or_default();
        if let Err(i) = v.binary_search(&id) {
            v.insert(i, id);
        }
    }

    /// Every id at `k` (appended to `out`; an id is in one layer only).
    pub fn get(&self, k: &K, out: &mut Vec<u32>) {
        if let Some(v) = self.top.get(k) {
            out.extend_from_slice(v);
        }
        for l in &self.below {
            if let Some(v) = l.get(k) {
                out.extend_from_slice(v);
            }
        }
    }

    pub fn view(&self) -> Self {
        assert!(self.top.is_empty(), "view of an index with an unfrozen top");
        Rev { below: self.below.clone(), top: FxHashMap::default() }
    }

    pub fn freeze(&mut self) {
        let top: FxHashMap<K, Box<[u32]>> = std::mem::take(&mut self.top).into_iter().map(|(k, v)| (k, v.into_boxed_slice())).collect();
        self.below.insert(0, Rc::new(top));
    }

    pub fn top_bytes(&self) -> usize {
        self.top.capacity() * (std::mem::size_of::<(K, Vec<u32>)>() + 1) + self.top.values().map(|v| v.capacity() * 4).sum::<usize>()
    }

    pub fn layer_bytes(&self, i: usize) -> usize {
        self.below.get(i).map_or(0, |l| l.capacity() * (std::mem::size_of::<(K, Box<[u32]>)>() + 1) + l.values().map(|v| v.len() * 4).sum::<usize>())
    }

    pub fn top_len(&self) -> usize {
        self.top.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_override_and_tops_hold_only_differences() {
        let mut base: LMap<u32, u32> = LMap::default();
        base.insert(1, 10);
        base.insert(2, 20);
        base.freeze();
        let mut over = base.view();
        over.remove(&1);
        over.insert(3, 30);
        over.freeze();
        let mut w = over.view();
        assert_eq!((w.get(&1), w.get(&2), w.get(&3), w.len()), (None, Some(&20), Some(&30), 2));
        w.insert(1, 10);
        assert_eq!(w.top_len(), 1);
        w.insert(3, 30);
        assert_eq!(w.top_len(), 1, "a value equal to the one below holds no entry");
        w.remove(&1);
        assert_eq!((w.top_len(), w.len()), (0, 2));
        let mut ks: Vec<u32> = w.keys().copied().collect();
        ks.sort();
        assert_eq!(ks, [2, 3]);
    }

    #[test]
    fn multimaps_stay_sorted_and_restore_to_nothing() {
        let mut base: LMap<u32, Vec<u32>> = LMap::default();
        base.push(1, 5);
        base.push(1, 3);
        base.freeze();
        let mut w = base.view();
        w.push(1, 4);
        assert_eq!(w.at(&1), [3, 4, 5]);
        w.pull(1, &4);
        assert_eq!(w.top_len(), 0);
        w.pull(1, &3);
        w.pull(1, &5);
        assert!(w.at(&1).is_empty() && w.is_empty());
    }
}
