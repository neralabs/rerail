//! Source and physical DAG sharing are measured separately from execution.
use super::{
    finite_lower::{self, Instruction},
    lower::Lower,
    publication_tests::policy_utils,
};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::json;

#[test]
fn finite_ir_emits_one_body_for_reached_calls_with_distinct_control_edges() {
    let rules = [json!({"all":vec![json!({"matches":"u"});8]})];
    let policy = policy_utils(
        &rules,
        json!({"u":{"all":[{"pattern":"$X"},{"value":{"gt":0}}]}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.regions.len(), 1);
    assert_eq!(
        program
            .code
            .iter()
            .filter(|n| matches!(n.op, Instruction::Call(_)))
            .count(),
        1
    );
    let Instruction::Boolean {
        all: true,
        children,
    } = &program.code[program.entry].op
    else {
        panic!("eight ordered edges")
    };
    assert_eq!(children.len(), 8);
    assert!(children.iter().all(|pc| *pc == children[0]));
    for work in [0, 1, 2, 3, 4, 7, 8, 16, 64] {
        let result = super::publication_tests::check_work_compiler(
            &policy,
            &rules,
            &["1;"],
            None,
            work,
            super::compile_job_finite,
        );
        if work == 0 {
            assert_eq!(result["output"][0][4], -2);
        }
    }
}

#[test]
fn finite_ir_retains_original_resource_bounds_for_catalog_dags() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut evidence = vec![];
    let mut failures = vec![];
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
            let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
            match lower.plan(&grammar.matcher).and_then(|plan|finite_lower::lower(&plan)) {
                Ok(program)=>evidence.push(json!({"policy":name,"lowering":lower.profile(0),"control":program.code.len(),"functions":program.functions.len(),"regions":program.regions.len(),"ir":program.inventory()})),
                Err(error)=>{
                    evidence.push(json!({"policy":name,"lowering":lower.profile(0),"error":error}));
                    if name=="bandit-b615-huggingface-unsafe-download" {
                        let mut diagnostic=Lower::new_unqualified_inventory(grammar.language,grammar.three_valued);
                        let result=diagnostic.plan(&grammar.matcher);
                        evidence.push(json!({"policy":name,"UNQUALIFIED_NONADMITTING_SOURCE_INVENTORY":diagnostic.profile(0),"completed":result.is_ok(),"error":result.err()}));
                    }
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_FINITE_IR_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn finite_ir_shares_total_predicate_definitions_across_effectful_functions() {
    let rules = [json!({"any":[{"matches":"u"},{"matches":"v"}]})];
    let policy = policy_utils(
        &rules,
        json!({"u":{"kind":"number","value":{"eq":1}},"v":{"kind":"number","value":{"eq":2}}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    assert_eq!(program.functions.len(), 2);
    assert_eq!(
        program
            .regions
            .iter()
            .filter(|p| matches!(p.op, super::Op::Predicate(_)))
            .count(),
        1
    );
    assert_eq!(
        program
            .regions
            .iter()
            .filter(|p| matches!(p.op, super::Op::Value(_)))
            .count(),
        2
    );
}
