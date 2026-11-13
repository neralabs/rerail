//! A maintained stratum's start from its kept relation (smoking gun 8, t34).
use super::spec::hex;
use super::*;

/// Smoking gun 8 (t34, 2026-11-13), red until fixed. A maintained stratum's relation starts as its kept rows less
/// those gone (`Start::Base`, `h__old` less `h__del`). `Rel::based` cloned the kept rows and every index, and
/// `Rel::compacted` pushed every row again with a new dedup index, so each maintained stratum cost its kept
/// relation's size, not its delta's: 4.47 s of an edit's 12.85 s, against 0.76 s of joins. The kept rows are the
/// start's own, shared, not copied.
#[test]
#[ignore = "smoking gun 8 open (t34): a maintained relation's start copies its kept rows and indexes"]
fn a_maintained_relation_starts_from_its_kept_rows_without_copying_them() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db.execute_batch("CREATE TABLE gun8_s(c0, c1); CREATE TABLE gun8_kept(c0, c1); CREATE TABLE gun8_next(c0, c1);").unwrap();
    let n = 20_000i64;
    {
        let mut insert = db.prepare("INSERT INTO gun8_s VALUES (?1, ?2)").unwrap();
        for i in 0..n {
            insert.execute([i, i % 97]).unwrap();
        }
    }
    let seed = format!("pos t:b:{}:{}:ii v0 v1\nhead v0 v1", hex("gun8_s"), hex("c0, c1"));
    open(&db, &format!("name gun8_kept\nrel gun8_kept ii empty\nseed 0 {}", hex(&seed))).unwrap();
    run(&db, "gun8_kept", 0).unwrap();
    close(&db, "gun8_kept").unwrap();
    let old = DERIVED.lock().unwrap().iter().find(|((_, t), _)| t == "gun8_kept").map(|(_, r)| Arc::clone(r)).expect("the kept relation");
    assert_eq!(old.n, n as usize);
    open(&db, &format!("name gun8_next\nrel gun8_next ii base {} -", hex("gun8_kept"))).unwrap();
    let comp = COMPONENTS.lock().unwrap().iter().find(|((_, name), _)| name == "gun8_next").map(|(_, c)| Arc::clone(c)).expect("the maintained component");
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    let rel = &store.rels[0];
    assert_eq!(rel.n, old.n, "it starts as every kept row");
    assert!(std::ptr::eq(rel.data.as_ptr(), old.rows.as_ptr()), "its rows are the kept relation's, not a copy of its {n} rows");
}
