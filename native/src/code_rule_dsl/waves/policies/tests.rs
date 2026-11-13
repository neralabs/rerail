use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs").join(pack);
    let mut paths = Vec::new();
    files(&root.join("rules"), &mut paths);
    files(&root.join("candidates/rules"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let compiled = CodeRuleDslRuntime::compile(std::fs::read_to_string(&path).ok()?, "json".to_owned(), CAPABILITIES.to_owned()).ok()?;
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
    let forbids = lowered.program.rules.iter().filter(|rule| rule.body.iter().any(|l| matches!(l, Literal::Ast { .. }))).count();
    assert!(forbids >= lowered.clauses.len(), "{forbids} forbid rules, {} lowered", lowered.clauses.len());
    // Rules of one shape are one rule over its table: each lowered atom is a row, or a rule kept as written.
    let wave = crate::code_rule_dsl::logic::rr::parse(&lowered.logic.rules).unwrap();
    let rows: usize = wave.facts.iter().filter(|(rel, _)| rel.starts_with("violation_shape_")).map(|(_, rows)| rows.len()).sum();
    let kept = wave.rules.iter().filter(|rule| !rule.body.iter().any(|l| l.atom().is_some_and(|a| a.rel.starts_with("violation_shape_")))).count();
    assert!(rows > 0, "the pack's forbid rules share a shape");
    assert_eq!(rows + kept, lowered.clauses.len(), "each lowered atom once");
    assert!(wave.rules.len() < lowered.clauses.len() / 4, "{} rules for {} atoms", wave.rules.len(), lowered.clauses.len());
    // YAML rules demand every clause on every document of the chunk.
    let demand = lowered.logic.demand.as_ref().expect("a demand program");
    assert_eq!(demand.rules.iter().filter(|rule| rule.head.rel.starts_with("magic_ast_")).count(), lowered.clauses.len());
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
    let rule = |clause: &str, body: Vec<Literal>| Rule {
        head: Atom { rel: "violation".into(), args: vec![Term::Str("p".into()), Term::Str(clause.to_owned().into()), var("D"), var("N")], neg: false, gated: false },
        body: std::iter::once(atom(WAVE, vec![var("D")])).chain(body).collect(),
    };
    let ast = |i: usize, n: &str| atom(&format!("ast_{i}"), vec![var("D"), var(n)]);
    let seven = Literal::Cmp { l: var("D"), op: "=", r: Term::Int(7) };
    let prog = Program {
        rules: vec![rule("a", vec![ast(0, "N")]), rule("b", vec![seven, ast(1, "N")]), rule("c", vec![ast(0, "M"), ast(2, "N")])],
        ..Program::default()
    };
    let mut wave = Program::default();
    declare_violation(&mut wave);
    let demand = demand_program(prog, &wave, 3).unwrap().unwrap();
    let logic = Logic { rules: String::new(), demand: Some(demand), bound: Vec::new() };
    let sql = logic.demand_statement(&store()).unwrap().expect("one statement");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TEMP TABLE wave_document(position INTEGER, d INTEGER); INSERT INTO temp.wave_document VALUES (0, 5), (1, 7)").unwrap();
    let mut rows = db.prepare(&sql).unwrap().query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    rows.sort();
    assert_eq!(rows, [(0, 5), (0, 7), (1, 7), (2, 5), (2, 7)]);
}

/// A `py.*` family is bound before the batch, so a rule that reads one before its atom narrows that
/// atom's demand: clause 0 is demanded only on the documents with a call (`py_call`).
#[test]
fn py_families_narrow_demand_before_the_batch() {
    let rule = |clause: &str, body: Vec<Literal>| Rule {
        head: Atom { rel: "violation".into(), args: vec![Term::Str("p".into()), Term::Str(clause.to_owned().into()), var("D"), var("N")], neg: false, gated: false },
        body: std::iter::once(atom(WAVE, vec![var("D")])).chain(body).collect(),
    };
    let calls = atom("py_call", vec![var("D"), var("C"), var("F")]);
    let prog = Program { rules: vec![rule("a", vec![calls, atom("ast_0", vec![var("D"), var("N")])])], ..Program::default() };
    let mut wave = Program { rules: prog.rules.clone(), ..Program::default() };
    declare_violation(&mut wave);
    declare_py(&mut wave);
    assert_eq!(wave.decls["py_call"].iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["d", "call", "callee"]);
    let demand = demand_program(prog, &wave, 1).unwrap().unwrap();
    let logic = Logic { rules: String::new(), demand: Some(demand), bound: Vec::new() };
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
    let prog = merged(crate::code_rule_dsl::logic::rr::parse(src).unwrap());
    assert_eq!(prog.rules.len(), 2, "{:?}", prog.rules.iter().map(ToString::to_string).collect::<Vec<_>>());
    assert_eq!(prog.rules[0].to_string(), "violation($K0, $K1, $V0, $V1) :- violation_shape_0($K0, $K1, $K2), matched($K2, $V0, $V1)");
    assert_eq!(prog.rules[1].to_string(), r#"violation("p", "c", $D, $N) :- matched(2, $D, $N), $D = 7"#);
    let rows = &prog.facts["violation_shape_0"];
    assert_eq!(rows, &vec![
        vec![Value::Str("p".into()), Value::Str("a".into()), Value::Int(0)],
        vec![Value::Str("q".into()), Value::Str("b".into()), Value::Int(1)],
    ]);
    assert_eq!(prog.decls["violation_shape_0"].iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["sym", "sym", "int"]);
    // The wave's text parses back to the same program.
    let back = crate::code_rule_dsl::logic::rr::parse(&wave_text(&prog).unwrap()).unwrap();
    assert_eq!(back.rules, prog.rules);
    assert_eq!(back.facts, prog.facts);
}
