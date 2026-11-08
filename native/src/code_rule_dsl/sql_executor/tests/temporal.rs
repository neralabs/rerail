use super::*;

const SHORTEST: &str = ".decl edge(a:number,b:number,w:number)\n.decl root(n:number)\n.input edge,root\n.decl distance(n:number,d:min<>) widen 3\ndistance(n,0) :- root(n).\ndistance(b,d+w) :- distance(a,d), edge(a,b,w).\n.output distance\n";

#[test]
fn partial_lattice_folds_preserve_duplicate_support_composite_keys_and_retractions() {
    for lattice in ["min", "max", "flat"] {
        let (_directory, mut e) = executor();
        bind(
            &mut e,
            "seed",
            &["id", "g", "k", "v"],
            json!([[1, 0, 0, 3], [2, 0, 0, 3], [3, 0, 0, 7], [4, 1, 0, 4]]),
        );
        bind(&mut e, "edge", &["a", "b"], json!([[0, 1], [1, 2], [2, 1]]));
        bind(&mut e, "other", &["a", "b"], json!([[0, 1]]));
        let p = format!(
            ".decl seed(id:number,g:number,k:number,v:number)\n.decl edge(a:number,b:number)\n.decl other(a:number,b:number)\n.input seed,edge,other\n.decl item(g:number,k:number,v:{lattice}<>) widen 4\nitem(g,k,v) :- seed(_,g,k,v).\nitem(g,b,v) :- item(g,a,v),edge(a,b).\nitem(g,b,v) :- item(g,a,v),other(a,b).\n.output item\n"
        );
        e.run(p.clone(), "python".into(), None).unwrap();
        let initial_value = match lattice {
            "min" => 3,
            "max" => 7,
            _ => crate::code_rule_dsl::logic::native::TOP,
        };
        assert_eq!(
            rows(&e, "SELECT c2 FROM item WHERE c0=0 AND c1=2"),
            json!([[wire::integer(initial_value)]])
        );
        for patch in [
            json!([{"name":"seed","delete":[[1,0,0,3]]}]),
            json!([{"name":"seed","delete":[[2,0,0,3]]}]),
            json!([{"name":"other","delete":[[0,1]]}]),
            json!([{"name":"seed","delete":[[3,0,0,7],[4,1,0,4]]}]),
            json!([{"name":"seed","insert":[[5,1,0,9],[6,0,0,2]]}]),
            json!([{"name":"other","insert":[[0,1]]},{"name":"edge","delete":[[0,1]]}]),
        ] {
            change(&mut e, patch);
            check(
                &e,
                &p,
                &[
                    ("seed", &["id", "g", "k", "v"]),
                    ("edge", &["a", "b"]),
                    ("other", &["a", "b"]),
                ],
                if lattice == "flat" {
                    &["item"]
                } else {
                    &["item", "__sql_changes_item"]
                },
            );
        }
    }
}

fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], rows: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":rows}).to_string())
        .unwrap();
}
fn initial() -> (tempfile::TempDir, CodeRuleSqlExecutor) {
    let (d, mut e) = executor();
    bind(
        &mut e,
        "edge",
        &["a", "b", "w"],
        json!([[0, 1, 5], [0, 2, 1], [2, 1, 1], [1, 3, 2], [3, 2, 1]]),
    );
    bind(&mut e, "root", &["n"], json!([[0]]));
    e.run(SHORTEST.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    assert!(e.executions[0].recursive.is_empty());
    (d, e)
}
fn check(e: &CodeRuleSqlExecutor, p: &str, inputs: &[(&str, &[&str])], outputs: &[&str]) {
    let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
    for (name, columns) in inputs {
        bind(
            &mut fresh,
            name,
            columns,
            rows(e, &format!("SELECT * FROM {name}")),
        );
    }
    fresh.run(p.into(), "python".into(), None).unwrap();
    for r in outputs {
        let width = e
            .connection()
            .prepare(&format!("SELECT * FROM {r}"))
            .unwrap()
            .column_count();
        let order = (1..=width)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            rows(e, &format!("SELECT * FROM {r} ORDER BY {order}")),
            rows(&fresh, &format!("SELECT * FROM {r} ORDER BY {order}"))
        );
    }
}
fn change(e: &mut CodeRuleSqlExecutor, patch: Value) -> Value {
    let report: Value = serde_json::from_str(
        &e.apply_changes(patch.to_string(), None)
            .unwrap_or_else(|err| panic!("{patch}: {err}")),
    )
    .unwrap();
    if report["programs"] == json!([]) {
        return report;
    }
    assert_eq!(
        report["programs"][0]["refreshed_strata"],
        json!([]),
        "{report}"
    );
    assert!(
        report["programs"][0]["recursive_frames"].as_u64().unwrap() > 0,
        "{report}"
    );
    report
}

#[test]
fn unpartitioned_lattices_remove_cyclic_support_and_extend_retained_iterations() {
    let (_d, mut e) = initial();
    assert_eq!(
        rows(&e, "SELECT * FROM distance ORDER BY c0"),
        json!([[0, 0], [1, 2], [2, 1], [3, 4]])
    );
    for patch in [
        json!([{"name":"edge","delete":[[2,1,1]]}]),
        json!([{"name":"root","delete":[[0]]}]),
        json!([{"name":"root","insert":[[2]]}]),
        json!([{"name":"edge","insert":[[3,4,1],[4,5,1],[5,6,1],[6,7,1]]}]),
        json!([{"name":"edge","delete":[[1,3,2]],"insert":[[2,3,9]]}]),
        json!([{"name":"edge","insert":[[0,1,1]]},{"name":"root","insert":[[0]],"delete":[[2]]}]),
    ] {
        let slots = rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '*__it*'",
        );
        change(&mut e, patch);
        assert_eq!(
            rows(
                &e,
                "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '*__it*'"
            ),
            slots,
            "extending retained iterations must reuse SQL slots"
        );
        check(
            &e,
            SHORTEST,
            &[("edge", &["a", "b", "w"]), ("root", &["n"])],
            &["distance"],
        );
    }
}

#[test]
fn unpartitioned_iterations_restore_native_state_after_failure_and_can_retry() {
    let (_d, mut e) = initial();
    let before = rows(&e, "SELECT * FROM distance ORDER BY c0");
    let patch = json!([{"name":"edge","delete":[[2,1,1]],"insert":[[3,4,1],[4,5,1],[5,6,1]]}]);
    e.connection().execute_batch("CREATE TEMP TRIGGER fail_temporal BEFORE INSERT ON distance BEGIN SELECT RAISE(ABORT,'fail temporal publish'); END").unwrap();
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("fail temporal publish")
    );
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), before);
    e.connection()
        .execute_batch("DROP TRIGGER fail_temporal")
        .unwrap();
    assert!(e.apply_changes(patch.to_string(), Some(1)).is_err());
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), before);
    change(&mut e, patch);
    check(
        &e,
        SHORTEST,
        &[("edge", &["a", "b", "w"]), ("root", &["n"])],
        &["distance"],
    );
    e.run(SHORTEST.into(), "python".into(), None).unwrap();
    check(
        &e,
        SHORTEST,
        &[("edge", &["a", "b", "w"]), ("root", &["n"])],
        &["distance"],
    );
    e.reset().unwrap();
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '*__it*'"
        ),
        json!([[0]])
    );
}

#[test]
fn unpartitioned_negative_cycles_preserve_widening_and_retract_from_top() {
    let (_d, mut e) = initial();
    for patch in [
        json!([{"name":"edge","insert":[[1,1,-1]]}]),
        json!([{"name":"edge","delete":[[1,1,-1]]}]),
        json!([{"name":"edge","insert":[[1,1,-1]]},{"name":"root","delete":[[0]]}]),
        json!([{"name":"root","insert":[[0]]}]),
    ] {
        change(&mut e, patch);
        check(
            &e,
            SHORTEST,
            &[("edge", &["a", "b", "w"]), ("root", &["n"])],
            &["distance", "__sql_changes_distance"],
        );
    }
}

#[test]
fn temporal_shortest_paths_match_cold_execution_through_many_simultaneous_edits() {
    let (_d, mut e) = initial();
    let mut state = 71u64;
    for turn in 0..40 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = ((state >> 19) % 8) as i64;
        let b = ((state >> 28) % 8) as i64;
        let weight = ((state >> 42) % 7) as i64 - 2;
        let row = json!([a, b, weight]);
        let patch = if turn % 3 == 0 {
            json!([{"name":"edge","delete":[row]},{"name":"root","insert":[[a]],"delete":[[b]]}])
        } else {
            json!([{"name":"edge","insert":[row]}])
        };
        change(&mut e, patch);
        check(
            &e,
            SHORTEST,
            &[("edge", &["a", "b", "w"]), ("root", &["n"])],
            &["distance", "__sql_changes_distance"],
        );
    }
}

#[test]
fn sparse_recursive_edits_do_not_scan_disconnected_unpartitioned_paths() {
    let (_d, mut e) = executor();
    let groups = 10000i64;
    bind(
        &mut e,
        "edge",
        &["a", "b", "w"],
        json!(
            (0..groups)
                .flat_map(|g| {
                    let n = g * 10;
                    [
                        [n, n + 1, 5],
                        [n, n + 2, 1],
                        [n + 2, n + 1, 1],
                        [n + 1, n + 3, 2],
                        [n + 3, n + 2, 1],
                    ]
                })
                .collect::<Vec<_>>()
        ),
    );
    bind(
        &mut e,
        "root",
        &["n"],
        json!((0..groups).map(|g| vec![g * 10]).collect::<Vec<_>>()),
    );
    e.run(SHORTEST.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    e.connection().execute_batch("CREATE TEMP TRIGGER guard_recursive_delete BEFORE DELETE ON distance WHEN OLD.c0<170 OR OLD.c0>179 BEGIN SELECT RAISE(ABORT,'unrelated recursive delete'); END; CREATE TEMP TRIGGER guard_recursive_insert BEFORE INSERT ON distance WHEN NEW.c0<170 OR NEW.c0>179 BEGIN SELECT RAISE(ABORT,'unrelated recursive insert'); END").unwrap();
    for patch in [
        json!([{"name":"edge","delete":[[172,171,1]]}]),
        json!([{"name":"root","delete":[[170]]}]),
        json!([{"name":"root","insert":[[170]]},{"name":"edge","insert":[[172,171,1]]}]),
    ] {
        e.set_limits(json!({"max_steps":50000}).to_string())
            .unwrap();
        let native_before = e.versions.columns_read();
        let (result, work) =
            super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(work.data_scans.get() < 1000, "{work:?}");
        let native_columns = e.versions.columns_read() - native_before;
        assert!(
            native_columns < 20000,
            "{native_columns} native columns: {report}"
        );
        assert!(
            report["version_nodes"].as_u64().unwrap() < 10000,
            "{report}"
        );
        eprintln!(
            "unpartitioned recursive edit: {work:?}, {native_columns} native columns, {} version nodes",
            report["version_nodes"]
        );
    }
    e.set_limits("{}".into()).unwrap();
    assert_eq!(
        rows(
            &e,
            "SELECT * FROM distance WHERE c0>=170 AND c0<180 ORDER BY c0"
        ),
        json!([[170, 0], [171, 2], [172, 1], [173, 4]])
    );
}

#[test]
fn recursive_walks_keep_scalar_and_negative_blocks_in_iteration_caches() {
    let (_d, mut e) = executor();
    e.bind_snapshot(json!({"files":[{"path":"a.py","language":"python","source":"def first():\n    work()\n    def second():\n        other()\n"}]}).to_string()).unwrap();
    let functions = rows(
        &e,
        "SELECT n FROM node WHERE kind_name('python',kind)='function_definition' ORDER BY n",
    );
    assert_eq!(functions.as_array().unwrap().len(), 2);
    bind(&mut e, "start", &["n"], json!([functions[0].clone()]));
    bind(&mut e, "blocked", &["n"], json!([]));
    let p = ".decl node(d:number,n:node,k:symbol,s:number,e:number)\n.decl tape(n:node,p:node,s:number,d:number)\n.decl field(p:node,f:symbol,c:node)\n.decl start(n:node)\n.decl blocked(n:node)\n.input node,tape,field,start,blocked\n.decl scope(k:symbol)\nscope(\"function_definition\").\n.decl distance(n:node,v:min<>) widen 4\ndistance(n,0) :- start(n).\ndistance(n,d+c) :- node(_,n,_,_,_), nearest(n,a,scope), distance(a,d), !blocked(n), c=count : {field(a,_,_)}.\n.output distance\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    let expected = rows(&e, "SELECT * FROM distance ORDER BY c0");
    let nested = functions[1][0].clone();
    for patch in [
        json!([{"name":"blocked","insert":[[nested]]}]),
        json!([{"name":"blocked","delete":[[nested]]}]),
    ] {
        change(&mut e, patch);
    }
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), expected);
}

#[test]
fn recursive_global_aggregates_keep_physical_sentinels_and_cold_widening() {
    let (_d, mut e) = executor();
    bind(&mut e, "seed", &["v"], json!([[42]]));
    let p = ".decl seed(v:number)\n.input seed\n.decl amount(v:max<>) widen 3\n.decl item(v:number)\nitem(v) :- seed(v).\nitem(v) :- amount(v), v<6.\namount(n) :- n=count : {item(_)}.\n.output amount,item\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    for patch in [
        json!([{"name":"seed","delete":[[42]]}]),
        json!([{"name":"seed","insert":[[42],[47]]}]),
        json!([{"name":"seed","delete":[[42],[47]]}]),
    ] {
        change(&mut e, patch);
        check(
            &e,
            p,
            &[("seed", &["v"])],
            &["amount", "item", "__sql_changes_amount"],
        );
    }
}

#[test]
fn recursive_registered_joins_retract_unsupported_cycles_and_keep_empty_keys_absent() {
    let (_d, mut e) = executor();
    super::registered_joins::register_bits(&mut e);
    bind(
        &mut e,
        "edge",
        &["a", "b", "w"],
        json!([[0, 1, 1], [1, 2, 2], [2, 1, 4]]),
    );
    bind(&mut e, "root", &["n"], json!([[0]]));
    let p = ".lattice summary_bits<number> { Lub -> @summary_bits }\n.decl edge(a:number,b:number,w:number)\n.decl root(n:number)\n.input edge,root\n.decl mask(n:number,v:summary_bits<>) widen 20\nmask(n,0) :- root(n).\nmask(b,v) :- mask(a,v), edge(a,b,_).\nmask(b,w) :- mask(a,_), edge(a,b,w).\n.output mask\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    for patch in [
        json!([{"name":"edge","delete":[[2,1,4]]}]),
        json!([{"name":"root","delete":[[0]]}]),
        json!([{"name":"root","insert":[[1]]}]),
        json!([{"name":"edge","insert":[[2,1,4]]}]),
    ] {
        change(&mut e, patch);
        let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
        super::registered_joins::register_bits(&mut fresh);
        bind(
            &mut fresh,
            "edge",
            &["a", "b", "w"],
            rows(&e, "SELECT * FROM edge"),
        );
        bind(&mut fresh, "root", &["n"], rows(&e, "SELECT * FROM root"));
        fresh.run(p.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM mask ORDER BY c0"),
            rows(&fresh, "SELECT * FROM mask ORDER BY c0")
        );
    }
}

#[test]
fn recursive_aggregate_bodies_with_joins_use_retained_helpers() {
    let (_d, mut e) = executor();
    bind(&mut e, "seed", &["g", "v"], json!([[17, 42]]));
    bind(&mut e, "owner", &["g"], json!([[17], [18]]));
    bind(&mut e, "allowed", &["v"], json!([[0], [1], [2], [42]]));
    let p = ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.decl allowed(v:number)\n.input seed,owner,allowed\n.decl item(g:number,v:number)\n.decl amount(g:number,n:max<>) widen 20\nitem(g,v) :- seed(g,v).\nitem(g,v) :- amount(g,v), v<3.\namount(g,n) :- owner(g), n=count : {item(_,v), allowed(v)}.\n.output amount,item\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    for patch in [
        json!([{"name":"seed","delete":[[17,42]]}]),
        json!([{"name":"seed","insert":[[17,42]]},{"name":"allowed","delete":[[1]]}]),
    ] {
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        check(
            &e,
            p,
            &[
                ("seed", &["g", "v"]),
                ("owner", &["g"]),
                ("allowed", &["v"]),
            ],
            &["amount", "item"],
        );
    }
}

#[test]
fn recursive_splitters_retain_the_immutable_integer_domain() {
    let (_d, mut e) = executor();
    bind(&mut e, "owner", &["n"], json!([[1], [2]]));
    bind(&mut e, "seed", &["s"], json!([["a.b.c"]]));
    let p = ".decl owner(n:number)\n.decl seed(s:symbol)\n.input owner,seed\n.decl best(n:number,v:max<>) widen 20\nbest(n,0) :- owner(n).\nbest(n,strlen(m)) :- owner(n), best(_,v), v>=0, seed(s), split_dot(s,m,x).\n.output best\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    for patch in [
        json!([{"name":"seed","delete":[["a.b.c"]],"insert":[["long.word.suffix"]]}]),
        json!([{"name":"owner","delete":[[2]]},{"name":"seed","delete":[["long.word.suffix"]]}]),
    ] {
        change(&mut e, patch);
        let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
        bind(&mut fresh, "owner", &["n"], rows(&e, "SELECT * FROM owner"));
        bind(
            &mut fresh,
            "seed",
            &["s"],
            rows(&e, "SELECT sym_text(s) FROM seed"),
        );
        fresh.run(p.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM best ORDER BY c0"),
            rows(&fresh, "SELECT * FROM best ORDER BY c0")
        );
    }
}

#[test]
fn unsupported_checked_ctes_leave_existing_results_and_release_private_slots() {
    use super::super::super::logic::sql::{
        select::{Cte, FromItem, Query, Relation as Rel, Select},
        sql_ast::SqlExpression as E,
    };
    let (_d, e) = initial();
    let before = rows(&e, "SELECT * FROM distance ORDER BY c0");
    let slots = rows(
        &e,
        "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '*__it*'",
    );
    let mut run = e.executions[0].clone();
    run.temporal.clear();
    let emitted = Arc::make_mut(&mut run.emitted);
    let old = emitted.queries[0].take().unwrap();
    let width = old.body.columns.len();
    let mut ctes = old.ctes;
    ctes.push(Cte::walk("custom_extension", None, vec![old.body]));
    let q = Query {
        ctes,
        body: Select {
            columns: (0..width)
                .map(|i| {
                    (
                        E::Qualified {
                            alias: "x".into(),
                            column: format!("c{i}"),
                        },
                        Some(format!("c{i}")),
                    )
                })
                .collect(),
            from: vec![FromItem::cross(Rel::Cte("custom_extension".into()), "x")],
            ..Default::default()
        },
    };
    emitted.program.steps[0].sql = format!(
        "INSERT OR IGNORE INTO new_{} {}",
        emitted.program.steps[0].output,
        q.render()
    );
    emitted.queries[0] = Some(q);
    let tx = e.versions.transaction(&e.db).unwrap();
    let mut work = maintenance::Work::new(&e.db, &e.scratch);
    work.versions = Some(&e.versions);
    super::super::temporal::prepare(&mut work, &e.versions, &mut run, 999).unwrap();
    assert!(run.temporal.is_empty());
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '*__it*'"
        ),
        slots
    );
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), before);
    work.cleanup().unwrap();
    tx.commit();
}

#[test]
fn lifting_an_unrelated_walk_preserves_constant_seed_timing_and_widening() {
    let (_d, mut e) = executor();
    e.bind_snapshot(json!({"files":[{"path":"a.py","language":"python","source":"def first():\n    work()\n"}]}).to_string()).unwrap();
    bind(&mut e, "root", &["n"], json!([[1]]));
    let numeric = ".decl root(n:number)\n.input root\n.decl level_value(n:number,v:max<>) widen 2\nlevel_value(1,0).\nlevel_value(n,1) :- root(n).\nlevel_value(n,v+1) :- level_value(n,v), v<2.\n.output level_value\n";
    let full = format!(
        "{numeric}.decl node(d:number,n:node,k:symbol,s:number,e:number)\n.decl tape(n:node,p:node,s:number,d:number)\n.input node,tape\n.decl scope(k:symbol)\nscope(\"function_definition\").\n.decl owns(n:node,a:node)\nowns(n,a) :- node(_,n,\"call\",_,_), nearest(n,a,scope).\n.output owns\n"
    );
    e.run(full, "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT c1>1000000 FROM level_value WHERE c0=1"),
        json!([[1]])
    );
    for patch in [
        json!([{"name":"root","insert":[[2]]}]),
        json!([{"name":"root","delete":[[1],[2]]}]),
    ] {
        change(&mut e, patch);
        check(
            &e,
            numeric,
            &[("root", &["n"])],
            &["level_value", "__sql_changes_level_value"],
        );
    }
}

#[test]
fn sparse_extended_iterations_keep_negative_reducer_inputs_after_empty_frontiers() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "seed",
        &["g", "v"],
        json!((0..3000).map(|g| [g, 42]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..3000).map(|g| [g]).collect::<Vec<_>>()),
    );
    bind(&mut e, "blocked", &["g"], json!([]));
    let p = ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.decl blocked(g:number)\n.input seed,owner,blocked\n.decl amount(g:number,v:max<>) widen 20\n.decl item(g:number,v:number)\nitem(g,v) :- seed(g,v).\nitem(g,v) :- amount(g,v), v<4, !blocked(g).\namount(g,n) :- owner(g), n=count : {item(g,_)}.\n.output amount,item\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    e.connection().execute_batch("CREATE TEMP TRIGGER guard_count_delete BEFORE DELETE ON amount WHEN OLD.c0<>17 BEGIN SELECT RAISE(ABORT,'unrelated count delete'); END; CREATE TEMP TRIGGER guard_count_insert BEFORE INSERT ON amount WHEN NEW.c0<>17 BEGIN SELECT RAISE(ABORT,'unrelated count insert'); END").unwrap();
    for (patch, amount, items) in [
        (
            json!([{"name":"seed","delete":[[17,42]]}]),
            4,
            json!([[0], [1], [2], [3]]),
        ),
        (json!([{"name":"blocked","insert":[[17]]}]), 0, json!([])),
        (
            json!([{"name":"blocked","delete":[[17]]},{"name":"seed","insert":[[17,47]]}]),
            5,
            json!([[0], [1], [2], [3], [47]]),
        ),
    ] {
        e.set_limits(json!({"max_steps":50000}).to_string())
            .unwrap();
        let (result, work) =
            super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(work.data_scans.get() < 1000, "{work:?}");
        eprintln!("recursive negative count edit: {work:?}");
        e.set_limits("{}".into()).unwrap();
        assert_eq!(
            rows(&e, "SELECT c1 FROM amount WHERE c0=17"),
            json!([[amount]])
        );
        assert_eq!(
            rows(&e, "SELECT c1 FROM item WHERE c0=17 ORDER BY c1"),
            items
        );
        assert_eq!(
            rows(&e, "SELECT count(*) FROM amount WHERE c0<>17 AND c1=5"),
            json!([[2999]])
        );
    }
}

#[test]
fn recursive_sum_caches_defer_unused_group_overflow_and_restore_native_state() {
    let (_d, mut e) = executor();
    let max = wire::integer(i64::MAX);
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 1, 2], [2, 2, max], [3, 2, 1]]),
    );
    bind(&mut e, "owner", &["g"], json!([[1]]));
    let p = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl amount(g:number,v:max<>) widen 4\namount(g,s) :- owner(g), s=sum x : {event(_,g,x)}.\namount(g,v+1) :- amount(g,v), v<3.\n.output amount\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    assert!(
        e.apply_changes(json!([{"name":"owner","insert":[[2]]}]).to_string(), None)
            .unwrap_err()
            .reason
            .contains("integer overflow")
    );
    assert_eq!(rows(&e, "SELECT * FROM owner"), json!([[1]]));
    assert_eq!(rows(&e, "SELECT * FROM amount"), json!([[1, 3]]));
    change(
        &mut e,
        json!([{"name":"event","delete":[[2,2,max]]},{"name":"owner","insert":[[2]]}]),
    );
    check(
        &e,
        p,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["amount", "__sql_changes_amount"],
    );
}
