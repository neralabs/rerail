//! Nonadmitting complete emission counts. Never prepare or execute this SQL.
use super::super::{Slots, finite_lower, lower::Lower};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::json;
#[test]
#[ignore = "compile-only inventory; not admission or execution qualification"]
fn unqualified_finite_emission_inventory_only() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("packs/python/rules/bandit-b615-huggingface-unsafe-download.json");
    let p = CodeRuleDslRuntime::compile(
        std::fs::read_to_string(path).unwrap(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let grammar = &grammar_matchers(&p)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let mut reports = vec![];
    for recursive in [false, true] {
        let program = finite_lower::lower_with_recursion(&plan, recursive).unwrap();
        let mut sql = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: false,
            generation: Generation {
                unqualified_operator_inventory: true,
                peak_tracking: true,
                memo_clause: Some(0),
                memo_layout: lower.memo_layout(true),
                ..Default::default()
            },
        };
        let input = Demand {
            name: sql.relation(
                "unqualified inventory input",
                "SELECT 1 k,0 file,0 node,1000 work".into(),
                true,
                "compile only",
            ),
            slots: Slots::new(),
        };
        sql.generation.components.borrow_mut().roots = input.name.clone();
        let result = sql.finite_program(&program, &input);
        sql.publish_components();
        reports.push(json!({"UNQUALIFIED_NONADMITTING_EMISSION":true,"shareRecursion":recursive,"result":result.as_ref().map(|_|"generated").map_err(Clone::clone),"operators":sql.generation.operators.get(),"operatorCategories":*sql.generation.operator_kinds.borrow(),"sqlBytes":sql.definitions.iter().map(String::len).sum::<usize>(),"ir":program.inventory(),"lowering":lower.profile(0),"regions":sql.profile}));
    }
    if let Ok(path) = std::env::var("RERAIL_FINITE_EMISSION_INVENTORY") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}
