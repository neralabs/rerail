//! Native scalar summaries over one input version. Compatibility belongs to logical analysis.
use rerail_blocks::vector::Vector;
use super::*;
use crate::code_rule_dsl::logic::analysis::{
    bags,
    scalars::{Expression as E, Kind, Scalar},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bag {
    columns: Vector<usize>,
    tests: Vec<(usize, usize)>,
    pub value: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Summary {
    pub count: i64,
    pub sum: i128,
    pub min: Option<i64>,
    pub max: Option<i64>,
}
impl Summary {
    pub fn add(&mut self, value: Option<i64>) {
        self.count += 1;
        if let Some(v) = value {
            self.sum += i128::from(v);
            self.min = Some(self.min.map_or(v, |old| old.min(v)));
            self.max = Some(self.max.map_or(v, |old| old.max(v)));
        }
    }
    pub fn result(self, kind: Kind) -> Result<Option<i64>> {
        Ok(match kind {
            Kind::Count => Some(self.count),
            Kind::Sum => Some(super::super::aggregation::sum_value(self.sum)?),
            Kind::Min => self.min,
            Kind::Max => self.max,
            _ => return Err(err("unsupported native scalar")),
        })
    }
}
/// A bag's summaries by group key, in a block (decision 0478 step 6d): open addressing, each slot `1 + width + 6`
/// words: used (0 for none), the key's values, then the summary (count, sum's low and high words, min, max, which of
/// min and max it has). It doubles past half full.
#[derive(Default)]
struct Groups {
    width: usize,
    words: rerail_blocks::vector::Vector<i64>,
    len: usize,
}
impl Groups {
    const SUMMARY: usize = 6;
    fn stride(&self) -> usize {
        1 + self.width + Self::SUMMARY
    }
    fn slots(&self) -> usize {
        self.words.len() / self.stride().max(1)
    }
    fn hash(key: &[i64]) -> usize {
        rerail_blocks::key_hash(key) as usize
    }
    /// The slot holding `key`, or the empty one it would go in.
    fn slot(&self, key: &[i64]) -> usize {
        let (st, m) = (self.stride(), self.slots() - 1);
        let mut at = Self::hash(key) & m;
        loop {
            let w = &self.words[at * st..][..st];
            if w[0] == 0 || w[1..1 + self.width] == *key {
                return at;
            }
            at = (at + 1) & m;
        }
    }
    fn get(&self, key: &[i64]) -> Option<Summary> {
        if self.len == 0 || key.len() != self.width {
            return None;
        }
        let st = self.stride();
        let w = &self.words[self.slot(key) * st..][..st];
        if w[0] == 0 {
            return None;
        }
        let v = &w[1 + self.width..];
        Some(Summary {
            count: v[0],
            sum: i128::from(v[1] as u64) | i128::from(v[2]) << 64,
            min: (v[5] & 1 != 0).then_some(v[3]),
            max: (v[5] & 2 != 0).then_some(v[4]),
        })
    }
    fn insert(&mut self, key: &[i64], s: Summary) {
        if self.len == 0 && self.words.is_empty() || self.width != key.len() {
            *self = Groups { width: key.len(), words: crate::code_rule_dsl::facts::rows::vector_in(None, 0), len: 0 };
            self.words.resize(16 * self.stride(), 0);
        }
        if 2 * (self.len + 1) > self.slots() {
            let mut grown = Groups { width: self.width, words: crate::code_rule_dsl::facts::rows::vector_in(None, 0), len: 0 };
            grown.words.resize(2 * self.words.len(), 0);
            let st = self.stride();
            for at in 0..self.slots() {
                let w = &self.words[at * st..][..st];
                if w[0] != 0 {
                    let to = grown.slot(&w[1..1 + self.width]) * st;
                    grown.words[to..to + st].copy_from_slice(w);
                    grown.len += 1;
                }
            }
            *self = grown;
        }
        let st = self.stride();
        let at = self.slot(key) * st;
        let w = &mut self.words[at..at + st];
        if w[0] == 0 {
            self.len += 1;
        }
        w[0] = 1;
        w[1..1 + key.len()].copy_from_slice(key);
        let v = &mut w[1 + key.len()..];
        v[0] = s.count;
        v[1] = s.sum as u64 as i64;
        v[2] = (s.sum >> 64) as i64;
        v[3] = s.min.unwrap_or(0);
        v[4] = s.max.unwrap_or(0);
        v[5] = i64::from(s.min.is_some()) | i64::from(s.max.is_some()) << 1;
    }
    fn clear(&mut self) {
        *self = Groups::default();
    }
    fn len(&self) -> usize {
        self.len
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Default)]
struct Memo {
    epoch: (usize, usize, usize, usize),
    groups: Groups,
    preferred_value: Option<Arc<Bag>>,
    builds: usize,
    /// Counted under a read lock: a hit takes no write lock.
    lookups: std::sync::atomic::AtomicUsize,
}
/// The pool's shards, by a group's key: a join's morsels probe a summary a row, from every thread at once.
const SHARDS: usize = 32;

/// What a bag's memo holds, every shard's together (`Pool::memos`).
#[derive(Clone, Debug)]
pub(super) struct MemoStats {
    pub bag: Arc<Bag>,
    pub epoch: (usize, usize, usize, usize),
    pub groups: usize,
    pub builds: usize,
    pub lookups: usize,
}

/// An input owns its cache. Component epochs include the visible row range and the rows deleted in place
/// (`Rel::based`'s dead count, which only grows); rebound inputs own a new pool.
///
/// Sharded by the group's key: with one lock, ten threads probing a summary a row waited on it for most of their
/// time (on all, `public_best_len` and `public_best_route` held the run's thread 0.9 s each with 3 ms of its CPU,
/// 0471). A key's memo is in its shard, each shard with its own epoch; each shard knows every bag's preferred value.
pub(super) struct Pool([parking_lot::RwLock<HashMap<Arc<Bag>, Memo>>; SHARDS]);
impl Default for Pool {
    fn default() -> Self {
        Self(std::array::from_fn(|_| parking_lot::RwLock::default()))
    }
}
impl Pool {
    fn shard(&self, key: &[i64]) -> &parking_lot::RwLock<HashMap<Arc<Bag>, Memo>> {
        let hash = key.iter().fold(0u64, |h, &k| (h.rotate_left(5) ^ k as u64).wrapping_mul(0x517c_c1b7_2722_0a95));
        &self.0[(hash >> (64 - SHARDS.trailing_zeros())) as usize]
    }

    /// Each bag's memo, every shard's summed (the epoch is the latest a shard saw).
    pub fn memos(&self) -> Vec<MemoStats> {
        let mut out: Vec<MemoStats> = Vec::new();
        for shard in &self.0 {
            for (bag, memo) in shard.read().iter() {
                let at = match out.iter().position(|m| &m.bag == bag) {
                    Some(at) => at,
                    None => {
                        out.push(MemoStats { bag: bag.clone(), epoch: memo.epoch, groups: 0, builds: 0, lookups: 0 });
                        out.len() - 1
                    }
                };
                let m = &mut out[at];
                m.epoch = m.epoch.max(memo.epoch);
                (m.groups, m.builds, m.lookups) = (m.groups + memo.groups.len(), m.builds + memo.builds, m.lookups + memo.lookups.load(std::sync::atomic::Ordering::Relaxed));
            }
        }
        out
    }

    /// The bag a lookup of `bag` reads: its preferred value's, where it has one.
    fn selected(&self, bag: &Arc<Bag>) -> Arc<Bag> {
        if bag.value.is_some() {
            return bag.clone();
        }
        self.0[0].read().get(bag).and_then(|m| m.preferred_value.clone()).unwrap_or_else(|| bag.clone())
    }

    pub fn register(&self, bag: &Arc<Bag>) {
        // A count over a bag whose value's sum is proven the same bag reads that bag's summaries (its count).
        let preferred = bag.value.and_then(|value| {
            let mut count = bag.as_ref().clone();
            count.value = None;
            let scalar = |kind, value| Scalar {
                kind,
                source: "native input version".into(),
                keys: bag.columns.iter().map(|&c| E::Column(c)).collect(),
                predicates: bag
                    .tests
                    .iter()
                    .map(|&(a, b)| E::Apply("=".into(), vec![E::Column(a), E::Column(b)]))
                    .collect(),
                lookup: vec![],
                value,
                dependent: false,
                non_null: true,
            };
            scalar(Kind::Count, E::Integer(1)).same_bag(&scalar(Kind::Sum, E::Column(value))).proven.then(|| Arc::new(count))
        });
        for shard in &self.0 {
            let mut pool = shard.write();
            pool.entry(bag.clone()).or_default();
            if let Some(count) = &preferred {
                pool.entry(count.clone()).or_default().preferred_value.get_or_insert_with(|| bag.clone());
            }
        }
    }
    pub fn evaluate(
        &self,
        bag: &Arc<Bag>,
        key: &[i64],
        epoch: (usize, usize, usize, usize),
        build: impl FnOnce(Option<usize>) -> Summary,
    ) -> Summary {
        let mut selected = bag.clone();
        let shard = self.shard(key);
        // A hit: under the read lock.
        {
            let pool = shard.read();
            let memo = if bag.value.is_none() {
                pool.get(bag).and_then(|m| m.preferred_value.as_ref()).map_or_else(|| pool.get(bag), |p| pool.get(p))
            } else {
                pool.get(bag)
            };
            if let Some(memo) = memo
                && memo.epoch == epoch
                && let Some(summary) = memo.groups.get(key)
            {
                memo.lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return summary;
            }
        }
        {
            let mut pool = shard.write();
            if selected.value.is_none() {
                selected = pool
                    .get(bag)
                    .and_then(|m| m.preferred_value.clone())
                    .unwrap_or(selected);
            }
            let memo = pool.entry(selected.clone()).or_default();
            memo.lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if memo.epoch != epoch {
                memo.groups.clear();
                memo.epoch = epoch;
            }
            if let Some(summary) = memo.groups.get(key) {
                return summary;
            }
        }
        let summary = build(selected.value);
        let mut pool = shard.write();
        let memo = pool.get_mut(&selected).unwrap();
        memo.builds += 1;
        memo.groups.insert(key, summary);
        summary
    }
}

pub(super) fn explain(p: &Section<steps::Plan>, store: &Store) {
    use crate::code_rule_dsl::logic::profile;
    if !profile::on() {
        return;
    }
    let mut consumers = vec![];
    for (step, s) in p.steps.iter().enumerate() {
        if let Step::Unused { proof } = s {
            consumers.push(serde_json::json!({"step":step,"evaluation_elided":{"proven":proof.proven,"reason":proof.reason.as_str()}}));
            continue;
        }
        let Step::Scalar { src, bag, .. } = s else { continue };
        let Some(bag) = bag.as_ref().map(|h| p.bound(h)) else { continue };
        let pool = match src {
            Src::In(r, _) => &p.bound(r).summaries,
            Src::Full(r) | Src::FullMulti(r, _) | Src::Delta(r) => &store.rels[*r].summaries,
        };
        let selected = pool.selected(bag);
        if let Some(memo) = pool.memos().into_iter().find(|m| m.bag == selected) {
            consumers.push(serde_json::json!({"step":step,"source_version_cache":format!("{:p}",pool),"columns":selected.columns,"tests":selected.tests,"value_column":selected.value,"epoch":memo.epoch,"cached_groups":memo.groups,"folds":memo.builds,"lookups":memo.lookups}));
        }
    }
    profile::decision("native scalar storage", || {
        serde_json::json!({"consumers":consumers}).to_string()
    });
}

pub(super) fn register(p: &Section<steps::Plan>, store: Option<&Store>) {
    for step in p.steps.iter() {
        if let Step::Scalar { src, bag, .. } = step
            && let Some(bag) = bag.as_ref().map(|h| p.bound(h))
        {
            match src {
                Src::In(rel, _) => p.bound(rel).summaries.register(bag),
                Src::Full(r) | Src::FullMulti(r, _) | Src::Delta(r) => {
                    if let Some(store) = store {
                        store.rels[*r].summaries.register(bag)
                    }
                }
            }
        }
    }
}

/// Adapt physical steps to the same bag proof used by SQL. Lookup slots stay local to the consumer.
/// Each scalar step's bag, by the same proof SQL's uses (`Edit::Bag`).
pub(super) fn plan(p: &Section<steps::Plan>) -> Vec<Edit> {
    let steps = &p.steps;
    let mut previous = std::collections::BTreeSet::new();
    let mut positions = Vec::new();
    let mut inputs = Vec::new();
    for (step, s) in steps.iter().enumerate() {
        match s {
            Step::Func { out, .. }
            | Step::Cat { out, .. }
            | Step::DocOf { out, .. }
            | Step::StrJoin { out, .. } => {
                previous.insert(*out);
            }
            Step::Split { a, b, .. } => {
                previous.extend(a.iter().chain(b).copied());
            }
            Step::Let { a, b: Key::Slot(b) } if previous.contains(b) => {
                previous.insert(*a);
            }
            // A bounded min or max (decision 0511) depends on its probe's bound: it shares no bag.
            Step::Scalar { bound: Some(_), out, .. } => {
                previous.insert(*out);
                continue;
            }
            _ => {}
        }
        let Step::Scalar {
            src,
            keys,
            tests,
            col,
            kind,
            out,
            ..
        } = s
        else {
            continue;
        };
        let source = match src {
            Src::In(rel, _) => format!("input:{:p}", Arc::as_ptr(p.bound(rel))),
            Src::Full(r) | Src::FullMulti(r, _) => format!("full:{r}"),
            Src::Delta(r) => format!("delta:{r}"),
        };
        let dependent = keys
            .iter()
            .any(|(_, k)| matches!(k, Key::Slot(s) if previous.contains(s)));
        previous.insert(*out);
        positions.push(step);
        inputs.push((
            bags::Site {
                rule: 0,
                head: "native join".into(),
                scalar: step,
            },
            Scalar {
                kind: *kind,
                source,
                keys: keys.iter().map(|(c, _)| E::Column(*c)).collect(),
                lookup: keys
                    .iter()
                    .map(|(_, k)| match k {
                        Key::Slot(s) => E::Column(*s),
                        Key::Const(v) => E::Integer(*v),
                    })
                    .collect(),
                predicates: tests
                    .iter()
                    .map(|(a, b)| E::Apply("=".into(), vec![E::Column(*a), E::Column(*b)]))
                    .collect(),
                value: col.map_or(E::Integer(1), E::Column),
                dependent,
                non_null: true,
            },
        ));
    }
    let analysis = bags::analyze(inputs);
    let mut edits = Vec::new();
    for family in &analysis.bags.families {
        if !analysis.bags.inputs[family.witness]
            .same_bag(&analysis.bags.inputs[family.witness])
            .proven
        {
            continue;
        }
        let value = match analysis.bags.inputs[family.witness].value {
            E::Column(c) => Some(c),
            _ => None,
        };
        for &member in &family.members {
            if let Step::Scalar { keys, tests, .. } = &steps[positions[member]] {
                edits.push(Edit::Bag(positions[member], Arc::new(Bag {
                    columns: keys.iter().map(|(c, _)| *c).collect(),
                    tests: tests.to_vec(),
                    value,
                })));
            }
        }
    }
    crate::code_rule_dsl::logic::profile::decision("native scalar summaries", || {
        serde_json::json!({
        "analysis":analysis,
        "execution":"One count, exact sum and extrema fold per demanded bag and lookup key; pure compatible consumers reuse it",
        "storage_boundary":"Immutable input object, or component row count and visible delta range; dependent consumers do not reuse summaries",
        "sum_range_check":"At each scalar use, after exact accumulation",
    }).to_string()
    });
    edits
}

#[cfg(test)]
mod groups_tests {
    use super::*;

    /// Decision 0478 step 6d: a bag's summaries by key are slots in a block, found again by their key, whatever the
    /// growth; a key of another width is no hit.
    #[test]
    fn a_bags_summaries_by_key_are_in_a_block() {
        let mut g = Groups::default();
        for k in 0..500i64 {
            g.insert(&[k, -k], Summary { count: k, sum: i128::from(k) << 70, min: (k % 2 == 0).then_some(k), max: Some(-k) });
        }
        assert!(g.words.facts().is_some() && g.len() == 500);
        for k in [0i64, 1, 77, 499] {
            let s = g.get(&[k, -k]).expect("a summary");
            assert_eq!((s.count, s.sum, s.min, s.max), (k, i128::from(k) << 70, (k % 2 == 0).then_some(k), Some(-k)));
        }
        assert!(g.get(&[500, -500]).is_none() && g.get(&[1]).is_none());
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn string_join_observes_deferred_sum_lookup_keys() {
        let db = connection();
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let mut input = Rel::new("input", b"i");
        input.push(&[i64::MAX]);
        input.push(&[1]);
        let mut values = Rel::new("values", b"ii");
        values.push(&[i64::MAX, 7]);
        let mut store = Store {
            rels: vec![input, values, Rel::new("output", b"i")],
            syms: shared,
        };
        let rule = parse_join(
            &db,
            "sum c0 _ 0 v0\nstrjoin c1 v0 _ 1:0 - x:2c v1\nhead v1",
            &mut syms,
        )
        .unwrap();
        let mut fresh = vec![Fresh::default(); 3];
        assert!(
            run_join(&rule, 2, &store, &mut syms, &mut fresh)
                .unwrap_err()
                .to_string()
                .contains("overflow")
        );
        store.rels[0].push(&[-1]);
        assert_eq!(
            run_join(&rule, 2, &store, &mut syms, &mut fresh).unwrap(),
            1
        );
        assert_eq!(syms.name(fresh[2].data()[0]), "7");
    }
    #[test]
    fn native_sum_range_checks_follow_observation_after_aliases_and_filters() {
        let db = connection();
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let mut input = Rel::new("input", b"i");
        input.push(&[i64::MAX]);
        input.push(&[1]);
        let mut store = Store {
            rels: vec![input, Rel::new("output", b"i")],
            syms: shared,
        };
        let filtered = parse_join(
            &db,
            "sum c0 _ 0 v0\neq v1 v0\nlt i:1 i:0\nhead v1",
            &mut syms,
        )
        .unwrap();
        let observed = parse_join(&db, "sum c0 _ 0 v0\neq v1 v0\nhead v1", &mut syms).unwrap();
        let compared = parse_join(
            &db,
            "sum c0 _ 0 v0\neq v1 v0\nlt i:0 v1\nhead i:99",
            &mut syms,
        )
        .unwrap();
        let mut fresh = vec![Fresh::default(); 2];
        assert_eq!(
            run_join(&filtered, 1, &store, &mut syms, &mut fresh).unwrap(),
            0
        );
        assert!(
            run_join(&observed, 1, &store, &mut syms, &mut fresh)
                .unwrap_err()
                .to_string()
                .contains("overflow")
        );
        assert!(
            run_join(&compared, 1, &store, &mut syms, &mut fresh)
                .unwrap_err()
                .to_string()
                .contains("overflow")
        );
        assert_eq!(fresh[1].n, 0);
        store.rels[0].push(&[-1]);
        assert_eq!(
            run_join(&observed, 1, &store, &mut syms, &mut fresh).unwrap(),
            1
        );
        assert_eq!(fresh[1].data(), vec![i64::MAX]);
    }
    #[test]
    fn shared_native_fold_preserves_duplicates_empty_results_and_range_checks() {
        let pool = Pool::default();
        let bag = Arc::new(Bag {
            columns: rerail_blocks::vector![0],
            tests: vec![],
            value: Some(1),
        });
        let builds = std::cell::Cell::new(0);
        let build = |_| {
            builds.set(builds.get() + 1);
            let mut result = Summary::default();
            for v in [7, 7, 3] {
                result.add(Some(v));
            }
            result
        };
        for kind in [Kind::Count, Kind::Sum, Kind::Min, Kind::Max] {
            let s = pool.evaluate(&bag, &[1], (3, 0, 3, 0), build);
            assert_eq!(
                s.result(kind).unwrap(),
                Some(match kind {
                    Kind::Count => 3,
                    Kind::Sum => 17,
                    Kind::Min => 3,
                    _ => 7,
                })
            );
        }
        assert_eq!(builds.get(), 1);
        let s = pool.evaluate(&bag, &[1], (4, 0, 4, 0), build);
        assert_eq!(s.count, 3);
        assert_eq!(
            builds.get(),
            2,
            "a changed component version invalidates the cached group"
        );
        let empty = Summary::default();
        assert_eq!(empty.result(Kind::Count).unwrap(), Some(0));
        assert_eq!(empty.result(Kind::Sum).unwrap(), Some(0));
        assert_eq!(empty.result(Kind::Min).unwrap(), None);
        let mut s = Summary::default();
        s.add(Some(i64::MAX));
        s.add(Some(1));
        assert!(s.result(Kind::Sum).is_err());
        assert_eq!(
            s.result(Kind::Max).unwrap(),
            Some(i64::MAX),
            "unused sums cannot fail extrema"
        );
        s.add(Some(-1));
        assert_eq!(s.result(Kind::Sum).unwrap(), Some(i64::MAX));
    }

    fn connection() -> Connection {
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&path).unwrap());
        Connection::open(path).unwrap()
    }
    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    fn native_join_shares_summaries_across_rules_and_independent_slots() {
        let db = connection();
        db.execute_batch("CREATE TEMP TABLE owners(g INTEGER); INSERT INTO owners VALUES(1),(2),(3);
            CREATE TEMP TABLE events(id INTEGER,g INTEGER,v INTEGER); INSERT INTO events VALUES(0,1,7),(1,1,7),(2,1,3),(3,2,-2);").unwrap();
        let event = format!("t:b:{}:{}:iii", hex("events"), hex("id,g,v"));
        let owner = format!("t:b:{}:{}:i", hex("owners"), hex("g"));
        let spec = format!(
            "pos {owner} v0\ncount {event} _ v0 _ v1\nsum {event} _ v0 _ 2 v2\nmin {event} _ v0 _ 2 v3\nmax {event} _ v0 _ 2 v4\nhead v0 v1 v2 v3 v4"
        );
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let counts = parse_join(
            &db,
            &format!("pos {owner} v0\ncount {event} _ v0 _ v1\nhead v0 v1"),
            &mut syms,
        )
        .unwrap();
        let j = parse_join(&db, &spec, &mut syms).unwrap();
        let j2 = parse_join(&db, &spec.replace("v0", "v5"), &mut syms).unwrap();
        let store = Store {
            rels: vec![Rel::new("output", b"iiiii"), Rel::new("counts", b"ii")],
            syms: shared,
        };
        let mut counted = vec![Fresh::default(); 2];
        run_join(&counts, 1, &store, &mut syms, &mut counted).unwrap();
        assert_eq!(counted[1].data(), vec![1, 3, 2, 1, 3, 0]);
        for j in [&j, &j2] {
            let mut fresh = vec![Fresh::default()];
            assert_eq!(run_join(j, 0, &store, &mut syms, &mut fresh).unwrap(), 2);
            assert_eq!(fresh[0].data(), vec![1, 3, 17, 3, 7, 2, 1, -2, -2, -2]);
        }
        let Step::Scalar {
            src: Src::In(rel, _),
            ..
        } = &j.steps[1]
        else {
            panic!("native scalar")
        };
        let active = j.bound(rel).summaries.memos().into_iter().filter(|m| m.groups > 0).collect::<Vec<_>>();
        assert_eq!(active.len(), 1);
        let memo = &active[0];
        assert_eq!(
            memo.builds, 3,
            "one fold per demanded key, including an empty group, across both rules"
        );
        // Rebinding equal-sized inputs must create a different immutable summary owner.
        forget_inputs(&db);
        db.execute_batch("UPDATE events SET v=1").unwrap();
        let next = parse_join(&db, &spec, &mut syms).unwrap();
        let mut fresh = vec![Fresh::default()];
        run_join(&next, 0, &store, &mut syms, &mut fresh).unwrap();
        assert_eq!(fresh[0].data(), vec![1, 3, 3, 1, 1, 2, 1, 1, 1, 1]);
    }

    #[test]
    fn component_summaries_invalidate_on_append_and_delta_range_changes() {
        let db = connection();
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let mut input = Rel::new("input", b"ii");
        input.push(&[1, 7]);
        input.push(&[1, 3]);
        input.set_delta(0, 2);
        let mut store = Store {
            rels: vec![input, Rel::new("output", b"iii")],
            syms: shared,
        };
        let full = parse_join(
            &db,
            "count c0 i:1 _ v0\nsum c0 i:1 _ 1 v1\nmax c0 i:1 _ 1 v2\nhead v0 v1 v2",
            &mut syms,
        )
        .unwrap();
        let delta = parse_join(
            &db,
            "count d0 i:1 _ v0\nsum d0 i:1 _ 1 v1\nmax d0 i:1 _ 1 v2\nhead v0 v1 v2",
            &mut syms,
        )
        .unwrap();
        let mut run = |store: &Store, j: &JoinRule| {
            let mut fresh = vec![Fresh::default(); 2];
            run_join(j, 1, store, &mut syms, &mut fresh).unwrap();
            fresh[1].data()
        };
        assert_eq!(run(&store, &full), vec![2, 10, 7]);
        assert_eq!(run(&store, &delta), vec![2, 10, 7]);
        store.rels[0].push(&[1, 12]);
        store.rels[0].set_delta(2, 3);
        assert_eq!(run(&store, &full), vec![3, 22, 12]);
        assert_eq!(run(&store, &delta), vec![1, 12, 12]);
        store.rels[0].set_delta(3, 3);
        assert_eq!(run(&store, &delta), Vec::<i64>::new());
        assert_eq!(run(&store, &full), vec![3, 22, 12]);
    }

    /// A row deleted in place (`Rel::based`, decision 0090) leaves the relation's length and delta range as they were.
    /// The fold must skip it, and a group cached before the deletion must not be read after it.
    #[test]
    fn component_summaries_skip_and_invalidate_on_rows_deleted_in_place() {
        let db = connection();
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let mut input = Rel::new("input", b"ii");
        for row in [[1, 7], [1, 3], [1, 12]] {
            input.push(&row);
        }
        input.set_delta(0, 3);
        let mut store = Store {
            rels: vec![input, Rel::new("output", b"iii")],
            syms: shared,
        };
        let full = parse_join(
            &db,
            "count c0 i:1 _ v0\nsum c0 i:1 _ 1 v1\nmax c0 i:1 _ 1 v2\nhead v0 v1 v2",
            &mut syms,
        )
        .unwrap();
        let mut run = |store: &Store, j: &JoinRule| {
            let mut fresh = vec![Fresh::default(); 2];
            run_join(j, 1, store, &mut syms, &mut fresh).unwrap();
            fresh[1].data()
        };
        assert_eq!(run(&store, &full), vec![3, 22, 12]);
        assert!(super::kill(&mut store.rels[0].dead, 2));
        store.rels[0].ndead += 1;
        assert_eq!(run(&store, &full), vec![2, 10, 7], "the dead row is in no group, and the cached group is stale");
    }

    #[test]
    fn repeated_native_consumers_fold_each_demanded_bag_once() {
        let db = connection();
        db.execute_batch("CREATE TEMP TABLE events AS WITH RECURSIVE groups(g) AS (VALUES(0) UNION ALL SELECT g+1 FROM groups WHERE g<999), vals(i,v) AS (VALUES(0,7),(1,7),(2,3)) SELECT g*3+i AS id,g,v FROM groups CROSS JOIN vals;
            CREATE TEMP TABLE owners AS WITH RECURSIVE copies(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM copies WHERE i<19) SELECT g*20+i AS id,g FROM (SELECT DISTINCT g FROM events) CROSS JOIN copies;").unwrap();
        let event = format!("t:b:{}:{}:iii", hex("events"), hex("id,g,v"));
        let owner = format!("t:b:{}:{}:ii", hex("owners"), hex("id,g"));
        let spec = format!(
            "pos {owner} v0 v1\ncount {event} _ v1 _ v2\nsum {event} _ v1 _ 2 v3\nmin {event} _ v1 _ 2 v4\nmax {event} _ v1 _ 2 v5\nhead v0 v2 v3 v4 v5"
        );
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let j = parse_join(&db, &spec, &mut syms).unwrap();
        let store = Store {
            rels: vec![Rel::new("output", b"iiiii")],
            syms: shared,
        };
        let mut fresh = vec![Fresh::default()];
        assert_eq!(
            run_join(&j, 0, &store, &mut syms, &mut fresh).unwrap(),
            20_000
        );
        assert!(
            fresh[0]
                .data()
                .as_chunks::<5>().0.iter()
                .all(|row| row[1..] == [3, 17, 3, 7])
        );
        let Step::Scalar {
            src: Src::In(rel, _),
            ..
        } = &j.steps[1]
        else {
            panic!("native input")
        };
        let memos = j.bound(rel).summaries.memos();
        assert_eq!(
            memos.iter().map(|m| m.builds).sum::<usize>(),
            1000,
            "80,000 scalar uses must share 1,000 bag folds"
        );
        assert_eq!(memos.iter().map(|m| m.groups).sum::<usize>(), 1000);
    }

    #[test]
    fn mutable_input_views_pin_their_version_before_any_scalar_fold() {
        use crate::code_rule_dsl::facts::vtab::{Modules, Source};
        struct Cells(Arc<Mutex<Vector<[i64; 2]>>>);
        impl Source for Cells {
            fn columns(&self) -> Vec<String> {
                vec!["g".into(), "v".into()]
            }
            fn len(&self) -> usize {
                self.0.lock().unwrap().len()
            }
            fn estimated_matches(&self, _: usize, _: bool) -> usize {
                self.len()
            }
            fn indexed(&self, _: usize) -> bool {
                false
            }
            fn lookup(&self, _: usize, _: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
                None
            }
            fn value(&self, r: usize, c: usize) -> i64 {
                self.0.lock().unwrap()[r][c]
            }
            fn frozen(&self) -> Option<Arc<dyn Source>> {
                Some(Arc::new(Cells(Arc::new(Mutex::new(
                    self.0.lock().unwrap().clone(),
                )))))
            }
        }
        let db = connection();
        let values = Arc::new(Mutex::new(rerail_blocks::vector![[1, 7], [1, 3]]));
        Modules::default()
            .bind(&db, "versioned", Arc::new(Cells(values.clone())))
            .unwrap();
        let shared = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        let source = format!("t:b:{}:{}:ii", hex("main.versioned"), hex("g,v"));
        let spec = format!("count {source} i:1 _ v0\nsum {source} i:1 _ 1 v1\nhead v0 v1");
        let first = parse_join(&db, &spec, &mut syms).unwrap();
        values.lock().unwrap()[0][1] = 100;
        let store = Store {
            rels: vec![Rel::new("output", b"ii")],
            syms: shared,
        };
        let mut run = |j: &JoinRule| {
            let mut fresh = vec![Fresh::default()];
            run_join(j, 0, &store, &mut syms, &mut fresh).unwrap();
            fresh[0].data()
        };
        assert_eq!(run(&first), vec![2, 10]);
        forget_inputs(&db);
        let second = parse_join(&db, &spec, &mut Interner::for_loading(&store.syms)).unwrap();
        assert_eq!(run(&second), vec![2, 103]);
        assert_eq!(run(&first), vec![2, 10]);
    }
}
