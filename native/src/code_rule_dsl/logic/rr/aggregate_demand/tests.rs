use super::*;
use crate::code_rule_dsl::logic::{
    front::{self, Literal},
    rr,
};

fn program(extra: &str) -> Program {
    rr::parse(&format!(
        r#"
.decl wanted(world: number, s: number, x: number)
.decl ancestor(world: number, s: number, p: number)
.decl binding(world: number, p: number, x: number, value: number)
.input wanted, ancestor, binding
.decl result(world: number, s: number, x: number, k: number)
.output result
{extra}
"#
    ))
    .unwrap()
}

#[test]
fn zero_count_is_an_antijoin_with_demanded_world_scope_and_name() {
    let mut p = program(
        "result(w,s,x,0) :- wanted(w,s,x), count : { ancestor(w,s,p), binding(w,p,x,v) } = 0.",
    );
    optimize(&mut p);
    let reader = p.rules.iter().find(|r| r.head.rel == "result").unwrap();
    assert!(
        !reader.body.iter().any(|l| matches!(l, Literal::Agg { .. })),
        "zero count still enumerates: {reader}"
    );
    assert!(
        reader
            .body
            .iter()
            .any(|l| matches!(l, Literal::Atom(a) if a.neg))
    );
    let helper = p
        .rules
        .iter()
        .find(|r| r.head.rel.starts_with("rr_agg") && !r.head.rel.contains("demand"))
        .unwrap();
    assert!(helper.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.starts_with("rr_demand_agg") && a.args.len() == 3)), "unrestricted helper: {helper}");
    front::strata(&p).unwrap();
}

#[test]
fn general_count_and_sum_restrict_groups_without_dropping_witness_columns() {
    for expression in ["count", "sum v"] {
        let mut p = program(&format!(
            "result(w,s,x,k) :- wanted(w,s,x), k = {expression} : {{ ancestor(w,s,p), binding(w,p,x,v) }}."
        ));
        optimize(&mut p);
        let helper = p
            .rules
            .iter()
            .find(|r| r.head.rel.starts_with("rr_agg") && !r.head.rel.contains("demand"))
            .unwrap();
        assert_eq!(
            helper.head.args.len(),
            5,
            "count/sum must retain distinct witnesses"
        );
        assert!(helper.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.starts_with("rr_demand_agg") && a.args.len() == 3)), "{helper}");
        front::strata(&p).unwrap();
    }
}

#[test]
fn recursive_consumer_does_not_create_an_aggregate_dependency_cycle() {
    let mut p = program(
        "result(w,s,x,0) :- wanted(w,s,x).\nresult(w,s,x,k) :- result(w,s,x,0), wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    optimize(&mut p);
    front::strata(&p).unwrap();
    let demands: Vec<_> = p
        .rules
        .iter()
        .filter(|r| r.head.rel.starts_with("rr_demand_agg"))
        .collect();
    assert!(!demands.is_empty());
    assert!(demands.iter().all(|r| {
        !r.body
            .iter()
            .filter_map(Literal::atom)
            .any(|a| a.rel == "result")
    }));
}

/// Actual helper rows stay proportional to demanded keys, despite unrelated scopes and names.
#[test]
fn aggregate_demand_executes_only_requested_groups_and_keeps_empty_groups() {
    use crate::code_rule_dsl::logic::plan;
    let mut source = String::from(
        r#"
.decl wanted(w:number,s:number,x:number)
.decl ancestor(w:number,s:number,p:number)
.decl binding(w:number,p:number,x:number,v:number)
.decl counts(w:number,s:number,x:number,k:number)
.decl sums(w:number,s:number,x:number,k:number)
.decl absent(w:number,s:number,x:number)
.output counts, sums, absent
wanted(1,1,1). wanted(2,1,1). wanted(3,1,1).
binding(1,1,1,7). binding(1,1,1,9). binding(1,1,1,9).
binding(2,1,1,20).
counts(w,s,x,k) :- wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.
sums(w,s,x,k) :- wanted(w,s,x), k = sum v : { ancestor(w,s,p), binding(w,p,x,v) }.
absent(w,s,x) :- wanted(w,s,x), count : { ancestor(w,s,p), binding(w,p,x,v) } = 0.
"#,
    );
    for w in 1..=2 {
        for s in 1..=200 {
            source.push_str(&format!("ancestor({w},{s},1).\n"));
        }
        for x in 2..=100 {
            source.push_str(&format!("binding({w},1,{x},10).\n"));
        }
    }
    let mut p = rr::parse(&source).unwrap();
    optimize(&mut p);
    let helpers: BTreeSet<_> = p
        .rules
        .iter()
        .filter(|r| r.head.rel.starts_with("rr_agg"))
        .map(|r| r.head.rel.clone())
        .collect();
    p.outputs.extend(helpers.iter().cloned());
    let file = crate::code_rule_dsl::prefilter::tests::temp();
    let dump = zstd::decode_all(
        std::fs::File::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/code_rule_dsl/logic/plan/golden/facts.sql.zst"),
        )
        .unwrap(),
    )
    .unwrap();
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch(std::str::from_utf8(&dump).unwrap())
        .unwrap();
    let result = plan::run_sql(&p, &file).unwrap();
    assert_eq!(
        result.outputs["counts"],
        ["1\t1\t1\t2", "2\t1\t1\t1", "3\t1\t1\t0"]
    );
    assert_eq!(
        result.outputs["sums"],
        ["1\t1\t1\t16", "2\t1\t1\t20", "3\t1\t1\t0"]
    );
    assert_eq!(result.outputs["absent"], ["3\t1\t1"]);
    for h in &helpers {
        assert_eq!(
            result.outputs[h].len(),
            3,
            "unrequested groups published in {h}"
        );
    }
    // Keep the frozen text-symbol extension compatible; unsupported reductions must fall back to SQL.
    let _serial = crate::code_rule_dsl::logic::profile::one_run();
    let was_on = crate::code_rule_dsl::logic::profile::on();
    crate::code_rule_dsl::logic::profile::reset();
    crate::code_rule_dsl::logic::profile::set(true);
    let native = plan::run_sql_native(&p, &file, "program", |db| {
        let rc = unsafe {
            crate::code_rule_dsl::logic::native::extension::sqlite3_extension_init(
                db.handle(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
        Ok(())
    })
    .unwrap();
    assert_eq!(native.outputs, result.outputs);
    // Measure the production atom-backed engine, not the frozen legacy extension above.
    crate::code_rule_dsl::logic::profile::reset();
    let path = crate::code_rule_dsl::prefilter::tests::temp();
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    let lowered = plan::lower::lower(&p, crate::code_rule_dsl::language::GrammarDialect::Python,
        &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
    let native = plan::run_atoms_native_over(&lowered, &db, "main", plan::Stats::empty(&lowered.program),
        BTreeSet::new(), "program").unwrap();
    assert_eq!(native.outputs, result.outputs);
    assert!(native.native.contains("sums"), "production sum must use the native reduction");
    let dir = file.with_extension("profile");
    crate::code_rule_dsl::logic::profile::write(&dir, "Demanded aggregate growth gate").unwrap();
    crate::code_rule_dsl::logic::profile::set(was_on);
    let trace = std::fs::read_to_string(dir.join("native.tsv")).unwrap();
    let mut measured = 0;
    for line in trace.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if !cols[3].contains("seed rr_agg") {
            continue;
        }
        measured += 1;
        for step in cols[2].split(',') {
            let (_, output) = step.split_once(':').unwrap();
            assert!(
                output.parse::<u64>().unwrap() <= 3,
                "unrequested native join expansion: {line}"
            );
        }
    }
    assert!(
        measured >= helpers.len(),
        "missing native helper measurements: {trace}"
    );
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_file(file).unwrap();
}

#[test]
fn already_restricted_helpers_are_stable_and_recursive_only_guards_are_refused() {
    let mut p = program(
        "result(w,s,x,k) :- wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    let before = p.rules.clone();
    optimize(&mut p);
    assert_eq!(p.rules, before, "optimization must be idempotent");
    let p = program(
        "result(w,s,x,0) :- wanted(w,s,x).\nresult(w,s,x,k) :- result(w,s,x,0), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    assert!(
        !p.rules
            .iter()
            .any(|r| r.head.rel.starts_with("rr_demand_agg")),
        "recursive result is not a safe demand source"
    );
    front::strata(&p).unwrap();
}

#[test]
fn multiple_consumers_union_their_safe_demands() {
    // Model reuse of a private helper after lowering; consumers may demand different keys.
    let mut p = program(
        "result(w,s,x,k) :- wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    let demand = p
        .rules
        .iter()
        .find(|r| r.head.rel.starts_with("rr_demand_agg"))
        .unwrap()
        .head
        .rel
        .clone();
    p.rules.retain(|r| r.head.rel != demand);
    p.decls.remove(&demand);
    for r in &mut p.rules {
        r.body.retain(|l| l.atom().is_none_or(|a| a.rel != demand));
    }
    let mut second = p
        .rules
        .iter()
        .find(|r| r.head.rel == "result")
        .unwrap()
        .clone();
    second.body.push(Literal::Cmp {
        l: Term::Var("w".into()),
        op: "=",
        r: Term::Int(2),
    });
    p.rules.push(second);
    optimize(&mut p);
    assert_eq!(
        p.rules
            .iter()
            .filter(|r| r.head.rel.starts_with("rr_demand_agg"))
            .count(),
        2
    );
    front::strata(&p).unwrap();
}

#[test]
#[ignore = "pyflakes.rr writes the outer binding as a rule (`outer_bound`, 913cb3f7a), not an aggregate over bound_in: no rr_agg helper reads it"]
fn pyflakes_outer_binding_existence_keeps_load_and_nearest_scope_demands() {
    let p = rr::parse("#include \"python/pyflakes.rr\"\n.output unbound_ref\n").unwrap();
    let helpers: Vec<_> = p
        .rules
        .iter()
        .filter(|r| {
            r.head.rel.starts_with("rr_agg")
                && r.body
                    .iter()
                    .any(|l| l.atom().is_some_and(|a| a.rel == "bound_in"))
        })
        .collect();
    assert!(!helpers.is_empty());
    for helper in helpers {
        let guard = helper
            .body
            .iter()
            .filter_map(Literal::atom)
            .find(|a| a.rel.starts_with("rr_demand_agg"))
            .expect("outer bindings still expanded globally");
        let demands: Vec<_> = p.rules.iter().filter(|r| r.head.rel == guard.rel).collect();
        assert!(
            demands.iter().all(|r| r
                .body
                .iter()
                .any(|l| l.atom().is_some_and(|a| a.rel == "load_ref"))),
            "load demand lost: {demands:?}"
        );
        assert!(
            demands
                .iter()
                .all(|r| r.body.iter().any(|l| matches!(l, Literal::Nearest { .. }))),
            "nearest scope delayed: {demands:?}"
        );
    }
    front::strata(&p).unwrap();
}


#[test]
fn nested_zero_counts_preserve_outer_groups_in_sql_and_native_execution() {
    use crate::code_rule_dsl::logic::{native, plan};
    let p = rr::parse(r#"
.decl ident(x:symbol)
ident("a"). ident("b").
.decl fn(f:number)
fn(1). fn(2). fn(3).
.decl ref(n:number,f:number,x:symbol)
ref(10,1,"a"). ref(11,1,"b"). ref(20,2,"a").
.decl hit(f:number)
.output hit
hit(f) :- fn(f), count : { ident(x), count : { ref(n,f,x) } = 0 } > 0.
"#).unwrap();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::prefilter::tests::temp();
        let store = crate::content_store::ContentStore::open(&path).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        native::register(&db).unwrap();
        let lowered = plan::lower::lower(&p, crate::code_rule_dsl::language::GrammarDialect::Python,
            &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
        let result = plan::run_atoms_native_over(&lowered, &db, "main", plan::Stats::empty(&lowered.program),
            BTreeSet::new(), modes).unwrap();
        assert_eq!(result.outputs["hit"], ["2", "3"], "outer groups lost in {modes:?}");
    }
}
