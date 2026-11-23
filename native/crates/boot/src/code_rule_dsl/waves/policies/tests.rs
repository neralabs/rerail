use super::*;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::path::{Path, PathBuf};

fn files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, found);
        } else if path.extension().is_some_and(|e| e == "json") {
            found.push(path);
        }
    }
}

/// A pack's compiled policies, by file stem.
fn policies(pack: &str) -> Vec<(String, CompiledCodeRuleDsl)> {
    let root = crate::source_tree::native().join("../packs").join(pack);
    let mut paths = Vec::new();
    files(&root.join("rules"), &mut paths);
    files(&root.join("candidates/rules"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let compiled = crate::code_rule_dsl::rule_api::compile(&std::fs::read_to_string(&path).ok()?, "json", CAPABILITIES).ok()?;
            Some((path.file_stem()?.to_string_lossy().into_owned(), compiled))
        })
        .collect()
}

fn store() -> ContentStore {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    ContentStore::open(&std::env::temp_dir().join(format!("rerail-policies-{}-{nonce}.sqlite", std::process::id()))).unwrap()
}

/// The Python pack through the logic language: every document round-trips, every lowered clause is an
/// `ast` atom of its forbid rule, and the wave's rules derive `violation` from `matched`.
#[test]
fn the_python_pack_runs_as_its_logic_program() {
    let policies = policies("python");
    let lowered = lower(&policies).unwrap();
    assert!(!lowered.clauses.is_empty());
    assert_eq!(lowered.clauses.len(), lowered.shapes.len());
    let program = &lowered.program;
    let forbids = program.rules().filter(|rule| rule.body().any(|l| matches!(l.view(), Lit::Ast { .. }))).count();
    assert!(forbids >= lowered.clauses.len(), "{forbids} forbid rules, {} lowered", lowered.clauses.len());
    // Rules of one shape are one rule over its table: each lowered atom is a row, or a rule kept as written.
    let wave = &lowered.logic.rules;
    let rows: usize = wave.fact_names().iter().filter(|rel| rel.starts_with("violation_shape_")).map(|rel| wave.fact_rows(rel).len()).sum();
    let kept = wave.rules().filter(|&rule| !reads(wave, rule).any(|rel| rel.starts_with("violation_shape_"))).count();
    assert!(rows > 0, "the pack's forbid rules share a shape");
    assert_eq!(rows + kept, lowered.clauses.len(), "each lowered atom once");
    assert!(wave.rules().count() < lowered.clauses.len() / 4, "{} rules for {} atoms", wave.rules().count(), lowered.clauses.len());
    // YAML rules demand every clause on every document of the chunk.
    let demand = lowered.logic.demand.as_ref().expect("a demand program");
    assert_eq!(demand.rules().filter(|&rule| demand.rel_name(rule.head().rel()).starts_with("magic_ast_")).count(), lowered.clauses.len());
    let sql = lowered.logic.demand_statement(&store()).unwrap().expect("one statement");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TEMP TABLE wave_document(position INTEGER, d INTEGER); INSERT INTO temp.wave_document VALUES (0, 5), (1, 9)").unwrap();
    let rows: usize = db.prepare(&sql).unwrap().query_map([], |_| Ok(())).unwrap().count();
    assert_eq!(rows, 2 * lowered.clauses.len());
}

/// Demand follows the rules: a document filter before an atom narrows its clause's demand, and an atom
/// read after another is demanded wherever the seed is (widened: the other is the batch's).
#[test]
fn magic_sets_demand_each_clause_where_its_rules_read_it() {
    let demand = demand_of(
        "violation(\"p\", \"a\", D, N) :- wave(D), ast_0(D, N).
         violation(\"p\", \"b\", D, N) :- wave(D), D = 7, ast_1(D, N).
         violation(\"p\", \"c\", D, N) :- wave(D), ast_0(D, M), ast_2(D, N).",
        3,
    );
    let logic = Logic { rules: nothing().unwrap(), demand: Some(demand), bound: Vec::new() };
    let sql = logic.demand_statement(&store()).unwrap().expect("one statement");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TEMP TABLE wave_document(position INTEGER, d INTEGER); INSERT INTO temp.wave_document VALUES (0, 5), (1, 7)").unwrap();
    let mut rows = db.prepare(&sql).unwrap().query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    rows.sort();
    assert_eq!(rows, [(0, 5), (0, 7), (1, 7), (2, 5), (2, 7)]);
}

/// The demand program of the rules of `src` (each reading `ast_i` atoms after `wave` seeds), declared as the wave
/// declares them.
fn demand_of(src: &str, atoms: usize) -> Arc<Version> {
    let v = crate::code_rule_dsl::logic::rr::parse_version(src).unwrap();
    let mut b = Builder::base();
    let mut read = BTreeSet::new();
    for r in v.rules() {
        b.copy(&v, r);
        read.extend(reads(&v, r).map(str::to_owned));
    }
    declare_violation(&mut b);
    declare_read(&mut b, &read);
    demand_program(b, atoms).unwrap().unwrap()
}

/// A `py.*` family is bound before the batch, so a rule that reads one before its atom narrows that
/// atom's demand: clause 0 is demanded only on the documents with a call (`py_call`).
#[test]
fn py_families_narrow_demand_before_the_batch() {
    let mut b = Builder::base();
    declare_read(&mut b, &["py_call".to_owned()].into());
    assert_eq!(b.columns_of("py_call").unwrap().iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["d", "call", "callee"]);
    let demand = demand_of("violation(\"p\", \"a\", D, N) :- wave(D), py_call(D, C, F), ast_0(D, N).", 1);
    let logic = Logic { rules: nothing().unwrap(), demand: Some(demand), bound: Vec::new() };
    let sql = logic.demand_statement(&store()).unwrap().expect("one statement");
    assert!(sql.contains("py_call"), "{sql}");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TEMP TABLE wave_document(position INTEGER, d INTEGER); INSERT INTO temp.wave_document VALUES (0, 5), (1, 7);
         CREATE TABLE py_call(d INTEGER, call INTEGER, callee INTEGER); INSERT INTO py_call VALUES (7, 30064771073, 30064771074)",
    )
    .unwrap();
    let rows = db.prepare(&sql).unwrap().query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(rows, [(0, 7)]);
}

/// Rules equal once their variables are renamed and their constants taken out are one rule over a table of
/// their constants; a rule of its own shape is kept as written.
#[test]
fn rules_of_one_shape_are_one_rule_over_their_constants() {
    let src = r#"
        .decl violation(policy: symbol, clause: symbol, d: number, n: number)
        violation("p", "a", D, N) :- matched(0, D, N).
        violation("q", "b", X, Y) :- matched(1, X, Y).
        violation("p", "c", D, N) :- matched(2, D, N), D = 7.
    "#;
    let prog = merged(&crate::code_rule_dsl::logic::rr::parse_version(src).unwrap()).unwrap();
    let rules: Vec<String> = prog.rules().map(|r| prog.show_rule(r)).collect();
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert_eq!(rules[0], "violation($K0, $K1, $D, $N) :- violation_shape_0($K0, $K1, $K2), matched($K2, $D, $N)", "{rules:?}");
    assert_eq!(rules[1], r#"violation("p", "c", $D, $N) :- matched(2, $D, $N), $D = 7"#, "{rules:?}");
    assert_eq!(prog.fact_rows("violation_shape_0"), vec![
        List::from(vec![Value::Str("p".into()), Value::Str("a".into()), Value::Int(0)]),
        List::from(vec![Value::Str("q".into()), Value::Str("b".into()), Value::Int(1)]),
    ]);
    assert_eq!(prog.columns_of("violation_shape_0").unwrap().iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["sym", "sym", "int"]);
}
