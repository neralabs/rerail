use super::*;
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, RuleRef, T, TermRef, Version, about::About, analyses};
use crate::code_rule_dsl::logic::rr;
use std::sync::Arc;

fn program(extra: &str) -> Arc<Version> {
    rr::parse_version(&format!(
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

/// The rules of `v` whose head's relation passes `keep`.
fn rules<'v>(v: &'v Version, keep: impl Fn(&str) -> bool) -> Vec<RuleRef<'v>> {
    v.rules().filter(|r| keep(v.rel_name(r.head().rel()))).collect()
}

/// Rule `r`'s body atoms: each one's relation, whether it is negated, and its arguments.
fn atoms<'v>(v: &'v Version, r: RuleRef<'v>) -> Vec<(&'v str, bool, Vec<TermRef<'v>>)> {
    r.body().filter_map(|l| l.atom()).map(|a| (v.rel_name(a.rel()), a.neg(), a.args().collect())).collect()
}

/// `v` stratifies.
fn stratifies(v: &Version) {
    analyses::strata(v, &About::of(v)).unwrap();
}

/// The helper and its demand guard: the helper reads the guard over three columns.
fn guarded_helper(p: &Version) -> RuleRef<'_> {
    let helper = rules(p, |h| h.starts_with("rr_agg") && !h.contains("demand"))[0];
    assert!(atoms(p, helper).iter().any(|(rel, _, args)| rel.starts_with("rr_demand_agg") && args.len() == 3), "unrestricted helper: {}", p.show_rule(helper));
    helper
}

#[test]
fn zero_count_is_an_antijoin_with_demanded_world_scope_and_name() {
    let p = program(
        "result(w,s,x,0) :- wanted(w,s,x), count : { ancestor(w,s,p), binding(w,p,x,v) } = 0.",
    );
    let reader = rules(&p, |h| h == "result")[0];
    assert!(!reader.body().any(|l| matches!(l.view(), Lit::Agg { .. })), "zero count still enumerates: {}", p.show_rule(reader));
    assert!(atoms(&p, reader).iter().any(|(_, neg, _)| *neg));
    guarded_helper(&p);
    stratifies(&p);
}

#[test]
fn general_count_and_sum_restrict_groups_without_dropping_witness_columns() {
    for expression in ["count", "sum v"] {
        let p = program(&format!(
            "result(w,s,x,k) :- wanted(w,s,x), k = {expression} : {{ ancestor(w,s,p), binding(w,p,x,v) }}."
        ));
        let helper = guarded_helper(&p);
        assert_eq!(helper.head().args().count(), 5, "count/sum must retain distinct witnesses");
        stratifies(&p);
    }
}

#[test]
fn recursive_consumer_does_not_create_an_aggregate_dependency_cycle() {
    let p = program(
        "result(w,s,x,0) :- wanted(w,s,x).\nresult(w,s,x,k) :- result(w,s,x,0), wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    stratifies(&p);
    let demands = rules(&p, |h| h.starts_with("rr_demand_agg"));
    assert!(!demands.is_empty());
    assert!(demands.iter().all(|&r| !atoms(&p, r).iter().any(|(rel, _, _)| *rel == "result")));
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
    let p = rr::parse_version(&source).unwrap();
    let helpers: BTreeSet<String> = p
        .rules()
        .map(|r| p.rel_name(r.head().rel()))
        .filter(|rel| rel.starts_with("rr_agg"))
        .map(str::to_owned)
        .collect();
    let p = {
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&p);
        let outputs: Vec<String> = p.output_names().into_iter().chain(helpers.iter().cloned()).collect();
        b.fields().set_outputs(&outputs);
        b.finish().unwrap().0
    };
    let file = crate::code_rule_dsl::fixtures::temp();
    let dump = zstd::decode_all(
        std::fs::File::open(
            rerail_store::source_tree::native()
                .join("src/code_rule_dsl/logic/plan/golden/facts.sql.zst"),
        )
        .unwrap(),
    )
    .unwrap();
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch(std::str::from_utf8(&dump).unwrap())
        .unwrap();
    let result = plan::run_sql(p.clone(), &file).unwrap();
    assert_eq!(
        result.shown()["counts"],
        [["1", "1", "1", "2"], ["2", "1", "1", "1"], ["3", "1", "1", "0"]]
    );
    assert_eq!(
        result.shown()["sums"],
        [["1", "1", "1", "16"], ["2", "1", "1", "20"], ["3", "1", "1", "0"]]
    );
    assert_eq!(result.shown()["absent"], [["3", "1", "1"]]);
    for h in &helpers {
        assert_eq!(
            result.shown()[h.as_str()].len(),
            3,
            "unrequested groups published in {h}"
        );
    }
    // Keep the frozen text-symbol extension compatible; unsupported reductions must fall back to SQL.
    let _serial = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let was_on = crate::code_rule_dsl::logic::profile::on();
    crate::code_rule_dsl::logic::profile::reset();
    crate::code_rule_dsl::logic::profile::set(true);
    let native = plan::run_sql_native(p.clone(), &file, "program", |db| {
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
    assert_eq!(native.shown(), result.shown());
    // Measure the production atom-backed engine, not the frozen legacy extension above.
    crate::code_rule_dsl::logic::profile::reset();
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    let lowered = plan::lower::lowered(p, crate::code_rule_dsl::language::GrammarDialect::Python,
        &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
    let native = plan::run_atoms_native_over(&lowered, &db, "main", plan::Stats::empty_of_lowered(&lowered).unwrap(),
        BTreeSet::new(), "program").unwrap();
    assert_eq!(native.shown(), result.shown());
    assert!(native.native.contains("sums"), "production sum must use the native reduction");
    let dir = file.with_extension("profile");
    crate::code_rule_dsl::logic::profile::report::write(&dir, "Demanded aggregate growth gate").unwrap();
    crate::code_rule_dsl::logic::profile::set(was_on);
    let trace = std::fs::read_to_string(dir.join("native.tsv")).unwrap();
    let mut measured = 0;
    for line in trace.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        // The profile is the process's, and a test beside this one records its own program's steps into it, helpers
        // of the same names among them (a sibling's `seed rr_agg2($f, $x)` once read as this run's growth). This
        // program's helper seeds read their own demand: `rr_demand_agg_<helper>($s, $w, $x)`.
        if !helpers.iter().any(|h| cols[3].starts_with(&format!("seed {h}(")) && cols[3].contains(&format!("rr_demand_agg_{h}($s, $w, $x)"))) {
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
        "missing native helper measurements: {trace}\nplacements:\n{}",
        std::fs::read_to_string(dir.join("placement.tsv")).unwrap_or_default()
    );
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_file(file).unwrap();
}

#[test]
fn already_restricted_helpers_are_stable_and_recursive_only_guards_are_refused() {
    let text = "result(w,s,x,k) :- wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.";
    let shown = |v: &Version| v.rules().map(|r| v.show_rule(r)).collect::<Vec<_>>();
    assert_eq!(shown(&program(text)), shown(&program(text)), "optimization must be idempotent");
    let p = program(
        "result(w,s,x,0) :- wanted(w,s,x).\nresult(w,s,x,k) :- result(w,s,x,0), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    assert!(rules(&p, |h| h.starts_with("rr_demand_agg")).is_empty(), "recursive result is not a safe demand source");
    stratifies(&p);
}

#[test]
fn multiple_consumers_union_their_safe_demands() {
    use crate::code_rule_dsl::logic::program_image::{Builder, LitRef, Rewrite, fields::FieldsMut, manager::Pipeline, t_of};
    // Model reuse of a private helper after lowering; consumers may demand different keys.
    let p = program(
        "result(w,s,x,k) :- wanted(w,s,x), k = count : { ancestor(w,s,p), binding(w,p,x,v) }.",
    );
    let demand = p.rel_name(rules(&p, |h| h.starts_with("rr_demand_agg"))[0].head().rel()).to_owned();
    // The program without the demand: its rules, its declaration and its reads go.
    struct Unread<'a>(&'a Version, &'a str);
    impl Rewrite for Unread<'_> {
        fn keep(&mut self, l: LitRef<'_>) -> bool {
            l.atom().is_none_or(|a| self.0.rel_name(a.rel()) != self.1)
        }
    }
    let mut b = Builder::derive(&p);
    b.fields().undeclare(&demand);
    for r in p.rules().filter(|r| p.rel_name(r.head().rel()) != demand) {
        b.rewrite(&p, r, &mut Unread(&p, &demand)).finish().unwrap();
    }
    // A second reader of `result`'s rule, at another key.
    let result = rules(&p, |h| h == "result")[0];
    let head: Vec<T<'_>> = result.head().args().map(|t| t_of(result, t)).collect();
    let mut second = b.rule().head("result", &head);
    for l in result.body().filter(|l| l.atom().is_none_or(|a| p.rel_name(a.rel()) != demand)) {
        second = second.lit(&p, result, l);
    }
    second.cmp(T::Var("w"), CmpOp::Eq, T::Int(2)).finish().unwrap();
    let (v, _) = b.finish().unwrap();
    // The pass placed again, on the version of the edited program.
    let mut pipe = Pipeline::new("test", v);
    pipe.program(&mut crate::code_rule_dsl::logic::plan::corpus::preparation::lowering_tail::AggregateDemand);
    let (out, _) = pipe.finish().unwrap();
    assert_eq!(rules(&out, |h| h.starts_with("rr_demand_agg")).len(), 2, "{}", out.show());
    stratifies(&out);
}

#[test]
#[ignore = "pyflakes.rerail writes the outer binding as a rule (`outer_bound`, 913cb3f7a), not an aggregate over bound_in: no rr_agg helper reads it"]
fn pyflakes_outer_binding_existence_keeps_load_and_nearest_scope_demands() {
    let p = rr::parse_version("#include \"python/pyflakes.rerail\"\n.output unbound_ref\n").unwrap();
    let helpers: Vec<_> = p.rules().filter(|&r| p.rel_name(r.head().rel()).starts_with("rr_agg") && atoms(&p, r).iter().any(|(rel, _, _)| *rel == "bound_in")).collect();
    assert!(!helpers.is_empty());
    for helper in helpers {
        let guard = atoms(&p, helper).into_iter().find(|(rel, _, _)| rel.starts_with("rr_demand_agg")).expect("outer bindings still expanded globally").0;
        let demands = rules(&p, |h| h == guard);
        let shown = demands.iter().map(|&r| p.show_rule(r)).collect::<Vec<_>>();
        assert!(demands.iter().all(|&r| atoms(&p, r).iter().any(|(rel, _, _)| *rel == "load_ref")), "load demand lost: {shown:?}");
        assert!(demands.iter().all(|&r| r.body().any(|l| matches!(l.view(), Lit::Nearest { .. }))), "nearest scope delayed: {shown:?}");
    }
    stratifies(&p);
}

#[test]
fn nested_zero_counts_preserve_outer_groups_in_sql_and_native_execution() {
    use crate::code_rule_dsl::logic::{native, plan};
    let p = rr::parse_version(r#"
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
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = crate::content_store::ContentStore::open(&path).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        native::register(&db).unwrap();
        let lowered = plan::lower::lowered(p.clone(), crate::code_rule_dsl::language::GrammarDialect::Python,
            &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
        let result = plan::run_atoms_native_over(&lowered, &db, "main", plan::Stats::empty_of_lowered(&lowered).unwrap(),
            BTreeSet::new(), modes).unwrap();
        assert_eq!(result.shown()["hit"], [["2"], ["3"]], "outer groups lost in {modes:?}");
    }
}

/// A guard that reads a key only through a comparison (`byte(q, 92)` with `q = p - 1`) restricts the demand too: the
/// demand of an aggregate that reads `p` in its body, asked where the byte before p is a backslash, is those
/// positions, not every byte (the pairs below each, every pair of the text's bytes, were 17.9 M rows of a 6 KB text).
/// That is the aggregate as written (`RERAIL_ASOF=off`): by default it is bounded by its probe (decision 0511), one
/// helper of the bytes alone, and asks no demand.
#[test]
fn a_guard_reached_through_a_comparison_restricts_the_demand() {
    let src = r#"
.decl byte(i: number, v: number)
.input byte
.decl odd(p: number)
.output odd
odd(p) :- byte(p, _), q = @sub(p, 1), byte(q, 92), m = max j : { byte(j, v), v != 92, j < p }, @sub(p, m) = 3.
"#;
    let bounded = rr::parse_version(src).unwrap();
    assert!(rules(&bounded, |h| h.starts_with("rr_demand_agg") || h.starts_with("rr_ctx")).is_empty(), "a bounded aggregate asks no demand and keeps no context of its probes: {}", bounded.show());
    stratifies(&bounded);
    let _as_written = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[("RERAIL_ASOF", "off")]);
    let p = rr::parse_version(src).unwrap();
    let demand = *rules(&p, |h| h.starts_with("rr_demand_agg")).first().unwrap_or_else(|| panic!("no demand: {}", p.show()));
    assert!(
        atoms(&p, demand).iter().any(|(rel, _, args)| *rel == "byte" && matches!(args[1], TermRef::Int(92))),
        "the demand reads every byte: {}",
        p.show_rule(demand)
    );
    stratifies(&p);
}
