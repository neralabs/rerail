use super::*;

#[test]
fn original_cpython_programs_use_private_projections_and_bounded_join_work() {
    let raw =
        zstd::decode_all(&include_bytes!("../../facts/vtab/golden/corpus.json.zst")[..]).unwrap();
    let corpus: Vec<(String, String)> = serde_json::from_slice(&raw).unwrap();
    for (program, output, count) in [
        (
            include_str!("../../logic/plan/golden/programs/workload.rr"),
            "type_of",
            1841,
        ),
        (
            include_str!("../../logic/plan/golden/programs/calls.rr"),
            "call_target",
            256,
        ),
    ] {
        let (_dir, mut e) = executor();
        e.bind_snapshot(json!({"files":corpus.iter().map(|(p,s)|json!({"path":p,"source":s,"language":"python"})).collect::<Vec<_>>()} ).to_string()).unwrap();
        e.set_limits(json!({"max_steps":200_000_000}).to_string())
            .unwrap();
        e.run(program.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, &format!("SELECT count(*) FROM {output}")),
            json!([[count]])
        );
        // Public facts, including loc-wide docof, survive the compiler's private view bindings.
        assert!(
            rows(&e, "SELECT count(*) FROM docof")[0][0]
                .as_u64()
                .unwrap()
                >= 16048
        );
        assert!(
            rows(&e, "SELECT count(*) FROM kind")[0][0]
                .as_u64()
                .unwrap()
                > 0
        );
    }
}

#[test]
fn large_integers_and_typed_symbols_round_trip_without_javascript_numbers() {
    let (_dir, mut e) = executor();
    let large = json!({"integer":"9007199254740993"});
    e.bind_relation(json!({"name":"labels","columns":["id","label"],"types":["number","symbol"],"rows":[[large,"HELLO"]]}).to_string()).unwrap();
    assert_eq!(
        rows(&e, "SELECT id,sym_text(label) FROM labels"),
        json!([[large, "HELLO"]])
    );
    let q: Value = serde_json::from_str(
        &e.query(
            "SELECT ?1, -9223372036854775808, 9223372036854775807".into(),
            Some(json!([large]).to_string()),
            None,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        q["rows"],
        json!([[large,{"integer":"-9223372036854775808"},{"integer":"9223372036854775807"}]])
    );
    for value in [
        json!({"integer":"9223372036854775808"}),
        json!(1.5),
        json!(null),
    ] {
        assert!(
            e.bind_relation(json!({"name":"bad","columns":["v"],"rows":[[value]]}).to_string())
                .is_err()
        );
    }
    assert!(
        e.bind_relation(
            json!({"name":"bad","columns":["v"],"types":["number"],"rows":[["hello"]]}).to_string()
        )
        .is_err()
    );
}

#[test]
fn grammar_ids_decode_with_their_dialect_and_domain() {
    let (_dir, mut e) = executor();
    e.bind_snapshot(json!({"files":[{"path":"a.py","language":"python","source":"def hello():\n    return 1\n"}]}).to_string()).unwrap();
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM node WHERE kind_name('python',kind)='function_definition'"
        ),
        json!([[1]])
    );
    assert!(
        rows(
            &e,
            "SELECT count(*) FROM field WHERE field_name('python',f)='name'"
        )[0][0]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        rows(
            &e,
            "SELECT kind_name('python',-1),field_name('python',9223372036854775807)"
        ),
        json!([[null, null]])
    );
}

#[test]
fn boolean_relations_and_aggregate_only_rules_cover_empty_and_nonempty_inputs() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"flag","columns":[],"rows":[[]]}).to_string())
        .unwrap();
    e.bind_relation(json!({"name":"item","columns":["id"],"rows":[[1],[2]]}).to_string())
        .unwrap();
    let program = ".decl flag()\n.decl item(n:number)\n.input flag,item\n.decl blocked()\n.decl allowed()\nallowed() :- flag(), !blocked().\n.decl absent()\nabsent() :- !blocked().\n.decl total(n:number)\ntotal(n) :- n = count : { item(_) }.\n.output allowed,total,absent\n";
    let run: Value =
        serde_json::from_str(&e.run(program.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(run["logical_arities"]["allowed"], 0);
    assert_eq!(
        rows(&e, "SELECT EXISTS(SELECT 1 FROM absent)"),
        json!([[1]])
    );
    assert_eq!(
        rows(
            &e,
            "SELECT EXISTS(SELECT 1 FROM allowed), (SELECT c0 FROM total)"
        ),
        json!([[1, 2]])
    );
    e.bind_relation(json!({"name":"item","columns":["id"],"rows":[]}).to_string())
        .unwrap();
    e.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&e, "SELECT c0 FROM total"), json!([[0]]));
    assert!(
        e.run(
            ".decl bad(x:number,n:number)\nbad(x,n) :- n = count : { item(_) }.\n.output bad\n"
                .into(),
            "python".into(),
            None
        )
        .is_err()
    );
}

#[test]
fn shortest_paths_min_max_and_zero_key_lattices_merge_improved_values() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"weighted","columns":["a","b","w"],"rows":[[0,1,5],[0,2,1],[2,1,1],[1,3,2],[2,3,10],[3,2,1]]}).to_string()).unwrap();
    let program = ".decl weighted(a:number,b:number,w:number)\n.input weighted\n.decl distance(n:number,v:min<>) widen 100\ndistance(0,0).\ndistance(b,d+w) :- distance(a,d), weighted(a,b,w).\n.decl best(v:min<>) widen 100\nbest(v) :- distance(_,v).\n.output distance,best\n";
    e.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM distance ORDER BY c0"),
        json!([[0, 0], [1, 2], [2, 1], [3, 4]])
    );
    assert_eq!(rows(&e, "SELECT c1 FROM best"), json!([[0]]));
    e.run(".decl growing(n:number,v:max<>) widen 2\ngrowing(1,0).\ngrowing(n,v+1) :- growing(n,v), v<100.\n.output growing\n".into(),"python".into(),None).unwrap();
    assert_eq!(
        rows(&e, "SELECT c1 FROM growing"),
        json!([[wire::integer(super::super::super::logic::native::TOP)]])
    );
    e.run(
        ".decl one(v:flat<>)\none(1). one(2).\n.output one\n".into(),
        "python".into(),
        None,
    )
    .unwrap();
    assert_eq!(
        rows(&e, "SELECT c1 FROM one"),
        json!([[wire::integer(super::super::super::logic::native::TOP)]])
    );
}

#[test]
fn reruns_are_atomic_and_invalidation_preserves_independent_results() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3], [3, 4]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    let consumer = ".decl reach(a:number,b:number)\n.input reach\n.decl ends(n:number)\nends(n) :- reach(1,n).\n.output ends\n";
    e.run(consumer.into(), "python".into(), None).unwrap();
    e.run(
        ".decl unrelated(n:number)\nunrelated(42).\n.output unrelated\n".into(),
        "python".into(),
        None,
    )
    .unwrap();
    assert!(e.run(CLOSURE.into(), "python".into(), Some(1)).is_err());
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[6]]));
    assert_eq!(rows(&e, "SELECT count(*) FROM ends"), json!([[3]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert!(e.query("SELECT * FROM ends".into(), None, None).is_err());
    e.run(consumer.into(), "python".into(), None).unwrap();
    edge(&mut e, json!([[8, 9]]));
    assert!(e.query("SELECT * FROM reach".into(), None, None).is_err());
    assert!(e.query("SELECT * FROM ends".into(), None, None).is_err());
    assert_eq!(rows(&e, "SELECT * FROM unrelated"), json!([[42]]));
    e.reset().unwrap();
    assert!(
        e.query("SELECT * FROM unrelated".into(), None, None)
            .is_err()
    );
    assert_eq!(rows(&e, "SELECT * FROM edge"), json!([[8, 9]]));
}

#[test]
fn budgets_stop_single_queries_and_rule_runs_and_leave_the_session_usable() {
    let (_dir, mut e) = executor();
    e.set_limits(json!({"max_steps":5000}).to_string()).unwrap();
    let query = "WITH RECURSIVE x(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM x WHERE n<10000000) SELECT sum(n) FROM x";
    assert!(
        e.query(query.into(), None, None)
            .unwrap_err()
            .reason
            .contains("interrupt")
    );
    assert_eq!(rows(&e, "SELECT 42"), json!([[42]]));
    edge(
        &mut e,
        json!((0..128).map(|n| vec![n, n + 1]).collect::<Vec<_>>()),
    );
    assert!(e.run(CLOSURE.into(), "python".into(), None).is_err());
    e.set_limits("{}".into()).unwrap();
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[8256]]));
    e.set_limits(json!({"timeout_ms":0}).to_string()).unwrap();
    assert!(e.query(query.into(), None, None).is_err());
    e.set_limits("{}".into()).unwrap();
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[8256]]));
}

#[test]
fn closure_work_is_bounded_by_candidates_not_the_accumulated_relation() {
    let (_dir, mut e) = executor();
    edge(
        &mut e,
        json!((0..1023).map(|n| vec![n, n + 1]).collect::<Vec<_>>()),
    );
    e.set_limits(json!({"max_steps":120_000_000}).to_string())
        .unwrap();
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[523776]]));
    e.create_index("reach".into(), vec![1]).unwrap();
    let plan = rows(&e, "EXPLAIN QUERY PLAN SELECT c0 FROM reach WHERE c1=1023").to_string();
    assert!(plan.contains("__sql_index_reach_1"), "{plan}");
    assert!(e.create_index("atoms".into(), vec![1]).is_err());
    assert!(e.create_index("reach".into(), vec![2]).is_err());
}

#[test]
fn custom_lattice_operations_require_registration_and_execute_in_sql() {
    use rusqlite::functions::{Aggregate, Context, FunctionFlags};
    struct Bits;
    impl Aggregate<i64, i64> for Bits {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<i64> {
            Ok(0)
        }
        fn step(&self, ctx: &mut Context<'_>, value: &mut i64) -> rusqlite::Result<()> {
            *value |= ctx.get::<i64>(0)?;
            Ok(())
        }
        fn finalize(&self, _: &mut Context<'_>, value: Option<i64>) -> rusqlite::Result<i64> {
            Ok(value.unwrap_or(0))
        }
    }
    let (_dir, mut e) = executor();
    let spec =
        json!({"aggregate":"sql_bit_or","merge":"sql_bit_union","widen":"sql_bit_union","top":7});
    assert!(e.register_lattice("bits".into(), spec.to_string()).is_err());
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    e.connection()
        .create_aggregate_function(c"sql_bit_or", 1, det, Bits)
        .unwrap();
    e.connection()
        .create_scalar_function(c"sql_bit_union", 2, det, |ctx| {
            Ok(ctx.get::<i64>(0)? | ctx.get::<i64>(1)?)
        })
        .unwrap();
    e.register_lattice("bits".into(), spec.to_string()).unwrap();
    e.run(".lattice bits<number> { Lub -> @bits }\n.decl permission(k:number,v:bits<>) widen 3\npermission(1,1). permission(1,2). permission(2,4).\n.output permission\n".into(),"python".into(),None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM permission ORDER BY c0"),
        json!([[1, 3], [2, 4]])
    );
}

#[test]
fn lattice_top_has_the_correct_domain_and_absorbs_arithmetic_transfers() {
    let (_dir, mut e) = executor();
    e.run(".decl unknown(k:number,v:min<>) widen 3\nunknown(0,top).\nunknown(1,v+5) :- unknown(0,v).\n.decl known(k:number)\nknown(k) :- unknown(k,v), x=v, x != top.\n.output unknown,known\n".into(),"python".into(),None).unwrap();
    let top = wire::integer(-super::super::super::logic::native::TOP);
    assert_eq!(
        rows(&e, "SELECT * FROM unknown ORDER BY c0"),
        json!([[0, top], [1, top]])
    );
    assert_eq!(rows(&e, "SELECT count(*) FROM known"), json!([[0]]));
}

#[test]
fn corpus_sql_uses_the_shared_wave_kernel_for_min_lattices() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), "x=1\n").unwrap();
    let program = ".decl seed(k:number,v:number)\nseed(1,3). seed(1,8).\n.decl smallest(k:number,v:min<>) widen 10\nsmallest(k,v) :- seed(k,v).\n.output smallest\n";
    let result = super::super::super::tools::logic(
        &[program.into()],
        &root,
        None,
        &dir.path().join("out"),
        "",
        false,
        1024,
    );
    result.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/smallest.csv")).unwrap(),
        "1\t3\n"
    );
}

#[test]
fn both_sql_entry_points_share_custom_lattices_and_logical_zero_arity_outputs() {
    use crate::code_rule_dsl::logic::native::{self, Lattice};
    struct Bits;
    impl Lattice for Bits {
        fn lub(&self, a: i64, b: i64) -> i64 {
            a | b
        }
        fn top(&self) -> i64 {
            7
        }
        fn finite(&self) -> bool {
            true
        }
    }
    native::declare("shared_sql_test_bits", Arc::new(Bits));
    let program = ".lattice shared_sql_test_bits<number> { Lub -> @shared_sql_test_bits }\n.decl seed(k:number,v:number)\nseed(1,3). seed(1,8).\n.decl smallest(k:number,v:min<>) widen 10\nsmallest(k,v) :- seed(k,v).\n.decl largest(k:number,v:max<>) widen 10\nlargest(k,v) :- seed(k,v).\n.decl best(v:min<>) widen 10\nbest(v) :- seed(_,v).\n.decl permission(k:number,v:shared_sql_test_bits<>) widen 3\n.decl bit(k:number,v:number)\nbit(1,1). bit(1,2).\npermission(k,v) :- bit(k,v).\n.decl flag()\nflag() :- seed(1,3).\n.decl total(n:number)\ntotal(n) :- n = count : {seed(_,_)}.\n.output smallest,largest,best,permission,flag,total\n";
    let (_d, mut e) = executor();
    e.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM permission"), json!([[1, 3]]));
    assert_eq!(rows(&e, "SELECT c1 FROM best"), json!([[3]]));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), "x=1\n").unwrap();
    super::super::super::tools::logic(
        &[program.into()],
        &root,
        None,
        &dir.path().join("out"),
        "",
        false,
        1024,
    )
    .unwrap();
    for (name, text) in [
        ("smallest", "1\t3\n"),
        ("largest", "1\t8\n"),
        ("best", "3\n"),
        ("permission", "1\t3\n"),
        ("flag", "\n"),
        ("total", "2\n"),
    ] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(format!("out/{name}.csv")))
                .unwrap_or_else(|e| panic!("{name}: {e}")),
            text,
            "{name}"
        );
    }
}

#[test]
fn frontend_boolean_units_preserve_logical_arity_and_round_trip() {
    let (_dir,mut e)=executor();
    e.bind_relation(json!({"name":"flag","columns":[],"rows":[[]]}).to_string()).unwrap();
    let text=".decl flag()\n.input flag\n.decl copied()\ncopied() :- flag().\n.decl ordinary(rr_unit:number)\nordinary(7).\n.output copied,ordinary\n";
    let parsed=rr::parse(text).unwrap();let printed=rr::print(&parsed).unwrap();let again=rr::parse(&printed).unwrap();
    assert_eq!(parsed.nullary,again.nullary);assert_eq!(parsed.decls,again.decls);assert_eq!(parsed.rules,again.rules);
    let lower:Value=serde_json::from_str(&e.lower(text.into(),"python".into()).unwrap()).unwrap();
    assert_eq!(lower["logical_arities"]["flag"],0);assert_eq!(lower["logical_arities"]["copied"],0);assert_eq!(lower["logical_arities"]["ordinary"],1);
    e.run(text.into(),"python".into(),None).unwrap();assert_eq!(rows(&e,"SELECT * FROM ordinary"),json!([[7]]));
    e.apply_changes(json!([{"name":"flag","delete":[[]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&e,"SELECT count(*) FROM copied"),json!([[0]]));
}

#[test]
fn combined_corpus_programs_keep_boolean_output_metadata_after_renaming() {
    let dir=tempfile::tempdir().unwrap();let root=dir.path().join("src");std::fs::create_dir(&root).unwrap();std::fs::write(root.join("a.py"),"x=1\n").unwrap();
    let text=".decl seed(n:number)\nseed(1).\n.decl flag()\nflag() :- seed(1).\n.output flag\n";
    crate::code_rule_dsl::tools::logic(&[text.into(),text.into()],&root,None,&dir.path().join("out"),"",false,1024).unwrap();
    for name in ["p0_flag","p1_flag"]{assert_eq!(std::fs::read_to_string(dir.path().join(format!("out/{name}.csv"))).unwrap(),"\n");}
}
