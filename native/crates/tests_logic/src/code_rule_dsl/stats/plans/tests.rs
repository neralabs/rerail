use super::*;
use crate::code_rule_dsl::fixtures::{open, temp};

fn reads(values: &[(&str, u64)]) -> BTreeMap<String, u64> {
    values.iter().map(|(stat, value)| ((*stat).to_owned(), *value)).collect()
}

fn compiled(program: &str) -> Compiled {
    Compiled { decisions: format!("{{\"program\":\"{program}\"}}"), program: program.as_bytes().to_vec() }
}

#[test]
fn intervals_are_log2_buckets() {
    assert_eq!([0, 1, 2, 3, 4, 7, 8, 1000].map(interval), [(0, 0), (1, 1), (2, 3), (2, 3), (4, 7), (4, 7), (8, 15), (512, 1023)]);
    assert_eq!(interval(u64::MAX), (1 << 63, u64::MAX));
}

#[test]
fn a_plan_stays_valid_while_every_statistic_it_read_stays_in_its_bucket() {
    let path = temp();
    let store = open(&path);
    let at = store.database_path().to_path_buf();
    let (a, b) = ([1; 32], [2; 32]);
    {
        let mut plans = Plans::default();
        plans.put(&at, a, compiled("a"), &reads(&[("rows:1:5", 100), ("rows:1:6:x", 3)]));
        plans.put(&at, b, compiled("b"), &reads(&[("rows:1:5", 100)]));
        plans.flush(&store);
        let now = reads(&[("rows:1:5", 127), ("rows:1:6:x", 2)]);
        assert_eq!(plans.valid(&at, None, &[a, b], &now).len(), 2);
        // One statistic leaves its bucket: only the plan that read it is invalid.
        let now = reads(&[("rows:1:5", 127), ("rows:1:6:x", 4)]);
        assert_eq!(plans.valid(&at, None, &[a, b], &now).into_keys().collect::<Vec<_>>(), [b]);
        // A statistic the caller did not read cannot confirm a plan.
        assert!(plans.valid(&at, None, &[a], &reads(&[("rows:1:5", 127)])).is_empty());
        // Recompiling replaces the decisions and their intervals.
        plans.put(&at, a, compiled("a2"), &reads(&[("rows:1:5", 127)]));
        plans.flush(&store);
        assert_eq!(plans.valid(&at, None, &[a], &now)[&a].program, b"a2");
    }
    store.written().unwrap();
    // A reopened store answers from its kept facts, read where they lie, without the in-memory copy.
    drop(store);
    let store = open(&path);
    let mut plans = Plans::default();
    let kept = Plans::open(&store).unwrap();
    assert!(kept.is_some(), "the plans are kept facts of the store");
    let now = reads(&[("rows:1:5", 64)]);
    let valid = plans.valid(&at, kept.as_deref(), &[a, b, [3; 32]], &now);
    assert_eq!(valid.keys().copied().collect::<Vec<_>>(), [a, b]);
    assert_eq!(*valid[&b], compiled("b"));
    assert_eq!(valid[&a].program, b"a2");
    // Then from memory: the stale answer for 200 comes without reading the store.
    assert!(plans.valid(&at, None, &[a, b], &reads(&[("rows:1:5", 200)])).is_empty());
}

#[test]
fn feedback_is_an_ewma_per_plan_and_backend() {
    let store = open(&temp());
    let plan = [7; 32];
    assert_eq!(feedback(&store, plan, "cpu").unwrap(), None);
    observe(&store, plan, "cpu", 0.5, 100.0).unwrap();
    observe(&store, plan, "cpu", 0.1, 20.0).unwrap();
    observe(&store, plan, "metal", 0.9, 1.0).unwrap();
    let cpu = feedback(&store, plan, "cpu").unwrap().unwrap();
    assert!((cpu.selectivity - 0.4).abs() < 1e-9 && (cpu.ns_per_root - 80.0).abs() < 1e-9);
    assert_eq!(cpu.samples, 2);
    assert_eq!(feedback(&store, plan, "metal").unwrap().unwrap().samples, 1);
}
