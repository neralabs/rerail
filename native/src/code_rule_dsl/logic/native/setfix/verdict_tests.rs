//! The CPU's text tests read and keep their verdicts in the store's rolling cache (decision 0450 step 2d, R3).
use super::*;

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// A text test's verdict is made once per symbol and kept in the store's cache, where the next test of that symbol
/// reads it: a verdict put there before the run (that `zzz` passes `@prefix(_, "foo")`) is what the run reads, so it
/// is the cache's answer, not the matcher's, that the rule takes. Each other name's verdict is kept as it ran.
#[test]
fn a_cpu_text_test_reads_and_keeps_its_verdicts_in_the_stores_cache() {
    let store = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&store));
    let db = Connection::open(&store).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
    db.execute_batch("CREATE TEMP TABLE n (c0, c1); INSERT INTO n VALUES ('foo1', 1), ('foo2', 2), ('bar', 3), ('zzz', 4);").unwrap();
    let syms = symbols(&db).unwrap();
    let sym = |s: &str| u32::try_from(syms.write().unwrap().id(s)).unwrap();
    let (foo1, bar, zzz) = (sym("foo1"), sym("bar"), sym("zzz"));
    let m = crate::code_rule_dsl::logic::strings::Matcher::new("prefix", &["foo".to_owned()]).unwrap();
    let test = crate::code_rule_dsl::text::table::test_id(&m.plans).unwrap();
    let atoms = Arc::clone(syms.read().unwrap().atoms());
    let verdicts = atoms.kept();
    assert!(verdicts.put(test, zzz, true));
    let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
    let rule = format!("pos {n} v0 v1\ntest prefix v0 x:{} 1\nhead v0 v1", hex("foo"));
    db.execute_batch("CREATE TEMP TABLE h0 (c0, c1);").unwrap();
    let spec = ["name cpu_text".to_owned(), "rel h0 si empty".into(), format!("seed 0 {}", hex(&rule))].join("\n");
    open(&db, &spec).unwrap();
    run(&db, "cpu_text", 0).unwrap();
    close(&db, "cpu_text").unwrap();
    let mut got: Vec<i64> = db.prepare("SELECT c1 FROM temp.h0").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    got.sort_unstable();
    assert_eq!(got, [1, 2, 4], "foo1, foo2, and zzz by the verdict the cache held");
    use crate::content_store::atoms::{FAILS, PASSES};
    assert_eq!((verdicts.get(test, foo1), verdicts.get(test, bar)), (PASSES, FAILS), "the verdicts the run made, kept");
}

/// A text function of a symbol reads and keeps its text in the store's one rolling cache (decision 0450 step 2e), by
/// the key the device keeps it by: a text put there before the run (`dir` of `a/b/c` is `KEPT`) is what the run
/// takes, so the cache answers, not the function; each other name's text is kept as it ran.
#[test]
fn a_cpu_text_function_reads_and_keeps_its_text_in_the_stores_cache() {
    let store = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&store));
    let db = Connection::open(&store).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
    db.execute_batch("CREATE TEMP TABLE n (c0, c1); INSERT INTO n VALUES ('a/b/c', 1), ('x/y.py', 2), ('top', 3);").unwrap();
    let syms = symbols(&db).unwrap();
    let sym = |s: &str| u32::try_from(syms.write().unwrap().id(s)).unwrap();
    let (abc, xy) = (sym("a/b/c"), sym("x/y.py"));
    let atoms = Arc::clone(syms.read().unwrap().atoms());
    let kept = atoms.kept();
    let dir = crate::code_rule_dsl::text::automata::text_functions("/dir[]").unwrap();
    assert!(kept.keep_text(dir, abc, "KEPT"));
    let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
    let rule = format!("pos {n} v0 v1\nfunc dir v2 v0\nhead v2 v1");
    db.execute_batch("CREATE TEMP TABLE h0 (c0, c1);").unwrap();
    let spec = ["name cpu_fn".to_owned(), "rel h0 si empty".into(), format!("seed 0 {}", hex(&rule))].join("\n");
    open(&db, &spec).unwrap();
    run(&db, "cpu_fn", 0).unwrap();
    close(&db, "cpu_fn").unwrap();
    let mut got: Vec<(String, i64)> = db.prepare("SELECT sym_text(c0), c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    got.sort_unstable();
    assert_eq!(got, [("".to_owned(), 3), ("KEPT".to_owned(), 1), ("x".to_owned(), 2)], "a/b/c by the text the cache held");
    assert_eq!(kept.text(dir, xy).as_deref(), Some("x"), "the text the run made, kept");
}
