use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::json;
fn plan(op: Op) -> Plan {
    Plan {
        effects: super::super::effects::Effects::of(&op),
        op,
        mentions: Default::default(),
        writes: Default::default(),
        node_writes: Default::default(),
        walk_filter: None,
    }
}
fn node(op: Instruction) -> super::super::finite_lower::Node {
    super::super::finite_lower::Node {
        op,
        effects: Default::default(),
        mentions: Default::default(),
        writes: Default::default(),
        node_writes: Default::default(),
    }
}
fn program(regions: Vec<Plan>, code: Vec<Instruction>, entry: usize) -> Program {
    Program {
        source: None,
        share_search: false,
        code: code.into_iter().map(node).collect(),
        functions: vec![],
        regions,
        entry,
        mentions: Default::default(),
        writes: Default::default(),
        node_writes: Default::default(),
    }
}
fn search(body: usize) -> Instruction {
    Instruction::Search {
        axis: Axis::Child,
        end: false,
        field: None,
        body,
        stop: None,
        bounds: None,
        filter: None,
    }
}
fn structural() -> Program {
    program(
        vec![
            plan(Op::Predicate(Set::Kinds(vec![1]))),
            plan(Op::Predicate(Set::Kinds(vec![2]))),
        ],
        vec![
            Instruction::Eval(0),
            Instruction::Eval(1),
            Instruction::Call(0),
            Instruction::Boolean {
                all: true,
                children: vec![1, 2],
            },
            search(3),
            Instruction::Boolean {
                all: true,
                children: vec![0, 4],
            },
        ],
        5,
    )
}
#[test]
fn sequential_metadata_prefix_stops_at_first_utility_and_keeps_neighbor_guard() {
    let p = structural();
    let guard = entry(&p).unwrap();
    assert!(matches!(guard, Set::Intersect(_)));
    assert!(preimage(&guard));
    let mut derive = Derive {
        program: &p,
        code: vec![None; p.code.len()],
        plans: HashMap::new(),
        remaining: LIMIT,
        cyclic: false,
    };
    assert!(!derive.code(p.entry).unwrap().complete);
}
#[test]
fn utility_stops_counts_effect_prefixes_and_partial_negation_are_boundaries() {
    let mut p = structural();
    p.entry = 2;
    assert!(entry(&p).is_none());
    p.entry = 4;
    if let Instruction::Search { stop, .. } = &mut p.code[4].op {
        *stop = Some(0);
    }
    assert!(entry(&p).is_none());
    p.code[4] = node(search(3));
    if let Instruction::Search { bounds, .. } = &mut p.code[4].op {
        *bounds = Some((1, None, None));
    }
    assert!(entry(&p).is_none());
    p.code[4] = node(search(3));
    p.code.push(node(Instruction::Not(4)));
    p.entry = 6;
    assert!(entry(&p).is_none());
    for effect in [
        Op::Value(0),
        Op::CaptureBind(0),
        Op::Charge(0, Rc::new(plan(Op::Predicate(Set::AllNodes)))),
        Op::Predicate(Set::Regex(0)),
    ] {
        let opaque = plan(Op::All(vec![
            plan(effect),
            plan(Op::Predicate(Set::Preimage(
                Edge::Child,
                Box::new(Set::Kinds(vec![2])),
            ))),
        ]));
        let p = program(vec![opaque], vec![Instruction::Eval(0)], 0);
        assert!(entry(&p).is_none());
    }
}
#[test]
fn any_requires_every_arm_and_not_requires_full_metadata_purity() {
    let mut p = structural();
    p.code.push(node(Instruction::Boolean {
        all: false,
        children: vec![4, 2],
    }));
    p.entry = 6;
    assert!(entry(&p).is_none());
    p.code.push(node(Instruction::Not(1)));
    p.code.push(node(search(7)));
    p.entry = 8;
    assert!(entry(&p).is_some());
    let set = Set::Intersect(vec![
        Set::Preimage(Edge::Child, Box::new(Set::Kinds(vec![1]))),
        Set::Regex(0),
    ]);
    let p = program(
        vec![plan(Op::Predicate(set))],
        vec![Instruction::Eval(0)],
        0,
    );
    assert!(
        entry(&p).is_none(),
        "atomic set predicates cannot expose a prefix across a regex callback"
    );
}
#[test]
fn shared_boolean_dags_and_cycles_keep_derivation_bounded() {
    let mut code = vec![Instruction::Eval(0)];
    for index in 0..80 {
        code.push(Instruction::Boolean {
            all: false,
            children: vec![index, index],
        });
    }
    code.push(search(80));
    let p = program(vec![plan(Op::Predicate(Set::Kinds(vec![1])))], code, 81);
    assert!(
        entry(&p).is_none(),
        "oversized expanded guard must fall back instead of expanding a shared DAG exponentially"
    );
    let p = program(
        vec![],
        vec![Instruction::Boolean {
            all: true,
            children: vec![0],
        }],
        0,
    );
    assert!(entry(&p).is_none());
}
#[test]
fn catalog_entry_guard_readout_obeys_actual_compiled_order() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut evidence = vec![];
    for name in [
        "bandit-b615-huggingface-unsafe-download",
        "ruff-pie810-multiple-starts-ends-with",
        "ruff-sim101-duplicate-isinstance-call",
        "ruff-sim113-enumerate-for-loop",
        "ruff-sim905-split-static-string",
    ] {
        let text =
            std::fs::read_to_string(root.join(format!("packs/python/rules/{name}.json"))).unwrap();
        let policy = CodeRuleDslRuntime::compile(text, "json".into(), CAPABILITIES.into()).unwrap();
        for grammar in grammar_matchers(&policy) {
            let mut lower =
                super::super::lower::Lower::new_finite(grammar.language, grammar.three_valued);
            match lower.plan(&grammar.matcher).and_then(|plan|super::super::finite_lower::lower(&plan)) {
                Ok(program)=>evidence.push(json!({"policy":name,"guard":format!("{:?}",entry(&program)),"control":program.code.len()})),
                Err(error)=>evidence.push(json!({"policy":name,"error":error})),
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_FINITE_GUARD_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({"guardApplied":false,"programs":evidence})).unwrap(),
        )
        .unwrap();
    }
    eprintln!(
        "FINITE_ENTRY_GUARD_READOUT {}",
        json!({"guardApplied":false,"programs":evidence})
    );
}
#[test]
fn neighbor_guard_candidate_has_native_width_baseline() {
    let rule = json!({"all":[{"kind":"call"},{"has":{"stopBy":"neighbor","all":[{"kind":"attribute"},{"matches":"u"}]}}]});
    let policy = super::super::parse_tests::policy(
        "python",
        rule.clone(),
        json!({"u":{"any":[{"value":{"regex":"."}},{"regex":"."}]}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let mut lower = super::super::lower::Lower::new_finite(grammar.language, grammar.three_valued);
    let lowered = lower.plan(&grammar.matcher).unwrap();
    assert!(entry(&super::super::finite_lower::lower(&lowered).unwrap()).is_some());
    for width in [0, 16, 128] {
        let mut source = (0..width)
            .map(|index| format!("plain_{index}()\n"))
            .collect::<String>();
        source.push_str("'a b'.split()\nobject.attr()\n");
        for allowance in [0, 1, 2, 8, 32, 512, 10000] {
            for fault in [None, Some("object.attr")] {
                let result = super::super::publication_tests::check_work_compiler(
                    &policy,
                    std::slice::from_ref(&rule),
                    &[&source],
                    fault,
                    allowance,
                    super::super::compile_job_finite,
                );
                if allowance == 10000 && fault.is_none() {
                    eprintln!(
                        "FINITE_ENTRY_GUARD_WIDTH {}",
                        json!({"guardApplied":false,"width":width,"execution":result["metrics"]["execution"],"sqlBytes":result["sql"].as_str().unwrap().len()})
                    );
                }
            }
        }
    }
}
