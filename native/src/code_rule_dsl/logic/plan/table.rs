//! Decision 0209 §3: the planner's statistics as flat arrays.
//!
//! A relation is numbered once in the process (`rel_id`), by its name. A statistic of relations (`RelMap`) is an
//! array by that number. A statistic of columns (`ColMap`) gives each relation number its columns' shared vector: a
//! column is found by two array reads once its relation's number is known, and a strand's model shares a relation's
//! columns by a count (decision 0210 §9). Readers that hold numbers (`Model`'s prepared atoms) read by number; readers
//! by name pay one lookup of the name, not a string built per key.

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

/// A statistic of columns (`Model::ndv`, `Stats::fan`, `dfreq`, `heavy`): per relation number, its columns one shared
/// vector. A reader with the relation's number finds a column by two reads (decision 0209 §3); a strand's model
/// (`Model::fork`) takes a relation's columns by a count, and a write copies only that relation's (decision 0210 §9).
#[derive(Clone, Debug)]
pub struct ColMap<T> {
    rels: Vec<Option<Arc<Vec<Option<T>>>>>,
}

impl<T> Default for ColMap<T> {
    fn default() -> Self {
        ColMap { rels: Vec::new() }
    }
}

impl<T: Clone> ColMap<T> {
    pub fn new() -> Self {
        Self::default()
    }
    #[inline]
    pub fn at_rel(&self, rel: RelId, c: usize) -> Option<&T> {
        self.rels.get(rel as usize)?.as_ref()?.get(c)?.as_ref()
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
        let r = find(k.0.as_ref())? as usize;
        let cols = self.rels.get_mut(r)?.as_mut()?;
        cols.get(k.1)?.as_ref()?;
        Arc::make_mut(cols)[k.1].as_mut()
    }
    /// The slot of column `c` of `rel`, its vector made, widened or unshared for it.
    fn slot(&mut self, rel: RelId, c: usize) -> &mut Option<T> {
        let r = rel as usize;
        if self.rels.len() <= r {
            self.rels.resize_with(r + 1, || None);
        }
        let cols = Arc::make_mut(self.rels[r].get_or_insert_with(Default::default));
        if cols.len() <= c {
            cols.resize(c + 1, None);
        }
        &mut cols[c]
    }
    pub fn insert_at_rel(&mut self, rel: RelId, c: usize, v: T) -> Option<T> {
        self.slot(rel, c).replace(v)
    }
    pub fn insert<S: AsRef<str>>(&mut self, k: (S, usize), v: T) -> Option<T> {
        self.insert_at_rel(rel_id(k.0.as_ref()), k.1, v)
    }
    /// The value at `k`, `v` put there first where there is none (as `entry(…).or_insert(v)`).
    pub fn or_insert<S: AsRef<str>>(&mut self, k: (S, usize), v: T) -> &mut T {
        self.slot(rel_id(k.0.as_ref()), k.1).get_or_insert(v)
    }
    /// The value of `k`, made with `f` where it has none.
    pub fn entry_or_insert_with<S: AsRef<str>>(&mut self, k: (S, usize), f: impl FnOnce() -> T) -> &mut T {
        self.slot(rel_id(k.0.as_ref()), k.1).get_or_insert_with(f)
    }
    pub fn remove<S: AsRef<str>>(&mut self, k: &(S, usize)) -> Option<T> {
        let r = find(k.0.as_ref())? as usize;
        let cols = self.rels.get_mut(r)?.as_mut()?;
        cols.get(k.1)?.as_ref()?;
        Arc::make_mut(cols)[k.1].take()
    }
    pub fn contains_key<S: AsRef<str>>(&self, k: &(S, usize)) -> bool {
        self.get(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.rels.iter().flatten().map(|cols| cols.iter().filter(|v| v.is_some()).count()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The columns with a value: (relation, column), value.
    pub fn iter(&self) -> impl Iterator<Item = ((Arc<str>, usize), &T)> {
        self.rels.iter().enumerate().filter_map(|(r, cols)| Some((r, cols.as_ref()?))).flat_map(|(r, cols)| {
            let name = rel_name(r as RelId);
            cols.iter().enumerate().filter_map(move |(c, v)| Some(((Arc::clone(&name), c), v.as_ref()?)))
        })
    }
    /// Relation `rel`'s columns with a value. The iterator borrows `self` only, not `rel` (`use<'a, T>`; Relax's
    /// checker refused the capture of `rel`'s lifetime that edition 2024 implies, E0310).
    pub fn of_rel<'a>(&'a self, rel: &str) -> impl Iterator<Item = (usize, &'a T)> + use<'a, T> {
        let cols = find(rel).and_then(|r| self.rels.get(r as usize)).and_then(Option::as_ref);
        cols.into_iter().flat_map(|cols| cols.iter().enumerate().filter_map(|(c, v)| Some((c, v.as_ref()?))))
    }
    pub fn clear(&mut self) {
        self.rels.clear();
    }
    /// Keeps the columns `keep` says to.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, usize, &mut T) -> bool) {
        for (r, cols) in self.rels.iter_mut().enumerate() {
            let Some(cols) = cols else { continue };
            let name = rel_name(r as RelId);
            // A shared vector is copied only where a column goes.
            let mut gone: Vec<usize> = Vec::new();
            for (c, v) in Arc::make_mut(cols).iter_mut().enumerate() {
                if let Some(x) = v.as_mut()
                    && !keep(&name, c, x)
                {
                    gone.push(c);
                }
            }
            let cols = Arc::make_mut(cols);
            for c in gone {
                cols[c] = None;
            }
        }
    }
    /// Takes `from`'s columns of `rel` below `width`, shared by a count where none is cut (`Model::fork`).
    pub fn share_from(&mut self, from: &ColMap<T>, rel: &str, width: usize) {
        let Some(r) = find(rel) else { return };
        let Some(cols) = from.rels.get(r as usize).and_then(Option::as_ref) else { return };
        let r = r as usize;
        if self.rels.len() <= r {
            self.rels.resize_with(r + 1, || None);
        }
        self.rels[r] = Some(if cols.len() <= width { Arc::clone(cols) } else { Arc::new(cols[..width].to_vec()) });
    }
    /// Sets `rel`'s columns below `width` to `from`'s (each taken, or none), as one entry each was moved
    /// (`Model::absorb`).
    pub fn take_from(&mut self, from: &mut ColMap<T>, rel: &str, width: usize) {
        let Some(r) = find(rel) else { return };
        let ri = r as usize;
        let theirs = from.rels.get_mut(ri).and_then(Option::take);
        let mine = self.rels.get(ri).and_then(Option::as_ref);
        if let Some(t) = &theirs
            && mine.is_none()
            && t.len() <= width
        {
            if self.rels.len() <= ri {
                self.rels.resize_with(ri + 1, || None);
            }
            self.rels[ri] = Some(Arc::clone(t));
            return;
        }
        for c in 0..width {
            match theirs.as_ref().and_then(|t| t.get(c)).cloned().flatten() {
                Some(v) => {
                    self.insert_at_rel(r, c, v);
                }
                None => {
                    self.remove(&(rel, c));
                }
            }
        }
    }
}

impl<T: Clone, S: AsRef<str>> FromIterator<((S, usize), T)> for ColMap<T> {
    fn from_iter<I: IntoIterator<Item = ((S, usize), T)>>(it: I) -> Self {
        let mut m = ColMap::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl<T: Clone, S: AsRef<str>> Extend<((S, usize), T)> for ColMap<T> {
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

impl<T: Clone, S: AsRef<str>> std::ops::Index<&(S, usize)> for ColMap<T> {
    type Output = T;
    #[track_caller]
    fn index(&self, k: &(S, usize)) -> &T {
        self.get(k).unwrap_or_else(|| panic!("no statistic of {}.{}", k.0.as_ref(), k.1))
    }
}

/// Decision 0209 §4, level 1: how many of a column's values are reported by their counts.
pub const HEAVY: usize = 64;
/// The counters a column keeps: twice what it reports, so a value that grows into the top is seen as a delta is
/// merged (a Misra–Gries summary's 2k counters).
pub const KEPT: usize = 2 * HEAVY;

/// A column's heavy values: its most common values with their exact rows, and what is left (the tail), made over
/// `rows` rows (decision 0209 §4).
#[derive(Clone, Debug, PartialEq)]
pub struct HeavyCol {
    /// The values and their rows, by value (for lookups), at most `KEPT`.
    pub top: Vec<(i64, f64)>,
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
    /// Every value the column holds, as a Bloom filter (`logic::bloom`, `STAT_WORDS` words, decision 0500): whether
    /// a value outside `top` is held at all, and how many values two columns share. A merge adds the values it adds
    /// and keeps those it removes, so a value it refuses is never held. `None` where it was not made.
    pub keys: Option<std::sync::Arc<crate::code_rule_dsl::logic::bloom::Filter>>,
}

/// The words of a column's filter (`HeavyCol::keys`): 16 KiB, one size, so any two columns' filters compare.
pub const STAT_WORDS: usize = crate::code_rule_dsl::logic::bloom::MAX_WORDS;

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
        let mut keys = crate::code_rule_dsl::logic::bloom::Filter::new(STAT_WORDS);
        for &(v, _) in &all {
            keys.add(v);
        }
        if all.len() > KEPT {
            all.select_nth_unstable_by(KEPT - 1, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            all.truncate(KEPT);
        }
        let mut top: Vec<(i64, f64)> = all.into_iter().map(|(v, n)| (v, n as f64)).collect();
        top.sort_unstable_by_key(|t| t.0);
        let held: f64 = top.iter().map(|t| t.1).sum();
        HeavyCol { tail_rows: rows - held, tail_ndv: ndv - top.len() as f64, top, err: 0.0, rows, ndv, sq, cube, keys: Some(std::sync::Arc::new(keys)) }
    }
    /// Decision 0209 §4's merge (O172): the column after a change of `added` and `removed` rows a value, its
    /// distinct values now `ndv` (the sketch's). A kept value's count moves by its change, exact if it was. A value
    /// not kept that the change adds comes in at what it added, and the kept stay the `KEPT` largest; the rest are
    /// the tail. sq and cube move by each value's change, a tail value's taken at the tail's mean.
    pub fn merge(&self, added: &FxHashMap<i64, u64>, removed: &FxHashMap<i64, u64>, ndv: f64) -> HeavyCol {
        let mean = self.tail_mean();
        let mut counts: FxHashMap<i64, f64> = self.top.iter().copied().collect();
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
        let mut all: Vec<(i64, f64)> = counts.into_iter().filter(|c| c.1 > 0.0).collect();
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
        // The values added are added to the filter; those removed stay in it (a filter only grows).
        let keys = self.keys.as_ref().map(|k| {
            let mut k = (**k).clone();
            for &v in added.keys() {
                k.add(v);
            }
            std::sync::Arc::new(k)
        });
        HeavyCol { tail_rows: (rows - held).max(0.0), tail_ndv: ndv - all.len() as f64, top: all, err, rows, ndv, sq: sq.max(rows), cube: cube.max(rows), keys }
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
    /// A value's rows: its own where it is heavy, none where the column's filter refuses it, else the tail's mean.
    #[inline]
    pub fn rows_of(&self, v: i64) -> f64 {
        self.count(v).unwrap_or_else(|| if self.keys.as_ref().is_some_and(|k| !k.may_hold(v)) { 0.0 } else { self.tail_mean() })
    }

    /// The column's filter where it can tell values apart: `None` past `FULL` of its bits set, where most values
    /// would pass and the counts it gives mean little.
    fn telling(&self) -> Option<&crate::code_rule_dsl::logic::bloom::Filter> {
        const FULL: f64 = 0.9;
        self.keys.as_deref().filter(|k| (k.ones() as f64) < FULL * (k.words.len() * 64) as f64)
    }
}

/// Decision 0209 §4's price of a probe: the rows column `here` holds per binding of its variable, where the binding
/// was drawn through the columns `by` (the atoms placed before that hold it). A value's weight is the product of its
/// rows in each of `by`, so a value common in all is drawn more often: value by value over the heavy values of any,
/// a value heavy in one only taken at the others' tail mean (none where a column's filter refuses it), and the tails'
/// values at their means: those `by` share drawn, those `here` shares too met. `None` where no value is drawn.
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
    // The values heavy in none: each column's tail less the values of `vals` it holds in its tail. A binding is
    // drawn from a value every column of `by` holds (`drawn`), and finds rows where `here` holds it too (`met`); a
    // drawn value `here` lacks is a binding of no rows. Both are counted by containment (as many as the fewest any
    // has left) or, where two columns' filters tell values apart (decision 0500), as many as those two share, less
    // the heavy values both hold.
    let left = |h: &HeavyCol| (h.tail_ndv - (vals.len() - h.top.len()) as f64).max(0.0);
    let shared = |x: &HeavyCol, y: &HeavyCol| -> Option<f64> {
        let (a, b) = (x.telling()?, y.telling()?);
        // The same bits (a self-join, or one key set): containment's count is exact there, the estimate not.
        if std::ptr::eq(a, b) || a.words == b.words {
            return None;
        }
        let both = a.shared(b)?;
        let heavy_both = vals.iter().filter(|&&v| x.rows_of(v) > 0.0 && y.rows_of(v) > 0.0).count() as f64;
        Some((both - heavy_both).max(0.0))
    };
    let mut drawn = by.iter().map(|h| left(h)).fold(f64::INFINITY, f64::min);
    for (i, x) in by.iter().enumerate() {
        for y in &by[i + 1..] {
            drawn = shared(x, y).map_or(drawn, |s| drawn.min(s));
        }
    }
    if !drawn.is_finite() {
        drawn = left(here);
    }
    let mut met = drawn.min(left(here));
    for h in by {
        met = shared(here, h).map_or(met, |s| met.min(s));
    }
    let w: f64 = by.iter().map(|h| h.tail_mean()).product();
    num += met * w * here.tail_mean();
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
        let values: Vec<Option<Vec<Vec<i64>>>> = rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
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
    rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
        reqs.into_par_iter()
            .map(|(src, _, cols)| {
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
    })
}

/// A request of `heavy_batch`: a relation's source, the engine's relation where it holds one, and the columns.
pub type HeavyReq = (Option<std::sync::Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, Option<std::sync::Arc<crate::code_rule_dsl::logic::native::component::InRel>>, Vec<usize>);

/// The batches made, on the CPU and on the GPU.
#[derive(Default)]
struct Batches([std::sync::atomic::AtomicU64; 2]);

pub fn batches() -> [u64; 2] {
    rerail_prof::code_rule_dsl::logic::profile::local::<Batches>().0.each_ref().map(|b| b.load(std::sync::atomic::Ordering::Relaxed))
}

/// Whether a round of `rows` rows (each column's rows, summed) is made on the GPU: `RERAIL_STATS_GPU` on or off,
/// else from `GPU_ROWS`, where one launch and the columns' upload cost less than the CPU's hashing.
fn gpu_pays(rows: usize) -> bool {
    const GPU_ROWS: usize = 1 << 20;
    rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_STATS_GPU").as_deref() {
        Ok("on") => Some(true),
        Ok("off") => Some(false),
        _ => None,
    }).unwrap_or(rows >= GPU_ROWS)
}

/// Columns' heavy values made on the registered device (`SolverDevice::column_stats`), exact; `None` without one.
/// The columns read in place are counted (`in_place`).
pub fn heavy_of_device(cols: &[crate::code_rule_dsl::logic::device::StatColumn<'_>]) -> Option<Vec<HeavyCol>> {
    let (made, read) = crate::code_rule_dsl::logic::device::solver()?.column_stats(cols, KEPT, STAT_WORDS).ok()?;
    rerail_prof::code_rule_dsl::logic::profile::local::<InPlace>().0.fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
    Some(made.into_iter().map(|m| {
        let mut top: Vec<(i64, f64)> = m.top.iter().map(|&(v, n)| (v, n as f64)).collect();
        top.sort_unstable_by_key(|t| t.0);
        let held: f64 = top.iter().map(|t| t.1).sum();
        let (rows, ndv) = (m.rows as f64, m.ndv as f64);
        let keys = (m.keys.len() == STAT_WORDS).then(|| std::sync::Arc::new(crate::code_rule_dsl::logic::bloom::Filter { words: m.keys.into_boxed_slice() }));
        HeavyCol { tail_rows: rows - held, tail_ndv: ndv - top.len() as f64, top, err: 0.0, rows, ndv, sq: m.sq as f64, cube: m.cube as f64, keys }
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
        assert!((heavy_fan(&h, &[&h]).unwrap() - true2 / 500.0).abs() < 1e-9);
        assert!((heavy_fan(&h, &[&h, &h]).unwrap() - true3 / true2).abs() < 1e-9);
        assert_eq!((h.sq, h.cube), (true2, true3));
    }
    /// Two columns whose tails share 50 of 2,000 keys (decision 0500): containment would price a probe as if all
    /// 2,000 met; the columns' filters price it by about the 50 that do.
    #[test]
    fn the_columns_filters_count_the_tail_keys_two_columns_share() {
        let a: Vec<(i64, u64)> = (0..2000).map(|v| (v, 2)).chain([(1_000_000, 500)]).collect();
        let b: Vec<(i64, u64)> = (1950..3950).map(|v| (v, 3)).chain([(1_000_000, 400)]).collect();
        let (ha, hb) = (HeavyCol::of_counts(a.clone()), HeavyCol::of_counts(b.clone()));
        assert!(ha.telling().is_some() && hb.telling().is_some(), "a 2,000-key column's filter tells keys apart");
        // The exact fan: rows of b per row of a, a row of a meeting none a binding of no rows.
        let bm: FxHashMap<i64, u64> = b.iter().copied().collect();
        let num: f64 = a.iter().map(|&(v, n)| n as f64 * bm.get(&v).map_or(0.0, |&m| m as f64)).sum();
        let exact = num / a.iter().map(|&(_, n)| n as f64).sum::<f64>();
        let est = heavy_fan(&hb, &[&ha]).unwrap();
        // Without the filters: every tail key a has left met as if b held it.
        let mut blind = (ha.clone(), hb.clone());
        blind.0.keys = None;
        blind.1.keys = None;
        let without = heavy_fan(&blind.1, &[&blind.0]).unwrap();
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
        let (m3, f3) = (heavy_fan(&merged, &[&merged, &merged]).unwrap(), heavy_fan(&full, &[&full, &full]).unwrap());
        assert!((m3 / f3 - 1.0).abs() < 0.02, "{m3} against {f3}");
        assert!(merged.err <= 2.0 + 30.0, "a value left out held at most its rows: {}", merged.err);
    }
}
