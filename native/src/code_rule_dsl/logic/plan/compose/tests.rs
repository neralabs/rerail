use super::*;

fn compile_text(text: &str) -> (Program, Compiled) {
    let p = super::super::super::rr::parse(text).unwrap();
    compile_program(p)
}

fn compile_program(p: Program) -> (Program, Compiled) {
    let (p, _) = sql_kernel::normalize(&p).unwrap();
    assert!(accepts(&p).unwrap());
    let db = Connection::open_in_memory().unwrap();
    let inputs = Inputs {
        facts: "main.".into(),
        stats: "main.".into(),
        atoms: None,
        native: BTreeSet::new(),
        batch: HashMap::new(),
        given: Some(Stats::empty(&p)),
        kept: BTreeSet::new(),
    };
    let mut model = make_plan(&p, &db, inputs).unwrap();
    let q = compile(&p, &mut model).unwrap();
    (p, q)
}

#[test]
fn constants_fanout_and_fanin_execute_one_query() {
    for depth in [1, 32] {
        let mut text = ".decl seed(x:number)\nseed(1). seed(1). seed(2).\n".to_owned();
        let mut prev = "seed".to_owned();
        for n in 0..depth {
            let rel = format!("hop{n}");
            text += &format!(".decl {rel}(x:number)\n{rel}(x) :- {prev}(x).\n");
            prev = rel;
        }
        text += &format!(
            ".decl a(x:number)\na(x) :- {prev}(x).\n.decl b(x:number)\nb(x) :- {prev}(x), x=2.\n.decl result(x:number)\nresult(x) :- a(x).\nresult(x) :- b(x).\n.output result,b\n"
        );
        let (p, q) = compile_text(&text);
        let db = Connection::open_in_memory().unwrap();
        let statements = super::super::corpus::Statements::on(&db);
        let out = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
        assert_eq!(out.outputs["result"], ["1", "2"]);
        assert_eq!(out.outputs["b"], ["2"]);
        assert_eq!(
            statements.count(),
            1,
            "composition must not execute per relation"
        );
        assert!(!q.sql.contains("INSERT"));
        assert!(!q.sql.contains("new_"));
    }
}

#[test]
fn demanded_aggregates_and_negation_preserve_empty_groups_and_set_semantics() {
    let (p, q) = compile_text(
        ".decl owner(g:number)\nowner(1). owner(2). owner(3).\n.decl item(g:number,x:number)\nitem(1,7). item(1,7). item(1,8). item(2,9).\n.decl totals(g:number,n:number)\ntotals(g,n) :- owner(g), n=count : {item(g,_)}.\n.decl absent(g:number)\nabsent(g) :- owner(g), !item(g,_).\n.output totals,absent\n",
    );
    let db = Connection::open_in_memory().unwrap();
    let out = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
    assert_eq!(out.outputs["totals"], ["1\t2", "2\t1", "3\t0"]);
    assert_eq!(out.outputs["absent"], ["3"]);
}

#[test]
fn relation_names_are_case_sensitive_through_joins_aggregates_and_negation() {
    let (p,q)=compile_text(".decl leaf(x:number)\nleaf(1). leaf(2).\n.decl LEAF(x:number)\nLEAF(x) :- leaf(x), x=2.\n.decl missing(x:number)\nmissing(x) :- leaf(x), !LEAF(x).\n.decl counts(n:number)\ncounts(n) :- n=count : { LEAF(_) }.\n.output leaf,LEAF,missing,counts\n");
    let db=Connection::open_in_memory().unwrap();
    let stmts=super::super::corpus::Statements::on(&db);
    let out=q.execute(&db,&p,None,&mut Hooks::default()).unwrap();
    assert_eq!(out.outputs["leaf"],["1","2"]);
    assert_eq!(out.outputs["LEAF"],["2"]);
    assert_eq!(out.outputs["missing"],["1"]);
    assert_eq!(out.outputs["counts"],["1"]);
    assert_eq!(stmts.count(),1,"physical naming must not split the query");
}

#[test]
fn nonlinear_recursion_requires_an_explicit_physical_plan() {
    let p =
        super::super::super::rr::parse(".decl r(x:number)\nr(1). r(y) :- r(x), r(y).\n.output r\n")
            .unwrap();
    assert!(!accepts(&p).unwrap());
}

#[test]
fn closure_uses_a_recursive_cte_and_terminates_on_cycles() {
    let (p, q) = compile_text(
        ".decl edge(a:number,b:number)\nedge(1,2). edge(2,3). edge(3,1).\n.decl reach(n:number)\nreach(1). reach(y) :- reach(x), edge(x,y).\n.output reach\n",
    );
    let db = Connection::open_in_memory().unwrap();
    let statements = super::super::corpus::Statements::on(&db);
    let out = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
    assert_eq!(out.outputs["reach"], ["1", "2", "3"]);
    assert_eq!(statements.count(), 1);
}

#[test]
fn composed_queries_read_adopted_inputs_and_replace_their_versions() {
    use crate::code_rule_dsl::logic::native;
    let path = crate::code_rule_dsl::prefilter::tests::temp();
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    native::register(&db).unwrap();
    native::register_symbols(&db).unwrap();
    let p=super::super::super::rr::parse(".decl supplied(x:number)\n.input supplied\n.decl result(x:number)\nresult(x) :- supplied(x).\n.output result\n").unwrap();
    let lowered = lower::lower(
        &p,
        crate::code_rule_dsl::language::dialect_by_label("python").unwrap(),
        &mut |s| {
            store
                .intern_atoms(s)
                .map_err(|e| Error::Plan(e.to_string()))
        },
    )
    .unwrap();
    let _plans = KeepPlans::start(None);
    for n in [7, 9] {
        native::component::retain(&db, "supplied", vec![vec![n]], vec![b'i']).unwrap();
        native::component::adopt(
            &db,
            "supplied",
            native::component::held(&db, "supplied").unwrap(),
        );
        let mut stats = Stats::empty(&lowered.program);
        stats.measured("supplied", [("c0".into(), [1, 1, 1, 1])], []);
        let out = run_atoms_native_over(&lowered, &db, "main", stats, BTreeSet::new(), "program")
            .unwrap();
        assert_eq!(out.outputs["result"], [n.to_string()]);
        if enabled() {
            assert!(out.kept.contains("result"), "decoded output must also remain a resident fact");
            assert_eq!(native::component::derived(&db,"result").unwrap().value(0,0),n);
        }
        assert!(native::component::adopted(&db).is_empty());
    }
}

#[test]
fn pruning_dead_native_producers_removes_their_dependency_edges() {
    let p = super::super::super::rr::parse(
        r#"
#include "core.rr"
.decl wanted(n:node)
wanted(n) :- node(_,n,_,_,_).
.decl targets(q:symbol)
targets("requests.get").
.decl unused(n:node,q:symbol)
unused(n,q) :- python_qualified(n,q,wanted,targets).
.decl result(n:number)
result(1).
.output result
"#,
    )
    .unwrap();
    assert!(!p.natives.is_empty());
    let pruned = super::super::super::front::prune(p);
    assert!(
        pruned.natives.is_empty(),
        "dead native edges must not create cross-file exports"
    );
    assert!(
        !super::super::super::front::needed(&pruned, pruned.outputs.clone()).contains("wanted")
    );
}

#[test]
fn partial_scalars_cannot_publish_null_or_survive_projection() {
    let (p, q) = compile_text(
        r#"
.decl input(s:symbol)
input("12"). input("bad").
.decl converted(n:number)
converted(to_number(s)) :- input(s).
.decl discarded(s:symbol)
discarded(s) :- input(s), n=to_number(s).
.decl groups(n:number)
groups(n) :- n=count : { converted(_) }.
.output converted,discarded,groups
"#,
    );
    let db = Connection::open_in_memory().unwrap();
    super::super::super::strings::register(&db).unwrap();
    let out = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
    assert_eq!(out.outputs["converted"], ["12"]);
    assert_eq!(out.outputs["discarded"], ["12"]);
    assert_eq!(out.outputs["groups"], ["1"]);
}

#[test]
fn empty_extremum_has_no_binding_even_when_its_value_is_discarded() {
    let (p, q) = compile_text(
        r#"
.decl owner(g:number)
owner(1). owner(2).
.decl item(g:number,x:number)
item(1,7).
.decl minima(g:number,n:number)
minima(g,n) :- owner(g), n=min x : {item(g,x)}.
.decl admitted(g:number)
admitted(g) :- owner(g), n=max x : {item(g,x)}.
.output minima,admitted
"#,
    );
    let db = Connection::open_in_memory().unwrap();
    let out = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
    assert_eq!(out.outputs["minima"], ["1\t7"]);
    assert_eq!(out.outputs["admitted"], ["1"]);
}

#[test]
fn a_constant_split_prefix_does_not_enumerate_positions() {
    for prefix in ["os".to_owned(), "πΩ".to_owned(), "a".repeat(5000)] {
        let source = format!(
            r#"
.decl names(q:symbol)
names("{prefix}.waitpid"). names("{prefix}.path.waitpid"). names("other.waitpid").
.decl supplied(q:symbol,x:symbol)
supplied("{prefix}.waitpid", "waitpid"). supplied("{prefix}.waitpid", "wrong").
.decl bound(x:symbol)
bound(x) :- supplied(q,x), split_dot(q, "{prefix}", x).
.decl result(x:symbol)
result(x) :- names(q), split_dot(q, "{prefix}", x).
.output result,bound
"#
        );
        // Production simplification turns constant cat operands into variables with
        // later equalities. Exercise that normalized form, including text-mode SQL.
        let mut p = super::super::super::rr::parse(&source).unwrap();
        super::super::super::simplify::simplify(&mut p);
        let (p, q) = compile_program(p);
        assert!(
            !q.sql.contains("rdl_int"),
            "a known prefix requires one string test, not a position scan"
        );
        let db = Connection::open_in_memory().unwrap();
        super::super::super::strings::register(&db).unwrap();
        let rows = q.execute(&db, &p, None, &mut Hooks::default()).unwrap();
        assert_eq!(rows.outputs["result"], ["path.waitpid", "waitpid"]);
        assert_eq!(rows.outputs["bound"], ["waitpid"]);
    }
}

#[test]
fn exponentially_shared_ctes_use_bounded_bulk_stages() {
    let mut text=".decl seed(x:number)\n.input seed\n".to_owned();
    let mut prev="seed".to_owned();
    for n in 0..24 {
        let rel=format!("level{n}");
        text+=&format!(".decl {rel}(x:number)\n{rel}(x) :- {prev}(x), {prev}(y), y >= x.\n");
        prev=rel;
    }
    text+=&format!(".output {prev}\n");
    let p=super::super::super::rr::parse(&text).unwrap();
    let db=Connection::open_in_memory().unwrap();
    let mut inputs=Inputs {facts:"main.".into(),stats:"main.".into(),atoms:Some([0;3]),native:BTreeSet::new(),batch:HashMap::new(),given:Some(Stats::empty(&p)),kept:BTreeSet::new()};
    use crate::code_rule_dsl::logic::native::component;
    component::retain(&db,"seed",vec![vec![1],vec![2]],vec![b'i']).unwrap();
    stages::bind(&p,&db,&mut inputs,"seed",component::derived(&db,"seed").unwrap()).unwrap();
    let mut model=make_plan(&p,&db,inputs.clone()).unwrap();
    let unbounded=compile(&p,&mut model).unwrap();
    assert!(unbounded.boundary().is_some(),"the expansion must require a boundary");
    assert!(!unbounded.prepares(&db).unwrap(),"SQLite must reject the unbounded graph before staging repairs it");
    let statements=super::super::corpus::Statements::on(&db);
    let out=stages::run(&p,&db,inputs,None,&mut Hooks::default()).unwrap();
    assert_eq!(out.outputs[&prev],["1","2"]);
    assert!((2..=6).contains(&statements.count()),"24 dependent joins need a few bulk stages, not 24 executions: {}",statements.count());
}

#[test]
fn quiet_composed_maintenance_keeps_the_same_resident_fact() {
    use crate::code_rule_dsl::logic::native::component;
    let p=super::super::super::rr::parse(".decl changed(x:number)\n.input changed\n.decl h__old(x:number)\n.input h__old\n.decl h__del(x:number)\nh__del(x) :- changed(x).\n.decl h__ins(x:number)\nh__ins(x) :- changed(x).\n.decl h(x:number)\nh(x) :- h__old(x), !h__del(x).\nh(x) :- h__ins(x).\n.output h\n").unwrap();
    let db=Connection::open_in_memory().unwrap();
    let mut inputs=Inputs {facts:"main.".into(),stats:"main.".into(),atoms:Some([0;3]),native:BTreeSet::new(),batch:HashMap::new(),given:Some(Stats::empty(&p)),kept:BTreeSet::from(["h".into()])};
    for (r,rows) in [("changed",vec![]),("h__old",vec![vec![7],vec![9]])] {
        component::retain(&db,r,rows,vec![b'i']).unwrap();
        stages::bind(&p,&db,&mut inputs,r,component::derived(&db,r).unwrap()).unwrap();
    }
    let old=component::held(&db,"h__old").unwrap();
    let before=quiet_strata();
    let out=stages::run(&p,&db,inputs,None,&mut Hooks::default()).unwrap();
    assert!(out.kept.contains("h"));
    assert!(quiet_strata()>before);
    assert!(Arc::ptr_eq(&old,&component::held(&db,"h").unwrap()),"quiet output must retain its indexes, not copy its rows at publication");
}

#[test]
fn restored_case_distinct_inputs_keep_their_own_bindings() {
    use crate::code_rule_dsl::logic::native::component;
    let p=super::super::super::rr::parse(".decl leaf(x:number)\n.input leaf\n.decl LEAF(x:number)\n.input LEAF\n.decl missing(x:number)\nmissing(x) :- leaf(x), !LEAF(x).\n.output missing\n").unwrap();
    let db=Connection::open_in_memory().unwrap();
    for value in [2,1] {
        let mut inputs=Inputs {facts:"main.".into(),stats:"main.".into(),atoms:Some([0;3]),native:BTreeSet::new(),batch:HashMap::new(),given:Some(Stats::empty(&p)),kept:BTreeSet::new()};
        for (r,rows) in [("leaf",vec![vec![1],vec![2]]),("LEAF",vec![vec![value]])] {
            component::retain(&db,r,rows,vec![b'i']).unwrap();
            stages::bind(&p,&db,&mut inputs,r,component::derived(&db,r).unwrap()).unwrap();
        }
        let out=stages::run(&p,&db,inputs,None,&mut Hooks::default()).unwrap();
        assert_eq!(out.outputs["missing"],[(3-value).to_string()]);
    }
}
