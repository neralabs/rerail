use super::*;
use crate::code_rule_dsl::logic::rr::parse_version;
use crate::code_rule_dsl::logic::rr::parse_version as parse;
use crate::code_rule_dsl::logic::program_image::Lit;
use crate::code_rule_dsl::logic::plan::corpus::preparation::magic::{KeyDemand, ProgramDemand};
use crate::code_rule_dsl::logic::plan::corpus::preparation::prune::Prune;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
use crate::content_store::ContentStore;

/// Stable order keys have gaps at nested granules. A SQL descent follows the tree,
/// includes a nested scope itself, and excludes the nodes inside that scope.
#[test]
fn sql_descendant_walks_cross_sparse_order_keys_and_stop_at_nested_scopes() {
    let p = parse_version(
        r#"
.decl roots(f:node)
roots(100). roots(102). roots(101). roots(105). roots(106).
.decl targets(n:node)
targets(103). targets(104). targets(107).
.decl scope(k:symbol)
scope("function_definition").
.decl seen(f:node,n:node)
seen(f,n) :- roots(f), nearest(n,f,scope).
.decl wanted(f:node,n:node)
wanted(f,n) :- roots(f), targets(n), nearest(n,f,scope).
.output seen,wanted
"#,
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        r#"
CREATE TABLE roots(c0 INTEGER);
INSERT INTO roots VALUES(100),(102),(101),(105),(106);
CREATE TABLE targets(c0 INTEGER);
INSERT INTO targets VALUES(103),(104),(107);
CREATE TABLE node(n INTEGER PRIMARY KEY,kind TEXT);
INSERT INTO node VALUES(100,'function_definition'),(101,'return_statement'),
    (102,'function_definition'),(103,'return_statement'),(104,'return_statement'),
    (105,'function_definition'),(106,'block'),(107,'return_statement');
CREATE TABLE child(p INTEGER,c INTEGER,idx INTEGER);
CREATE INDEX child_parent ON child(p);
INSERT INTO child VALUES(100,101,0),(100,102,1),(102,103,0),(100,104,2),(106,107,0);
CREATE TABLE tape(n INTEGER PRIMARY KEY,par INTEGER,size INTEGER,depth INTEGER);
INSERT INTO tape VALUES(100,-1,4,0),(101,100,0,1),(102,100,1,1),(103,102,0,2),
    (104,100,0,1),(105,-1,0,0),(106,-1,1,0),(107,106,0,1);
CREATE TABLE node_order(n INTEGER PRIMARY KEY,k INTEGER UNIQUE);
INSERT INTO node_order VALUES(100,0),(101,1),(102,262144),(103,262145),(104,524290),
    (105,786432),(106,1048576),(107,1048577);
"#,
    )
    .unwrap();
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()));
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    for (name, expected) in [
        ("seen", vec![(100, 101), (100, 102), (100, 104), (102, 103)]),
        ("wanted", vec![(100, 104), (102, 103)]),
    ] {
        let k = p.rules().position(|r| p.rel_name(r.head().rel()) == name).unwrap();
        let b = body::nth(&p, k);
        let order = (0..b.len())
            .map(|at| Step { at, mode: matches!(b.view(at), Lit::Nearest { .. }).then_some(Mode::Descend) })
            .collect::<Vec<_>>();
        let query = model
            .checked_query(body::Placed::whole(p.clone()).fields().unwrap(), &b, &order, None, None)
            .unwrap();
        let sql = query.render();
        let mut rows = db
            .prepare(&sql)
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows.sort();
        assert_eq!(rows, expected, "{sql}");
    }
}

#[test]
fn native_scalar_consumers_match_sql_with_shadowed_values_and_constant_filters() {
    let p = parse_version(".decl event(k:number,id:number,g:number,v:number)\nevent(9,0,1,7).\nevent(9,1,1,7).\nevent(9,2,1,3).\nevent(8,3,1,90).\nevent(9,4,2,-2).\nevent(9,5,4,9223372036854775807).\nevent(9,6,4,1).\nevent(9,7,4,-1).\n.decl owner(g:number,v:number)\nowner(1,99).\nowner(2,99).\nowner(3,99).\nowner(4,99).\n.decl result(g:number,v:number,n:number,s:number,lo:number,hi:number)\nresult(g,v,n,s,lo,hi) :- owner(g,v), n=count : {event(9,_,g,_)}, s=sum v : {event(9,_,g,v)}, lo=min x : {event(9,_,g,x)}, hi=max y : {event(9,_,g,y)}, dead=sum z : {event(_,_,_,z)}, unused=dead.\n.decl repeated(g:number,s:number)\nrepeated(g,s) :- owner(g,_), s=sum x : {event(x,_,g,x)}.\n.output result,repeated\n").unwrap();
    let mut outputs=Vec::new();
    for modes in ["","program"] {
        let path=crate::code_rule_dsl::fixtures::temp();
        let store=ContentStore::open(&path).unwrap();
        let python=crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered=lower::lowered(p.clone(),python,&mut |s|store.intern_atoms(s).map_err(|e|Error::Plan(e.to_string()))).unwrap();
        let db=Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got=run_atoms_native_over(&lowered,&db,"main",Stats::empty_of_lowered(&lowered).unwrap(),BTreeSet::new(),modes).unwrap();
        if !modes.is_empty() {assert!(got.native.contains("result"),"scalar result must execute natively");}
        outputs.push(got.shown().clone());
    }
    assert_eq!(outputs[0],outputs[1]);
    assert_eq!(outputs[1]["repeated"],[["1","0"],["2","0"],["3","0"],["4","0"]]);
    assert_eq!(outputs[1]["result"],[["1","99","3","17","3","7"],["2","99","1","-2","-2","-2"],["4","99","3","9223372036854775807","-1","9223372036854775807"]]);
}

/// Two rules of one recursive stratum with one body and different heads (a magic relation and its reader's, as
/// decision 0145's demand writes them) each run their whole plan in the first wave: the plans are kept apart by
/// head, so neither head loses the rows the first wave derives.
#[test]
fn rules_of_one_body_and_two_heads_both_run_their_whole_plan() {
    let p = parse_version(".decl seed(x:number)\nseed(1).\n.decl e(x:number,y:number)\ne(1,2).\ne(2,3).\ne(3,4).\n.decl a(x:number)\n.decl b(x:number)\na(x) :- seed(x).\na(y) :- a(x), e(x,y).\nb(y) :- a(x), e(x,y).\na(x) :- b(x).\n.output a,b\n").unwrap();
    for modes in ["","program"] {
        let path=crate::code_rule_dsl::fixtures::temp();
        let store=ContentStore::open(&path).unwrap();
        let python=crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered=lower::lowered(p.clone(),python,&mut |s|store.intern_atoms(s).map_err(|e|Error::Plan(e.to_string()))).unwrap();
        let db=Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got=run_atoms_native_over(&lowered,&db,"main",Stats::empty_of_lowered(&lowered).unwrap(),BTreeSet::new(),modes).unwrap();
        assert_eq!(got.shown()["a"],vec![["1"], ["2"], ["3"], ["4"]],"modes {modes:?}");
        assert_eq!(got.shown()["b"],vec![["2"], ["3"], ["4"]],"modes {modes:?}");
    }
}

/// Decision 0462: a derived relation past the width cap stops the run, its error naming the relation and the rule
/// deriving it; under `RERAIL_WIDEST=cut` the run goes on with the relation cut there, and says so at its end.
#[test]
fn a_relation_past_the_width_cap_stops_the_run_or_is_cut() {
    use super::super::native::width;
    let facts: String = (1..=100).map(|i| format!("l({i}).\nr({i}).\n")).collect();
    let p = parse_version(&format!(".decl l(x:number)\n.decl r(x:number)\n{facts}.decl pairs(x:number,y:number)\npairs(x, y) :- l(x), r(y).\n.output pairs\n")).unwrap();
    let run = || {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        // The native strata (`program`), as a corpus run's: the SQL strata's inserts are bounded by the memory ceiling.
        run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), "program")
    };
    // Each run's limits are its own (no process setting): the test holds each run's scope, as a command does.
    let stopped = {
        let _run = super::super::run::scope(|| width::Settings { cap: 1_000, cut: false, ceiling: None });
        run()
    };
    let (cut, record) = {
        let own = super::super::run::scope(|| width::Settings { cap: 1_000, cut: true, ceiling: None });
        let cut = run();
        (cut, own.finish().expect("the test's run is the outermost"))
    };
    let report = record.width.incomplete();
    let error = stopped.expect_err("a relation past the cap stops the run").to_string();
    assert!(error.contains("`pairs` reached 1000 rows") && error.contains("l(") && error.contains("r("), "the error names the relation and its rule: {error}");
    let out = cut.expect("cut, the run goes on");
    let kept = out.shown()["pairs"].len();
    assert!(kept > 0 && kept <= 1_000, "the rows made before the cap are kept, none past it: {kept}");
    let report = report.expect("a cut is reported at the end");
    assert!(report.contains("`pairs`") && report.contains("derived by pairs(") && report.contains("l("), "the report names the relation and its rule: {report}");
}

/// Whether this process numbers nodes in preorder, as T's facts and outputs do (`RERAIL_NODE_IDS=preorder`). The
/// tests that compare with T run only then: under ids per definition (the default, decision 0094 §2a) a program that
/// orders nodes reads `node_order`, which T's facts do not have, and outputs show ids as `slot.index`.
fn t_numbering(test: &str) -> bool {
    let preorder = !crate::code_rule_dsl::facts::vtab::syntax::definition_ids();
    if !preorder {
        eprintln!("{test}: skipped, T's facts number nodes in preorder; run with RERAIL_NODE_IDS=preorder");
    }
    preorder
}

/// A head that qualifies a bound name by a constant scope (`sym_at($e, @cat(1539, $x))`, the builtins' symbols)
/// has a native step: the name is made into a slot before the head. A stratum the native engine declines runs in
/// SQLite, a wait on SQL tables between waves that are otherwise in memory.
#[test]
fn a_head_that_qualifies_by_a_constant_scope_runs_natively() {
    let p = parse_version(".decl p(e: number, x: symbol)\np(1, \"a\").\np(2, \"b\").\n.decl q(e: number, s: symbol)\nq(e, @dot(\"m\", x)) :- p(e, x).\n.output q\n").unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes).unwrap();
        if !modes.is_empty() {
            assert!(got.native.contains("q"), "q runs natively: {:?}", got.native);
        }
        let mut o = got.shown()["q"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 2, "{:?}", outputs[1]);
}

/// Device waves (decision 0095 §5): three closures of one level, each over a graph with a fork and a cycle, wait
/// for their bit rows' launches, which run as one batch when the level ends; the rows are SQL's.
#[test]
fn closures_of_one_level_launch_in_one_wave() {
    let mut src = String::new();
    for (k, edges) in [(1, "(1,2),(1,3),(2,4),(3,4),(4,1),(5,6)"), (2, "(10,11),(10,12),(12,13),(13,10)"), (3, "(20,21),(21,22),(21,23),(23,20),(24,24)")] {
        src += &format!(".decl e{k}(a: number, b: number)\n");
        for e in edges.split("),(") {
            src += &format!("e{k}({}).\n", e.trim_matches(|c| c == '(' || c == ')'));
        }
        src += &format!(".decl r{k}(a: number, b: number)\nr{k}(a, b) :- e{k}(a, b).\nr{k}(a, c) :- r{k}(a, b), e{k}(b, c).\n");
    }
    src += ".decl o(k: number, a: number, b: number)\no(1, a, b) :- r1(a, b).\no(2, a, b) :- r2(a, b).\no(3, a, b) :- r3(a, b).\n.output o\n";
    let p = parse_version(&src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::super::lanes::wave::flushed();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes).unwrap();
        let after = super::super::lanes::wave::flushed();
        if !modes.is_empty() {
            for r in ["r1", "r2", "r3"] {
                assert!(got.native.contains(r), "{r} runs natively");
            }
            assert_eq!((after.0 - before.0, after.1 - before.1), (1, 3), "the level's three closures in one wave of three launches");
        }
        let mut o = got.shown()["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 17 + 12 + 13, "{:?}", outputs[1]);
}

/// Device waves (decision 0095 §5) across components (decision 0113): a level of twenty closures is more than one
/// component takes (its estimate reaches `k` fixed costs first), so the scoreboard issues it as several. Their bit
/// rows' launches are still the level's one wave, not one wave per component.
#[test]
fn closures_of_a_level_wider_than_a_component_launch_in_one_wave() {
    const N: usize = 20;
    let mut src = String::new();
    for k in 0..N {
        let b = 100 * k as i64;
        src += &format!(".decl e{k}(a: number, b: number)\n");
        for (x, y) in [(1, 2), (1, 3), (2, 4), (3, 4), (4, 1)] {
            src += &format!("e{k}({}, {}).\n", b + x, b + y);
        }
        src += &format!(".decl r{k}(a: number, b: number)\nr{k}(a, b) :- e{k}(a, b).\nr{k}(a, c) :- r{k}(a, b), e{k}(b, c).\n");
    }
    src += ".decl o(k: number, a: number, b: number)\n";
    for k in 0..N {
        src += &format!("o({k}, a, b) :- r{k}(a, b).\n");
    }
    src += ".output o\n";
    let p = parse_version(&src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::super::lanes::wave::flushed();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes).unwrap();
        let after = super::super::lanes::wave::flushed();
        if !modes.is_empty() {
            for k in 0..N {
                assert!(got.native.contains(&format!("r{k}")), "r{k} runs natively");
            }
            assert_eq!((after.0 - before.0, after.1 - before.1), (1, N as u64), "the level's closures in one wave of {N} launches");
        }
        let mut o = got.shown()["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 16 * N, "{:?}", outputs[1].len());
}

/// Decision 0123 §9: a probe with no kept plan is planned when its delta first has rows. In `{r, s, t}`, `s` never
/// has a row (`g` is empty, though not to the program's prune: no edge is a loop), so `r`'s probe by `s` is never
/// planned; the probes by `r` are planned when the first wave returns, and the run derives what the SQL run does.
/// The probes' counts are the process's (`lazy_probes_counted`), read by difference around the run: in a process of
/// the test's own (`test_support::alone`), since another test's run plans probes it left before the run began, and in
/// the shared process `planned` grew past `left` within the run's window.
#[test]
fn a_probe_is_planned_when_its_delta_first_has_rows() {
    crate::code_rule_dsl::test_support::alone("code_rule_dsl::logic::plan::tests::a_probe_is_planned_when_its_delta_first_has_rows_child");
}

#[test]
#[ignore = "child process of a_probe_is_planned_when_its_delta_first_has_rows"]
fn a_probe_is_planned_when_its_delta_first_has_rows_child() {
    let src = ".decl e(a: number, b: number)\ne(1, 2).\ne(2, 3).\ne(3, 4).\ne(4, 1).\n\
               .decl g(a: number)\ng(a) :- e(a, a).\n.decl h(a: number)\nh(3).\n\
               .decl r(a: number, b: number)\n.decl s(a: number, b: number)\n.decl t(a: number, b: number)\n\
               r(a, b) :- e(a, b).\nr(a, c) :- r(a, b), e(b, c).\nr(a, c) :- s(a, b), e(b, c).\nr(a, b) :- t(b, a).\n\
               s(a, b) :- r(a, b), g(a).\nt(a, b) :- r(a, b), h(b).\n\
               .decl o(k: number, a: number, b: number)\no(1, a, b) :- r(a, b).\no(2, a, b) :- t(a, b).\n.output o\n";
    let p = parse_version(src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        // The probes' counts are the run's (decision 0442): one run holds them.
        let _run = super::super::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let _plans = KeepPlans::start(None);
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::lazy_probes_counted();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes).unwrap();
        let after = super::lazy_probes_counted();
        if !modes.is_empty() {
            assert!(got.native.iter().any(|c| c.split('/').any(|r| r == "r")), "r's component runs natively: {:?}", got.native);
            // The probes searched: r by r, r by s, s by r, t by r (`r(a, b) :- t(b, a)` is a copy, with no order to
            // find). The process is the test's own, so the counts moved by this run's alone.
            let (left, planned) = (after.0 - before.0, after.1 - before.1);
            assert!(left >= 4 && planned >= 3, "left {left}, planned {planned}; native {:?}, inlined {:?}", got.native, got.inlined);
            assert!(left - planned >= 1, "r's probe by s is never planned: left {left}, planned {planned}");
        }
        let mut o = got.shown()["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    // r: the 4-cycle's closure (16) and t's reversals, all in it; t: r's rows into 3 (4).
    assert_eq!(outputs[1].len(), 16 + 4, "{:?}", outputs[1]);
}

fn root() -> std::path::PathBuf {
    crate::source_tree::native().join("src/code_rule_dsl/logic")
}

/// T's expected rows of `rel` (its golden `<rel>.csv`, a fixture T wrote): its lines, as T shows its rows. A fixture
/// read to compare against, never a run's outputs (those are read through `output::Outputs`).
/// Rows of shown values as T's goldens show rows: tab-separated (a rendering to compare against them, never read).
fn golden_form(rows: &[Vec<String>]) -> Vec<String> {
    rows.iter().map(|r| r.join("\t")).collect()
}

fn golden_rows(golden: &std::path::Path, rel: &str) -> Vec<String> {
    let file = golden.join(format!("{rel}.csv"));
    std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display())).lines().map(str::to_owned).collect()
}

/// The facts T's exporter and `stats.py` built for nine CPython 3.13 modules (`json/`, `ftplib`, `shlex`,
/// `fnmatch`, `glob`), loaded from their compressed dump into a new file.
fn facts() -> std::path::PathBuf {
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    let path = crate::code_rule_dsl::fixtures::temp();
    Connection::open(&path).unwrap().execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    path
}

/// T's program `name` through T's `rdl.py sql` front (`front_of`).
fn program(name: &str) -> Arc<Version> {
    let src = std::fs::read_to_string(root().join(format!("plan/golden/programs/{name}.rerail"))).unwrap();
    front_of(parse_version(&src).unwrap())
}

/// The relation of literal `at` of `p`'s rule `k`, if it is an atom.
fn rel_at(p: &Version, k: usize, at: usize) -> Option<String> {
    let b = body::nth(p, k);
    b.atom(at).map(|a| b.rel(a).to_owned())
}

/// Literal `at` of `p`'s rule `k`, written out.
fn lit_at(p: &Version, k: usize, at: usize) -> String {
    let r = p.rules().nth(k).expect("the rule");
    p.show_lit(r, r.body().nth(at).expect("the literal"))
}

/// `p` with every rule's head renamed `to`, as maintenance names a delta's form of a rule.
fn renamed_head(p: &Version, to: &str) -> Arc<Version> {
    struct Head<'s>(&'s str);
    impl crate::code_rule_dsl::logic::program_image::Rewrite for Head<'_> {
        fn head_as(&mut self, _a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>) -> Option<String> {
            Some(self.0.to_owned())
        }
    }
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::derive(p);
    for r in p.rules() {
        b.rewrite(p, r, &mut Head(to)).finish().unwrap();
    }
    b.finish().unwrap().0
}

/// T's `rdl.py sql` front over `v`: prune, demand (`.demand`'s magic sets, or the adornment of every derived relation
/// for `.demand *`), prune.
fn front_of(v: Arc<Version>) -> Arc<Version> {
    let every = ProgramFields::demand(&*v).iter().any(|d| d == "*");
    let mut p = Pipeline::new("T's front", v);
    p.program(&mut Prune::default());
    let (mut keys, mut adorned) = (KeyDemand::default(), ProgramDemand { every: true, ..ProgramDemand::default() });
    if every {
        p.program(&mut adorned);
    } else {
        p.program(&mut keys);
    }
    p.program(&mut Prune::default());
    let v = p.finish().unwrap().0;
    if let Some(Err(e)) = adorned.outcome {
        panic!("{e}");
    }
    v
}

/// `p` lowered for `dialect` with `nearest` read from the transducer (`preparation::Nearest`), as the corpus run
/// reads it: the program, and the relations the run makes, each with its set's kind atoms.
fn lowered_with_nearest(p: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>) -> (lower::Lowered, Vec<(String, Vec<i64>)>) {
    let (v, mut domains, ops) = lower::lower_version(p, dialect, intern).unwrap();
    let mut pipeline = Pipeline::new("nearest", v);
    let mut nearest = crate::code_rule_dsl::logic::plan::corpus::preparation::Nearest::default();
    pipeline.rules(&mut nearest);
    let v = pipeline.finish().unwrap().0;
    for (rel, _) in &nearest.sets {
        domains.insert(rel.clone(), vec![lower::Domain::Node, lower::Domain::Node]);
    }
    (lower::Lowered::of_placed(body::Placed::whole(v), domains, ops, dialect), nearest.sets)
}

fn chosen_line(c: &Chosen) -> String {
    let walks: Vec<String> =
        c.walks.iter().map(|(l, m)| format!("[{}, {}]", serde_json::to_string(l).unwrap(), serde_json::to_string(m).unwrap())).collect();
    format!("{}\t{}\t[{}]\t{}", c.head, c.anchor, walks.join(", "), c.work)
}

/// On T's facts, the port chooses T's plans (anchor, walks, estimated work, rule for rule) and derives T's outputs.
#[test]
fn t_programs_plan_and_run_like_t() {
    if !t_numbering("t_programs_plan_and_run_like_t") {
        return;
    }
    let facts = facts();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(program(name), &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lines: Vec<String> = got.chosen.iter().map(chosen_line).collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        for i in (0..lines.len().max(expected.len())).filter(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        let mut files: Vec<String> = std::fs::read_dir(&golden)
            .unwrap()
            .filter_map(|e| e.unwrap().file_name().to_str()?.strip_suffix(".csv").map(str::to_owned))
            .collect();
        files.sort();
        assert_eq!(got.shown().keys().cloned().collect::<Vec<_>>(), files, "{name}");
        assert!(!lines.is_empty() && got.shown().values().any(|r| !r.is_empty()), "{name}");
        for (rel, rows) in &got.shown() {
            let expected = golden_rows(&golden, rel);
            let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
            if golden_form(rows) != expected {
                failures.push(format!("{name} {rel}: {} rows, expected {}", rows.len(), expected.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// T's native modes (`RDL_NATIVE`) on T's extension derive T's outputs: each component a mode runs natively gives
/// the rows T's pure SQL mode gives (the goldens). T runs one program per process, and its extension keeps
/// process-wide state (the loaded tape, the native inputs cached by their SQL, the symbols), so each case runs in a
/// child process of this test, as T's `rdl.py` does.
#[test]
fn t_programs_run_natively_like_t() {
    if !t_numbering("t_programs_run_natively_like_t") {
        return;
    }
    const CASE: &str = "RERAIL_RDL_NATIVE_CASE";
    if let Ok(case) = std::env::var(CASE) {
        let (modes, name) = case.split_once(' ').expect("modes and program");
        let load = |db: &Connection| {
            let rc = unsafe {
                crate::code_rule_dsl::logic::native::extension::sqlite3_extension_init(db.handle(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
            Ok(())
        };
        let got = run_sql_native(program(name), &facts(), modes, load).unwrap_or_else(|e| panic!("{case}: {e}"));
        let golden = root().join(format!("plan/golden/{name}"));
        for (rel, rows) in &got.shown() {
            let expected = golden_rows(&golden, rel);
            let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
            assert_eq!(golden_form(rows), expected, "{case} {rel}");
        }
        println!("NATIVE {}", got.native.iter().cloned().collect::<Vec<_>>().join(" "));
        return;
    }
    let mut failures = Vec::new();
    let mut ran = BTreeSet::new();
    let exe = std::env::current_exe().unwrap();
    let cases = ["nearest", "fixpoint", "fixpoint,kseed", "fixpoint,vtab", "fixpoint,direct", "upsert", "setfix", "setfix,join,nearest", "all", "program"]
        .into_iter()
        .flat_map(|modes| ["workload", "workload_demand", "calls"].map(move |name| (modes, name)))
        .collect::<Vec<_>>();
    // Six children at a time: the cases are independent, and six fit the memory budget.
    for chunk in cases.chunks(6) {
        let children = chunk
            .iter()
            .map(|&(modes, name)| {
                let child = std::process::Command::new(&exe)
                    .args(["--exact", "code_rule_dsl::logic::plan::tests::t_programs_run_natively_like_t", "--nocapture", "--test-threads=1"])
                    .env(CASE, format!("{modes} {name}"))
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                (modes, name, child)
            })
            .collect::<Vec<_>>();
        for (modes, name, child) in children {
            let out = child.wait_with_output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !out.status.success() {
                failures.push(format!("{modes} {name}: {}", String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.contains("panicked") || l.contains(name)).collect::<Vec<_>>().join(" | ")));
            }
            // The harness prints the test's name first, on the same line.
            for line in stdout.lines().filter_map(|l| l.split_once("NATIVE ").map(|(_, c)| c)) {
                ran.extend(line.split_whitespace().map(|c| format!("{modes} {name} {c}")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for mode in ["fixpoint", "setfix", "program"] {
        assert!(ran.iter().any(|c| c.starts_with(mode)), "no component ran natively under {mode}: {ran:?}");
    }
}

/// T's statistics atomized (`corpus::atomized_statistics`), for the fixture's Python files.
fn atomized_statistics(t: &Connection, db: &Connection, store: &ContentStore, first: &HashMap<i64, i64>) {
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    super::corpus::atomized_statistics(t, db, store, first, python).unwrap();
}

/// `got`'s outputs, node ids as T's (`first`: each document's first node in T's facts), against T's goldens.
fn outputs_match(name: &str, got: &Outcome, first: &HashMap<i64, i64>, failures: &mut Vec<String>) {
    let golden = root().join(format!("plan/golden/{name}"));
    assert!(!got.shown().is_empty() && got.shown().values().any(|r| !r.is_empty()), "{name}");
    for (rel, rows) in &got.outputs {
        let nodes = &got.nodes[rel];
        // Each row in T's numbering (a node by its document's first node in T's facts), as T's golden shows it.
        let mut rows: Vec<String> = (0..rows.len())
            .map(|i| {
                (0..rows.width())
                    .map(|c| match rows.cell(i, c) {
                        output::Cell::Int(v) if nodes.contains(&c) && v != output::NULL => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                        output::Cell::Int(output::NULL) => "None".to_owned(),
                        output::Cell::Int(v) => v.to_string(),
                        output::Cell::Text(t) => t.to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect();
        rows.sort();
        rows.dedup();
        let expected = golden_rows(&golden, rel);
        let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
        if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
            let i = (0..rows.len().max(expected.len())).find(|&i| rows.get(i).map(String::as_str) != expected.get(i).copied());
            failures.push(format!(
                "{name} {rel}: {} rows, expected {}; first difference {:?} vs {:?}",
                rows.len(),
                expected.len(),
                i.and_then(|i| rows.get(i)),
                i.and_then(|i| expected.get(i))
            ));
        }
    }
}

/// O13: lowered to atoms and run over the base-relation vtabs of the same files (`facts::vtab::syntax`), T's programs
/// get T's plans and, decoded, T's outputs.
#[test]
fn t_programs_over_atoms_plan_and_run_like_t() {
    if !t_numbering("t_programs_over_atoms_plan_and_run_like_t") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    for (name, source) in syntax::sources(&store, syntax::fixtures::documents()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    atomized_statistics(&t, &db, &store, &first);
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let p = program(name);
        let lowered = lower::lowered(p.clone(), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let got = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{name}: {e}"));
        // The plans in the unlowered program's terms, as T printed them.
        let lines: Vec<String> = got
            .chosen
            .iter()
            .map(|c| {
                let rule = p.rules().nth(c.rule).unwrap();
                let body: Vec<_> = rule.body().collect();
                chosen_line(&Chosen {
                    head: p.show_atom(rule, rule.head()),
                    anchor: p.show_lit(rule, body[c.steps[0].0]),
                    walks: c.steps.iter().filter_map(|&(at, m)| m.map(|m| (p.show_lit(rule, body[at]), m.name()))).collect(),
                    ..c.clone()
                })
            })
            .collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        for i in (0..lines.len().max(expected.len())).filter(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        outputs_match(name, &got, &first, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O14: the planner's programs as wave programs (`emit`) run through the frontier loop
/// (`waves::schedule::run`) give the rows `run_atoms` gives with its own stratum loop, planned from
/// the estimates and again from the counts the first run's boundary recorded.
#[test]
fn t_programs_as_wave_programs_give_run_atoms_outputs() {
    if !t_numbering("t_programs_as_wave_programs_give_run_atoms_outputs") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::waves::{owner::Backing, schedule};
    use crate::content_store::Binding;
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let docs = syntax::fixtures::documents();
    store.bind_names(&docs.iter().map(|d| Binding { name: d.core.clone(), digest: d.digest }).collect::<Vec<_>>()).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    for (name, source) in syntax::sources(&store, docs.clone()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    atomized_statistics(&t, &db, &store, &first);
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let lowered = lower::lowered(program(name), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let expected = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{name}: {e}"));
        // Planned from the estimates, then from the counts the first run's boundary recorded.
        let mut counts = None;
        let mut first: Option<Vec<String>> = None;
        for pass in ["estimates", "counts"] {
            let emitted = emit::emit(&lowered, &db, "s", name, counts.as_ref(), &HashMap::new()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let backing = Backing::default();
            let scope = backing.enter(&store);
            let lease = scope.pool().take().unwrap();
            super::super::native::register_symbols(&lease).unwrap();
            // The steps read the documents' node intervals from the statistics schema, as `run_atoms`.
            lease.execute_batch("ATTACH ':memory:' AS s; CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER)").unwrap();
            let ranges: Vec<(i64, i64, i64)> = db
                .prepare("SELECT d, lo, hi FROM s.stat_docrange")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            for range in ranges {
                lease.execute("INSERT INTO s.stat_docrange VALUES (?1, ?2, ?3)", [range.0, range.1, range.2]).unwrap();
            }
            for statement in &emitted.setup {
                lease.execute_batch(statement).unwrap();
            }
            let outcome = schedule::batches::run(&lease, &store, &emitted.program, "/w", docs.clone(), &schedule::Limits::default(), &mut |_| Ok(None))
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (got, _) = emit::outputs(&lowered, &lease).unwrap();
            let sql: Vec<String> = emitted.program.steps.iter().map(|s| s.sql.clone()).collect();
            let replanned = first.as_ref().map_or(0, |f| f.iter().zip(&sql).filter(|(a, b)| a != b).count());
            eprintln!(
                "O14 {name} ({pass}): {} steps ({replanned} replanned) in {} waves; {} outputs",
                sql.len(),
                outcome.waves,
                got.len()
            );
            for (rel, rows) in &expected.shown() {
                if got.get(rel) != Some(rows) {
                    failures.push(format!("{name} {pass} {rel}: {} rows, run_atoms {}", got.get(rel).map_or(0, Vec::len), rows.len()));
                }
            }
            first.get_or_insert(sql);
            counts = Some(outcome.rows);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O12 (option a): `nearest` made by the transducer's file-local FIX (`typing::file_local`) and read as the input
/// `nearest_<set>(n, a)`, on the CPU and on the device work queue: T's programs derive T's outputs.
#[test]
#[cfg_attr(debug_assertions, ignore = "O82: aborts in the metal crate on a debug build (`slice::from_raw_parts` precondition, metal-0.33.0 lib.rs:82)")]
fn t_programs_with_the_transducers_nearest_derive_t_outputs() {
    if !t_numbering("t_programs_with_the_transducers_nearest_derive_t_outputs") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for binding in [Binding::Cpu, Binding::Metal] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        let modules = Modules::default();
        atomized_statistics(&t, &db, &store, &first);
        for name in ["workload", "workload_demand", "calls"] {
            let (lowered, sets) = lowered_with_nearest(program(name), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())));
            assert!(!sets.is_empty(), "{name} reads nearest");
            let placed = lowered.placed().unwrap();
            assert!(!placed.v.rules().flat_map(|r| r.body()).any(|l| matches!(l.view(), crate::code_rule_dsl::logic::program_image::Lit::Nearest { .. })));
            for (rel, source) in syntax::with_nearest(&store, syntax::fixtures::documents(), &sets, &Default::default(), binding, &Default::default()).unwrap() {
                if rel.starts_with(crate::code_rule_dsl::logic::front::NEAREST) {
                    db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64]).unwrap();
                    for (col, rows, ndv, sq, sqd) in syntax::statistics(source.as_ref()) {
                        db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd]).unwrap();
                    }
                }
                modules.bind(&db, &rel, source).unwrap();
            }
            let got = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{binding:?} {name}: {e}"));
            let before = failures.len();
            outputs_match(name, &got, &first, &mut failures);
            for failure in &mut failures[before..] {
                *failure = format!("{binding:?} {failure}");
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O13: T's native modes over atoms. Lowered with the transducer's `nearest` and run over the base-relation vtabs,
/// each component a mode runs in `logic::native` gives the rows T's pure SQL mode gives (the goldens).
#[test]
fn t_programs_run_natively_over_atoms_like_t() {
    if !t_numbering("t_programs_run_natively_over_atoms_like_t") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    atomized_statistics(&t, &db, &store, &first);
    let mut failures = Vec::new();
    let mut ran = BTreeSet::new();
    for name in ["workload", "workload_demand", "calls"] {
        let (lowered, sets) = lowered_with_nearest(program(name), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())));
        for (rel, source) in syntax::with_nearest(&store, syntax::fixtures::documents(), &sets, &Default::default(), Binding::Cpu, &Default::default()).unwrap() {
            if rel.starts_with(crate::code_rule_dsl::logic::front::NEAREST) {
                db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64]).unwrap();
                for (col, rows, ndv, sq, sqd) in syntax::statistics(source.as_ref()) {
                    db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd]).unwrap();
                }
            }
            modules.bind(&db, &rel, source).unwrap();
        }
        for modes in ["fixpoint", "fixpoint,vtab", "fixpoint,direct", "upsert", "setfix", "setfix,join", "all", "program"] {
            match run_atoms_native(&lowered, &db, "s", modes) {
                Ok(got) => {
                    let before = failures.len();
                    outputs_match(name, &got, &first, &mut failures);
                    for failure in &mut failures[before..] {
                        *failure = format!("{modes} {failure}");
                    }
                    ran.extend(got.native.iter().map(|c| format!("{modes} {name} {c}")));
                }
                Err(e) => failures.push(format!("{modes} {name}: {e}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for mode in ["fixpoint", "setfix", "program"] {
        assert!(ran.iter().any(|c| c.starts_with(mode)), "no component ran natively under {mode}: {ran:?}");
    }
}

/// The product's programs (`typing/programs/`, mutated from T's: no hand-written demand, list step 29) derive
/// T's outputs on T's facts, through the same front (prune, magic sets, prune). Their plans may differ from
/// T's; their outputs may not.
#[test]
#[ignore = "O95: T's facts predate relations calls.rerail now reads (`value`, `str`, …); the facts need them, empty or rebuilt"]
fn product_programs_derive_t_outputs() {
    let facts = facts();
    // T's facts predate `strprefix` (which calls.rerail's type relations read, decision 0047): empty here, so every
    // string is str. Likewise the guests (`parsed`, `host_node`, decision 0055): no string is parsed. No T output
    // reads them. Nor do they have `name`, `str`, `value` and `op_at` (core.rerail), which calls.rerail reads through
    // python/conditions.rerail (Typing's B6, B10): empty here, so no `sys.version_info` or `sys.platform` test is
    // decided and every branch is taken, as T assumed.
    Connection::open(&facts).unwrap().execute_batch(
        "CREATE TABLE strprefix(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.strprefix', 0);
         INSERT INTO stat_col VALUES ('strprefix', 'n', 0, 0, 0, 0), ('strprefix', 's', 0, 0, 0, 0);
         CREATE TABLE parsed(s INTEGER, grammar TEXT, root INTEGER); INSERT INTO stat_meta VALUES ('rows.parsed', 0);
         INSERT INTO stat_col VALUES ('parsed', 's', 0, 0, 0, 0), ('parsed', 'grammar', 0, 0, 0, 0), ('parsed', 'root', 0, 0, 0, 0);
         CREATE TABLE host_node(n INTEGER, h INTEGER); INSERT INTO stat_meta VALUES ('rows.host_node', 0);
         INSERT INTO stat_col VALUES ('host_node', 'n', 0, 0, 0, 0), ('host_node', 'h', 0, 0, 0, 0);
         CREATE TABLE value(n INTEGER, v TEXT); INSERT INTO stat_meta VALUES ('rows.value', 0);
         INSERT INTO stat_col VALUES ('value', 'n', 0, 0, 0, 0), ('value', 'v', 0, 0, 0, 0);
         CREATE TABLE str(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.str', 0);
         INSERT INTO stat_col VALUES ('str', 'n', 0, 0, 0, 0), ('str', 's', 0, 0, 0, 0);
         CREATE TABLE name(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.name', 0);
         INSERT INTO stat_col VALUES ('name', 'n', 0, 0, 0, 0), ('name', 's', 0, 0, 0, 0);
         CREATE TABLE op_at(n INTEGER, i INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.op_at', 0);
         INSERT INTO stat_col VALUES ('op_at', 'n', 0, 0, 0, 0), ('op_at', 'i', 0, 0, 0, 0), ('op_at', 's', 0, 0, 0, 0)",
    ).unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let src = crate::code_rule_dsl::logic::settings::program_text(&root().join(format!("../typing/programs/{name}.rerail"))).unwrap();
        let p = parse_version(&src).unwrap();
        // T has no library stage (decision 0059): with no `stage` fact, the rules it opens are dead, and their
        // unbound reads of demanded relations (`need_attr(c, a) :- stage("library"), found(c, a, _)`) are not asked.
        let p = {
            let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&p);
            b.retain(|r| !r.body().any(|l| matches!(l.view(), crate::code_rule_dsl::logic::program_image::Lit::Atom(a) if !a.neg() && p.rel_name(a.rel()) == "stage")));
            b.finish().unwrap().0
        };
        let p = front_of(p);
        // `found`'s demand is the magic-set relation, not a hand-written one.
        if name == "calls" {
            assert!(p.declared_names().iter().any(|rel| rel.starts_with("magic_found")), "calls: no magic relation for `found`");
        }
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(p, &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The product's own outputs (calls.rerail's type relations, decision 0047) are not T's: they have no golden.
        for (rel, rows) in got.shown().iter().filter(|(rel, _)| golden.join(format!("{rel}.csv")).exists()) {
            let expected = golden_rows(&golden, rel);
            let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
            let rows = golden_form(rows);
            if rows != expected {
                let got: BTreeSet<&str> = rows.iter().map(String::as_str).collect();
                let want: BTreeSet<&str> = expected.iter().copied().collect();
                let (missing, extra): (Vec<_>, Vec<_>) = (want.difference(&got).take(3).collect(), got.difference(&want).take(3).collect());
                failures.push(format!("{name} {rel}: {} rows, expected {}; missing {missing:?}, extra {extra:?}", rows.len(), expected.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The typing programs' own ingestion (list step 29: no T facts file, every chunk's statistics measured as it is
/// bound) derives what the run on T's facts derives, on T's fixture corpus, as one chunk and as several. Node ids
/// differ only in how they are written (`path:index` against T's global ids), so the own outputs are mapped to T's.
#[test]
fn own_ingestion_derives_what_t_facts_do() {
    if !t_numbering("own_ingestion_derives_what_t_facts_do") {
        return;
    }
    use crate::code_rule_dsl::typing::file_local::Binding;
    let raw = zstd::decode_all(std::fs::File::open(root().join("../facts/vtab/golden/corpus.json.zst")).unwrap()).unwrap();
    let corpus: Vec<(String, String)> = serde_json::from_slice(&raw).unwrap();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("corpus");
    for (path, source) in &corpus {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    let facts = facts();
    let t = Connection::open(&facts).unwrap();
    let first: HashMap<String, i64> =
        t.prepare("SELECT path, min(n) FROM doc JOIN node USING (d) GROUP BY d").unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap().map(|row| row.unwrap()).collect();
    // An own run's row in T's numbering: each node of a document T numbers, T's id for it; any other value as shown.
    let to_t = |row: output::Row<'_>| -> Vec<String> {
        row.values()
            .map(|v| match v.node().and_then(|n| n.at()) {
                Some((path, local)) if first.contains_key(path) => (first[path] + local).to_string(),
                _ => v.to_string(),
            })
            .collect()
    };
    let read = |out: &std::path::Path| output::Outputs::open(out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    for name in ["workload", "calls"] {
        let src = crate::code_rule_dsl::logic::settings::program_text(&root().join(format!("../typing/programs/{name}.rerail"))).unwrap();
        let with_t = dir.join(format!("out-{name}-t"));
        let report = corpus::run(&src, &dir, Some(&facts), &with_t, "program", Binding::Cpu, 1 << 18).unwrap();
        assert!(report.rows.values().any(|&n| n > 0), "{name}: no outputs");
        for chunk in [1_usize << 18, 1 << 12] {
            let own = dir.join(format!("out-{name}-own-{chunk}"));
            let got = corpus::run(&src, &dir, None, &own, "program", Binding::Cpu, chunk).unwrap();
            assert_eq!(got.files, corpus.len(), "{name}");
            for rel in report.rows.keys() {
                let mut mapped: Vec<Vec<String>> = read(&own).rows(rel).into_iter().map(to_t).collect();
                mapped.sort();
                mapped.dedup();
                let expected = output::shown(&with_t, rel).unwrap_or_else(|e| panic!("{}: {rel}: {e}", with_t.display()));
                assert_eq!(mapped.len(), expected.len(), "{name} {rel} at {chunk}-node chunks: rows");
                assert!(mapped == expected, "{name} {rel} at {chunk}-node chunks: rows differ");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// O42: the corpus runner's typing inputs and outputs, end to end. A site package is summarized into the cache
/// (a declaration that differs from what it derives is reported, not used); an application importing it reads the
/// summaries and a user declaration as `declared_ret` facts, which type its calls, and each typed node gets a
/// union term; with an empty cache the package's modules are pending, not ⊤, and the calls stay untyped.
#[test]
fn typed_corpus_reads_declarations_and_dependency_summaries() {
    use super::corpus::{Typing, run_typed};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("typed");
    let write = |p: &str, text: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("site/vendor/__init__.py", "from .core import make\n\ndef version():\n    return \"1.0\"\n");
    write("site/vendor/core.py", "def make():\n    return 1\n");
    write("site/vendor-1.0.dist-info/RECORD", "vendor/__init__.py,sha256=AAA,60\nvendor/core.py,sha256=BBB,30\nvendor-1.0.dist-info/RECORD,,\n");
    write("decl/make.pyi", "# rerail.types/v1\n# module: vendor.core\ndef make() -> str: ...\n");
    write("app/main.py", "from vendor import version\nfrom vendor.core import make\nimport other\na = version()\nb = make()\nc = other.thing()\n");
    write("decl/other.pyi", "# rerail.types/v1\n# module: other\ndef thing() -> bytes | None: ...\n");
    let src = std::fs::read_to_string(root().join("../typing/programs/workload.rerail")).unwrap();
    let cache = dir.join("cache");

    let summarize = Typing { summarize: Some(cache.clone()), declarations: vec![dir.join("decl/make.pyi")], ..Default::default() };
    let r = run_typed(&[&src], &dir.join("site"), None, &dir.join("out-site"), "program", Binding::Cpu, 1 << 18, &summarize).unwrap();
    assert_eq!(r.typing.summaries, 2, "{:?}", r.typing);
    assert_eq!(r.typing.conflicts, vec![("vendor.core.make".to_owned(), "int".to_owned(), "str".to_owned())]);

    let app = Typing { declarations: vec![dir.join("decl/other.pyi")], site: Some((dir.join("site"), cache.clone())), ..Default::default() };
    let r = run_typed(&[&src], &dir.join("app"), None, &dir.join("out-app"), "program", Binding::Cpu, 1 << 18, &app).unwrap();
    println!("{:?}", r.typing);
    assert!(r.typing.pending.is_empty(), "{:?}", r.typing);
    assert_eq!(r.typing.declared, 4, "version: str, make: int, thing: bytes | None");
    let types: BTreeSet<String> = crate::code_rule_dsl::logic::plan::output::shown(&dir.join("out-app"), "type_of").unwrap().into_iter().map(|r| r.last().unwrap().clone()).collect();
    assert_eq!(types, BTreeSet::from(["str", "int", "bytes", "None"].map(String::from)));
    // One term per typed node (the three calls and the three names they are assigned to): other.thing()'s, and
    // `c`'s through it, are the closed union bytes | None.
    let x = rusqlite::Connection::open(dir.join("out-app/cross.sqlite")).unwrap();
    let nodes: i64 = x.query_row("SELECT count(*) FROM node_type", [], |r| r.get(0)).unwrap();
    assert_eq!(nodes as usize, r.typing.typed_nodes);
    let unions: Vec<i64> = x.prepare("SELECT t.arity FROM node_type n JOIN type_term t USING (tid) WHERE t.ctor = 1").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(r.typing.typed_nodes, 6);
    assert_eq!(unions, vec![2, 2]);

    let cold = Typing { site: Some((dir.join("site"), dir.join("empty-cache"))), ..Default::default() };
    let r = run_typed(&[&src], &dir.join("app"), None, &dir.join("out-cold"), "program", Binding::Cpu, 1 << 18, &cold).unwrap();
    assert_eq!(r.typing.pending, vec!["vendor".to_owned(), "vendor.core".to_owned()]);
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&dir.join("out-cold"), "type_of").unwrap(), Vec::<Vec<String>>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

const CORPUS_STORE_CHILD: &str = "RERAIL_CORPUS_STORE_CHILD";

/// The child of `a_second_corpus_run_deletes_and_creates_its_store_again`, run without the tape cache: each run's
/// store is its own `store.sqlite`, which the next run deletes and creates again. Once a run is over nothing in the
/// process holds the store open, and the second run's symbols are its own store's.
#[test]
#[ignore = "a child process of the corpus store test"]
fn corpus_store_child() {
    let Some(dir) = std::env::var_os(CORPUS_STORE_CHILD).map(std::path::PathBuf::from) else { return };
    assert_eq!(std::env::var("RERAIL_TAPE_CACHE").as_deref(), Ok("off"));
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl name(s: symbol)
name(s) :- node(_, n, "identifier", _, _), text(n, s).
.output name
"#;
    let out = dir.join("out");
    let store = out.join("store.sqlite");
    for (i, [a, b]) in [["alpha", "beta"], ["gamma", "delta"]].into_iter().enumerate() {
        let root = dir.join(format!("corpus-{i}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), format!("{a} = {b}\n")).unwrap();
        super::corpus::run(program, &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut names = [a, b];
        names.sort_unstable();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "name").unwrap(), [[names[0]], [names[1]]]);
        // The run's connections close beside it (`close_beside`): once they are closed, nothing holds the store.
        super::corpus::closes_done();
        assert_eq!(crate::content_store::holding(&store), (0, 0), "run {i} left store handles or connections open (handles, connections)");
        // The kernel names files by their real path (macOS's /var is /private/var).
        let real = dunce::canonicalize(&out).unwrap().join(store.file_name().unwrap());
        let held: Vec<_> = open_files().into_iter().filter(|file| file.to_string_lossy().starts_with(&*real.to_string_lossy())).collect();
        assert!(held.is_empty(), "run {i} left its store open: {held:?}");
    }
    std::fs::write(dir.join("done"), "").unwrap();
}

/// The files this process holds open.
fn open_files() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/fd").unwrap().filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok()).collect()
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        // SAFETY: getdtablesize reads the limit; F_GETPATH writes at most PATH_MAX bytes into the buffer.
        let fds = unsafe { libc::getdtablesize() };
        (0..fds)
            .filter_map(|fd| {
                let mut buf = [0u8; libc::PATH_MAX as usize];
                (unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) } == 0).then(|| {
                    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..len]))
                })
            })
            .collect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// A process runs the corpus twice over its own store (`corpus_store_child`).
#[test]
fn a_second_corpus_run_deletes_and_creates_its_store_again() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("restore");
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--ignored", "--exact", "code_rule_dsl::logic::plan::tests::corpus_store_child", "--nocapture", "--test-threads=1"])
        .env(CORPUS_STORE_CHILD, &dir)
        .env("RERAIL_TAPE_CACHE", "off");
    let status = super::corpus::switches::started(|| child.output()).unwrap();
    // The child's own output, shown when it fails: a failure beside other tests says nothing without it (Q42).
    assert!(
        status.status.success() && dir.join("done").exists(),
        "the child failed: {}\n--- stdout\n{}\n--- stderr\n{}",
        status.status,
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Step 30's primitives through the corpus runner: `line(n, l)`, and literal tests on names and paths (`glob`,
/// `prefix`, `suffix`) in place of regexes.
#[test]
fn rules_test_names_paths_and_lines_without_regexes() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("strings");
    for (p, text) in [
        ("impl/sqlite/fts/manager.py", "x = 1\n_hidden = 2\nfetch_async = 3\n"),
        ("impl/base.py", "_also = 1\n"),
        ("lib/other.py", "_outside = 1\n"),
    ] {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.input doc, node, text, line
.decl private_in_impl(p: symbol, s: symbol, l: number)
private_in_impl(p, s, l) :- doc(d, p, _), glob("impl/**/*.py", p), node(d, n, "identifier", _, _), text(n, s),
    prefix("_", s), line(n, l).
.decl async_in_fts(s: symbol)
async_in_fts(s) :- doc(d, p, _), glob("impl/*/fts/**", p), node(d, n, "identifier", _, _), text(n, s), suffix("_async", s).
.output private_in_impl, async_in_fts
"#;
    let out = dir.join("out");
    let r = super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(r.files, 3);
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("private_in_impl"), [["impl/base.py", "_also", "1"], ["impl/sqlite/fts/manager.py", "_hidden", "2"]]);
    assert_eq!(read("async_in_fts"), [["fetch_async"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The tree axes (decision 0036): `inside(n, a, set)` holds every ancestor of `n` whose kind is in the set, not
/// only the nearest; `has(a, n, set)` every descendant of `a` whose kind is in the set (`core.rerail`: the set tests the
/// descendant); both negate.
#[test]
fn rules_read_inside_and_has_as_tape_relations() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("axes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def outer():\n    def inner():\n        g()\n    f()\nh()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl fn(kind: symbol)
fn("function_definition").
.decl call_k(kind: symbol)
call_k("call").
.decl callee(c: node, s: symbol)
callee(c, s) :- node(_, c, "call", _, _), field(c, "function", x), text(x, s).
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
// Every function a call is inside.
.decl call_in(c: symbol, f: symbol)
call_in(c, f) :- callee(n, c), inside(n, a, fn), fname(a, f).
// Functions that have a call to g somewhere below them.
.decl calls_g(f: symbol)
calls_g(f) :- fname(a, f), has(a, n, call_k), callee(n, "g").
// Functions with a function inside them, and without.
.decl nests(f: symbol)
nests(f) :- fname(a, f), has(a, _, fn).
.decl flat(f: symbol)
flat(f) :- fname(a, f), !has(a, _, fn).
// Calls inside no function.
.decl top(c: symbol)
top(c) :- callee(n, c), !inside(n, _, fn).
.output call_in, calls_g, top, nests, flat
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("call_in"), [["f", "outer"], ["g", "inner"], ["g", "outer"]]);
    assert_eq!(read("calls_g"), [["inner"], ["outer"]]);
    assert_eq!(read("top"), [["h"]]);
    assert_eq!(read("nests"), [["outer"]]);
    assert_eq!(read("flat"), [["inner"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A measurement, not a check: "calls inside a function" asked three ways over a corpus (`RERAIL_MEASURE_CORPUS`),
/// one way per run (`RERAIL_MEASURE` = `tape`, `interval` or `recursive`), each through the corpus runner. Prints the
/// rows and each phase's CPU (user + system).
/// - `tape`: `nearest` from the transducer's preorder pass (tape arithmetic);
/// - `interval`: byte-span containment against function definitions (a Datalog join, no recursion);
/// - `recursive`: an ancestor closure over `child` (Datalog recursion).
#[test]
#[ignore = "a measurement over a corpus on disk"]
fn measure_inside_three_ways() {
    let corpus = std::path::PathBuf::from(std::env::var("RERAIL_MEASURE_CORPUS").expect("RERAIL_MEASURE_CORPUS"));
    let way = std::env::var("RERAIL_MEASURE").unwrap_or_else(|_| "tape".into());
    let head = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.input node
.decl fn(kind: symbol)
fn("function_definition").
.decl hit(c: node)
.output hit
"#;
    let body = match way.as_str() {
        // No inside test: the fixed cost of the run and of its `hit` rows.
        "baseline" => "hit(c) :- node(_, c, \"call\", _, _).\n",
        "tape" => "hit(c) :- node(_, c, \"call\", _, _), nearest(c, f, fn).\n",
        "interval" => "hit(c) :- node(d, c, \"call\", s, e), node(d, f, \"function_definition\", fs, fe), fs <= s, e <= fe.\n",
        "recursive" => ".decl child(p: node, c: node, i: number)\n.input child\n.decl up(c: node, a: node)\nup(c, p) :- node(_, c, \"call\", _, _), child(p, c, _).\nup(c, a) :- up(c, p), child(a, p, _).\nhit(c) :- up(c, f), node(_, f, \"function_definition\", _, _).\n",
        other => panic!("no way {other}"),
    };
    let binding = if std::env::var("RERAIL_MEASURE_METAL").is_ok() { crate::code_rule_dsl::typing::file_local::Binding::Metal } else { crate::code_rule_dsl::typing::file_local::Binding::Cpu };
    let out = crate::code_rule_dsl::fixtures::temp().with_extension(format!("measure-{way}"));
    let r = super::corpus::run(&format!("{head}{body}"), &corpus, None, &out, "program", binding, 1 << 18).unwrap();
    let cpu: f64 = r.phases.iter().map(|p| p.1 + p.2).sum();
    println!("MEASURE {way}: {} files, {} nodes, {} chunks, hit rows {}, CPU {cpu:.2} s, peak RSS {:.0} MB", r.files, r.nodes, r.chunks, r.rows["hit"], r.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    for (name, user, sys, rss, statements) in &r.phases {
        println!("  {name:<16} {:.2} s CPU  rss {rss:.0} MB  {statements} statements", user + sys);
    }
    let _ = std::fs::remove_dir_all(&out);
}

// ---- aggregates ----

/// `join`, `sum` and aggregates inside terms, run over a corpus: join orders by its keys (numbers by value, symbols
/// by code point, a tuple key by its parts in turn), ties by the value, and gives "" of nothing; sum adds every row
/// of its body (two arguments of one length both count) and gives 0 of nothing.
#[test]
fn join_sum_and_aggregates_in_terms_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("aggregates");
    std::fs::create_dir_all(&dir).unwrap();
    // Identifiers, whose `text` is their name: `é` sorts after `z` by code point, and `B` before both.
    let m = (0..=10).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", ");
    std::fs::write(dir.join("a.py"), format!("f(bb, a, cc)\ng()\nh(é, z, B)\nm({m})\n")).unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl child(p: node, c: node, i: number)
.input node, field, text, child
.decl callee(c: node, s: symbol)
callee(c, s) :- node(_, c, "call", _, _), field(c, "function", x), text(x, s).
.decl arg(c: node, t: symbol, i: number)
arg(c, t, i) :- node(_, c, "call", _, _), field(c, "arguments", a), child(a, s, i), text(s, t).
.decl in_order(f: symbol, j: symbol)
in_order(f, j) :- callee(c, f), j = join t by i sep ", " : { arg(c, t, i) }.
.decl call_end(f: symbol, j: symbol)
call_end(f, j) :- callee(c, f), node(_, c, "call", _, e), j = to_string(e).
.decl call_end_joined(f: symbol, j: symbol)
call_end_joined(f, j) :- callee(c, f), j = join e by s sep "," : { node(_, c, "call", s, e) }.
.decl by_text(f: symbol, j: symbol)
by_text(f, j) :- callee(c, f), j = join t sep "|" : { arg(c, t, _) }.
.decl reversed(f: symbol, j: symbol)
reversed(f, j) :- callee(c, f), j = join t by k sep "" : { arg(c, t, i), k = 0 - i }.
.decl numbers(f: symbol, j: symbol)
numbers(f, j) :- callee(c, f), j = join i sep "+" : { arg(c, _, i) }.
.decl tuple(f: symbol, j: symbol)
tuple(f, j) :- callee(c, f), j = join t by (n, k) sep "," : { arg(c, t, i), n = strlen(t), k = 0 - i }.
.decl total(f: symbol, s: number, w: number, g: number)
total(f, s, w, g) :- callee(c, f), s = sum i : { arg(c, _, i) }, w = sum n : { arg(c, t, _), n = strlen(t) },
    g = sum strlen(t) - 1 : { arg(c, t, _) }.
.decl ccn(f: symbol, k: number)
ccn(f, 10 * count : { arg(c, _, _) } + sum i : { arg(c, _, i) }) :- callee(c, f).
.decl more(f: symbol)
more(f) :- callee(c, f), 1 + count : { arg(c, _, _) } > 3.
.output in_order, call_end, call_end_joined, by_text, reversed, numbers, tuple, total, ccn, more
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    let a = |order: &mut dyn Iterator<Item = i32>, sep: &str| order.map(|i| format!("a{i}")).collect::<Vec<_>>().join(sep);
    let mut text_order: Vec<String> = (0..=10).map(|i| format!("a{i}")).collect();
    text_order.sort();
    // Each call's row: its callee and the aggregate, a text.
    let calls = |m: String, [f, h]: [&str; 2]| vec![vec!["f".to_owned(), f.to_owned()], vec!["g".to_owned(), String::new()], vec!["h".to_owned(), h.to_owned()], vec!["m".to_owned(), m]];
    assert_eq!(read("in_order"), calls(a(&mut (0..=10), ", "), ["bb, a, cc", "é, z, B"]));
    // An input read without its constant column (`kind`), which comes before x and k.
    assert_eq!(read("call_end_joined"), read("call_end"));
    assert_eq!(read("by_text"), calls(text_order.join("|"), ["a|bb|cc", "B|z|é"]));
    assert_eq!(read("reversed"), calls(a(&mut (0..=10).rev(), ""), ["ccabb", "Bzé"]));
    let numbers = (0..=10).map(|i| i.to_string()).collect::<Vec<_>>().join("+");
    assert_eq!(read("numbers"), calls(numbers, ["0+1+2", "0+1+2"]));
    // By length, then by position from the end: a (1), cc (2, -2), bb (2, 0); a9 … a0 (2), a10 (3).
    assert_eq!(read("tuple"), calls(format!("{},a10", a(&mut (0..=9).rev(), ",")), ["a,cc,bb", "B,z,é"]));
    // bb and cc have one length: both count in w.
    assert_eq!(read("total"), [["f", "3", "5", "2"], ["g", "0", "0", "0"], ["h", "3", "3", "0"], ["m", "55", "23", "12"]]);
    assert_eq!(read("ccn"), [["f", "33"], ["g", "0"], ["h", "33"], ["m", "165"]]);
    assert_eq!(read("more"), [["f"], ["h"], ["m"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- functions ----

/// Runs `program` over one Python file, pure SQL (`""`) and on the native set engine (`"program"`): the outputs
/// `rels`, which both must give alike, and how many components the native run solved natively.
fn run_both(name: &str, program: &str, rels: &[&str]) -> (BTreeMap<String, Vec<Vec<String>>>, usize) {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let mut got = Vec::new();
    let mut native = 0;
    // Each mode derives from scratch: kept across runs (decision 0077 §4), the second run's strata were the first's,
    // and the native engine solved none.
    let _scratch = super::corpus::switches::set(&[("RERAIL_CROSS_K4", "off"), ("RERAIL_KEPT_PART", "off")]);
    for modes in ["", "program"] {
        let out = dir.join(format!("out-{}", if modes.is_empty() { "sql" } else { modes }));
        // Each run a store of its own: in one, the second run found the first's strata kept (decision 0117 step 3,
        // "quiet: no read changed") and the native set engine solved nothing.
        let store = super::corpus::test_store::copy();
        super::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.path().to_owned()));
        let r = super::corpus::run(program, &dir, None, &out, modes, crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{modes:?}: {e}"));
        super::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        native = r.native;
        got.push(rels.iter().map(|rel| (rel.to_string(), crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap())).collect::<BTreeMap<_, _>>());
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got[0], got[1], "SQL and native");
    (got.remove(0), native)
}

/// Soufflé's `to_number` (decimal, `0x`, `0b`, C's leading space and trailing text; no value where Soufflé stops
/// with an error), `max`, `min`, `%` (C's sign; no value for `% 0`), `cat` of any arity, and `@lower` as
/// JavaScript's `toLowerCase` (Unicode, final sigma). A function with no value gives no row, also through a binding
/// read only by a negation.
#[test]
fn souffle_functions_run() {
    let program = r#"
.decl s(t: symbol)
s("12"). s(" -7px"). s("0x1F"). s("-0b101"). s("abc"). s(""). s("99999999999999999999").
.decl num(t: symbol, n: number)
num(t, to_number(t)) :- s(t).
.decl small(n: number)
small(1). small(12).
.decl unlisted(t: symbol)
unlisted(t) :- s(t), k = to_number(t), !small(k).
.decl pair(a: number, b: number)
pair(7, 3). pair(-7, 3). pair(3, 0).
.decl ar(a: number, b: number, hi: number, lo: number)
ar(a, b, max(a, b), min(a, b)) :- pair(a, b).
.decl md(a: number, b: number, m: number)
md(a, b, m) :- pair(a, b), m = a % b.
.decl w(a: symbol, b: symbol)
w("x", "y"). w("ÀÉ İ", "ΑΣ").
.decl c(s: symbol)
c(cat(a, "-", b, "-", a)) :- w(a, b).
.decl low(s: symbol)
low(@lower(cat(a, " ", b))) :- w(a, b).
.output num, unlisted, ar, md, c, low
"#;
    let (got, _) = run_both("functions", program, &["num", "unlisted", "ar", "md", "c", "low"]);
    assert_eq!(got["num"], [[" -7px", "-7"], ["-0b101", "-5"], ["0x1F", "31"], ["12", "12"]]);
    assert_eq!(got["unlisted"], [[" -7px"], ["-0b101"], ["0x1F"]]);
    assert_eq!(got["ar"], [["-7", "3", "3", "-7"], ["3", "0", "3", "0"], ["7", "3", "7", "3"]]);
    assert_eq!(got["md"], [["-7", "3", "-1"], ["7", "3", "1"]]);
    assert_eq!(got["c"], [["x-y-x"], ["ÀÉ İ-ΑΣ-ÀÉ İ"]]);
    assert_eq!(got["low"], [["x y"], ["àé i\u{307} ας"]]);
}

/// `@dot("", b)` is `b` (`core.rerail`), built and split: a split of a bound whole gives `""` and the whole, and each
/// cut at a "." with a part before it; with a bound part, the other. Recursive (the native set engine) and not.
#[test]
fn dot_of_the_empty_name_is_the_name() {
    let program = r#"
.decl part(s: symbol)
part(""). part("a"). part("b.c").
.decl joined(m: symbol, x: symbol, q: symbol)
joined(m, x, @dot(m, x)) :- part(m), part(x).
.decl whole(q: symbol)
whole("a.b.c"). whole("x"). whole(".y").
.decl split(q: symbol, m: symbol, x: symbol)
split(q, m, x) :- whole(q), q = @dot(m, x).
.decl after(q: symbol, m: symbol, x: symbol)
after(q, m, x) :- whole(q), part(m), q = @dot(m, x).
.decl tail(x: symbol)
tail("b.c"). tail("x"). tail("y").
.decl before(q: symbol, m: symbol, x: symbol)
before(q, m, x) :- whole(q), tail(x), q = @dot(m, x).
.decl pre(q: symbol)
pre(q) :- whole(q).
pre(m) :- pre(q), q = @dot(m, x), x != "".
.output joined, split, after, before, pre
"#;
    let (got, native) = run_both("dot", program, &["joined", "split", "after", "before", "pre"]);
    assert!(native > 0, "the native set engine solved a component");
    assert_eq!(got["joined"], [["", "", ""], ["", "a", "a"], ["", "b.c", "b.c"], ["a", "", "a."], ["a", "a", "a.a"], ["a", "b.c", "a.b.c"], ["b.c", "", "b.c."], ["b.c", "a", "b.c.a"], ["b.c", "b.c", "b.c.b.c"]]);
    assert_eq!(got["split"], [[".y", "", ".y"], ["a.b.c", "", "a.b.c"], ["a.b.c", "a", "b.c"], ["a.b.c", "a.b", "c"], ["x", "", "x"]]);
    assert_eq!(got["after"], [[".y", "", ".y"], ["a.b.c", "", "a.b.c"], ["a.b.c", "a", "b.c"], ["x", "", "x"]]);
    assert_eq!(got["before"], [["a.b.c", "a", "b.c"], ["x", "", "x"]]);
    assert_eq!(got["pre"], [[""], [".y"], ["a"], ["a.b"], ["a.b.c"], ["x"]]);
}

/// `dfs_first`, `dfs_first_memo` and `match_all` over relations a native stratum made (held, not tables), on
/// both paths: the native engine makes their rows from the held relations, the SQL path from tables.
#[test]
fn native_operators_over_held_relations_match_the_sql_path() {
    let program = r#"
.decl e0(a: number, b: number, o: number)
e0(10, 20, 1). e0(20, 30, 5). e0(10, 40, 0). e0(40, 10, 2). e0(50, 60, 0).
.decl e(a: number, b: number, o: number)
e(a, b, o) :- e0(a, b, o), a > 0.
.decl t(x: number)
t(x) :- e0(_, x, _), x = 30.
.decl roots(r: number, k: symbol)
roots(r, k) :- e0(r, _, _), (r = 40, k = "a" ; r = 10, k = "b" ; r = 50, k = "c").
.decl hit(r: number, h: number, f: number)
hit(r, h, f) :- dfs_first(r, h, f, e, t).
.decl hm(r: number, h: number, f: number)
hm(r, h, f) :- dfs_first_memo(r, h, f, e, t, roots).
.decl s0(s: symbol)
s0("é xx xx"). s0("x"). s0("none").
.decl s(s: symbol)
s(x) :- s0(x), x != "".
.decl found(s: symbol, i: number, a: number, b: number, c: symbol)
found(x, i, a, b, c) :- s(x), match_all("(?<c>x+)", x, i, a, b).
.output hit, hm, found
"#;
    let (got, native) = run_both("held_operators", program, &["hit", "hm", "found"]);
    assert!(native > 0, "the native set engine solved a component");
    // And natively: no stratum of theirs fell back to SQL (strict runs refuse one inside the waves).
    let declined = {
        use crate::code_rule_dsl::logic::profile;
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("held_operators_native");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
        let _scratch = super::corpus::switches::set(&[("RERAIL_CROSS_K4", "off"), ("RERAIL_KEPT_PART", "off")]);
        // The run resets the profile as it starts; the test's run keeps it after (decision 0442: the run's records).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        let ran = super::corpus::run(program, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
        let declined = profile::declined();
        profile::set(false);
        let _ = std::fs::remove_dir_all(&dir);
        ran.unwrap();
        declined
    };
    assert!(!declined.keys().any(|k| ["rr_dfs", "rr_mall", "hit", "hm", "found"].iter().any(|r| k.contains(r))), "{declined:?}");
    assert_eq!(got["hit"], [["10", "30", "1"], ["20", "30", "5"], ["40", "30", "2"]]);
    assert_eq!(got["hm"], [["10", "30", "1"], ["40", "30", "2"]]);
    let found = |row: [&str; 5]| got["found"].contains(&row.map(str::to_owned).to_vec());
    assert!(found(["é xx xx", "1", "5", "7", "xx"]) && found(["x", "0", "0", "1", "x"]), "{:?}", got["found"]);
}

/// A head's `@dot` with a computed part (`walk_qualname(f, @dot(p, @nfkc(x)))`, data-layer `purity-ast.rerail`), and an
/// ordered comparison with an extremum's value (`k < v`, `v = min l : {…}`, `post-op-side-effects.rerail`), on both
/// paths. The native engine declined both: the head's part was not a slot, and `v` had no type.
#[test]
fn a_heads_computed_part_and_an_extremums_comparison_run_natively() {
    let program = r#"
.decl nm(f: number, p: symbol, x: symbol)
nm(1, "a", "ﬁ"). nm(2, "", "b").
.decl q(f: number, s: symbol)
q(f, @dot(p, @nfkc(x))) :- nm(f, p, x).
.decl l(f: number, x: number)
l(1, 5). l(1, 3). l(2, 7).
.decl r(f: number, k: number)
r(1, 2). r(1, 4). r(2, 9). r(2, 1).
.decl early(f: number, k: number)
early(f, k) :- r(f, k), v = min x : { l(f, x) }, k < v.
.decl late(f: number, k: number)
late(f, k) :- r(f, k), v = max x : { l(f, x) }, k >= v.
.output q, early, late
"#;
    let (got, native) = run_both("head_part", program, &["q", "early", "late"]);
    assert!(native > 0, "the native set engine solved a component");
    assert_eq!(got["q"], [["1", "a.fi"], ["2", "b"]]);
    assert_eq!(got["early"], [["1", "2"], ["2", "1"]]);
    assert_eq!(got["late"], [["2", "9"]]);
}

// ---- violations ----

/// A `.violation` at a `loc` (`core.rerail`): a node's finding is at its document's path and line; a `@place(path,
/// line)`'s at that path and line, line 0 for the file itself and a path that need not exist; a place passed through
/// another relation keeps its path and line.
#[test]
fn violations_at_places_are_found_at_their_path_and_line() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("places");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f()\n\ng()\n").unwrap();
    let program = r#"
.policy "t:places" revision 1
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, path: symbol, module: symbol)
.decl line(n: node, l: number)
.input node, doc, line
.violation LONG(at: loc, what: symbol) code "FILE_LENGTH" "{what}"
    suppress ruff "E501"
LONG(n, "call") :- node(_, n, "call", _, _).
LONG(@place(p, 0), "file") :- doc(_, p, _).
LONG(@place("missing.json", 3), "missing") :- doc(_, _, _).
.decl spot(d: number, at: loc)
spot(d, @place(p, 7)) :- doc(d, p, _).
.violation PIN(at: span) "pin"
PIN(s) :- spot(_, s).
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(
        read("violation"),
        // The sixth column is the finding's (0165): a call's 0-based byte column, 0 for a place.
        [
            ["t:places", "LONG", "FILE_LENGTH", "a.py", "0", "0"],
            ["t:places", "LONG", "FILE_LENGTH", "a.py", "1", "0"],
            ["t:places", "LONG", "FILE_LENGTH", "a.py", "3", "0"],
            ["t:places", "LONG", "FILE_LENGTH", "missing.json", "3", "0"],
            ["t:places", "PIN", "PIN", "a.py", "7", "0"],
        ]
    );
    // The violation's own rows show a place as one.
    assert_eq!(read("PIN"), [["@place(\"a.py\", 7)"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- tree axes and node sets ----

/// The axes over a set of nodes (`fnode`) or of kinds rules derive (`fk`), read as rules; `!nearest` with the
/// ancestor named, `!within_stop`, `!earlier` and `!split_dot`; `precedes_stop` and `next_sibling` over `child`.
#[test]
fn axes_read_node_sets_and_negate() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("node-sets");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def a():\n    pass\ndef b():\n    def inner():\n        g()\n    f()\ndef skip():\n    def deep():\n        k()\nh()\nx = 1\n").unwrap();
    std::fs::write(dir.join("b.py"), "z()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl child(p: node, c: node, i: number)
.decl next_sibling(a: node, b: node)
.decl docof(n: node, d: number)
.input node, field, text, child, next_sibling, docof
.decl fn(kind: symbol)
fn("function_definition").
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
.decl stmt_call(e: node, c: symbol)
stmt_call(e, c) :- node(_, e, "expression_statement", _, _), child(e, x, 0), field(x, "function", y), text(y, c).
.decl label(n: node, s: symbol)
label(n, s) :- fname(n, s).
label(n, s) :- stmt_call(n, s).
.decl top_fn(f: node, s: symbol)
top_fn(f, s) :- fname(f, s), !nearest(f, _, fn).
// A set of nodes: the functions but inner; and a set of kinds that a rule derives.
.decl fnode(n: node)
fnode(n) :- fname(n, x), x != "inner".
.decl fk(k: symbol)
fk(k) :- node(_, f, k, _, _), field(f, "name", _).
.decl skipnode(n: node)
skipnode(n) :- fname(n, "skip").
.decl near_d(c: symbol, f: symbol)
near_d(c, f) :- stmt_call(e, c), nearest(e, a, fnode), fname(a, f).
.decl in_d(c: symbol, f: symbol)
in_d(c, f) :- stmt_call(e, c), inside(e, a, fnode), fname(a, f).
.decl any_d(c: symbol)
any_d(c) :- stmt_call(e, c), inside(e, _, fnode).
.decl top_d(c: symbol)
top_d(c) :- stmt_call(e, c), !nearest(e, _, fnode).
.decl has_d(f: symbol, g: symbol)
has_d(f, g) :- fname(a, f), has(a, n, fnode), fname(n, g).
.decl hasany_d(f: symbol)
hasany_d(f) :- fname(a, f), has(a, _, fnode).
.decl nohas_d(f: symbol)
nohas_d(f) :- fname(a, f), !has(a, _, fnode).
.decl near_k(c: symbol, f: symbol)
near_k(c, f) :- stmt_call(e, c), nearest(e, a, fk), fname(a, f).
.decl has_k(f: symbol)
has_k(f) :- fname(a, f), has(a, _, fk).
.decl has_s(f: symbol, g: symbol)
has_s(f, g) :- fname(a, f), has(a, n, fn), fname(n, g).
.decl stmts_in(f: symbol, c: symbol)
stmts_in(f, c) :- top_fn(a, f), has(a, n, "expression_statement"), stmt_call(n, c).
.decl not_near_b(c: symbol)
not_near_b(c) :- stmt_call(e, c), top_fn(a, "b"), !nearest(e, a, fn).
.decl not_near_b_d(c: symbol)
not_near_b_d(c) :- stmt_call(e, c), top_fn(a, "b"), !nearest(e, a, fnode).
.decl not_own_b(c: symbol)
not_own_b(c) :- stmt_call(e, c), top_fn(a, "b"), !within_stop(e, a, fn).
.decl not_own_b_d(c: symbol)
not_own_b_d(c) :- stmt_call(e, c), top_fn(a, "b"), !within_stop(e, a, fnode).
.decl after_a(s: symbol)
after_a(s) :- top_fn(m, "a"), precedes_stop(m, n, fn), label(n, s).
.decl after_a_skip(s: symbol)
after_a_skip(s) :- top_fn(m, "a"), precedes_stop(m, n, skipnode), label(n, s).
.decl after_b(s: symbol)
after_b(s) :- top_fn(m, "b"), precedes_stop(m, n, "expression_statement"), label(n, s).
.decl not_after_a(s: symbol)
not_after_a(s) :- top_fn(m, "a"), label(n, s), !precedes_stop(m, n, fn).
.decl nxt(x: symbol, y: symbol)
nxt(x, y) :- next_sibling(m, n), label(m, x), label(n, y).
.decl not_before(x: symbol, y: symbol)
not_before(x, y) :- stmt_call(m, x), stmt_call(n, y), !earlier(m, n).
.decl q(s: symbol)
q("os.path"). q("os.path.join").
.decl parts(m: symbol, x: symbol)
parts(m, x) :- q(s), split_dot(s, m, x).
.decl in_os(x: symbol)
in_os(x) :- q(s), split_dot(s, "os", x).
.decl of_join(m: symbol)
of_join(m) :- q(s), split_dot(s, m, "join").
.decl not_split(s: symbol)
not_split(s) :- q(s), !split_dot(s, "os", "path").
.decl not_split2(s: symbol)
not_split2(s) :- q(s), !split_dot(s, "os", "path.join").
.output near_d, in_d, any_d, top_d, has_d, hasany_d, nohas_d, near_k, has_k, has_s, stmts_in, not_near_b, not_near_b_d
.output not_own_b, not_own_b_d, after_a, after_a_skip, after_b, not_after_a, nxt, not_before, parts, in_os, of_join, not_split, not_split2
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    let expected = [
        // g's nearest function is inner, which fnode leaves out.
        ("near_d", vec![vec!["f", "b"], vec!["g", "b"], vec!["k", "deep"]]),
        ("in_d", vec![vec!["f", "b"], vec!["g", "b"], vec!["k", "deep"], vec!["k", "skip"]]),
        ("any_d", vec![vec!["f"], vec!["g"], vec!["k"]]),
        ("top_d", vec![vec!["h"], vec!["z"]]),
        // `has` tests the set on the node below: b has only inner below it.
        ("has_d", vec![vec!["skip", "deep"]]),
        ("hasany_d", vec![vec!["skip"]]),
        ("nohas_d", vec![vec!["a"], vec!["b"], vec!["deep"], vec!["inner"]]),
        ("near_k", vec![vec!["f", "b"], vec!["g", "inner"], vec!["k", "deep"]]),
        ("has_k", vec![vec!["b"], vec!["skip"]]),
        ("has_s", vec![vec!["b", "inner"], vec!["skip", "deep"]]),
        ("stmts_in", vec![vec!["b", "f"], vec!["b", "g"], vec!["skip", "k"]]),
        ("not_near_b", vec![vec!["g"], vec!["h"], vec!["k"], vec!["z"]]),
        ("not_near_b_d", vec![vec!["h"], vec!["k"], vec!["z"]]),
        ("not_own_b", vec![vec!["g"], vec!["h"], vec!["k"], vec!["z"]]),
        ("not_own_b_d", vec![vec!["h"], vec!["k"], vec!["z"]]),
        ("after_a", vec![vec!["b"]]),
        ("after_a_skip", vec![vec!["b"], vec!["skip"]]),
        ("after_b", vec![vec!["h"], vec!["skip"]]),
        ("not_after_a", vec![vec!["a"], vec!["deep"], vec!["f"], vec!["g"], vec!["h"], vec!["inner"], vec!["k"], vec!["skip"], vec!["z"]]),
        ("nxt", vec![vec!["a", "b"], vec!["b", "skip"], vec!["inner", "f"], vec!["skip", "h"]]),
        ("not_before", vec![vec!["f", "f"], vec!["f", "g"], vec!["f", "z"], vec!["g", "g"], vec!["g", "z"], vec!["h", "f"], vec!["h", "g"], vec!["h", "h"], vec!["h", "k"], vec!["h", "z"], vec!["k", "f"], vec!["k", "g"], vec!["k", "k"], vec!["k", "z"], vec!["z", "f"], vec!["z", "g"], vec!["z", "h"], vec!["z", "k"], vec!["z", "z"]]),
        ("parts", vec![vec!["os", "path"], vec!["os.path", "join"]]),
        ("in_os", vec![vec!["path"]]),
        ("of_join", vec![vec!["os.path"]]),
        ("not_split", vec![vec!["os.path.join"]]),
        ("not_split2", vec![vec!["os.path"], vec!["os.path.join"]]),
    ];
    let wrong: Vec<String> = expected.iter().filter(|(rel, rows)| read(rel) != *rows).map(|(rel, rows)| format!("{rel}: {:?}, expected {rows:?}", read(rel))).collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An aggregate whose body is one built-in the lowering writes out (`within`, `earlier`) counts its rows: the
/// built-in is no relation the aggregate could read in place.
#[test]
fn aggregates_over_one_built_in_count_its_rows() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("agg-builtin");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f():\n    g(1)\n    h()\ndef e():\n    pass\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl child(p: node, c: node, i: number)
.input node, field, text, docof, child
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
.decl below(s: symbol, k: number)
below(s, k) :- fname(f, s), k = count : { within(n, f) }.
.decl after(s: symbol, k: number)
after(s, k) :- fname(f, s), k = count : { earlier(f, n) }.
.output below, after
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    let outputs = output::Outputs::open(&out).unwrap();
    let count = |rel: &str, s: &str| outputs.rows(rel).iter().find(|r| r.get(0).text() == Some(s)).and_then(|r| r.get(1).int());
    // f's subtree has more nodes than e's, and e comes after f, so fewer nodes follow it.
    assert!(count("below", "f").unwrap() > count("below", "e").unwrap(), "{:?}", read("below"));
    assert!(count("after", "f").unwrap() > count("after", "e").unwrap(), "{:?}", read("after"));
    assert!(count("below", "e").unwrap() > 0, "{:?}", read("below"));
    drop(outputs);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- recursion ----

/// Recursion that makes values bounded by the data runs to its fixpoint: a count along a call's arguments in
/// child order, their text joined in that order, a counter below a bound, and a text cut shorter at each step;
/// and negated conjunctions run as their helper relations, with a variable shared, a shared one only tested, an
/// alternative each, and none.
#[test]
fn bounded_value_recursion_and_negated_conjunctions_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("recursion");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f(x, y, z)\nw = os.path.join\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl arg_at(c: node, a: node, k: number)
arg_at(c, a, 0) :- node(_, c, "argument_list", _, _), child(c, a, 0).
arg_at(c, b, k + 1) :- arg_at(c, a, k), child(c, a, i), child(c, b, i + 1).
.decl arg_pos(s: symbol, k: number)
arg_pos(s, k) :- arg_at(_, a, k), text(a, s).
.decl pos(s: symbol, k: number)
pos(s, k) :- arg_pos(s, k).
.decl joined(c: node, i: number, v: symbol)
joined(c, 0, "") :- node(_, c, "argument_list", _, _).
joined(c, i + 1, cat(v, w)) :- joined(c, i, v), child(c, a, i), text(a, w).
.decl whole(v: symbol)
whole(v) :- joined(c, i, v), !child(c, _, i).
.decl up(x: number)
up(1).
up(x + 1) :- up(x), x < 4.
.decl dotted(s: symbol)
dotted("os.path.join").
.decl rest(s: symbol, i: number)
rest(t, 0) :- dotted(t).
rest(s, i + 1) :- rest(t, i), match("[^.]*\\.(?<s>.*)", t).
.decl first(s: symbol)
first(s) :- arg_pos(s, _), !(arg_pos(s, k), k > 0).
.decl mid(s: symbol)
mid(s) :- arg_pos(s, k), !(k = 0 ; k = 2).
.decl none(s: symbol)
none(s) :- arg_pos(s, 0), !(arg_pos(_, 5), text(_, _)).
.decl gone(s: symbol)
gone(s) :- arg_pos(s, 0), !(arg_pos(_, 2)).
.output pos, whole, up, rest, first, mid, none, gone
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("pos"), [["x", "0"], ["y", "1"], ["z", "2"]]);
    assert_eq!(read("whole"), [["xyz"]]);
    assert_eq!(read("up"), [["1"], ["2"], ["3"], ["4"]]);
    assert_eq!(read("rest"), [["join", "2"], ["os.path.join", "0"], ["path.join", "1"]]);
    assert_eq!(read("first"), [["x"]]);
    assert_eq!(read("mid"), [["y"]]);
    assert_eq!(read("none"), [["x"]]);
    assert_eq!(read("gone"), Vec::<Vec<String>>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A negated conjunction is correlated on a rule variable its aggregate reads, or that a built-in in it only tests
/// (`split_dot`); and a regex group of a string the regex does not match is no value, so a negation over it gives
/// no row, on both the SQL and the native paths.
#[test]
fn negations_over_aggregates_built_ins_and_missing_groups() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("negations");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f(1, 2, 3)\ng(1)\nos.path.join\nos.sep\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl name(n: node, s: symbol)
.input node, field, child, text, name
.decl callee(c: node, f: symbol)
callee(c, f) :- node(_, c, "call", _, _), field(c, "function", x), text(x, f).
.decl arg(c: node, a: node)
arg(c, a) :- callee(c, _), field(c, "arguments", l), child(l, a, _).
// Calls with at most two arguments.
.decl short(f: symbol)
short(f) :- callee(c, f), !(count : { arg(c, _) } > 2).
.decl attr(q: symbol)
attr(q) :- node(_, n, "attribute", _, _), name(n, q).
.decl keyword(x: symbol)
keyword("sep").
// Dotted names whose last segment is no keyword.
.decl plain(q: symbol)
plain(q) :- attr(q), !(split_dot(q, m, x), keyword(x)).
// Names with no `.`-separated tail that is a keyword; `g` has no tail, so it is kept.
.decl untailed(s: symbol)
untailed(s) :- callee(_, s), match("(?<t>[a-z]+)\\.(?<u>[a-z]+)", s), !keyword(u).
.decl tailless(s: symbol)
tailless(s) :- callee(_, s), !keyword(@rx_group(s, "^(?:[a-z]+\\.(?<u>[a-z]+))$", "u")).
.output short, plain, untailed, tailless
"#;
    for modes in ["", "program"] {
        let out = dir.join(format!("out{modes}"));
        super::corpus::run(program, &dir, None, &out, modes, crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
        assert_eq!(read("short"), [["g"]], "{modes}");
        assert_eq!(read("plain"), [["os.path"], ["os.path.join"]], "{modes}");
        assert_eq!(read("untailed"), Vec::<Vec<String>>::new(), "{modes}");
        assert_eq!(read("tailless"), Vec::<Vec<String>>::new(), "{modes}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `program` over `files` (path, source) by the corpus runner; each of `outputs` as written.
fn corpus_outputs(name: &str, files: &[(&str, &str)], program: &str, outputs: &[&str]) -> Vec<Vec<Vec<String>>> {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, source).unwrap();
    }
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let got = outputs.iter().map(|rel| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    got
}

/// A relation given both facts and rules holds both: its facts are not lost to its rules' rows.
#[test]
fn a_relation_of_facts_and_rules_keeps_its_facts() {
    let program = r#"
.decl str(n: node, s: symbol)
.decl text(n: node, s: symbol)
.input str, text
.decl other(x: symbol)
other("SyncX").
.decl pat(x: symbol)
pat("Sequence").
pat(x) :- other(x).
.decl hit(s: symbol)
hit(c) :- str(_, c), pat(x), word(x, c).
.decl pair(y: symbol, x: symbol)
pair("legacy", "a").
pair(y, y) :- text(_, y), prefix("le", y).
.decl joined(y: symbol, x: symbol)
joined(y, x) :- pair(y, x), text(_, y).
.output hit, joined
"#;
    let got = corpus_outputs("facts-and-rules", &[("a.py", "def f(a: \"Sequence\", b: \"SyncX\"):\n    legacy = lemon\n")], program, &["hit", "joined"]);
    assert_eq!(got[0], [["Sequence"], ["SyncX"]]);
    assert_eq!(got[1], [["legacy", "a"], ["legacy", "legacy"], ["lemon", "lemon"]]);
}

/// `max t : { r(d, x, t, _, _) }` groups by the variables bound outside it (`d`, `x`), not by the wildcards.
#[test]
fn an_aggregate_groups_by_its_bound_variables_not_its_wildcards() {
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.input node
.decl def(d: number, x: symbol, t: number, k: symbol, l: number)
def(d, "f", s, "function", 1) :- node(d, _, "function_definition", s, _).
def(d, "f", s, "assign", 2) :- node(d, _, "assignment", s, _).
.decl last(x: symbol, s: number, k: symbol)
last(x, s, k) :- def(d, x, s, k, _), s = max t : { def(d, x, t, _, _) }.
.decl first(x: symbol, s: number, k: symbol)
first(x, s, k) :- def(d, x, s, k, _), s = min t : { def(d, x, t, _, _) }.
.output last, first
"#;
    let got = corpus_outputs("aggregate-wildcards", &[("a.py", "def f():\n    pass\nf = 1\n")], program, &["last", "first"]);
    assert_eq!(got[0], [["f", "18", "assign"]]);
    assert_eq!(got[1], [["f", "0", "function"]]);
}

/// A variable named `top` is a variable where an atom binds it, not the lattice's top: `child(p, top, _)` joins as
/// `child(p, c, _)` does.
#[test]
fn a_variable_named_top_is_a_variable_where_an_atom_binds_it() {
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.input node, child
.decl lists(n: node)
lists(n) :- node(_, n, "argument_list", _, _).
.decl via(p: node, c: node)
via(p, c) :- lists(p), child(p, c, _).
.decl direct(p: node, c: node)
direct(p, top) :- node(_, p, "argument_list", _, _), child(p, top, _).
.decl kids(p: node, c: node)
kids(p, c) :- child(p, c, _).
.decl grand(a: node, c: node)
grand(a, c) :- kids(a, b), kids(b, c).
.decl chained(a: node, c: node)
chained(a, top) :- child(a, b, _), child(b, top, _).
.output via, direct, grand, chained
"#;
    let got = corpus_outputs("child-chain", &[("a.py", "class Base(SqlRenderable):\n    pass\n")], program, &["via", "direct", "grand", "chained"]);
    assert!(!got[0].is_empty(), "{got:?}");
    assert_eq!(got[1], got[0]);
    assert!(!got[2].is_empty(), "{got:?}");
    assert_eq!(got[3], got[2]);
}

/// `s` as a `.rerail` string literal (`rr::syntax`'s escapes: `\"`, `\\`, `\n`, `\t`, `\r`, `\u{…}`).
fn rr_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

/// W's value of `sql`, a scalar query, in the SQLite its relational stages run (with its `rx_*` functions), over a
/// table `t(n, value)` of `rows`, `n` their positions from 1.
fn w_value(rows: &[&str], sql: &str, params: &[&str]) -> String {
    let db = Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::waves::relational::install_regex_functions(&db).unwrap();
    db.execute_batch("CREATE TABLE t(n INTEGER PRIMARY KEY, value TEXT)").unwrap();
    for r in rows {
        db.execute("INSERT INTO t(value) VALUES (?1)", [r]).unwrap();
    }
    db.query_row(sql, rusqlite::params_from_iter(params), |r| r.get::<_, rusqlite::types::Value>(0))
        .map(|v| match v {
            rusqlite::types::Value::Text(s) => s,
            rusqlite::types::Value::Integer(i) => i.to_string(),
            other => panic!("{other:?}"),
        })
        .unwrap()
}

/// `rels` of `src` by plain SQL over text symbols (`run_sql`) and by the corpus runner over atoms (`plan::lower`, its
/// `sym_text`/`sym_atom`), each relation's rows of values as shown.
fn on_both_paths(name: &str, src: &str, rels: &[&str]) -> [Vec<Vec<Vec<String>>>; 2] {
    let text = run_sql(parse_version(src).unwrap_or_else(|e| panic!("{e}")), &facts()).unwrap_or_else(|e| panic!("{e}"));
    let text = rels.iter().map(|r| text.outputs.get(*r).map(|rows| rows.shown()).unwrap_or_default()).collect();
    [text, corpus_outputs(name, &[("a.py", "x = 1\n")], src, rels)]
}

/// `join` and `@json_str` rebuild the messages W's signed-list stages sign (`data-layer-post-op-side-effects`'s
/// `'{"allowlist":'||json_group_array(value ORDER BY value)||…`): quotes, backslashes, control characters, non-ASCII
/// (kept, as `json_group_array` keeps it) and astral characters; duplicates kept by a key that tells them apart
/// (`by (v, n)`), none ("[]"). `join` orders numbers by value and symbols by code point.
#[test]
fn join_and_json_str_rebuild_ws_signed_messages() {
    let entries = ["b", "a\"q", "back\\slash", "tab\tnl\nret\r", "\u{1}\u{1f}\u{7f}", "é ü 中", "😀", "Z", "", "\u{2028}", "a"];
    let registry = ["b", "a", "b", "é"];
    let w = |rows: &[&str], linter: &str| {
        let sql = format!("SELECT '{{\"allowlist\":'||coalesce((SELECT json_group_array(value) FROM (SELECT value FROM t ORDER BY value)),'[]')||',\"linter\":\"{linter}\",\"version\":1}}'");
        w_value(rows, &sql, &[])
    };
    let facts = |rel: &str, rows: &[&str]| rows.iter().enumerate().map(|(i, e)| format!("{rel}({}, {}).\n", i + 1, rr_str(e))).collect::<String>();
    let src = format!(
        r#".decl entry(n: number, e: symbol)
{}.decl reg(n: number, v: symbol)
{}.decl none(n: number, v: symbol)
none(n, v) :- reg(n, v), n < 0.
.decl unit(x: number)
unit(0).
.decl msg(l: symbol, m: symbol)
msg("surface", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"post_op_surface\",\"version\":1}}"))) :- unit(0),
    b = join q by e sep "," : {{ entry(_, e), q = @json_str(e) }}.
msg("runtime", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"post_op_runtime_allowlist\",\"version\":1}}"))) :- unit(0),
    b = join q by (v, n) sep "," : {{ reg(n, v), q = @json_str(v) }}.
msg("none", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"none\",\"version\":1}}"))) :- unit(0),
    b = join q by v sep "," : {{ none(_, v), q = @json_str(v) }}.
.decl w(l: symbol, m: symbol)
w("surface", {}).
w("runtime", {}).
w("none", {}).
.decl same(l: symbol)
same(l) :- msg(l, m), w(l, m).
.decl num(k: number, x: symbol)
num(10, "ten"). num(9, "nine"). num(100, "hundred"). num(9, "neuf").
.decl by_number(s: symbol)
by_number(s) :- unit(0), s = join x by k sep " " : {{ num(k, x) }}.
.output same, msg, by_number
"#,
        facts("entry", &entries),
        facts("reg", &registry),
        rr_str(&w(&entries, "post_op_surface")),
        rr_str(&w(&registry, "post_op_runtime_allowlist")),
        rr_str(&w(&[], "none")),
    );
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("join-json", &src, &["same", "msg", "by_number"])) {
        assert_eq!(got[0], [["none"], ["runtime"], ["surface"]], "{path}: {:?}", got[1]);
        assert_eq!(got[2], [["neuf nine ten hundred"]], "{path}");
    }
}

/// `@ed25519_ok` is W's `verify_ed25519`: RFC 8032's first vector verifies, a changed message or a malformed key or
/// signature does not, never an error (integrated-compatibility.test.ts, "verifies detached signatures without
/// treating malformed signatures as runtime failures"); and post-op-side-effects' reproducer gives 0.
#[test]
fn ed25519_ok_is_ws_verify_ed25519() {
    use base64::Engine as _;
    let key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    let sig = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
    let sig: Vec<u8> = (0..sig.len()).step_by(2).map(|i| u8::from_str_radix(&sig[i..i + 2], 16).unwrap()).collect();
    let sig = base64::engine::general_purpose::STANDARD.encode(sig);
    let src = format!(
        r#".decl unit(x: number)
unit(0).
.decl ok(c: symbol, v: number)
ok("rfc", v) :- unit(0), v = @ed25519_ok("{key}", "", "{sig}").
ok("changed", v) :- unit(0), v = @ed25519_ok("{key}", "changed", "{sig}").
ok("invalid", v) :- unit(0), v = @ed25519_ok("invalid", "", "invalid").
ok("unpadded", v) :- unit(0), v = @ed25519_ok("{key}", "", "{}").
.decl repro(v: number)
repro(v) :- m = join q by e sep "," : {{ e = "a", q = @json_str(e) }},
    v = @ed25519_ok("7e9776d87292ab633115bd6c07892a4bf56450fffb2dfcf13b3eba1866b9fbb6", cat("[", cat(m, "]")), "AAAA").
.output ok, repro
"#,
        sig.trim_end_matches('='),
    );
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("ed25519", &src, &["ok", "repro"])) {
        assert_eq!(got[0], [["changed", "0"], ["invalid", "0"], ["rfc", "1"], ["unpadded", "0"]], "{path}");
        assert_eq!(got[1], [["0"]], "{path}");
    }
}

/// `@rx_replace(s, re, by)` is W's SQL `rx_replace`: every match replaced, `$1`, `$name`, `${name}` and `$$` as the
/// `regex` crate reads them (`$1x` is the group named `1x`, absent: empty), empty matches included.
#[test]
fn rx_replace_is_ws_rx_replace() {
    let cases = [
        ("a  b\tc\n", r"\s+", ""),
        ("x=1, y=2", r"(?P<k>\w)=(\d)", "$2$k"),
        ("abc", "b", "$$"),
        ("aaa", "a*", "-"),
        ("pkg.mod.name", r"(?:\.[^.]+){1}$", ""),
        ("cost 5", r"(\d)", "${1}0"),
        ("a1b", r"(\d)", "$1x"),
        ("AB+/=c-d", "[^A-Za-z0-9+/=]", ""),
        ("é中😀", ".", "<$0>"),
    ];
    let mut src = String::from(".decl replacing(i: number, s: symbol, re: symbol, by: symbol)\n.decl w(i: number, v: symbol)\n");
    for (i, (s, re, by)) in cases.iter().enumerate() {
        let w = w_value(&[], "SELECT rx_replace(?1, ?2, ?3)", &[*s, *re, *by]);
        src += &format!("replacing({i}, {}, {}, {}).\nw({i}, {}).\n", rr_str(s), rr_str(re), rr_str(by), rr_str(&w));
    }
    src += ".decl got(i: number, v: symbol)\ngot(i, @rx_replace(s, re, by)) :- replacing(i, s, re, by).\n";
    src += ".decl differs(i: number)\ndiffers(i) :- got(i, v), !w(i, v).\ndiffers(i) :- w(i, v), !got(i, v).\n";
    src += ".decl cases(k: number)\ncases(k) :- replacing(0, _, _, _), k = count : { got(_, _) }.\n.output differs, cases\n";
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("rx-replace", &src, &["differs", "cases"])) {
        assert_eq!(got[0], Vec::<Vec<String>>::new(), "{path}");
        assert_eq!(got[1], [[cases.len().to_string()]], "{path}");
    }
}

/// `@nfkc(s)` is CPython's identifier normalization (PEP 3131): CPython 3.13.18's `unicodedata.normalize("NFKC", s)`,
/// which is also the `Name` id `ast.parse` gives each of these (but `x²`, no identifier): ligatures, letterlike and
/// mathematical letters, full- and half-width forms, a decomposed accent composed, titlecase digraphs, the Angstrom and
/// micro signs, a long s with a dot below; ASCII unchanged.
#[test]
fn nfkc_is_cpythons_identifier_normalization() {
    let cases = [
        ("\u{fb01}le", "file"),
        ("\u{210c}ello", "Hello"),
        ("x\u{b2}", "x2"),
        ("\u{ff21}b", "Ab"),
        ("\u{ff76}", "\u{30ab}"),
        ("e\u{301}t\u{e9}", "\u{e9}t\u{e9}"),
        ("ascii", "ascii"),
        ("\u{1c5}x", "D\u{17e}x"),
        ("\u{fb03}", "ffi"),
        ("\u{212b}", "\u{c5}"),
        ("\u{1d431}y", "xy"),
        ("\u{b5}s", "\u{3bc}s"),
        ("\u{1e9b}\u{323}", "\u{1e69}"),
    ];
    let mut src = String::from(".decl written(i: number, s: symbol)\n.decl cpython(i: number, v: symbol)\n");
    for (i, (s, v)) in cases.iter().enumerate() {
        src += &format!("written({i}, {}).\ncpython({i}, {}).\n", rr_str(s), rr_str(v));
    }
    src += ".decl got(i: number, v: symbol)\ngot(i, @nfkc(s)) :- written(i, s).\n";
    src += ".decl differs(i: number)\ndiffers(i) :- got(i, v), !cpython(i, v).\ndiffers(i) :- cpython(i, v), !got(i, v).\n";
    src += ".decl cases(k: number)\ncases(k) :- written(0, _), k = count : { got(_, _) }.\n.output differs, cases\n";
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("nfkc", &src, &["differs", "cases"])) {
        assert_eq!(got[0], Vec::<Vec<String>>::new(), "{path}");
        assert_eq!(got[1], [[cases.len().to_string()]], "{path}");
    }
}

/// `sum x : { … }` is SQL's sum over the body's rows (rows that differ elsewhere count twice), grouped by the
/// rule's variables, 0 over none (Soufflé's), as W's `coalesce(sum(x), 0)`.
#[test]
fn sum_adds_the_bodys_rows() {
    let w = |rows: &[&str]| w_value(rows, "SELECT coalesce(sum(CAST(value AS INTEGER)), 0) FROM t", &[]);
    let src = format!(
        r#".decl r(g: symbol, k: number, x: number)
r("a", 1, 5). r("a", 2, 5). r("a", 3, 7). r("b", 1, -2).
.decl g(g: symbol)
g("a"). g("b"). g("c").
.decl total(g: symbol, s: number)
total(g, s) :- g(g), s = sum x : {{ r(g, _, x) }}.
.decl w(g: symbol, s: number)
w("a", {}). w("b", {}). w("c", {}).
.decl differs(g: symbol)
differs(g) :- total(g, s), !w(g, s).
differs(g) :- w(g, s), !total(g, s).
.decl doubled(s: number)
doubled(s) :- g("a"), s = sum y : {{ r("a", k, x), y = x * 2, k > 1 }}.
.decl shown(g: symbol, s: number)
shown(g, s) :- total(g, s).
.output shown, differs, doubled
"#,
        w(&["5", "5", "7"]),
        w(&["-2"]),
        w(&[]),
    );
    // `shown`, not `total`: the corpus runner writes an empty file for an output another rule reads (`total` read
    // by `differs` came out empty whatever derived it, `total(g, s) :- w(g, s)` too).
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("sum", &src, &["shown", "differs", "doubled"])) {
        assert_eq!(got[0], [["a", "17"], ["b", "-2"], ["c", "0"]], "{path}: {got:?}");
        assert_eq!(got[1], Vec::<Vec<String>>::new(), "{path}");
        assert_eq!(got[2], [["24"]], "{path}");
    }
}

/// A demanded symbol with many captures must not submit one SQLite statement per match.
#[test]
fn repeated_symbol_captures_use_bulk_sql() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    unsafe extern "C" fn trace(_: u32, state: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
        // SAFETY: the counter outlives the callback, which is removed before it drops.
        unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
        0
    }
    let db = Connection::open_in_memory().unwrap();
    let source = (0..1000).map(|i| format!("k={i} ")).collect::<String>();
    let p = parse_version(&format!(r#"
.decl capture_input(s: symbol)
capture_input({}).
.decl captured(v: symbol)
captured(v) :- capture_input(s), match_all("k=(?<v>[0-9]+)", s).
.output captured
"#, serde_json::to_string(&source).unwrap())).unwrap();
    let count = AtomicUsize::new(0);
    unsafe { rusqlite::ffi::sqlite3_trace_v2(db.handle(), rusqlite::ffi::SQLITE_TRACE_STMT, Some(trace), std::ptr::from_ref(&count).cast_mut().cast()); }
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()));
    let result = run(&body::Placed::whole(p.clone()), &db, inputs, None, Hooks::default());
    unsafe { rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut()); }
    let result = result.unwrap();
    assert_eq!(result.shown()["captured"].len(), 1000);
    assert!(count.load(Ordering::Relaxed) < 100, "1000 matches submitted {} SQLite executions", count.load(Ordering::Relaxed));
}

/// A delta probe of a recursive component prices a lookup by the share of the binding's values the column holds
/// (containment): `fbind`'s shape, where `fcall` by `$a` (a few hundred rows, few distinct `$a`) is probed by
/// `pv`'s many `$a`. Priced as if every probe hit, `fcall` looked dearer than `tfix` by `$k`, and the plan read
/// `tfix` and `fgp` first (19 M rows of work for 220 in calls_cx).
#[test]
fn a_delta_probe_prices_a_lookup_by_the_bindings_it_can_hit() {
    let p = parse_version(
        ".decl pv(a:number, k:number, v:number)\n.decl fcall(c:number, f:number, i:number, a:number)\n.decl tfix(k:number, ka:number, w:number, x:number)\n\
         .decl fgp(f:number, i:number, ka:number, w:number, y:number)\n.decl fbind(c:number, f:number, y:number, x:number)\n\
         fbind(c, f, y, x) :- fcall(c, f, i, a), fgp(f, i, ka, w, y), pv(a, 1, k), tfix(k, ka, w, x).\n.output fbind\n",
    )
    .unwrap();
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats: Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    model.stats.nodes = 450_000.0;
    // Live counts, as a wave leaves them: rows, then distinct values by column.
    for (rel, rows, ndv) in [
        ("pv", 400_000.0, vec![300_000.0, 3.0, 5_000.0]),
        ("fcall", 400.0, vec![400.0, 50.0, 5.0, 100.0]),
        ("tfix", 550.0, vec![350.0, 50.0, 10.0, 300.0]),
        ("fgp", 100.0, vec![50.0, 5.0, 50.0, 10.0, 20.0]),
        ("fbind", 1.0, vec![1.0; 4]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel.to_owned(), c), n);
            model.stats.fan.insert((rel.to_owned(), c), rows / n);
        }
    }
    model.stats.solve(["pv", "fcall", "tfix", "fgp", "fbind"].into_iter().map(str::to_owned).collect());
    let r = 0usize;
    let ((order, _, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), Some(2)).unwrap();
    let order: Vec<String> = order.iter().filter_map(|s| rel_at(&p, r, s.at)).collect();
    assert_eq!(order[..2], ["pv", "fcall"], "{order:?}");
}

/// A restrictive relation outside the fixpoint has the same containment property as one inside it.
/// The delta's name domain is wider than the four names the restrictive input can match.
#[test]
fn a_delta_probe_prices_restrictive_external_inputs_by_the_values_they_can_hit() {
    let p = parse_version(".decl delta(k:number,a:number,t:number,f:number)\n.decl expand(o:number,t:number,k:number)\n.decl restrict(c:number,o:number,a:number)\n.input restrict\n.decl result(c:number,f:number)\nresult(c,f) :- restrict(c,o,a), expand(o,1,k), delta(k,a,2,f).\n.output result\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv, fan) in [
        ("delta",526.0,vec![526.0,173.0,1.0,526.0],vec![1.0,5.0,526.0,1.0]),
        ("expand",201_000.0,vec![10000.0,4.0,1878.0],vec![30.0,100000.0,200.0]),
        ("restrict",450.0,vec![450.0,450.0,4.0],vec![1.0,1.0,140.0]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c,(ndv,fan)) in ndv.into_iter().zip(fan).enumerate() {
            model.ndv.insert((rel,c),ndv);
            model.stats.fan.insert((rel,c),fan);
        }
    }
    model.stats.solve(["delta","expand","result"].into_iter().map(str::to_owned).collect());
    let r = 0usize;
    let ((order,work,_),_)=model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),Some(2)).unwrap();
    let order:Vec<_>=order.iter().filter_map(|s| rel_at(&p, r, s.at)).collect();
    assert_eq!(order, ["delta","restrict","expand"], "restrict the delta before its high-fanout probe: {order:?}, work={work}");
    assert!(work<10_000.0, "price misses as well as hits: {work}");

    // A cached live plan must notice a new value distribution even when every relation's size is unchanged.
    let _plans=KeepPlans::start(None);
    model.ndv.insert(("delta",1),1.0);
    let ((before,_,_),_)=model.plan_rule_live(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),Some(2)).unwrap();
    let before:Vec<_>=before.iter().filter_map(|s| rel_at(&p, r, s.at)).collect();
    assert_eq!(before,["delta","expand","restrict"],"the initial narrow-domain estimate makes expansion cheaper");
    model.ndv.insert(("delta",1),173.0);
    let ((after,_,_),_)=model.plan_rule_live(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),Some(2)).unwrap();
    let after:Vec<_>=after.iter().filter_map(|s| rel_at(&p, r, s.at)).collect();
    assert_eq!(after,["delta","restrict","expand"],"a size-only cache must not keep the obsolete order");

    // A tiny delta can have fewer names than the input yet no matching names. Distinct counts alone cannot
    // detect this. Changing only the values must invalidate the plan even at fixed counts and fanouts.
    model.ndv.insert(("delta",1),1.0);
    model.stats.dfreq.insert(("delta",1),HashMap::from([(Constant::Int(10),526)]));
    model.stats.dfreq.insert(("restrict",2),HashMap::from([(Constant::Int(10),450)]));
    let ((overlap,_,_),_)=model.plan_rule_live(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),Some(2)).unwrap();
    assert_eq!(overlap.iter().filter_map(|s| rel_at(&p, r, s.at)).collect::<Vec<_>>(),["delta","expand","restrict"]);
    model.stats.dfreq.insert(("delta",1),HashMap::from([(Constant::Int(11),526)]));
    assert_eq!(model.delta_misses(&body::nth(&p, r),Some(2)),[0]);
    let ((disjoint,work,_),_)=model.plan_rule_live(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),Some(2)).unwrap();
    assert_eq!(disjoint.iter().filter_map(|s| rel_at(&p, r, s.at)).collect::<Vec<_>>(),["delta","restrict","expand"]);
    assert!(work<2000.0,"disjoint key values must avoid expansion: {work}");
    assert!(model.delta_misses(&body::nth(&p, r),None).is_empty(),"a later predicate is not an already-applied demand");

    // A partial SQL constant memo does not prove absence. Complete native postings do.
    model.stats.dfreq.remove(&("restrict",2));
    let col=model.col("restrict",2);
    model.stats.col.insert(("restrict".into(),col.clone()),[450,2,0,0]);
    model.stats.memo.insert("restrict".into(),HashMap::from([((col.clone(),Constant::Int(10)),225)]));
    assert!(model.delta_misses(&body::nth(&p, r),Some(2)).is_empty());
    model.stats.memo.get_mut("restrict").unwrap().insert((col,Constant::Int(12)),225);
    assert_eq!(model.delta_misses(&body::nth(&p, r),Some(2)),[0]);
}

/// O125's retval regression: a kind-filtered node is not a random draw from the complete node domain.
#[test]
fn filtered_binding_domains_do_not_underprice_expansion_from_function_scopes() {
    let p = parse_version(".decl kind(n:number,k:number)\n.decl scope(n:number,f:number)\n.decl child(p:number,c:number,i:number)\n.input kind\n.input scope\n.input child\n.decl result(f:number,r:number)\nresult(f,r) :- kind(rs,1), child(rs,r,0), scope(rs,f), kind(f,2).\n.output result\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv, fan) in [
        ("kind",300_000.0,vec![300_000.0,100.0],vec![1.0,6000.0]),
        ("scope",160_836.0,vec![160_836.0,2181.0],vec![1.0,100.0]),
        ("child",180_000.0,vec![60_000.0,180_000.0,5.0],vec![3.0,1.0,60_000.0]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c,(ndv,fan)) in ndv.into_iter().zip(fan).enumerate() {
            model.ndv.insert((rel,c),ndv);
            model.stats.fan.insert((rel,c),fan);
        }
    }
    Arc::make_mut(&mut model.stats.kfreq).insert("1".into(),2063);
    Arc::make_mut(&mut model.stats.kfreq).insert("2".into(),2181);
    model.stats.dfreq.insert(("child",2),HashMap::from([(Constant::Int(0),60_000)]));
    // The stated rule (the parse may give others first).
    let r = p.rules().position(|r| p.rel_name(r.head().rel()) == "result").expect("the result rule");
    let b = body::nth(&p, r);
    assert_eq!(model.binding_domains(&b,3)[0],Some(2181.0));
    let ((order,work,_),_)=model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &b,None).unwrap();
    let first=order.iter().find(|s| b.atom(s.at).is_some()).unwrap().at;
    assert_eq!(lit_at(&p, r, first),lit_at(&p, r, 0),"return statements must precede scope expansion: {order:?}");
    assert!(work<20_000.0,"avoid the 160,836-row function-body expansion: {work}");

    // The same bound applies to an ordinary relation with a measured constant frequency.
    assert_eq!(model.binding_domains(&b,1)[1],Some(60_000.0));
    model.stats.dfreq.clear();
    assert_eq!(model.binding_domains(&b,1)[1],Some(180_000.0),"unknown conditional counts must not invent selectivity");
}

/// A probe narrowed by a constant is priced by the constant's share of the rows where its postings are counted, not
/// by one value's share (`ndv`): `ifn`'s shape on the o125 replay, where `pv($o, 218, $k)` by `$k` holds 644 rows a
/// value and `218` is one of 7 values but 43% of the rows. Priced at 92 a binding, `pv` was read before `icall` by
/// `$a` (131): 382 rows a binding were read, then all rejected (0.2 M rows of work at step 07; 554 now).
#[test]
fn a_hot_constant_is_priced_by_its_share_of_the_rows() {
    let p = parse_version(
        ".decl icall(c:number, o:number, a:number)\n.decl pv(o:number, k:number, v:number)\n.decl fv(k:number, a:number, m:number, f:number)\n\
         .decl ifn(c:number, f:number)\n\
         ifn(c, f) :- icall(c, o, a), pv(o, 218, k), fv(k, a, 193, f).\n.output ifn\n",
    )
    .unwrap();
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats: Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    model.stats.nodes = 450_000.0;
    // Live counts, as the replay's step left them: rows, then by column distinct values and skew-weighted fanout.
    for (rel, rows, cols) in [
        ("icall", 438.0, vec![(438.0, 1.0), (438.0, 1.0), (4.0, 131.1)]),
        ("pv", 28_518.0, vec![(15_950.0, 5.3), (7.0, 12_373.0), (1_693.0, 644.4)]),
        ("fv", 695.0, vec![(60.0, 46.9), (153.0, 19.2), (3.0, 489.6), (201.0, 8.0)]),
        ("ifn", 1.0, vec![(1.0, 1.0); 2]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c, (n, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel.to_owned(), c), n);
            model.stats.fan.insert((rel.to_owned(), c), fan);
        }
    }
    let values = |v: &[(i64, i64)]| v.iter().map(|&(k, n)| (Constant::Int(k), n)).collect::<HashMap<_, _>>();
    model.stats.dfreq.insert(("pv".to_owned(), 1), values(&[(218, 12_373), (202, 9_000), (156, 4_000), (1, 1_000), (2, 1_000), (3, 1_000), (4, 145)]));
    model.stats.dfreq.insert(("fv".to_owned(), 2), values(&[(193, 570), (1, 100), (2, 25)]));
    model.stats.solve(["pv", "fv", "ifn"].into_iter().map(str::to_owned).collect());
    let r = 0usize;
    let ((order, _, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), Some(2)).unwrap();
    let order: Vec<String> = order.iter().filter_map(|s| rel_at(&p, r, s.at)).collect();
    assert_eq!(order, ["fv", "icall", "pv"], "{order:?}");
}

/// An equality of a bound variable with an unbound one, `$s = $a` with `$s` bound, assigns `$a` (decision 0099 item
/// 4): the second atom is read by it, not crossed with the first and narrowed after (`fl4317_typing_root`: 14 k rows,
/// then 2.86 M, then 603). The rule runs to the same rows either way round.
#[test]
fn an_equality_binds_its_right_side_before_the_next_join() {
    let p = parse_version(
        ".decl p(s: number)\n.decl q(a: number)\n.decl o(s: number)\n.decl o2(s: number)\n\
         p(1). p(2). p(3). q(2). q(3). q(4).\n\
         o(s) :- p(s), q(a), s = a.\no2(s) :- p(s), q(a), 3 = a, s = a.\n.output o\n.output o2\n",
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    stats.card.insert("p", 14_000.0);
    stats.card.insert("q", 14_000.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    let r = p.rules().position(|r| p.rel_name(r.head().rel()) == "o").unwrap();
    let (order, work, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let shown: Vec<String> = order.iter().map(|s| lit_at(&p, r, s.at)).collect();
    assert_eq!(shown[1], "$s = $a", "the equality between the joins: {shown:?}, work {work}");
    assert!(work < 100_000.0, "a cross product priced: {work}");
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()));
    let out = run(&body::Placed::whole(p.clone()), &db, inputs, None, Hooks::default()).unwrap();
    let mut o: Vec<String> = out.shown()["o"].iter().map(|r| format!("{r:?}")).collect();
    o.sort();
    assert_eq!(o.len(), 2, "{o:?}");
    assert_eq!(out.shown()["o2"].len(), 1);
}

/// A concatenation given its inverse and a function made a join key (`simplify::ConcatInverses`, `FunctionKeys`)
/// derive the rows the rule did: a module's stub file found from its head (`modules.rerail`'s `py_try` "stubs"), and
/// the rest of a name by `substr`, which runs natively.
#[test]
fn join_keys_from_functions_derive_the_rules_rows() {
    let src = r#"
        .decl want(q: symbol)
        want("a.b"). want("c.d"). want("e"). want("zz").
        .decl stubs(r: symbol)
        stubs("s").
        .decl sub(r: symbol, w: symbol)
        sub("s", "x"). sub("s", "y").
        .decl entry(d: symbol, n: symbol)
        entry("s/x", "a.pyi"). entry("s/x", "e.pyi"). entry("s/y", "zz.pyi"). entry("s/y", "c"). entry("t/x", "a.pyi").
        .decl hit(q: symbol, b: symbol, h: symbol)
        hit(q, b, h) :- want(q), stubs(r), sub(r, w), b = cat(r, "/", w), h = @head(q), x = cat(h, ".pyi"), entry(b, x).
        .decl rest(q: symbol, m: symbol)
        rest(q, m) :- want(q), h = @head(q), m = substr(q, strlen(h), strlen(q) - strlen(h)).
        .output hit
        .output rest
    "#;
    let plain = parse_version(src).unwrap();
    let mut pipeline = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("test", std::sync::Arc::clone(&plain));
    let report = super::corpus::preparation::simplify::passes(&mut pipeline).unwrap();
    let keyed = pipeline.finish().unwrap().0;
    assert!(report.inverses >= 3, "the concatenations' parts: {report:?}");
    assert_eq!(report.keyed_by, ["want__key_head0"], "{report:?}");
    let rows = |p: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, modes: Option<&str>| -> Vec<Vec<String>> {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lowered(std::sync::Arc::clone(p), python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes.unwrap_or("")).unwrap();
        if modes.is_some() {
            assert!(got.native.contains("hit") && got.native.contains("rest"), "native: {:?}", got.native);
        }
        let text = got.shown();
        let mut out: Vec<Vec<String>> = ["hit", "rest"].iter().flat_map(|o| text[*o].iter().map(move |r| vec![o.to_string(), format!("{r:?}")])).collect();
        out.sort();
        out
    };
    let want = rows(&plain, None);
    assert_eq!(want.iter().filter(|r| r[0] == "hit").count(), 3, "{want:?}");
    assert_eq!(rows(&keyed, None), want);
    assert_eq!(rows(&keyed, Some("program")), want);
}

/// A gate (`answers::gate`'s `done_N_ad(t)` before `!N(t)`) holds only keys the rule's own literals give: after the
/// delta it is joined once its key is bound, not crossed in because an earlier wave's stats priced it at one row
/// (`py_try`'s "stubs": 404 rows crossed to 4 M, then rejected to 151).
#[test]
fn a_gate_is_joined_once_its_key_is_bound() {
    let mut p = parse_version(
        ".decl dir(b: symbol, x: symbol)\n.decl done_n_b(q: symbol)\n.decl n(q: symbol)\n.decl k(h: symbol, q: symbol)\n.decl o(q: symbol, b: symbol)\n\
         o(q, b) :- dir(b, x), done_n_b(q), !n(q), k(h, q), h = @str_between(x, \"\", \".pyi\").\n.output o\n",
    )
    .unwrap();
    // Its negated atom gated.
    struct Gate<'v>(&'v Version);
    impl crate::code_rule_dsl::logic::program_image::Rewrite for Gate<'_> {
        fn atom_as(&mut self, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>) -> Option<(String, bool, bool)> {
            a.neg().then(|| (self.0.rel_name(a.rel()).to_owned(), true, true))
        }
    }
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::derive(&p);
    for r in p.rules() {
        b.rewrite(&p, r, &mut Gate(&p)).finish().unwrap();
    }
    p = b.finish().unwrap().0;
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats: Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    for (rel, rows) in [("dir", 400.0), ("done_n_b", 1.0), ("n", 10.0), ("k", 10_000.0)] {
        model.stats.card.insert(rel, rows);
    }
    let r = 0usize;
    let ((order, _, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), Some(0)).unwrap();
    let b = body::nth(&p, r);
    let atoms: Vec<&str> = order.iter().filter_map(|s| b.atom(s.at)).filter(|a| !a.neg()).map(|a| b.rel(a)).collect();
    assert_eq!(atoms, ["dir", "k", "done_n_b"], "{:?}", order.iter().map(|s| lit_at(&p, r, s.at)).collect::<Vec<_>>());
}

/// A kept plan notices an exact constant frequency: zero postings of the constant make it start from them.
#[test]
fn changed_scope_demands_precede_walk_and_seed_caches_notice_constant_frequencies() {
    let p = parse_version(".decl walk(n:number,x:number,s:number)\n.decl bound(s:number,x:number)\n.decl changed(s:number,k:number)\n.decl out(n:number,x:number,s:number)\nout(n,x,s) :- walk(n,x,s), !bound(s,x), changed(s,1).\n.output out\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("walk",71976.0,vec![50000.0,10000.0,1000.0]),("bound",40000.0,vec![1000.0,10000.0]),("changed",7000.0,vec![7000.0,20.0])] {
        model.stats.card.insert(rel,rows);
        for (c,n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel,c),n);
            model.stats.fan.insert((rel,c),rows/n);
        }
    }
    // Walk is highly skewed. Distinct changed scopes are key probes, not random walk rows.
    model.stats.fan.insert(("walk",2),40000.0);
    model.stats.nodes=300000.0;
    let r = 0usize;
    let _plans=KeepPlans::start(None);
    // Unknown delta distribution first, then an exact zero for the constant, at identical row counts.
    let ((_,before,_),_) = model.plan_rule_reused(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),None).unwrap();
    model.stats.memo.insert("changed".into(),HashMap::from([((super::sql_col("changed",1),Constant::Int(1)),0)]));
    let ((order,work,_),_)=model.plan_rule_reused(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r),None).unwrap();
    assert_eq!(rel_at(&p, r, order[0].at).unwrap(),"changed", "the seed must not retain the old full-walk scan");
    // Zero postings make the plan cheaper than the one made before they were known. (It was under 10 when a
    // relation named `__plus` was priced as a delta by its distinct demand; that pricing went with the forms, 0364
    // R1, and `changed` is priced as the input it is, probed by its constant.)
    assert!(work < before, "zero constant postings precede all walk/binding work: {work} against {before}");
    // Its second half, a changed-input demand (`changed__plus`) led before the run's delta (`walk`), went with the
    // planner's maintenance names (0364 R1: `reads_change`, `changes_with_the_delta`): no relation is a changed
    // input by its name, and a delta leads its own rule.
}

/// Smoking gun 2 (t34, 2026-11-13): a relation empty when its rule is planned (annotation demand's `rel_base`
/// before its guest round made `rr_ast_1_1`) priced every later step at 0 rows, so every order was free: `pkg` (40
/// rows) was scanned with nothing bound before `docof` was probed by what was bound (10,728 -> 25.8 M -> 10,728 rows
/// once `rr_ast_1_1` had rows; its EXPLAIN priced every step 0.00 rows). It is priced at least one row.
#[test]
fn a_relation_empty_when_planned_does_not_make_a_cross_product_free() {
    let p = parse_version(".decl e(x:number)\n.decl big(y:number)\n.decl link(x:number,y:number)\n.decl out(x:number,y:number)\nout(x,y) :- e(x), big(y), link(x,y).\n.output out\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    // `rel_base`'s sizes: `rr_ast_1_1` empty (no rows, so no fanout), `pkg` 40 rows, `docof` 90,100 keyed by its node.
    for (rel, rows, ndv) in [("e", 0.0, vec![1.0]), ("big", 40.0, vec![40.0]), ("link", 90_100.0, vec![90_100.0, 40.0])] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), rows / n);
        }
    }
    let r = 0usize;
    let _plans = KeepPlans::start(None);
    let explain = explaining(true);
    let (_, explained) = model.plan_rule_reused(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    drop(explain);
    let steps = explained.expect("explained").steps;
    let zero: Vec<&str> = steps.iter().filter(|s| s.rows <= 0.0).map(|s| s.literal.as_str()).collect();
    assert!(zero.is_empty(), "no step is priced at no rows, so no order after the empty relation is free: {zero:?} of {:?}", steps.iter().map(|s| (s.literal.as_str(), s.rows)).collect::<Vec<_>>());
}

/// Smoking gun 10 (t34, 2026-11-13), red until fixed. `frag_str(s) :- ann_str(s), d = @doc(s), doc_role(d,
/// "fragment")` is planned before the guest round makes the fragment documents: "fragment" counts 0, so `doc_role`
/// leads, and `d = @doc(s)` keys nothing, so each fragment then scans every string. Its line did about a row of work,
/// and the planning budget (decision 0122) measures growth by the relations' rows alone (`budget_takes`): `doc_role`
/// grew 9 times, under the budget, so the order was kept with 2,455 fragments (7.1 M rows, none kept). A constant's
/// count that moved from none to many is growth, and the line is searched again.
#[test]
fn a_constant_counted_none_when_planned_is_growth_when_it_has_rows() {
    let p = parse_version(".decl ann_str(s: node)\n.decl doc_role(d: number, r: number)\n.decl frag_str(s: node)\nfrag_str(s) :- ann_str(s), d = @doc(s), doc_role(d, 7).\n.output frag_str\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    let size = |model: &mut Model, roles: f64, fragments: i64| {
        for (rel, rows, ndv) in [("ann_str", 100_000.0, vec![100_000.0]), ("doc_role", roles, vec![roles, 4.0])] {
            model.stats.card.insert(rel, rows);
            for (c, n) in ndv.into_iter().enumerate() {
                model.ndv.insert((rel, c), n);
                model.stats.fan.insert((rel, c), (rows / n).max(1.0));
            }
        }
        // Under the column's name as the model reads it (`Model::col`: `c1`, `doc_role` being no input), so that the
        // count is the one the search prices.
        let role = model.col("doc_role", 1);
        model.stats.memo.insert("doc_role".into(), HashMap::from([((role, Constant::Int(7)), fragments)]));
    };
    model.stats.nodes = 300_000.0;
    let r = 0usize;
    let _plans = KeepPlans::start(None);
    let lead = |order: &[Step]| rel_at(&p, r, order[0].at);
    // Planned before the guest round: about no fragment yet (production's estimate was 0.01 rows), and the line
    // ran over none.
    size(&mut model, 300.0, 1);
    let ((order, work, _), _) = model.plan_rule_reused(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("doc_role"), "with about no fragment, `doc_role` leads: {work} {:?}", order.iter().map(|s| lit_at(&p, r, s.at)).collect::<Vec<_>>());
    model.ran_as_planned(&body::nth(&p, r), None, 1.0);
    // The guest round made 2,455 fragment documents.
    size(&mut model, 2_755.0, 2_455);
    let ((order, _, _), _) = model.plan_rule_reused(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("ann_str"), "2,455 fragments, each scanning every string, are searched again");
}

/// A kept plan's price is a prediction, and its line's run is what it cost: one that ran far over its kept price is
/// not reused at that price. corpus0 (2026-11-21): `open_tuple`'s seed was kept at 753 rows of work, ran 41 M rows a
/// stratum, and was reused at 753 by each of its 7 strata, since no size it read moved 2x. `r(x) :- a(x, y), c(y)`,
/// priced over a search (so decision 0123 §7's misprice, for orders priced under one, does not hold it): its line
/// ran a million times its price, and the next planning, its sizes moved under 2x, does not take the kept price.
#[test]
fn a_kept_plan_whose_line_ran_far_over_its_price_is_not_reused_at_it() {
    let p = parse(".decl a(x: number, y: number)\n.decl c(y: number)\n.decl r(x: number)\nr(x) :- a(x, y), c(y).\n.output r\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("a", 1_000_000.0, vec![1_000_000.0, 100_000.0]), ("c", 1_000.0, vec![1_000.0])] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), (rows / n).max(1.0));
        }
    }
    model.stats.nodes = 1_000_000.0;
    let _plans = KeepPlans::start(None);
    let placed = body::Placed::whole(p.clone());
    let fields = placed.fields().unwrap();
    let own = body::nth(&p, 0);
    let ((_, work, _), _) = model.plan_rule_reused(fields, &own, None).unwrap();
    assert!(work > search_rows(&own, None), "priced over a search, so the misprice rule does not apply: {work}");
    model.ran_as_planned(&own, None, work * 1e6);
    // `a` grows half again, under the 2x that keeps a price near: a re-pricing prices that, the kept price does not.
    model.stats.card.insert("a", 1_500_000.0);
    model.stats.fan.insert(("a", 1), 15.0);
    let ((_, again, _), _) = model.plan_rule_reused(fields, &own, None).unwrap();
    assert_ne!(again, work, "a line that ran a million times its kept price is not reused at that price");
}

/// A recheck (decision 0364 stage 3) runs a rule once per candidate row, its head's variables bound before its first
/// step: the planner prices its orders from that binding and one row (Q208), and keeps the order apart from the
/// rule's own. `r(x) :- a(x, y), c(y).`: alone, `c`'s 10 rows lead and probe `a`'s million by `y`; with `x` bound,
/// `a` probed by `x` gives a row, which `c` tests. The recheck's plan, once it ran under a search's work, is taken
/// as it is kept.
#[test]
fn a_recheck_is_priced_with_its_head_bound_and_kept_apart() {
    let p = parse(".decl a(x: number, y: number)\n.decl c(y: number)\n.decl r(x: number)\nr(x) :- a(x, y), c(y).\n.output r\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("a", 1_000_000.0, vec![1_000_000.0, 100_000.0]), ("c", 10.0, vec![10.0])] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), (rows / n).max(1.0));
        }
    }
    model.stats.nodes = 1_000_000.0;
    let r = p.rule(crate::code_rule_dsl::logic::program_image::RuleId(0)).unwrap();
    let lits: Vec<_> = r.body().collect();
    let _plans = KeepPlans::start(None);
    let placed = body::Placed::whole(p.clone());
    let fields = placed.fields().unwrap();
    let own = body::nth(&p, 0);
    let bound = own.with_head_bound();
    let lead = |order: &[Step]| lits[order[0].at].atom().map(|a| p.rel_name(a.rel()).to_string());
    let ((order, _, _), _) = model.plan_rule_reused(fields, &own, None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("c"), "alone, the small relation leads");
    let ((order, work, _), _) = model.plan_rule_reused(fields, &bound, None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("a"), "with its head bound, the atom it binds leads: {work}");
    assert!(work < 10.0, "a few rows a candidate: {work}");
    // Ran under a search's work: taken as it is kept, its order the same; and kept apart from the rule's own, which
    // a shared key would have taken as it is too.
    model.ran_as_planned(&bound, None, 2.0);
    let ((again, _, _), _) = model.plan_rule_reused(fields, &bound, None).unwrap();
    assert_eq!(lead(&again).as_deref(), Some("a"));
    let ((alone_again, _, _), _) = model.plan_rule_reused(fields, &own, None).unwrap();
    assert_eq!(lead(&alone_again).as_deref(), Some("c"), "the rule's own order is its own");
}

/// Smoking gun 10's cold case (t34, 2026-11-13): with no kept plan, `frag_str` is planned before the guest round
/// makes `doc_role`'s rows. Empty when priced, `doc_role(d, 7)` first looked as cheap as a row, and `ann_str`
/// crossed with it read every string per role row (7.1 M rows, none kept). A cross product after relations that
/// had no rows when priced is priced from a search's rows: `ann_str` leads, and `doc_role` is probed by `@doc`.
#[test]
fn a_cross_product_after_a_relation_empty_when_planned_is_not_free() {
    let p = parse_version(".decl ann_str(s: node)\n.decl doc_role(d: number, r: number)\n.decl frag_str(s: node)\nfrag_str(s) :- ann_str(s), d = @doc(s), doc_role(d, 7).\n.output frag_str\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("ann_str", 100_000.0, vec![100_000.0]), ("doc_role", 0.0, vec![0.0, 0.0])] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), 1.0);
        }
    }
    model.stats.nodes = 300_000.0;
    let r = 0usize;
    let (order, work, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let lead = rel_at(&p, r, order[0].at);
    assert_eq!(lead.as_deref(), Some("ann_str"), "`doc_role`, empty when planned, does not lead a cross product: {work} {:?}", order.iter().map(|s| lit_at(&p, r, s.at)).collect::<Vec<_>>());
}

/// Decision 0297: an order's price is the executor's count of its work (`profile::plan_work`): each step costs the
/// rows it passes on, a join and a filter alike, and a filter passes on its kind's measured share. A scan of 1,000
/// rows, a test against a constant (`!=`: 0.97) and a bound of an id's range (0.98) cost 1,000 + 970 + 950.6.
#[test]
fn a_filter_is_priced_at_the_rows_it_passes_on() {
    let p = parse_version(".decl a(x: number)\n.decl r(x: number)\nr(x) :- a(x), x != 3, x < 6917529027641081856.\n.output r\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    model.stats.card.insert("a", 1_000.0);
    model.ndv.insert(("a", 0), 1_000.0);
    model.stats.fan.insert(("a", 0), 1.0);
    let r = 0usize;
    let (order, work, rows) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let shown: Vec<String> = order.iter().map(|s| lit_at(&p, r, s.at)).collect();
    let want = 1_000.0 + 1_000.0 * 0.97 + 1_000.0 * 0.97 * 0.98;
    assert!((work - want).abs() < 1e-6 * want, "work {work}, wanted {want}: {shown:?}");
    assert!((rows - 1_000.0 * 0.97 * 0.98).abs() < 1e-6 * rows, "rows out {rows}");
}

/// A model with no statistics but those each test sets (decision 0299's tests).
fn bare_model(p: &Arc<Version>) -> Model {
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()}
}

/// `(order shown, work)` of `p`'s first rule.
fn priced_first_rule(model: &Model, p: &Arc<Version>) -> (Vec<String>, f64) {
    let r = 0usize;
    let (order, work, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(p, r), None).unwrap();
    (order.iter().map(|s| lit_at(p, r, s.at)).collect(), work)
}

/// Work queue Q240: past the step that binds the head's last variable the walk needs one witness, and the search
/// prices it so, as a fixed order's pricing does. `xk(v) :- m(v, n), l(n, y), c(n, z), k(v)` (a findings key: a
/// document's nodes by `m`) starts from `k`'s 300 documents, each a row through `m`, `l` and `c`. Priced at `m`'s
/// fanout by document (3,000), the search took `c`'s every node first: 19 M rows a step in half of PostHog's chunks.
/// A cross product is not priced so: `k(v), c(n, z)` reads `c` until a row passes the rest, maybe all of it.
#[test]
fn the_search_prices_the_steps_past_a_bound_head_as_one_witness() {
    let p = parse(".decl m(v: number, n: number)\n.decl l(n: number, y: number)\n.decl c(n: number, z: number)\n.decl k(v: number)\n.decl xk(v: number)\nxk(v) :- m(v, n), l(n, y), c(n, z), k(v).\n.output xk\n").unwrap();
    let mut model = bare_model(&p);
    for (rel, rows, cols) in [("m", 250_000.0, vec![(300.0, 3_000.0), (250_000.0, 1.0)]), ("l", 250_000.0, vec![(250_000.0, 1.0), (1_000.0, 250.0)]), ("c", 250_000.0, vec![(250_000.0, 1.0), (100.0, 2_500.0)]), ("k", 300.0, vec![(300.0, 1.0)])] {
        model.stats.card.insert(rel, rows);
        for (c, (ndv, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel, c), ndv);
            model.stats.fan.insert((rel, c), fan);
        }
    }
    model.stats.nodes = 250_000.0;
    let (order, work) = priced_first_rule(&model, &p);
    assert!(order[0].starts_with("k("), "the documents lead: {order:?} at {work}");
    assert!(order[1].starts_with("m("), "then the documents' nodes, not a cross product: {order:?} at {work}");
    assert!(work < 2_000.0, "a row a document through each step: {work} {order:?}");
}

/// Decision 0299: `within(n, a)` read from its container yields the container's descendants: the mean size of a
/// node with children (`Stats::subtree_fan`), not the view's fanout, which is of the lookups made so far.
#[test]
fn within_read_from_its_container_is_priced_by_subtree_size() {
    let p = parse_version(".decl c(t: number)\n.decl within(n: number, a: number)\n.decl r(n: number)\nr(n) :- c(t), within(n, t).\n.output r\n").unwrap();
    let mut model = bare_model(&p);
    // 1,000 leaves and 10 nodes of 50.
    model.stats.size = std::sync::Arc::new(HashMap::from([("1".to_owned(), 1.0), ("2".to_owned(), 50.0)]));
    model.stats.kfreq = std::sync::Arc::new([("1".to_owned(), 1_000), ("2".to_owned(), 10)].into_iter().collect());
    model.stats.nodes = 1_010.0;
    assert_eq!(model.stats.subtree_fan(), Some(50.0));
    model.stats.card.insert("c", 10.0);
    model.ndv.insert(("c", 0), 10.0);
    model.stats.fan.insert(("c", 0), 1.0);
    model.stats.card.insert("within", 1_000.0);
    for (c, ndv, fan) in [(0, 400.0, 2.5), (1, 70.0, 14.0)] {
        model.ndv.insert(("within", c), ndv);
        model.stats.fan.insert(("within", c), fan);
    }
    let (order, work) = priced_first_rule(&model, &p);
    assert_eq!(order, ["c($t)", "within($n, $t)"]);
    assert!((work - (10.0 + 10.0 * 50.0)).abs() < 1e-6, "work {work}");
}

/// t50 step 4: `within(n, b)` read from a part of a node of known kind (`field($g, "body", $b)` of a function) is
/// priced at that kind's size: the part's subtree is at most its parent's. As a node with children it was a few nodes,
/// and `rf_in` enumerated every function's body from it before checking the read's own function.
#[test]
fn within_read_from_a_part_is_priced_by_its_parents_size() {
    let src = ".decl kind(n: number, k: symbol)\n.decl field(p: number, f: symbol, c: number)\n.decl within(n: number, a: number)\n.decl r(n: number)\nr(n) :- kind(g, \"fdef\"), field(g, \"body\", b), within(n, b).\n.output r\n";
    let p = parse(src).unwrap();
    let mut model = bare_model(&p);
    // 10 functions of 400 nodes, 2,000 nodes of 3: a node with children is about 5 nodes.
    model.stats.size = std::sync::Arc::new(HashMap::from([("fdef".to_owned(), 400.0), ("x".to_owned(), 3.0)]));
    model.stats.kfreq = std::sync::Arc::new([("fdef".to_owned(), 10), ("x".to_owned(), 2_000)].into_iter().collect());
    model.stats.nodes = 6_010.0;
    assert!(model.stats.subtree_fan().is_some_and(|f| f < 6.0));
    let (order, work) = priced_first_rule(&model, &p);
    assert_eq!(order.last().map(String::as_str), Some("within($n, $b)"), "{order:?}");
    // The body's descendants, each function's: 10 × 400, not 10 × 5.
    assert!(work >= 10.0 * 400.0, "work {work}: {order:?}");
}

/// Decision 0299: a relation of the program's facts alone has its rows, distinct values and counts before it is
/// made, not the default size.
#[test]
fn a_relation_of_facts_is_sized_when_planned() {
    let p = parse_version(".decl ck(k: number, g: number)\n.decl a(c: number, k: number)\n.decl r(c: number)\nck(5, 1). ck(6, 1). ck(7, 2).\nr(c) :- ck(k, _), a(c, k).\n.output r\n").unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    stats.card.insert("a", 1_000.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    assert_eq!(model.stats.card.get("ck").copied(), Some(3.0));
    assert_eq!(model.ndv.at("ck", 1).copied(), Some(2.0));
    let groups = model.stats.dfreq.at("ck", 1).expect("the facts' counts");
    assert_eq!(groups.get(&Constant::Int(1)).copied(), Some(2));
}

/// Decision 0299: `kind($c, $k)` with `$k` bound by an atom whose values are counted reads those kinds' nodes,
/// each as often as it binds: (10 + 30) / 2 a binding, not every node over the kinds.
#[test]
fn a_kind_bound_by_counted_values_reads_their_nodes() {
    let p = parse_version(".decl ck(k: number)\n.decl kind(n: number, k: number)\n.decl r(c: number)\nr(c) :- ck(k), kind(c, k).\n.output r\n").unwrap();
    let mut model = bare_model(&p);
    model.stats.kfreq = std::sync::Arc::new([("5".to_owned(), 10), ("6".to_owned(), 30), ("7".to_owned(), 10_000)].into_iter().collect());
    model.stats.card.insert("ck", 2.0);
    model.ndv.insert(("ck", 0), 2.0);
    model.stats.fan.insert(("ck", 0), 1.0);
    model.stats.dfreq.insert(("ck".to_owned(), 0), HashMap::from([(Constant::Int(5), 1), (Constant::Int(6), 1)]));
    model.stats.card.insert("kind", 10_040.0);
    let (order, work) = priced_first_rule(&model, &p);
    assert_eq!(order, ["ck($k)", "kind($c, $k)"]);
    assert!((work - (2.0 + 2.0 * 20.0)).abs() < 1e-6, "work {work}");
}

/// Q220: a variable an equality binds to a constant joins nothing. `kind($m, $k)` after `$k = 5` shares only `$k`
/// with what is joined, so the greedy order takes it only once `$m` is joined, not while `text($n, $s)` joins:
/// `p299_attempt`'s kept greedy order (planned where its text test matched little) read every node of the kind for
/// each of 15 k texts (1.16 M rows, none kept), the kind priced at 11 a binding against the text's 45. Sized so the
/// greedy order prices under a search's work and is taken (`search_rows`: 432 rows for 4 joins).
#[test]
fn a_variable_bound_to_a_constant_does_not_connect_a_step() {
    let p = parse(".decl t(s: number)\n.decl kind(n: number, k: number)\n.decl child(p: number, c: number)\n.decl text(n: number, s: number)\n.decl r(m: number)\nr(m) :- t(s), k = 5, kind(m, k), child(m, n), text(n, s).\n.output r\n").unwrap();
    let mut model = bare_model(&p);
    model.stats.kfreq = std::sync::Arc::new([("5".to_owned(), 3), ("6".to_owned(), 9_997)].into_iter().collect());
    model.stats.nodes = 10_000.0;
    for (rel, rows, cols) in [("t", 2.0, vec![(2.0, 1.0)]), ("kind", 10_000.0, vec![(10_000.0, 1.0), (2.0, 5_000.0)]),
                              ("child", 10_000.0, vec![(5_000.0, 2.0), (10_000.0, 1.0)]), ("text", 4_500.0, vec![(4_500.0, 1.0), (1_125.0, 4.0)])] {
        model.stats.card.insert(rel, rows);
        for (c, (ndv, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel, c), ndv);
            model.stats.fan.insert((rel, c), fan);
        }
    }
    let (order, work) = priced_first_rule(&model, &p);
    let at = |s: &str| order.iter().position(|o| o == s).unwrap_or_else(|| panic!("{s} in {order:?}"));
    assert_eq!(order[0], "t($s)", "{order:?}");
    assert!(at("text($n, $s)") < at("kind($m, $k)") && at("child($m, $n)") < at("kind($m, $k)"),
            "`kind($m, $k)` waits for `$m`, joined through `$s`: {order:?} (work {work})");
}

/// Decision 0320: a node bound by a field is of a kind as often as the field's children are, and a field read by a
/// node of a kind yields that kind's children by it. Every child by field 7 is of kind 5 here, a hundredth of the
/// nodes: the kind's share of the nodes passed one row in a hundred, where every row passes.
#[test]
fn a_kind_after_a_field_is_priced_by_the_fields_children() {
    let p = parse_version(".decl field(p: number, f: number, c: number)\n.decl kind(n: number, k: number)\n.decl r(c: number)\nr(c) :- field(p, 7, c), kind(c, 5).\n.output r\n").unwrap();
    let model = |fielded: bool| {
        let mut model = bare_model(&p);
        model.stats.kfreq = std::sync::Arc::new([("5".to_owned(), 100), ("6".to_owned(), 9_900)].into_iter().collect());
        model.stats.nodes = 10_000.0;
        model.stats.card.insert("kind", 10_000.0);
        model.stats.card.insert("field", 1_000.0);
        model.stats.dfreq.insert(("field".to_owned(), 1), HashMap::from([(Constant::Int(7), 100), (Constant::Int(8), 900)]));
        if fielded {
            model.stats.fielded([("6", "5", "7", 100.0), ("6", "6", "8", 900.0)].map(|(p, c, f, n)| (p.to_owned(), c.to_owned(), f.to_owned(), n)));
        }
        model
    };
    // Either order reads 100 rows and keeps them: field 7's edges, then all of them of kind 5; or kind 5's nodes, each
    // a child by field 7 once.
    let (_, work) = priced_first_rule(&model(true), &p);
    assert!((work - 200.0).abs() < 1e-6, "work {work}");
    let (_, independent) = priced_first_rule(&model(false), &p);
    assert!(independent < 150.0, "without the fields' kinds a kind passes its share of the nodes: {independent}");
    let f = &model(true).stats.fields;
    assert_eq!(f.child_share("5", "7"), Some(1.0));
    assert_eq!(f.parent_share("6", "7"), Some(1.0));
    assert_eq!(f.joint_share("6", "5", "8", false), Some(0.0));
    assert_eq!(f.edges("5", "7", false), Some(100.0));
    assert_eq!(f.child_share("5", "9"), None, "an unmeasured field says nothing");
}

/// Q44: a relation of the component being solved has no measured distinct values yet. A lookup of it by a column
/// is priced at PostgreSQL's prior (200 values a column, `ndv_or_default`), not one value: as one, every key was the
/// whole relation, and a transitive closure's self-join was planned at its relation's rows a binding. A relation not
/// being solved keeps one value, and is read whole, then probed back.
#[test]
fn a_lookup_of_a_relation_being_solved_is_priced_by_the_distinct_prior() {
    let p = parse(".decl d(x: number)\n.decl w(x: number, s: number)\n.decl r(s: number)\nr(s) :- d(x), w(x, s).\n.output r\n").unwrap();
    let model = |solving: bool| {
        let mut model = bare_model(&p);
        model.stats.nodes = 4_000.0;
        model.stats.card.insert("d", 10.0);
        model.ndv.insert(("d", 0), 10.0);
        model.stats.fan.insert(("d", 0), 1.0);
        model.stats.card.insert("w", 4_000.0);
        if solving {
            model.stats.solving.insert("w".to_owned());
        }
        model
    };
    // `d`'s 10 rows, each probing `w` by `x`: 4,000 rows / 200 values, 20 a key.
    let (order, work) = priced_first_rule(&model(true), &p);
    assert_eq!(order[0], "d($x)");
    assert!((work - 210.0).abs() < 1.0, "work {work}");
    // Not being solved, `w`'s column is one value: a probe by it reads all 4,000 rows, so `w` is read first.
    let (order, work) = priced_first_rule(&model(false), &p);
    assert_eq!(order[0], "w($x, $s)", "work {work}");
    assert!(work >= 8_000.0, "work {work}");
}

/// Decision 0299: a probe on several bound columns reads at least the rows of one combination the relation holds.
/// Independence divided `w`'s two rows a node by its 400 names, though a node has one name.
#[test]
fn a_probe_on_several_columns_reads_a_combinations_rows() {
    let p = parse_version(".decl d(n: number, x: number)\n.decl w(n: number, x: number, s: number)\n.decl r(s: number)\nr(s) :- d(n, x), w(n, x, s).\n.output r\n").unwrap();
    let mut model = bare_model(&p);
    model.stats.card.insert("d", 100.0);
    for c in 0..2 {
        model.ndv.insert(("d", c), 100.0);
        model.stats.fan.insert(("d", c), 1.0);
    }
    model.stats.card.insert("w", 1_000.0);
    for (c, ndv, fan) in [(0, 500.0, 2.0), (1, 400.0, 2.5), (2, 50.0, 20.0)] {
        model.ndv.insert(("w", c), ndv);
        model.stats.fan.insert(("w", c), fan);
    }
    let (order, work) = priced_first_rule(&model, &p);
    assert_eq!(order, ["d($n, $x)", "w($n, $x, $s)"]);
    assert!((work - (100.0 + 100.0 * 1.0)).abs() < 1e-6, "work {work}");
}

/// Decision 0299's floor holds where the probed variables can determine each other: bound by one atom, or by atoms
/// joined to each other. Bound by atoms nothing joins (a cross product), each pair of values is one of the bindings'
/// product, and most pairs are in no row. corpus0 (2026-11-21): `open_tuple(t) :- kind(t, "tuple"), unparen(c, m),
/// …, child(t, c, _)`, `$t` bound by `kind` and `$c` by `unparen`, priced `child` at a row a binding, so every one of
/// the 2.1 M pairs had a witness: the order was priced at 753 rows and read 41 M, 790 of them a witness.
#[test]
fn a_probe_on_columns_bound_by_unjoined_atoms_is_not_floored_at_a_combination() {
    let p = parse_version(".decl k(t: number)\n.decl u(c: number, m: number)\n.decl ch(t: number, c: number, o: number)\n.decl h(t: number)\nh(t) :- k(t), u(c, m), ch(t, c, o).\n.output h\n").unwrap();
    let mut model = bare_model(&p);
    for (rel, rows, cols) in [("k", 91.0, vec![(91.0, 1.0)]), ("u", 23_813.0, vec![(23_813.0, 1.0), (23_813.0, 1.0)]), ("ch", 70_000.0, vec![(23_000.0, 3.0), (70_000.0, 1.0), (10.0, 7_000.0)])] {
        model.stats.card.insert(rel, rows);
        for (c, (ndv, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel, c), ndv);
            model.stats.fan.insert((rel, c), fan);
        }
    }
    model.stats.nodes = 70_000.0;
    let placed = body::Placed::whole(p.clone());
    let pf = placed.fields().unwrap();
    let b = body::nth(&p, 0);
    let crossed: Vec<Step> = (0..3).map(|at| Step { at, mode: None }).collect();
    let explain = explaining(true);
    let x = model.explain_order(pf, &b, None, &crossed).expect("the order is priced");
    drop(explain);
    let (cross_work, rows) = (x.work, x.rows);
    let pairs = 91.0 * 23_813.0;
    // At most the least share a probe is priced at (a hundredth of its bindings), not a row each.
    assert!(rows <= pairs / 100.0 * (1.0 + 1e-9), "few of the {pairs} pairs are rows of `ch`: priced {rows}");
    let (order, work) = priced_first_rule(&model, &p);
    assert_eq!(order[1], "ch($t, $c, $o)", "`ch` probed by `$t`, not a cross product: {order:?} at {work}");
    assert!(cross_work > 10.0 * work, "the cross product prices over the joined order: {cross_work} against {work}");
}

#[test]
fn live_statistics_invalidate_only_changed_distribution_slots() {
    let p=parse_version(".decl a(x:number,y:number)\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Default::default()};
    let sample=|key| LiveRel {n:10,delta:2,ndv:vec![10,1],sq:vec![10,100],freq:vec![(1,HashMap::from([(Constant::Int(key),10)]))],keys:Vec::new(),counts:Vec::new()};
    assert_eq!(live_stats(&mut model,"a",sample(1)),vec![0,1,2]);
    assert!(live_stats(&mut model,"a",sample(1)).is_empty());
    assert_eq!(live_stats(&mut model,"a",sample(2)),vec![2],"equal counts and fanouts with new keys must invalidate");
}

#[test]
fn projection_statistics_keep_demanded_constants_only_when_rows_are_preserved() {
    let p=parse_version(".decl raw(d:number,n:number,k:number)\n.input raw\n.decl tags(n:number,k:number)\ntags(n,k) :- raw(_,n,k).\n.decl filtered(n:number,k:number)\nfiltered(n,k) :- raw(1,n,k).\n.decl kinds(k:number)\nkinds(k) :- raw(_,_,k).\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Arc::new(p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect::<HashSet<String>>())};
    model.stats.card.insert("raw",7000.0);
    for (c,n) in [10.0,7000.0,20.0].into_iter().enumerate() {
        model.ndv.insert(("raw",c),n); model.stats.fan.insert(("raw",c),7000.0/n);
    }
    model.stats.memo.insert("raw".into(),HashMap::from([(("c0".into(),Constant::Int(1)),100),(("c2".into(),Constant::Int(9)),12)]));
    for k in 0..p.rules().count() { let b = body::nth(&p, k); model.view_stats(b.rel(b.head()), &b); }
    assert_eq!(model.stats.const_freq("tags","c1",&Constant::Int(9)),Some(12));
    assert_eq!(model.stats.const_freq("filtered","c1",&Constant::Int(9)),None,"filtered marginal counts are not exact");
    assert_eq!(model.stats.const_freq("kinds","c0",&Constant::Int(9)),None,"deduplicating projection changes frequencies");
}

#[test]
fn statistics_versions_accumulate_drift_and_publish_changed_keys() {
    let p=parse_version(".decl a(x:number,y:number)\n.decl b(x:number)\n.decl p(x:number)\np(x) :- a(x,_), b(x).\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Default::default()};
    let mut watch=replanning::Watch::new(&BTreeMap::from([("a".into(),0)]),&[(body::nth(&p,0),0)]);
    watch.delta(0,5);
    model.stats.card.insert("a",100.0); watch.record(0,0,&model,"a"); watch.record(0,1,&model,"a");
    assert!(watch.take(0));
    model.stats.card.insert("a",150.0); watch.record(0,0,&model,"a");
    assert!(!watch.take(0),"small drift keeps the priced signature");
    model.stats.card.insert("a",210.0); watch.record(0,0,&model,"a");
    assert!(watch.take(0),"small changes accumulate against the last published size");
    for key in [1,2] {
        model.stats.dfreq.insert(("a",0),HashMap::from([(Constant::Int(key),210)]));
        watch.record(0,1,&model,"a");
        assert!(watch.take(0),"new key domains publish even at identical row counts");
    }
    model.stats.fan.insert(("a",1),999.0); watch.record(0,2,&model,"a");
    assert!(!watch.take(0),"an unused wildcard column has no dependent plan");
    watch.delta(0,0);
    model.stats.card.insert("a",1000.0); watch.record(0,0,&model,"a");
    assert!(!watch.take(0)); watch.delta(0,1); assert!(watch.take(0));
}

/// Decision 0123 §7: an order kept at a price under a search's cost (taken greedily, or found by a search) is searched
/// again only when its line did a search's work when it last ran: the search then costs less than the line did.
#[test]
fn an_order_priced_under_a_search_is_searched_once_it_ran_over_one() {
    let p = parse_version(".decl a(x:number,y:number)\n.decl b(y:number,z:number)\n.decl c(z:number,w:number)\n.decl d(w:number)\nd(w) :- a(x,y), b(y,z), c(z,w).\n").unwrap();
    let (body, first) = (&body::nth(&p, 0), Some(0));
    let search = search_rows(body, first);
    // Three joins led by the delta: a fixed 400 rows and five candidates.
    assert_eq!(search, 405.0);
    assert!(!mispriced(10.0, None, body, first), "a line that never ran");
    assert!(!mispriced(10.0, Some(search.max(SEARCH_WORK) - 1.0), body, first), "it ran under a search's work");
    assert!(mispriced(10.0, Some(search.max(SEARCH_WORK)), body, first), "it ran a search's work, priced at 10 rows");
    assert!(!mispriced(search + 1.0, Some(1e9), body, first), "priced over a search: the re-price and replan rules hold it");
}

#[test]
fn a_plan_priced_on_an_empty_relation_is_not_kept_once_it_fills() {
    // Within the drift, both ways; an empty relation against one row is not (a cross product after it was free).
    assert!(near(&[1.0, 10.0], &[1.5, 19.0]));
    assert!(near(&[0.0, 10.0], &[0.0, 10.0]));
    assert!(!near(&[0.0, 10.0], &[1.0, 10.0]), "empty, then one row");
    assert!(!near(&[1.0], &[0.0]), "one row, then empty");
}

#[test]
fn child_ordinal_lookup_is_unique_despite_hot_marginal_statistics() {
    let p = parse_version(".decl parents(p:number)\n.input parents\n.decl child(p:number,c:number,idx:number)\n.input child\n.decl out(c:number)\nout(c) :- parents(p), child(p,c,0).\n").unwrap();
    let mut model = Model { shapes: Default::default(), docs_by_shift:false, analysis:Default::default(),         stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()), ndv: Default::default(), docof:Default::default(), view_src:Default::default(),
        inputs:Inputs::file().into(), derived:Default::default() };
    model.stats.card.insert("parents",100.0);
    model.ndv.insert(("parents",0),100.0);
    model.stats.card.insert("child",10000.0);
    model.ndv.insert(("child",0),100.0);
    model.ndv.insert(("child",1),10000.0);
    model.ndv.insert(("child",2),100.0);
    model.stats.fan.insert(("child",0),100.0);
    model.stats.memo.insert("child".into(),HashMap::from([(("idx".into(),Constant::Int(0)),9000)]));
    let (order,work,rows) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, 0),None).unwrap();
    assert_eq!(rel_at(&p, 0, order[0].at).unwrap(),"parents");
    assert!(rows<=100.0 && work<=400.0,"a parent/ordinal lookup has at most one result: {rows} rows, {work} work");
}

/// A planner change's measurement (O148): T's programs on T's facts, each statement's SQLite VM steps, the plans
/// chosen and whether each output is T's. Run on a build before the change and one after (`O148_OUT`, with
/// `RERAIL_NODE_IDS=preorder`) and compare them (`scripts/profiling/o148/compare.py`): the goldens (`rules.txt`) pin
/// plans that did no more work than the ones they replace.
#[test]
#[ignore = "measurement: a planner change's plans and their work, compared across two builds"]
fn o148_plans_and_their_work() {
    use rusqlite::ffi;
    use std::sync::Mutex;
    static COSTS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());
    unsafe extern "C" fn trace(event: u32, _: *mut std::ffi::c_void, p: *mut std::ffi::c_void, x: *mut std::ffi::c_void) -> std::ffi::c_int {
        if event == ffi::SQLITE_TRACE_PROFILE {
            let stmt = p.cast::<ffi::sqlite3_stmt>();
            // SAFETY: for SQLITE_TRACE_PROFILE, P is the statement and X its time in nanoseconds.
            let (ns, vm, text) = unsafe {
                (*x.cast::<i64>() as u64, ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 1) as u64,
                 std::ffi::CStr::from_ptr(ffi::sqlite3_sql(stmt)).to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" "))
            };
            COSTS.lock().unwrap().push((text, vm, ns));
        }
        0
    }
    let out = std::env::var("O148_OUT").expect("O148_OUT: the file to write");
    let mut lines = Vec::new();
    let facts = facts();
    for name in ["workload", "workload_demand", "calls"] {
        let db = Connection::open_in_memory().unwrap();
        db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())]).unwrap();
        db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;").unwrap();
        // SAFETY: the connection outlives the run; the callback reads only its arguments.
        unsafe { ffi::sqlite3_trace_v2(db.handle(), ffi::SQLITE_TRACE_PROFILE, Some(trace), std::ptr::null_mut()) };
        COSTS.lock().unwrap().clear();
        let got = super::run(&body::Placed::whole(program(name)), &db, Inputs::file(), None, Hooks::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        for c in &got.chosen {
            lines.push(format!("PLAN\t{name}\t{}", chosen_line(c)));
        }
        for (rel, rows) in &got.shown() {
            let expected = std::fs::read_to_string(root().join(format!("plan/golden/{name}/{rel}.csv"))).unwrap();
            let same = golden_form(rows) == expected.lines().collect::<Vec<_>>();
            lines.push(format!("OUT\t{name}\t{rel}\t{}\t{same}", rows.len()));
        }
        for (text, vm, ns) in COSTS.lock().unwrap().drain(..) {
            lines.push(format!("STMT\t{name}\t{vm}\t{ns}\t{text}"));
        }
    }
    std::fs::write(out, lines.join("\n") + "\n").unwrap();
}

#[test]
fn empty_required_inputs_defer_both_plan_shapes_until_growth() {
    let program = crate::code_rule_dsl::logic::rr::parse_version(
        ".decl a(x:number)\n.decl b(x:number)\n.decl p(x:number)\np(x) :- a(x), b(x).\n").unwrap();
    let mut w = replanning::Watch::new(&BTreeMap::from([("a".into(),0),("b".into(),1)]), &[(body::nth(&program,0),0)]);
    w.rows(0,100); w.rows(1,0); w.delta(0,100);
    assert!(!w.take(0));
    assert!(!w.take_whole(0), "a whole join over an empty input needs no search");
    w.rows(1,1);
    assert!(w.take(0)); assert!(w.take_whole(0));
    assert!(!w.take(0)); assert!(!w.take_whole(0));
}

/// A kept plan's distributions at another scale (O-replan-churn): a count of node ids in proportion to the nodes, a
/// column of few values as it was, and the bit per literal as it is.
#[test]
fn kept_distributions_are_near_in_proportion_to_the_nodes() {
    use super::near_distributions;
    let scale = 61997.0 / 22146.0;
    // ndv of node ids, fan, a kind's ndv; then one literal's bit.
    assert!(near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[61994.0, 1.0, 85.0, 1.0], scale, 1));
    assert!(!near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[61994.0, 1.0, 85.0, 4.0], scale, 1), "a literal's bit moved");
    assert!(!near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[400000.0, 1.0, 85.0, 1.0], scale, 1), "grew far past the scale");
    assert!(!near_distributions(&[-2.0, 1.0], &[5.0, 1.0], scale, 1), "proven empty against present");
}

/// O-join-shape-directives: `pv__ins`'s shape at O125 in small. With `retparam` tiny, the planner crosses `pv` with
/// it (no shared variable) before `posarg`; `.shape retparam bound f` holds it back until `cfn` binds `f`, in the rule
/// as written. With `posarg` sized by no statistic of its own, the guard takes
/// no cross product against it.
#[test]
fn a_join_shape_directive_holds_a_join_back_in_its_rule() {
    let rule = ".decl pv(a:number,k:number,v:number)\n.decl posarg(c:number,i:number,a:number)\n.decl cfn(c:number,e:number,f:number)\n\
        .decl retparam(f:number,i:number,x:number)\n.input pv, posarg, cfn, retparam\n.decl out(c:number,k:number,v:number)\n.output out\n\
        out(c, k, v) :- pv(a, k, v), posarg(c, i, a), cfn(c, _, f), retparam(f, i, _).\n";
    let model_for = |p: &Arc<Version>, sized: &[(&str, f64, Vec<f64>)]| {
        let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
        let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
        let mut model = Model { shapes: shapes_of(&body::Placed::whole(p.clone()).fields().unwrap()), docs_by_shift:false, analysis:Default::default(), stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()), ndv: Default::default(), docof:Default::default(), view_src:Default::default(), inputs:inputs.into(), derived:derived.into() };
        for (rel, rows, ndv) in sized {
            model.stats.card.insert(*rel, *rows);
            for (c, n) in ndv.iter().enumerate() {
                model.ndv.insert((*rel, c), *n);
                model.stats.fan.insert((*rel, c), rows / n);
            }
        }
        model.stats.nodes = 300000.0;
        model
    };
    let sizes = vec![("pv", 1000.0, vec![1000.0, 50.0, 500.0]), ("posarg", 100000.0, vec![50000.0, 5.0, 100.0]), ("cfn", 50000.0, vec![50000.0, 100.0, 2000.0]), ("retparam", 2.0, vec![2.0, 2.0, 2.0])];
    let order_of = |text: &str, sized: &[(&str, f64, Vec<f64>)], head: Option<&str>, first: Option<usize>| -> Vec<String> {
        let mut p = parse_version(text).unwrap();
        if let Some(h) = head {
            // A delta's form of the rule: its head renamed as maintenance names it.
            p = renamed_head(&p, h);
        }
        let model = model_for(&p, sized);
        let _plans = KeepPlans::start(None);
        let r = 0usize;
        let (order, _, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), first).unwrap();
        order.iter().filter_map(|s| rel_at(&p, r, s.at)).collect()
    };
    let at = |o: &[String], r: &str| o.iter().position(|x| x == r).unwrap();
    let free = order_of(rule, &sizes, None, None);
    assert!(at(&free, "retparam") < at(&free, "cfn"), "without the directive the planner crosses retparam in early: {free:?}");
    let shaped = format!("{rule}.shape retparam bound f\n");
    // (A delta's form of the rule, `out__ins`, was held back by the same directive; the forms are gone, 0364.)
    let o = order_of(&shaped, &sizes, None, None);
    assert!(at(&o, "retparam") > at(&o, "cfn"), "retparam waits for f: {o:?}");
    // Profiled, a directive the planner's own order already keeps is reported as no longer needed; one that changed
    // the order is reported with both estimates.
    {
        struct Off;
        impl Drop for Off {
            fn drop(&mut self) {
                crate::code_rule_dsl::logic::profile::set(false);
            }
        }
        // The test's own run, whose profile its runs record (decision 0442): a run in another test never reaches the
        // decisions it reads (`no longer needed` went missing when one ran beside the process's). Dropped after `Off`.
        let _one = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let _off = Off;
        crate::code_rule_dsl::logic::profile::set(true);
        crate::code_rule_dsl::logic::profile::reset();
        // EXPLAIN says which directive shaped the rule's plan.
        {
            let p = parse_version(&shaped).unwrap();
            let model = model_for(&p, &sizes);
            let _plans = KeepPlans::start(None);
            let r = 0usize;
            let (_, x) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
            assert_eq!(x.map(|x| x.shaped), Some(vec!["retparam bound f".to_owned()]));
        }
        let _ = order_of(&format!("{rule}.shape pv after posarg\n"), &sizes, None, None);
        let _ = order_of(&shaped, &sizes, None, None);
        let made: Vec<(String, String)> = crate::code_rule_dsl::logic::profile::decisions().into_keys().filter(|(t, _)| t.contains("join shape directive")).collect();
        crate::code_rule_dsl::logic::profile::set(false);
        assert!(made.iter().any(|(t, x)| t.contains("no longer needed") && x.contains("pv after posarg")), "{made:?}");
        assert!(made.iter().any(|(t, x)| t.starts_with("join shape directives") && x.contains("retparam bound f")), "{made:?}");
    }
    let after = order_of(&format!("{rule}.shape retparam after cfn\n"), &sizes, None, None);
    assert!(at(&after, "retparam") > at(&after, "cfn"), "{after:?}");
    // The directive names what its rule reads.
    let err = format!("{:?}", parse_version(&format!("{rule}.shape nothing after cfn\n")).unwrap_err());
    assert!(err.contains("no rule of `out` reads `nothing`"), "{err}");
    let err = format!("{:?}", parse_version(".shape retparam bound f\n").unwrap_err());
    assert!(err.contains("follows no rule"), "{err}");
    // No directive, `pv`'s delta driving, and `posarg` unsized: no cross product priced against its default.
    let no_posarg: Vec<(&str, f64, Vec<f64>)> = sizes.iter().filter(|s| s.0 != "posarg").cloned().collect();
    let guarded = order_of(rule, &no_posarg, None, Some(0));
    assert!(at(&guarded, "retparam") > at(&guarded, "posarg"), "{guarded:?}");
}

/// A computed value waits for the atoms still to join that read its arguments and not the value: they only narrow the
/// rows, so the value is computed (and its text interned) for the rows kept. `def_name`'s long-text twin read the text
/// of every node past the bound, then kept none at `field($f, "name", $n)`.
#[test]
fn a_computed_value_waits_for_the_atoms_that_narrow_its_arguments() {
    let p = parse_version(
        ".decl p(s: symbol)\n.decl q(s: symbol)\n.decl o(s: symbol, n: number)\n\
         p(\"a\"). p(\"bb\"). p(\"ccc\"). q(\"bb\").\n\
         o(s, n) :- p(s), n = @strlen(s), q(s).\n.output o\n",
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    stats.card.insert("p", 10.0);
    stats.card.insert("q", 14_000.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    let r = 0usize;
    let (order, _, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let shown: Vec<String> = order.iter().map(|s| lit_at(&p, r, s.at)).collect();
    let at = |prefix: &str| shown.iter().position(|s| s.starts_with(prefix)).unwrap_or_else(|| panic!("{prefix} in {shown:?}"));
    assert_eq!(at("p("), 0, "{shown:?}");
    assert!(at("$n = ") > at("q("), "the value is computed after q narrows: {shown:?}");
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()));
    let out = run(&body::Placed::whole(p.clone()), &db, inputs, None, Hooks::default()).unwrap();
    assert_eq!(out.shown()["o"].len(), 1, "{:?}", out.shown()["o"]);
}

/// A demanded relation's rule may be led by an atom a constant restricts, when it is smaller than the demand: it
/// reads one kind's nodes, not the corpus (Q59's guard is against a scan). Forced to its demand always, `not_name`
/// on 50 files of B-now did 66x the work (a 42 k-row demand, a kind of 498 nodes).
#[test]
fn a_demanded_rule_may_be_led_by_a_constant_restricted_atom() {
    let p = parse(
        ".decl x(v: number)\n.decl demand_x(v: number)\n.decl k(n: number, c: number)\n\
         demand_x(1). k(1, 7).\n\
         x(v) :- demand_x(v), k(v, 7).\n.output x\n",
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    stats.card.insert("demand_x", 5_000_000.0);
    stats.card.insert("k", 10.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    let r = p.rule(crate::code_rule_dsl::logic::program_image::RuleId(0)).unwrap();
    let lits: Vec<_> = r.body().collect();
    let b = body::nth(&p, 0);
    let (order, _, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &b, None).unwrap();
    let rels: Vec<&str> = order.iter().filter_map(|s| lits[s.at].atom()).map(|a| p.rel_name(a.rel())).collect();
    assert_eq!(rels[0], "k", "led by the restricted atom: {rels:?}");
    assert!(model.valid_live_driver(&order, &b, None), "a kept order led by it is kept");
}

/// A deferred join's request (`rr_deferred_pairs_x`, decision 0488) bounds its producer as a demand does (Q59): an
/// atom a constant restricts may lead it, as corpus0's `lambda_arg` led by its lambdas (229), not its 14,360
/// requested calls; an unrestricted atom may not.
#[test]
fn a_deferred_joins_producer_may_be_led_by_a_constant_restricted_atom() {
    let program = |body: &str| parse(&format!(
        ".decl x(v: number)\n.decl rr_deferred_pairs_x(v: number)\n.decl k(n: number, c: number)\n.decl col(n: number, c: number)\n\
         rr_deferred_pairs_x(1). k(1, 7). col(1, 2).\n\
         x(v) :- rr_deferred_pairs_x(v), {body}.\n.output x\n"
    )).unwrap();
    let lead = |body: &str| {
        let p = program(body);
        let db = Connection::open_in_memory().unwrap();
        let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
        stats.card.insert("rr_deferred_pairs_x", 5_000_000.0);
        stats.card.insert("k", 10.0);
        stats.card.insert("col", 10.0);
        let mut inputs = Inputs::file();
        inputs.facts.clear();
        inputs.given = Some(stats);
        let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
        let (order, _, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, 0), None).unwrap();
        let rule = p.rules().next().unwrap();
        rule.body().nth(order[0].at).and_then(|l| l.atom()).map(|a| p.rel_name(a.rel()).to_owned()).unwrap()
    };
    assert_eq!(lead("k(v, 7)"), "k", "led by the restricted atom");
    assert_eq!(lead("col(v, c)"), "rr_deferred_pairs_x", "an unrestricted atom does not lead");
}

/// A demanded relation's rule (decision 0048's split: `x` computed for `demand_x`'s keys) is driven by its demand,
/// whatever the statistics say (Q59): the guard is structural. Priced here as the outlier run saw it (the demand
/// large, a scan of `col` small), the search still starts from `demand_x`, and a kept order that does not is not
/// taken as it is.
#[test]
fn a_demanded_rule_is_driven_by_its_demand() {
    let p = parse(
        ".decl x(v: number)\n.decl demand_x(v: number)\n.decl col(n: number, c: number)\n.decl line(n: number, l: number)\n\
         demand_x(1). col(1, 2). line(1, 3).\n\
         x(v) :- demand_x(v), col(v, c), line(v, l), c < l.\n.output x\n",
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    stats.card.insert("demand_x", 5_000_000.0);
    stats.card.insert("col", 10.0);
    stats.card.insert("line", 10.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    let r = p.rule(crate::code_rule_dsl::logic::program_image::RuleId(0)).unwrap();
    let lits: Vec<_> = r.body().collect();
    let b = body::nth(&p, 0);
    let (order, _, _) = model.plan_rule(body::Placed::whole(p.clone()).fields().unwrap(), &b, None).unwrap();
    let rels: Vec<&str> = order.iter().filter_map(|s| lits[s.at].atom()).map(|a| p.rel_name(a.rel())).collect();
    assert_eq!(rels[0], "demand_x", "driven by its demand: {rels:?}");
    let at = |rel: &str| lits.iter().position(|l| l.atom().is_some_and(|a| p.rel_name(a.rel()) == rel)).unwrap();
    let led_by = |rel: &str| -> Vec<Step> {
        let mut o = vec![Step { at: at(rel), mode: None }];
        o.extend(order.iter().filter(|s| s.at != at(rel)).cloned());
        o
    };
    assert!(!model.valid_live_driver(&led_by("col"), &b, None), "a kept order led by a scan is not taken");
    assert!(model.valid_live_driver(&led_by("demand_x"), &b, None));
}

/// Decision 0354: with a batch pricer, the search hands back its best's and runners-up's steps from its own cells,
/// and they are the features the plan record of the same order gives (the samples the model learns from): each
/// step's rows, basis, access and source alike.
#[test]
fn the_searchs_own_steps_are_the_records_features() {
    struct NoPrice;
    impl joins::Pricer for NoPrice {
        fn price(&self, _: &[joins::Choice]) -> Option<Vec<Vec<f64>>> {
            None
        }
    }
    use crate::code_rule_dsl::logic::costs::join_steps::category;
    let p = parse(".decl delta(k:number,a:number,t:number,f:number)\n.decl expand(o:number,t:number,k:number)\n.decl restrict(c:number,o:number,a:number)\n.input restrict\n.decl result(c:number,f:number)\nresult(c,f) :- restrict(c,o,a), expand(o,1,k), delta(k,a,2,f), c != f.\n.output result\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv, fan) in [
        ("delta",526.0,vec![526.0,173.0,1.0,526.0],vec![1.0,5.0,526.0,1.0]),
        ("expand",201_000.0,vec![10000.0,4.0,1878.0],vec![30.0,100000.0,200.0]),
        ("restrict",450.0,vec![450.0,450.0,4.0],vec![1.0,1.0,140.0]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c,(ndv,fan)) in ndv.into_iter().zip(fan).enumerate() {
            model.ndv.insert((rel,c),ndv);
            model.stats.fan.insert((rel,c),fan);
        }
    }
    model.stats.solve(["delta","expand","result"].into_iter().map(str::to_owned).collect());
    joins::set_pricer(Some(std::sync::Arc::new(NoPrice)));
    let fields = body::Placed::whole(p.clone());
    let fields = fields.fields().unwrap();
    let b = body::nth(&p, 0);
    // The record made in full, as a profiled run makes it.
    let explain = explaining(true);
    for first in [None, Some(2)] {
        let got = model.search_steps_against_records(fields, &b, first).unwrap();
        assert!(got.len() >= 2, "the best and a runner-up, delta {first:?}: {}", got.len());
        for (steps, x) in &got {
            let x = x.as_ref().expect("a record of the order");
            assert_eq!(steps.len(), x.steps.len());
            for (s, e) in steps.iter().zip(&x.steps) {
                assert!((s.rows - e.rows).abs() <= 1e-9 * e.rows.abs().max(1.0), "{}: rows {} against the record's {}", e.literal, s.rows, e.rows);
                assert_eq!(category(&s.basis, 4), category(&e.basis, 4), "{}: basis", e.literal);
                assert_eq!(category(&s.access, 1), category(&e.access, 1), "{}: access", e.literal);
                assert_eq!(category(&s.source, 4), category(&e.source, 4), "{}: source", e.literal);
            }
        }
    }
    drop(explain);
    joins::set_pricer(None);
}

/// EXPLAIN lists each relation's layout, declared or inferred, among the decisions of the run that plans the program
/// (`make_plan`, decision 0361 step 3).
#[test]
fn explain_lists_each_relations_layout() {
    // `e` the program's facts, which the planner sizes itself.
    let p = parse(".decl e(p: node, c: node, i: number)\ne(1, 2, 0).\ne(1, 3, 1).\n.layout e key(c) tree(p -> c)\n.decl f(c: node, p: node)\n.output f\nf(c, p) :- e(p, c, _).\n").unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()));
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::reset();
    let _model = make_plan(&body::Placed::whole(p.clone()), &db, inputs).unwrap();
    let lines: Vec<String> = crate::code_rule_dsl::logic::profile::decisions().into_keys().filter(|(t, _)| t == "relation layouts (decision 0361)").map(|(_, x)| x).collect();
    crate::code_rule_dsl::logic::profile::set(false);
    assert!(lines.iter().any(|l| l == "e: key(c) (declared), tree(p -> c) (declared)"), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("f: key(c) (by rule #0000), tree(p -> c) (by rule #0000)")), "{lines:?}");
}

/// A layout's estimates (decision 0361 step 4): `child`'s key `c` one value a row, its few fan-out `p` a value per
/// three rows, its `i` dense within `p` at most a few-probe's 16; a relation of facts on every column needs no
/// statistics, one with a column no fact answers does.
#[test]
fn a_layout_prices_the_columns_its_facts_answer() {
    let p = parse(".decl child2(p: node, c: node, i: number)\n.input child2\n.layout child2 key(c) fanout(p -> c, few) order(i dense in p)\n\
                   .decl part(p: node, c: node, s: symbol)\n.input part\n.layout part key(c)\n\
                   .decl o(c: node, p: node)\n.output o\no(c, p) :- child2(p, c, _), part(_, c, _).\n").unwrap();
    let v = p.clone();
    let s = shapes::Shapes::of_version(&v);
    // Made once a version (its analysis), shared by every plan of it: not made again per `make_plan`.
    assert!(std::sync::Arc::ptr_eq(&s, &shapes::Shapes::of_version(&v)));
    let n = shapes::Counts::default();
    assert_eq!(s.ndv("child2", 1, 900.0, n), Some(900.0));
    assert_eq!(s.ndv("child2", 0, 900.0, n), Some(300.0));
    assert_eq!(s.ndv("child2", 2, 900.0, n), Some(16.0));
    assert_eq!(s.ndv("part", 0, 900.0, n), None);
    assert!(s.answers("child2", n) && !s.answers("part", n));
    // Inferred facts price their columns too: `o`'s `c` is `child2`'s key, its `p` the few fan-out.
    assert_eq!(s.ndv("o", 0, 60.0, n), Some(60.0));
    assert_eq!(s.ndv("o", 1, 60.0, n), Some(20.0));
    assert!(s.answers("o", n));
}

/// A column's values (`values(c, …)`, decision 0361 item 5): the documents and kinds by the run's counts, a share of
/// the rows, a bound; with them `node` is answered whole, and none of its statistics is made. A value bound is carried
/// to a column that copies it; a share is not.
#[test]
fn a_values_fact_prices_a_column_by_the_runs_counts() {
    let p = parse(".decl node2(d: number, n: node, kind: symbol, start: number, end: number)\n.input node2\n\
                   .layout node2 key(n) values(d, documents) values(kind, kinds) values(start, many) values(end, rows / 4)\n\
                   .decl lines(n: node, l: number)\n.input lines\n.layout lines key(n) values(l, within document)\n\
                   .decl k2(n: node, k: symbol, s: number)\n.output k2\nk2(n, k, s) :- node2(_, n, k, s, _).\n").unwrap();
    let v = p.clone();
    let s = shapes::Shapes::of_version(&v);
    let n = shapes::Counts { docs: Some(30.0), kinds: Some(200.0) };
    assert_eq!(s.ndv("node2", 0, 9000.0, n), Some(30.0));
    assert_eq!(s.ndv("node2", 2, 9000.0, n), Some(200.0));
    assert_eq!(s.ndv("node2", 2, 50.0, n), Some(50.0));
    assert_eq!(s.ndv("node2", 3, 9000.0, n), Some(9000.0));
    assert_eq!(s.ndv("node2", 4, 9000.0, n), Some(2250.0));
    assert_eq!(s.ndv("lines", 1, 9000.0, n), Some(300.0));
    assert!(s.answers("node2", n) && s.answers("lines", n));
    // Unknown counts answer nothing of them.
    assert_eq!(s.ndv("node2", 0, 9000.0, shapes::Counts::default()), None);
    assert!(!s.answers("node2", shapes::Counts::default()));
    // `k2` copies `node2`'s key and kinds, not its start's share.
    assert_eq!(s.ndv("k2", 1, 900.0, n), Some(200.0));
    assert_eq!(s.ndv("k2", 2, 900.0, n), None);
    // The line reads back as written.
    let f = crate::code_rule_dsl::logic::front::layout_facts("values(d, documents) values(i, within document) values(s, rows / 5) values(f, ~50)", &|w| Ok(["d", "i", "s", "f"].iter().position(|x| *x == w).unwrap())).unwrap();
    let shown: Vec<String> = f.iter().map(|f| f.show(&|c| ["d", "i", "s", "f"][c].to_owned())).collect();
    assert_eq!(shown, ["values(d, documents)", "values(i, within document)", "values(s, rows / 5)", "values(f, ~50)"]);
    assert!(f.iter().all(|f| crate::code_rule_dsl::logic::front::LayoutFact::of_placed(&f.placed()).as_ref() == Ok(f)));
}

/// A base relation's measurement counts the columns its layout does not answer, as before, and gives an answered one
/// its facts' estimate (decision 0361 item 5): `d` is priced at the run's 5 documents though the rows hold 2, and
/// `start` and `end_`, which no fact answers, are counted as `sampled_statistics` counts them.
#[test]
fn a_base_relations_answered_columns_are_not_counted() {
    let p = parse(".decl node2(d: number, n: node, kind: symbol, start: number, end: number)\n.input node2\n\
                   .layout node2 key(n) values(d, documents) values(kind, kinds)\n").unwrap();
    let v = p.clone();
    let s = shapes::Shapes::of_version(&v);
    let n = shapes::Counts { docs: Some(5.0), kinds: Some(3.0) };
    let names: Vec<String> = ["d", "n", "kind", "start", "end_"].iter().map(|c| (*c).to_owned()).collect();
    let values = vec![vec![0, 0, 0, 1, 1, 1], vec![1, 2, 3, 4, 5, 6], vec![7, 8, 9, 7, 8, 9], vec![0, 0, 5, 5, 9, 9], vec![1, 2, 3, 4, 5, 6]];
    // A source that keeps no statistics of its own (a held one counts every column when asked).
    let source = crate::code_rule_dsl::logic::plan::corpus::unmeasured_rows(names, values);
    let got = crate::code_rule_dsl::logic::plan::corpus::sampled_statistics_unanswered("node2", &source, &s, n);
    let all = crate::code_rule_dsl::logic::plan::corpus::sampled_statistics(&source);
    assert_eq!(got.iter().map(|g| (g.0.as_str(), g.2)).collect::<Vec<_>>(), [("d", 5), ("n", 6), ("kind", 3), ("start", 3), ("end_", 6)]);
    assert_eq!((&got[3], &got[4]), (&all[3], &all[4]), "the columns no fact answers, counted as before");
    // A key's postings are one row each.
    assert_eq!((got[1].3, got[1].4), (6, 6));
}

/// The program of the prepared-plan tests (decision 0361 step 5): `child2` and `kind2` shaped as core.rerail's `child` and
/// `kind`; `part` with no size; `up` declared, recursive.
fn prepared_program() -> Arc<Version> {
    parse(".decl child2(p: node, c: node, i: number)\n.input child2\n.layout child2 key(c) fanout(p -> c, few) order(i dense in p)\n\
           .decl kind2(n: node, k: symbol)\n.input kind2\n.layout kind2 key(n) size(per n) values(k, kinds)\n\
           .decl part(x: number, y: number)\n.input part\n\
           .decl o(c: node, k: symbol)\n.output o\no(c, k) :- child2(p, c, _), kind2(c, k), kind2(p, \"x\").\n\
           .decl s(c: node, k: symbol)\n.output s\ns(c, k) :- child2(p, c, _), kind2(c, k), kind2(p, k).\n\
           .decl q(c: node)\n.output q\nq(c) :- child2(_, c, _), part(c, _).\n\
           .decl up(a: node, b: node)\n.layout up fanout(a -> b, ~8) values(b, many)\n.output up\n\
           up(p, c) :- child2(p, c, _).\nup(a, c) :- up(a, b), child2(b, c, _).\n").unwrap()
}

/// The program's rules by head, each with its body's place.
fn bodies_by_head(placed: &body::Placed) -> Vec<(String, usize)> {
    (0..placed.len()).map(|k| { let b = placed.body(k); (b.rel(b.head()).to_owned(), k) }).collect()
}

/// Decision 0361 step 5: a rule over relations whose facts size them and price every column it reads is planned with
/// the program, from the facts alone; its order covers its literals. A rule reading a relation with no size (`part`)
/// is not: it is planned at run time. A recursive rule over a declared relation is planned for its delta too. A rule
/// that tests a constant on a column a `values` fact answers (`kind2(p, "x")`) is not either: the fact gives the
/// column's distinct values, not this one's rows, and the order turns on them (PostHog's `param`, work queue Q297).
#[test]
fn a_rule_over_shaped_relations_is_planned_with_the_program() {
    let p = prepared_program();
    let v = p.clone();
    let placed = body::Placed::whole(std::sync::Arc::clone(&v));
    let prep = prepared::Prepared::of_version(&v);
    let line = |head: &str, nth: usize, first: Option<usize>| {
        let k = bodies_by_head(&placed).into_iter().filter(|(h, _)| h == head).nth(nth).unwrap().1;
        crate::code_rule_dsl::logic::plan::line_key(&placed.body(k), first)
    };
    let o = prep.0.get(&line("s", 0, None)).expect("`s`'s rule is prepared");
    assert_eq!(o.order.len(), 3, "{o:?}");
    assert!(o.work_per_node > 0.0);
    assert!(!prep.0.contains_key(&line("o", 0, None)), "`o` tests a kind's constant: planned at run time");
    assert!(!prep.0.contains_key(&line("q", 0, None)), "`part` has no size: `q`'s rule is an exception");
    assert!(prep.0.contains_key(&line("up", 0, None)));
    assert!(prep.0.contains_key(&line("up", 1, None)) && prep.0.contains_key(&line("up", 1, Some(0))), "the recursive rule and its delta at `up`");
    assert_eq!(prep.0.len(), 4, "{:?}", prep.0.keys().collect::<Vec<_>>());
    // Made once a version.
    assert!(std::sync::Arc::ptr_eq(&prep, &prepared::Prepared::of_version(&v)));
}

/// A run takes a prepared line's order as kept: the search is not run (an order it would never choose, set here, is
/// what it gets), and its work is scaled to the run's nodes. A line with none is searched as before.
#[test]
fn a_run_takes_a_prepared_plan_without_searching() {
    let p = prepared_program();
    let placed = body::Placed::whole(p.clone());
    let f = placed.fields().unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    // The inputs' statistics, as a run measures them (a run's inputs have them; the facts are not what is tested).
    let mut given = Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap());
    for (rel, width) in [("child2", 3), ("kind2", 2), ("part", 2)] {
        given.measured(rel, (0..width).map(|c| (format!("c{c}"), [1_000, 300, 4_000, 4_000])), []);
        given.card.insert(rel, 1_000.0);
    }
    inputs.given = Some(given);
    // Atomized inputs, as a corpus run's: their indexes are the vtabs' (no SQL schema read).
    inputs.atoms = Some([0; 3]);
    let mut model = make_plan(&placed, &db, inputs).unwrap();
    let k = bodies_by_head(&placed).into_iter().find(|(h, _)| h == "o").unwrap().1;
    let b = placed.body(k);
    // The model's own plan (here the version's prepared one), reversed: an order no search of it chooses.
    let planned = model.plan_rule(f, &b, None).unwrap();
    let odd: Vec<Step> = planned.0.iter().rev().copied().collect();
    assert_ne!(odd, planned.0);
    model.stats.nodes = 2_000.0;
    model.stats.prepared = std::sync::Arc::new(prepared::Prepared(
        [(crate::code_rule_dsl::logic::plan::line_key(&b, None), prepared::PreparedPlan { order: odd.clone(), work_per_node: 3.0, rows_per_node: 0.5, assumed: vec![("child2".to_owned(), 0.5)] })].into_iter().collect(),
    ));
    // Taken as it is, whatever the run's statistics say of what it assumed (`child2` at 0.5 rows a node).
    let got = model.plan_rule(f, &b, None).unwrap();
    assert_eq!(got, (odd.clone(), 6_000.0, 1_000.0));
    // Another line (its delta) is not held: searched.
    assert!(model.plan_rule(f, &b, Some(0)).is_ok());
    // Found heavy on its prepared order, in a run: searched from then on, once (decision 0361 step 5 with 0203).
    let _plans = KeepPlans::start(None);
    let line = crate::code_rule_dsl::logic::plan::line_key(&b, None);
    // Horribly slow past 8 times its prepared price: 3 rows of work a node at 2,000 nodes is 6,000; 48,000 the line.
    assert!(!model.prepared_horribly_slow(&b, &[None], &odd, 40_000.0));
    assert!(model.prepared_horribly_slow(&b, &[None], &odd, 50_000.0));
    assert!(!model.prepared_horribly_slow(&b, &[None], &planned.0, 50_000.0), "not the order it runs");
    assert_eq!(model.prepared_heavy(&b, &[Some(0), None], &planned.0), None, "not the order it runs");
    assert_eq!(model.prepared_heavy(&b, &[Some(0), None], &odd), Some(line));
    assert_eq!(model.prepared_heavy(&b, &[Some(0), None], &odd), None, "once a line");
    let got = model.plan_rule(f, &b, None).unwrap();
    assert_ne!(got.0, odd, "searched, not the order prepared");
}

/// Decision 0361 step 6: a probe's rows past `BROKEN_BY` times what its relation's fan-out lets a probe return break
/// the fact. The run notes it once, the model's shapes leave the relation out (and every relation whose facts were
/// inferred from it), as a later model's do, and a prepared plan reading it is not taken. Rows within it break nothing,
/// nor a few probes.
#[test]
fn a_fan_out_the_rows_break_leaves_its_relation_to_the_statistics() {
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let p = prepared_program();
    let placed = body::Placed::whole(p.clone());
    let f = placed.fields().unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    let mut given = Stats::empty_of(&f);
    for (rel, width) in [("child2", 3), ("kind2", 2), ("part", 2)] {
        given.measured(rel, (0..width).map(|c| (format!("c{c}"), [1_000, 300, 4_000, 4_000])), []);
        given.card.insert(rel, 1_000.0);
    }
    inputs.given = Some(given);
    inputs.atoms = Some([0; 3]);
    let mut model = make_plan(&placed, &db, inputs).unwrap();
    let k = bodies_by_head(&placed).into_iter().find(|(h, _)| h == "o").unwrap().1;
    let b = placed.body(k);
    assert!(model.stats.shapes.0.contains_key("child2"));
    // `o(c, k) :- child2(p, c, _), kind2(c, k), kind2(p, "x")`, run as kind2(p, "x"), child2 (`p` bound: `few`, at
    // most 16 a probe), kind2(c, k) (`c` bound: its key).
    let order = [Step { at: 2, mode: None }, Step { at: 0, mode: None }, Step { at: 1, mode: None }];
    let line = crate::code_rule_dsl::logic::plan::line_key(&b, None);
    model.stats.prepared = std::sync::Arc::new(prepared::Prepared([(line, prepared::PreparedPlan { order: order.to_vec(), work_per_node: 3.0, rows_per_node: 0.5, assumed: vec![] })].into_iter().collect()));
    assert_eq!(model.plan_rule(f, &b, None).unwrap().0, order.to_vec(), "taken as prepared");
    assert!(prepared::run_counts().taken.load(std::sync::atomic::Ordering::Relaxed) > 0);
    // 10 children a parent: within `few`. 200 a parent, over 40 parents: too few probes to tell.
    assert!(!model.layout_broken(&b, &order, &[(0, 100), (100, 1_000), (1_000, 1_000)]));
    assert!(!model.layout_broken(&b, &order, &[(0, 40), (40, 8_000), (8_000, 8_000)]));
    assert!(model.stats.shapes.0.contains_key("child2"));
    // 200 a parent over 100 parents: past 8 times 16.
    assert!(model.layout_broken(&b, &order, &[(0, 100), (100, 20_000), (20_000, 20_000)]));
    let broken = shapes::run_broken().facts();
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(broken["child2"].contains("fanout"), "{broken:?}");
    assert!(!model.stats.shapes.0.contains_key("child2") && model.stats.shapes.left_out("child2"));
    assert!(model.stats.shapes.0.contains_key("kind2"), "kind2's facts hold");
    for (rel, from) in &model.stats.shapes.1 {
        if from.contains("child2") {
            assert!(model.stats.shapes.left_out(rel), "{rel}'s facts were inferred from child2's");
        }
    }
    // A later model of the run leaves it out too; a prepared plan reading it is not taken.
    assert!(shapes::Shapes::for_run(shapes::Shapes::of_version(&placed.v)).left_out("child2"));
    let taken = || prepared::run_counts().taken.load(std::sync::atomic::Ordering::Relaxed);
    let before = taken();
    model.plan_rule(f, &b, None).unwrap();
    assert_eq!(taken(), before, "searched, not taken as prepared");
    // Left out, it is not checked again: noted once.
    assert!(!model.layout_broken(&b, &order, &[(0, 100), (100, 20_000), (20_000, 20_000)]));
    assert_eq!(shapes::run_broken().facts().len(), 1);
}

/// The prepared plans as an image stores them (decision 0333 §4): read back the same.
#[test]
fn prepared_plans_are_stored_with_the_image_and_read_back() {
    use crate::code_rule_dsl::logic::program_image::manager::Analysis;
    let p = prepared_program();
    let v = p.clone();
    let prep = prepared::Prepared::of_version(&v);
    let rows = prep.rows(&v).expect("stored");
    assert_eq!(rows.len(), prep.0.len());
    assert_eq!(prepared::Prepared::of_rows(&v, &rows).as_ref(), Some(&*prep));
}

/// Decision 0361 step 5, end to end: a policy whose rule reads only shaped relations (`kind`, `child`) runs over
/// three files through `run_policies`, its strata planned from the prepared plans (the lines taken counted), and its
/// findings are what the rule says: a statement at a module's top.
#[test]
fn a_policy_run_takes_its_prepared_plans_and_finds_the_same() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("prepared-plans");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "f()\n").unwrap();
    std::fs::write(root.join("b.py"), "def g():\n    f()\n").unwrap();
    std::fs::write(root.join("c.py"), "x = 1\ng()\n").unwrap();
    let policy = root.with_extension("policy.rerail");
    std::fs::write(
        &policy,
        ".language python\n.violation hit(at: node) code \"X1\" \"a statement at a module's top\"\n\
         hit(n) :- kind(n, \"expression_statement\"), child(p, n, _), kind(p, \"module\").\n",
    )
    .unwrap();
    // The test's run holds the policy run (it joins it), so the plans counted taken are this run's alone.
    let run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let taken = prepared::run_counts().taken.load(std::sync::atomic::Ordering::Relaxed);
    drop(run);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&policy);
    let mut hits: Vec<(String, i64)> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(), d["line"].as_i64().or(d["start_line"].as_i64()).unwrap_or(-1)))
        .collect();
    hits.sort();
    assert_eq!(hits, [("a.py".to_owned(), 1), ("c.py".to_owned(), 1), ("c.py".to_owned(), 2)], "{report:#}");
    assert!(taken > 0, "no line was taken from the prepared plans");
}

/// A version kept with no prepared plan is read back as one (its id alone): else the next process makes it again, and
/// writes it, every warm run (`the_run_after_a_cold_one_writes_nothing`).
#[test]
fn a_version_with_no_prepared_plan_is_read_back() {
    let by: prepared::ByVersion = [([7u8; 32], std::sync::Arc::new(prepared::Prepared::default()))].into_iter().collect();
    let back = prepared::of_store_rows(&prepared::store_rows(&by));
    assert_eq!(back.get(&[7u8; 32]).map(|p| p.0.len()), Some(0), "kept, with no plan");
}

/// Decision 0361 step 6, end to end: a policy's derived relation declares `fanout(a -> b, one)`, and its rows break it
/// (a module's statements, 40 of them, under each module; recursive, so it is not inlined into its reader). The run
/// reads the executors' counts, finds the fact broken, plans the relation from statistics from then on, and goes on:
/// its findings are what the rules say.
#[test]
fn a_run_finds_a_fanout_its_rows_break_and_finds_the_same() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("broken-fanout");
    std::fs::create_dir_all(&root).unwrap();
    let statements: String = (0..40).map(|k| format!("f{k}()\n")).collect();
    std::fs::write(root.join("a.py"), &statements).unwrap();
    std::fs::write(root.join("b.py"), &statements).unwrap();
    std::fs::write(root.join("c.py"), "x = 1\n").unwrap();
    let policy = root.with_extension("policy.rerail");
    std::fs::write(
        &policy,
        ".language python\n\
         .decl under(a: node, b: node)\n.layout under fanout(a -> b, one)\n\
         under(m, n) :- kind(m, \"module\"), child(m, n, _).\n\
         under(m, n) :- under(m, b), kind(b, \"block\"), child(b, n, _).\n\
         .violation hit(at: node) code \"X1\" \"a call statement beside another\"\n\
         hit(n) :- kind(x, \"expression_statement\"), child(m, x, _), under(m, n), kind(n, \"expression_statement\"), n != x.\n",
    )
    .unwrap();
    // The test's run holds the policy run (it joins it), so the facts counted broken are this run's alone.
    let run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let broken = shapes::run_broken().facts().len();
    // The probes checked against a fact: none means no line priced by it was counted.
    let checked = shapes::run_broken().1.load(std::sync::atomic::Ordering::Relaxed);
    drop(run);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&policy);
    let mut hits: Vec<(String, i64)> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(), d["line"].as_i64().or(d["start_line"].as_i64()).unwrap_or(-1)))
        .collect();
    hits.sort();
    let want: Vec<(String, i64)> = ["a.py", "b.py"].iter().flat_map(|f| (1..=40).map(move |l| (f.to_string(), l))).collect();
    assert_eq!(hits, want, "{report:#}");
    assert_eq!(broken, 1, "the run found `under`'s fan-out broken ({checked} probes checked against a fact)");
}

/// Decision 0361 step 6, a key checked where the rows are written: `under` declares `key(a)`, and a module has 40
/// statements under it. Its reader probes it by `b` alone, so no probe's counts cover the key: the rows' writer finds
/// the second row of a key, the run plans `under` from statistics from then on, and its findings are the rules'.
#[test]
fn a_run_finds_a_key_its_rows_break_where_they_are_written() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("broken-key");
    std::fs::create_dir_all(&root).unwrap();
    let statements: String = (0..40).map(|k| format!("f{k}()\n")).collect();
    std::fs::write(root.join("a.py"), &statements).unwrap();
    std::fs::write(root.join("c.py"), "x = 1\n").unwrap();
    let policy = root.with_extension("policy.rerail");
    std::fs::write(
        &policy,
        ".language python\n\
         .decl under(a: node, b: node)\n.layout under key(a)\n\
         under(m, n) :- kind(m, \"module\"), child(m, n, _).\n\
         under(m, n) :- under(m, b), kind(b, \"block\"), child(b, n, _).\n\
         .violation hit(at: node) code \"X1\" \"a statement at a module's top\"\n\
         hit(n) :- kind(n, \"expression_statement\"), under(m, n), kind(m, \"module\").\n",
    )
    .unwrap();
    // The test's run holds the policy run (it joins it), so the facts counted broken are this run's alone.
    let run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let broken = shapes::run_broken().facts().len();
    drop(run);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&policy);
    let mut hits: Vec<(String, i64)> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(), d["line"].as_i64().or(d["start_line"].as_i64()).unwrap_or(-1)))
        .collect();
    hits.sort();
    let mut want: Vec<(String, i64)> = (1..=40).map(|l| ("a.py".to_owned(), l)).collect();
    want.push(("c.py".to_owned(), 1));
    assert_eq!(hits, want, "{report:#}");
    assert_eq!(broken, 1, "the rows' writer found `under`'s key broken");
}

/// Q228: the prepared plans a run made of a version are kept for the store by its identity, and a later process's run
/// of the same program (another version, built again) reads them, made none.
#[test]
fn a_later_run_reads_the_prepared_plans_an_earlier_one_made() {
    use crate::code_rule_dsl::logic::program_image::manager::VersionAnalyses as _;
    // Each build is a new version of the same program, parsed again: the later run's is not the earlier one's.
    let build = prepared_program;
    let held = {
        let _plans = KeepPlans::start(None);
        assert!(super::prepared_to_store().is_none(), "none made yet");
        let v = build();
        let made = prepared::Prepared::of_version(&v);
        assert_eq!((v.analyses().computed("prepared plans"), v.analyses().loaded("prepared plans")), (1, 0));
        let held = super::prepared_to_store().expect("made, for the store");
        assert_eq!(held.get(&crate::code_rule_dsl::logic::program_image::manager::identity(&v)), Some(&made));
        prepared::ByVersion::clone(&held)
    };
    // As the store gives it back: through its rows.
    let back = prepared::of_store_rows(&prepared::store_rows(&held));
    let _plans = KeepPlans::start(None);
    super::take_stored_prepared(back);
    let v = build();
    let read = prepared::Prepared::of_version(&v);
    assert_eq!((v.analyses().computed("prepared plans"), v.analyses().loaded("prepared plans")), (0, 1), "read, not made");
    assert_eq!(Some(&*read), held.values().next().map(|p| &**p));
    assert!(super::prepared_to_store().is_none(), "nothing new to store");
    assert!(std::sync::Arc::ptr_eq(&read, &prepared::Prepared::of_version(&v)), "held by the version");
}

/// A probe whose new variable later comparisons bound reads a range (`setfix::ranges`), and past the head's variables
/// a step passes on one witness a binding (`setfix::tail`). A zero count's helper, `h(a, b, p) :- ctx(a, b, p),
/// line(p, l, _), !at(p, l), l > a, l < b` (`bounded-composed-graph-paths`' `rr_agg15`; 91 files: 25,425 pairs, 6,027
/// lines of 9 files), was searched from `line` (46.7 M work), each line probing its file's pairs; from `ctx`, each
/// pair reads its lines between its ends and stops at the first.
#[test]
fn a_probe_bounded_by_comparisons_reads_a_range() {
    let p = parse(".decl ctx(a:number,b:number,p:number)\n.decl src(p:number,l:number,s:number)\n.decl at(p:number,l:number)\n.input ctx, src, at\n.decl line(p:number,l:number,s:number)\nline(p,l,s) :- src(p,l,s).\n.decl h(a:number,b:number,p:number)\nh(a,b,p) :- ctx(a,b,p), line(p,l,_), !at(p,l), l > a, l < b.\n.output h\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [
        ("ctx", 25_425.0, vec![212.0, 330.0, 2.0]),
        ("line", 6_027.0, vec![9.0, 3_535.0, 6_027.0]),
        ("at", 700.0, vec![9.0, 700.0]),
    ] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), rows / n);
        }
    }
    model.stats.solve(["h"].into_iter().map(str::to_owned).collect());
    let r = p.rules().position(|r| p.rel_name(r.head().rel()) == "h").unwrap();
    let ((order, work, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let b = body::nth(&p, r);
    let order: Vec<&str> = order.iter().filter_map(|s| b.atom(s.at)).filter(|a| !a.neg()).map(|a| b.rel(a)).collect();
    assert_eq!(order, ["ctx", "line"], "{order:?}, work={work}");
    // A pair's lines between its ends, a ninth of its file's as the comparisons are priced (a third each).
    assert!(work < 5e6, "a range a pair: {work}");
}

/// Decision 0361 step 5, derived relations: one the facts do not size is sized from its rules' plans, in dependency
/// order, so the rules that read it are prepared too. A union's rows are its rules' summed, and only where every rule
/// is planned (`u` has a branch over `part`, which has no size: what reads it is not prepared); a recursion's are its
/// base rules' times a nominal depth (`anc`, with no fact declared). `o`, testing a constant on `kind2`'s `values`
/// column, is planned at run time, and its rows still size it.
#[test]
fn a_rule_over_a_derived_relation_is_prepared_once_its_rules_size_it() {
    // Off by default (Q290): turned on here, as a measurement turns it on.
    let _on = rerail_prof::code_rule_dsl::logic::profile::bind(prepared::DerivedEstimates(Some(true)));
    let text = ".decl child2(p: node, c: node, i: number)\n.input child2\n.layout child2 key(c) fanout(p -> c, few) order(i dense in p)\n\
                   .decl kind2(n: node, k: symbol)\n.input kind2\n.layout kind2 key(n) size(per n) values(k, kinds)\n\
                   .decl part(x: number, y: number)\n.input part\n\
                   .decl o(c: node, k: symbol)\no(c, k) :- child2(p, c, _), kind2(c, k), kind2(p, \"x\").\n\
                   .decl r(c: node)\n.output r\nr(c) :- o(c, k), kind2(c, k).\n\
                   .decl u(c: node)\nu(c) :- o(c, _).\nu(c) :- part(c, _).\n\
                   .decl w(c: node)\n.output w\nw(c) :- u(c), kind2(c, _).\n\
                   .decl anc(a: node, b: node)\nanc(a, b) :- child2(a, b, _).\nanc(a, c) :- anc(a, b), child2(b, c, _).\n\
                   .decl z(a: node)\n.output z\nz(a) :- anc(a, b), kind2(b, _).\n";
    let v = parse(text).unwrap();
    let placed = body::Placed::whole(std::sync::Arc::clone(&v));
    let prep = prepared::Prepared::of_version(&v);
    let line = |head: &str, nth: usize, first: Option<usize>| {
        let k = bodies_by_head(&placed).into_iter().filter(|(h, _)| h == head).nth(nth).unwrap().1;
        crate::code_rule_dsl::logic::plan::line_key(&placed.body(k), first)
    };
    assert!(prep.0.contains_key(&line("r", 0, None)), "`o` sized by its rule: `r` is prepared");
    assert!(prep.0.contains_key(&line("u", 0, None)) && !prep.0.contains_key(&line("u", 1, None)), "`u`'s branch over `part` is not");
    assert!(!prep.0.contains_key(&line("w", 0, None)), "`u` is not sized: one of its rules is not planned");
    assert!(prep.0.contains_key(&line("anc", 1, None)) && prep.0.contains_key(&line("anc", 1, Some(0))), "the recursion, sized by its base rule");
    assert!(prep.0.contains_key(&line("z", 0, None)), "`anc` sized: `z` is prepared");
    // By default no derived relation is sized by an estimate: the recursion `anc`, which no fact sizes, is not, and `z`,
    // which reads it, is not prepared. `o`'s rule, over the inputs, tests a constant on `kind2`'s `values` column, so it
    // is planned at run time (0361, Q297), and `r` is prepared, as `o`'s facts (inferred from its rule) size it.
    // (Another program: a version of the same digest holds the prepared plans made above.)
    drop(_on);
    let v = parse(&format!("{text}.decl apart(x: number)\n")).unwrap();
    let placed = body::Placed::whole(std::sync::Arc::clone(&v));
    let prep = prepared::Prepared::of_version(&v);
    let line = |head: &str, nth: usize, first: Option<usize>| {
        let k = bodies_by_head(&placed).into_iter().filter(|(h, _)| h == head).nth(nth).unwrap().1;
        crate::code_rule_dsl::logic::plan::line_key(&placed.body(k), first)
    };
    assert!(!prep.0.contains_key(&line("o", 0, None)), "`o` tests a constant a `values` fact answers: planned at run time");
    assert!(prep.0.contains_key(&line("r", 0, None)), "`o`'s inferred facts size it");
    assert!(!prep.0.contains_key(&line("z", 0, None)), "`anc` is not sized: `z` is not prepared");
}

/// Decision 0488: where both columns' keys are kept, a probe hits as often as the keys they share say. Containment
/// takes the probed column's values as all present where it has as many distinct values as the binding (1,000
/// each here), so a join of disjoint values was priced as if each probe hit; their filters say none does.
#[test]
fn kept_keys_price_a_probe_by_the_keys_two_columns_share() {
    use crate::code_rule_dsl::logic::bloom::Keys;
    let p = parse_version(".decl a(x:number,y:number)\n.decl b(y:number,z:number)\n.decl c(z:number,w:number)\n.decl out(x:number,w:number)\nout(x,w) :- a(x,y), b(y,z), c(z,w).\n.output out\n").unwrap();
    let derived: HashSet<String> = ["a", "b", "c", "out"].into_iter().map(str::to_owned).collect();
    let work = |keys: Option<(Keys, Keys)>| {
        let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: derived.clone().into()};
        for (rel, rows, ndv, fan) in [("a", 10_000.0, [10_000.0, 1_000.0], [1.0, 10.0]), ("b", 1_000.0, [1_000.0, 1_000.0], [1.0, 1.0]), ("c", 1_000.0, [1_000.0, 1_000.0], [1.0, 1.0])] {
            model.stats.card.insert(rel, rows);
            for (c, (ndv, fan)) in ndv.into_iter().zip(fan).enumerate() {
                model.ndv.insert((rel, c), ndv);
                model.stats.fan.insert((rel, c), fan);
            }
        }
        if let Some((a, b)) = keys {
            model.stats.keys.insert(("a", 1), a);
            model.stats.keys.insert(("b", 0), b);
        }
        let ((_, work, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, 0), None).unwrap();
        work
    };
    let containment = work(None);
    let same = work(Some((Keys::of(0..1_000), Keys::of(0..1_000))));
    let apart = work(Some((Keys::of(0..1_000), Keys::of(5_000..6_000))));
    assert!((same / containment - 1.0).abs() < 0.1, "the same keys price as containment does: {same} against {containment}");
    assert!(apart < 0.5 * containment, "keys two columns do not share are probes that miss: {apart} against {containment}");
}

/// Decision 0488: a join of two columns skewed on the same few keys, priced by their sketches' counters (`join_size`):
/// the rows the two columns join, whichever is probed. Their fanouts and distinct values price a probe of `b` by `a`'s
/// rows at a tenth of what it makes (the keys both hold heavy are 1% of `a`'s distinct values, and most of its rows).
#[test]
fn join_counts_price_a_join_of_two_skewed_columns_by_its_rows() {
    use crate::code_rule_dsl::logic::join_size::JoinCounts;
    let p = parse_version(".decl a(x:number,y:number)\n.decl b(y:number,z:number)\n.decl out(x:number,z:number)\nout(x,z) :- a(x,y), b(y,z).\n.output out\n").unwrap();
    let derived: HashSet<String> = ["a", "b", "out"].into_iter().map(str::to_owned).collect();
    // `a.y`: 1,000 rows of each of 5 keys and 5,000 of one each; `b.y`: 100 of each of the 5, and 500 others.
    let ay: Vec<i64> = (0..5_000).map(|i| i % 5).chain(1_000..6_000).collect();
    let by: Vec<i64> = (0..500).map(|i| i % 5).chain(10_000..10_500).collect();
    let joined = 5.0 * 1_000.0 * 100.0;
    let last = |counts: bool| {
        let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: derived.clone().into()};
        for (rel, ys, y) in [("a", &ay, 1), ("b", &by, 0)] {
            let c = JoinCounts::of(ys.iter().copied());
            model.stats.card.insert(rel, c.rows);
            model.ndv.insert((rel, y), 5.0 + (ys.len() / 2) as f64);
            model.stats.fan.insert((rel, y), c.f2 / c.rows);
            model.ndv.insert((rel, 1 - y), c.rows);
            model.stats.fan.insert((rel, 1 - y), 1.0);
            if counts {
                model.stats.counts.insert((rel, y), std::sync::Arc::new(c));
            }
        }
        *model.plan_rule_step_rows(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, 0), None).unwrap().1.last().unwrap()
    };
    let (with, without) = (last(true), last(false));
    assert!((with / joined - 1.0).abs() < 0.25, "priced by the counters: {with} rows against {joined}");
    eprintln!("a join of skewed columns: {with} rows priced by the counters, {without} without, {joined} made");
}

/// Past a bound head the search passes on one witness a binding only for a step joined by a bound variable, as the
/// greedy `step` does: a cross product reads its rows until one passes the rest (Q240). A dict's keys and values
/// unwrapped, `h(t) :- tgt(t), child(t, k0, i), child(t, v0, j), unparen(k0, k), unparen(v0, v)`, was led from the
/// 233 targets through both `unparen` scans unjoined (69,690 rows each a target), priced at 461 work, and ran past
/// 100 s on 300 files; probed by `child` first, a target reads a few rows.
#[test]
fn a_cross_product_past_a_bound_head_is_not_priced_as_one_witness() {
    let p = parse_version(".decl tgt(t:number)\n.decl child(p:number,c:number,i:number)\n.decl unparen(e:number,t:number)\n.input tgt, child, unparen\n.decl h(t:number)\nh(t) :- tgt(t), child(t,k0,i), child(t,v0,j), unparen(k0,k), unparen(v0,v).\n.output h\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),stats:Stats::empty_of(&body::Placed::whole(p.clone()).fields().unwrap()),ndv: Default::default(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("tgt", 233.0, vec![233.0]), ("child", 69_650.0, vec![23_217.0, 69_650.0, 103.0]), ("unparen", 69_690.0, vec![69_690.0, 69_690.0])] {
        model.stats.card.insert(rel, rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel, c), n);
            model.stats.fan.insert((rel, c), rows / n);
        }
    }
    model.stats.solve(["h"].into_iter().map(str::to_owned).collect());
    let r = p.rules().position(|r| p.rel_name(r.head().rel()) == "h").unwrap();
    let ((order, work, _), _) = model.plan_rule_explained(body::Placed::whole(p.clone()).fields().unwrap(), &body::nth(&p, r), None).unwrap();
    let b = body::nth(&p, r);
    let order: Vec<_> = order.iter().filter_map(|s| b.atom(s.at)).map(|a| b.rel(a)).collect();
    // Each `unparen` after the `child` that binds its node: probed, never scanned.
    let first = |rel: &str| order.iter().position(|r| *r == rel).unwrap();
    assert_eq!(order[0], "tgt", "{order:?}, work={work}");
    assert!(first("child") < first("unparen"), "{order:?}, work={work}");
    assert!(work < 1e4, "{order:?}: {work}");
}


/// Past a bound head the walk stops at a binding's first witness (`witnessed`), and reads all of a binding's rows where
/// none passes. The greedy order priced such a step at one read a binding, as if its first row were a witness, where
/// the search and a kept order's pricing read every row walked (Q411). Decision 0431 R9's key over a tape value's
/// findings, `exists_10(d) :- demand(d), node(d, v, …), !placed(v), finding(v)` led from its 40 documents (corpus0,
/// one pack, 2026-11-21: 69,690 nodes, one finding), was priced at 118 rows of work through `node` by `$d`, under a
/// search's cost, so taken unsearched: it read each document's every node, 100 k rows a call where the finding's one
/// row, then `node` by `$d` and `$v`, reads about 80.
#[test]
fn a_walk_past_a_bound_head_reads_every_row_where_none_is_a_witness() {
    let p = parse_version(".decl dm(d: number)\n.decl node(d: number, n: number, k: number, s: number, e: number)\n.decl pl(n: number)\n.decl f(n: number)\n.decl e(d: number)\ne(d) :- dm(d), node(d, v, _, _, _), !pl(v), f(v).\n.output e\n").unwrap();
    let mut model = bare_model(&p);
    for (rel, rows, cols) in [("dm", 40.0, vec![(40.0, 1.0)]), ("node", 69_690.0, vec![(40.0, 1_742.0), (69_690.0, 1.0), (81.0, 860.0), (69_690.0, 1.0), (69_690.0, 1.0)]), ("pl", 3_000.0, vec![(3_000.0, 1.0)]), ("f", 1.0, vec![(1.0, 1.0)])] {
        model.stats.card.insert(rel, rows);
        for (c, (ndv, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel, c), ndv);
            model.stats.fan.insert((rel, c), fan);
        }
    }
    model.stats.nodes = 69_690.0;
    let pf = body::Placed::whole(p.clone());
    let b = body::nth(&p, 0);
    // Led from the demand, as its seed line is.
    let ((order, work, _), _) = model.plan_rule_explained(pf.fields().unwrap(), &b, Some(0)).unwrap();
    let order: Vec<String> = order.iter().map(|s| lit_at(&p, 0, s.at)).collect();
    let at = |lit: &str| order.iter().position(|o| o.starts_with(lit)).unwrap();
    assert!(at("f(") < at("node("), "the finding's row first, then `node` by `$d` and `$v`: {order:?} at {work}");
    assert!(work < 1_000.0, "{order:?} at {work}");
    // The walk of `node` by `$d` alone, priced as it runs: every node of the 40 documents.
    let walk: Vec<Step> = (0..4).map(|at| Step { at, mode: None }).collect();
    let explain = explaining(true);
    let x = model.explain_order(pf.fields().unwrap(), &b, Some(0), &walk).expect("the order is priced");
    drop(explain);
    assert!(x.work > 10.0 * work, "the walk is priced over the finding's row first: {} against {work}", x.work);
    assert!(x.steps[1].rows >= 69_000.0, "`node` by `$d` makes a document's nodes: {:?}", x.steps.iter().map(|s| (&s.literal, s.rows)).collect::<Vec<_>>());
}
