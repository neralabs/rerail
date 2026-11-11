use super::*;
use crate::code_rule_dsl::prefilter::tests::{open, temp};

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
    let (a, b) = ([1; 32], [2; 32]);
    store
        .candidate_transaction(true, |db| {
            let mut plans = Plans::default();
            plans.put(db, a, compiled("a"), &reads(&[("rows:1:5", 100), ("rows:1:6:x", 3)]))?;
            plans.put(db, b, compiled("b"), &reads(&[("rows:1:5", 100)]))?;
            let now = reads(&[("rows:1:5", 127), ("rows:1:6:x", 2)]);
            assert_eq!(plans.valid(db, &[a, b], &now)?.len(), 2);
            // One statistic leaves its bucket: only the plan that read it is invalid.
            let now = reads(&[("rows:1:5", 127), ("rows:1:6:x", 4)]);
            assert_eq!(plans.valid(db, &[a, b], &now)?.into_keys().collect::<Vec<_>>(), [b]);
            // A statistic the caller did not read cannot confirm a plan.
            assert!(plans.valid(db, &[a], &reads(&[("rows:1:5", 127)]))?.is_empty());
            // Recompiling replaces the decisions and their intervals.
            plans.put(db, a, compiled("a2"), &reads(&[("rows:1:5", 127)]))?;
            assert_eq!(plans.valid(db, &[a], &now)?[&a].program, b"a2");
            Ok(())
        })
        .unwrap();
    // A reopened store answers from its rows, in one statement, without the in-memory copy.
    drop(store);
    let store = open(&path);
    store
        .candidate_transaction(false, |db| {
            let mut plans = Plans::default();
            let now = reads(&[("rows:1:5", 64)]);
            let valid = plans.valid(db, &[a, b, [3; 32]], &now)?;
            assert_eq!(valid.keys().copied().collect::<Vec<_>>(), [a, b]);
            assert_eq!(*valid[&b], compiled("b"));
            // Then from memory: the stale answer for 200 comes without the rows.
            db.execute_batch("PRAGMA query_only = ON")?;
            assert!(plans.valid(db, &[a, b], &reads(&[("rows:1:5", 200)]))?.is_empty());
            db.execute_batch("PRAGMA query_only = OFF")
        })
        .unwrap();
}

#[test]
fn feedback_is_an_ewma_per_plan_and_backend() {
    let store = open(&temp());
    store
        .candidate_transaction(true, |db| {
            let plan = [7; 32];
            assert_eq!(feedback(db, plan, "cpu")?, None);
            observe(db, plan, "cpu", 0.5, 100.0)?;
            observe(db, plan, "cpu", 0.1, 20.0)?;
            observe(db, plan, "metal", 0.9, 1.0)?;
            let cpu = feedback(db, plan, "cpu")?.unwrap();
            assert!((cpu.selectivity - 0.4).abs() < 1e-9 && (cpu.ns_per_root - 80.0).abs() < 1e-9);
            assert_eq!(cpu.samples, 2);
            assert_eq!(feedback(db, plan, "metal")?.unwrap().samples, 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn sqlite_stat1_from_fact_statistics_steers_index_choice() {
    let store = open(&temp());
    store
        .candidate_transaction(true, |db| {
            db.execute_batch(
                "CREATE TABLE stat_probe(d INTEGER, a INTEGER, b INTEGER); \
                 CREATE INDEX stat_probe_a ON stat_probe(a); CREATE INDEX stat_probe_b ON stat_probe(b);",
            )?;
            let plan = |db: &Connection| -> Result<String> {
                db.query_row("EXPLAIN QUERY PLAN SELECT * FROM stat_probe WHERE a = 1 AND b = 2", [], |r| r.get(3))
            };
            let indexes: [(&str, &str, &[&str]); 2] = [("stat_probe", "stat_probe_a", &["a"]), ("stat_probe", "stat_probe_b", &["b"])];
            // a: 1000 rows over 1000 values; b: 1000 rows in one value.
            db.execute_batch(
                "INSERT INTO fact_stats VALUES ('stat_probe','a',1000,1000,1000,1000), ('stat_probe','b',1000,1,1000000,1000000)",
            )?;
            write_stat1(db, &indexes)?;
            assert_eq!(db.query_row("SELECT stat FROM sqlite_stat1 WHERE idx='stat_probe_b'", [], |r| r.get::<_, String>(0))?, "1000 1000");
            assert!(plan(db)?.contains("stat_probe_a"), "{}", plan(db)?);
            db.execute_batch(
                "UPDATE fact_stats SET ndv=1, sq=1000000 WHERE col='a'; UPDATE fact_stats SET ndv=1000, sq=1000 WHERE col='b'",
            )?;
            write_stat1(db, &indexes)?;
            assert!(plan(db)?.contains("stat_probe_b"), "{}", plan(db)?);
            Ok(())
        })
        .unwrap();
}
