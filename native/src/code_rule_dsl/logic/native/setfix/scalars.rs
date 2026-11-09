//! Native scalar summaries over one input version. Compatibility belongs to logical analysis.
use super::*;
use crate::code_rule_dsl::logic::analysis::{
    bags,
    scalars::{Expression as E, Kind, Scalar},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Bag {
    columns: Vec<usize>,
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
#[derive(Default)]
struct Memo {
    epoch: (usize, usize, usize),
    groups: HashMap<Vec<i64>, Summary>,
    preferred_value: Option<Arc<Bag>>,
    builds: usize,
    lookups: usize,
}
/// An input owns its cache. Component epochs include the visible row range; rebound inputs own a new pool.
#[derive(Default)]
pub(super) struct Pool(Mutex<HashMap<Arc<Bag>, Memo>>);
impl Pool {
    pub fn register(&self, bag: &Arc<Bag>) {
        let mut pool = self.0.lock().unwrap();
        pool.entry(bag.clone()).or_default();
        if let Some(value) = bag.value {
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
            if scalar(Kind::Count, E::Integer(1))
                .same_bag(&scalar(Kind::Sum, E::Column(value)))
                .proven
            {
                pool.entry(Arc::new(count))
                    .or_default()
                    .preferred_value
                    .get_or_insert_with(|| bag.clone());
            }
        }
    }
    pub fn evaluate(
        &self,
        bag: &Arc<Bag>,
        key: &[i64],
        epoch: (usize, usize, usize),
        build: impl FnOnce(Option<usize>) -> Summary,
    ) -> Summary {
        let mut selected = bag.clone();
        {
            let mut pool = self.0.lock().unwrap();
            if selected.value.is_none() {
                selected = pool
                    .get(bag)
                    .and_then(|m| m.preferred_value.clone())
                    .unwrap_or(selected);
            }
            let memo = pool.entry(selected.clone()).or_default();
            memo.lookups += 1;
            if memo.epoch != epoch {
                memo.groups.clear();
                memo.epoch = epoch;
            }
            if let Some(summary) = memo.groups.get(key) {
                return *summary;
            }
        }
        let summary = build(selected.value);
        let mut pool = self.0.lock().unwrap();
        let memo = pool.get_mut(&selected).unwrap();
        memo.builds += 1;
        memo.groups.insert(key.to_vec(), summary);
        summary
    }
}

pub(super) fn explain(steps: &[Step], store: &Store) {
    use crate::code_rule_dsl::logic::profile;
    if !profile::on() {
        return;
    }
    let mut consumers = vec![];
    for (step, s) in steps.iter().enumerate() {
        if let Step::Unused { proof } = s {
            consumers.push(serde_json::json!({"step":step,"evaluation_elided":proof}));
            continue;
        }
        let Step::Scalar {
            src,
            bag: Some(bag),
            ..
        } = s
        else {
            continue;
        };
        let pool = match src {
            Src::In(r, _) => &r.summaries,
            Src::Full(r) | Src::FullMulti(r, _) | Src::Delta(r) => &store.rels[*r].summaries,
        };
        let state = pool.0.lock().unwrap();
        let selected = state
            .get(bag)
            .and_then(|m| m.preferred_value.as_ref())
            .filter(|_| bag.value.is_none())
            .unwrap_or(bag);
        if let Some(memo) = state.get(selected) {
            consumers.push(serde_json::json!({"step":step,"source_version_cache":format!("{:p}",pool),"columns":selected.columns,"tests":selected.tests,"value_column":selected.value,"epoch":memo.epoch,"cached_groups":memo.groups.len(),"folds":memo.builds,"lookups":memo.lookups}));
        }
    }
    profile::decision("native scalar storage", || {
        serde_json::json!({"consumers":consumers}).to_string()
    });
}

pub(super) fn register(steps: &[Step], store: Option<&Store>) {
    for step in steps {
        if let Step::Scalar {
            src,
            bag: Some(bag),
            ..
        } = step
        {
            match src {
                Src::In(rel, _) => rel.summaries.register(bag),
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
pub(super) fn plan(steps: &mut [Step]) -> bags::Analysis {
    let mut previous = std::collections::BTreeSet::new();
    let mut positions = Vec::new();
    let mut inputs = Vec::new();
    for (step, s) in steps.iter().enumerate() {
        match s {
            Step::Func { out, .. } | Step::Cat { out, .. } | Step::DocOf { out, .. } => {
                previous.insert(*out);
            }
            Step::Split { a, b, .. } => {
                previous.extend(a.iter().chain(b).copied());
            }
            Step::Let { a, b: Key::Slot(b) } if previous.contains(b) => {
                previous.insert(*a);
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
            Src::In(rel, _) => format!("input:{:p}", Arc::as_ptr(rel)),
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
            if let Step::Scalar {
                keys, tests, bag, ..
            } = &mut steps[positions[member]]
            {
                *bag = Some(Arc::new(Bag {
                    columns: keys.iter().map(|(c, _)| *c).collect(),
                    tests: tests.clone(),
                    value,
                }));
            }
        }
    }
    register(steps, None);
    crate::code_rule_dsl::logic::profile::decision("native scalar summaries", || {
        serde_json::json!({
        "analysis":analysis,
        "execution":"One count, exact sum and extrema fold per demanded bag and lookup key; pure compatible consumers reuse it",
        "storage_boundary":"Immutable input object, or component row count and visible delta range; dependent consumers do not reuse summaries",
        "sum_range_check":"At each scalar use, after exact accumulation",
    }).to_string()
    });
    analysis
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(fresh[1].data, vec![i64::MAX]);
    }
    #[test]
    fn shared_native_fold_preserves_duplicates_empty_results_and_range_checks() {
        let pool = Pool::default();
        let bag = Arc::new(Bag {
            columns: vec![0],
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
            let s = pool.evaluate(&bag, &[1], (3, 0, 3), build);
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
        let s = pool.evaluate(&bag, &[1], (4, 0, 4), build);
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
        let path = crate::code_rule_dsl::prefilter::tests::temp();
        drop(crate::code_rule_dsl::prefilter::tests::open(&path));
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
        assert_eq!(counted[1].data, vec![1, 3, 2, 1, 3, 0]);
        for j in [&j, &j2] {
            let mut fresh = vec![Fresh::default()];
            assert_eq!(run_join(j, 0, &store, &mut syms, &mut fresh).unwrap(), 2);
            assert_eq!(fresh[0].data, vec![1, 3, 17, 3, 7, 2, 1, -2, -2, -2]);
        }
        let Step::Scalar {
            src: Src::In(rel, _),
            ..
        } = &j.steps[1]
        else {
            panic!("native scalar")
        };
        let pool = rel.summaries.0.lock().unwrap();
        let active = pool
            .values()
            .filter(|m| !m.groups.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(active.len(), 1);
        let memo = active[0];
        assert_eq!(
            memo.builds, 3,
            "one fold per demanded key, including an empty group, across both rules"
        );
        drop(pool);
        // Rebinding equal-sized inputs must create a different immutable summary owner.
        forget_inputs(&db);
        db.execute_batch("UPDATE events SET v=1").unwrap();
        let next = parse_join(&db, &spec, &mut syms).unwrap();
        let mut fresh = vec![Fresh::default()];
        run_join(&next, 0, &store, &mut syms, &mut fresh).unwrap();
        assert_eq!(fresh[0].data, vec![1, 3, 3, 1, 1, 2, 1, 1, 1, 1]);
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
            fresh[1].data.clone()
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
                .data
                .chunks_exact(5)
                .all(|row| row[1..] == [3, 17, 3, 7])
        );
        let Step::Scalar {
            src: Src::In(rel, _),
            ..
        } = &j.steps[1]
        else {
            panic!("native input")
        };
        let pool = rel.summaries.0.lock().unwrap();
        assert_eq!(
            pool.values().map(|m| m.builds).sum::<usize>(),
            1000,
            "80,000 scalar uses must share 1,000 bag folds"
        );
        assert_eq!(pool.values().map(|m| m.groups.len()).sum::<usize>(), 1000);
    }

    #[test]
    fn mutable_input_views_pin_their_version_before_any_scalar_fold() {
        use crate::code_rule_dsl::facts::vtab::{Modules, Source};
        struct Cells(Arc<Mutex<Vec<[i64; 2]>>>);
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
            fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
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
        let values = Arc::new(Mutex::new(vec![[1, 7], [1, 3]]));
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
            fresh[0].data.clone()
        };
        assert_eq!(run(&first), vec![2, 10]);
        forget_inputs(&db);
        let second = parse_join(&db, &spec, &mut Interner::for_loading(&store.syms)).unwrap();
        assert_eq!(run(&second), vec![2, 103]);
        assert_eq!(run(&first), vec![2, 10]);
    }
}
