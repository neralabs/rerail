use super::tests::{measure, policy};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::{Value, json};

fn diamond() -> Value {
    json!({
        "root":{"any":[{"kind":"assignment"},
            {"inside":{"stopBy":"neighbor","matches":"left"}},
            {"inside":{"stopBy":"neighbor","matches":"right"}}]},
        "left":{"inside":{"stopBy":"neighbor","matches":"root"}},
        "right":{"inside":{"stopBy":"neighbor","matches":"root"}}
    })
}

fn checked(rule: Value, utils: Value, sources: &[(String, String)]) -> Value {
    let p = policy(rule, utils);
    let result = measure(&grammar_matchers(&p)[0], sources);
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
    assert!(
        !result["relations"]["components"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    result
}

#[test]
fn relational_components_preserve_states_files_diamonds_and_consumers() {
    let sources = vec![
        ("a.py".into(), "x = (((a + b)))\nprint(x)\n".into()),
        ("b.py".into(), "print(x)\ny = ((c))\n".into()),
        ("empty.py".into(), String::new()),
    ];
    for rule in [
        json!({"matches":"root"}),
        json!({"matches":"left"}),
        json!({"any":[{"matches":"root"},{"matches":"right"}]}),
        json!({"all":[{"kind":"identifier"},{"not":{"matches":"root"}}]}),
        json!({"all":[{"matches":"left"},{"inside":{"stopBy":"end","kind":"parenthesized_expression"}}]}),
    ] {
        let result = checked(rule, diamond(), &sources);
        assert!(result["matches"].as_u64().unwrap() > 0);
        assert_eq!(
            result["relations"]["components"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            result["relations"]["components"][0]["members"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
}

#[test]
fn relational_components_allow_both_preorder_directions_and_multiple_hops() {
    let sources = vec![("directions.py".into(), "x = ((seed))\na = b + c\n".into())];
    for edge in ["inside", "has", "follows", "precedes"] {
        let utils = json!({
            "a":{"any":[{"regex":"^seed$"},{edge:{"stopBy":"neighbor","matches":"b"}}]},
            "b":{edge:{"stopBy":"neighbor",edge:{"stopBy":"neighbor","matches":"a"}}}
        });
        checked(json!({"matches":"a"}), utils, &sources);
    }
}

#[test]
fn relational_components_keep_external_dependencies_stratified() {
    let mut utils = diamond();
    utils["root"]["any"][0] = json!({"matches":"seed"});
    utils["seed"] = json!({"any":[{"kind":"identifier"},{"inside":{"stopBy":"neighbor","matches":"seed_step"}}]});
    utils["seed_step"] = json!({"inside":{"stopBy":"neighbor","matches":"seed"}});
    utils["root"]["any"]
        .as_array_mut()
        .unwrap()
        .push(json!({"all":[{"kind":"assignment"},{"not":{"matches":"seed"}}]}));
    let result = checked(
        json!({"matches":"root"}),
        utils,
        &[("strata.py".into(), "x = ((value))\nprint(x)\n".into())],
    );
    assert_eq!(
        result["relations"]["components"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn relational_components_without_seeds_are_empty() {
    let utils = json!({
        "a":{"inside":{"stopBy":"neighbor","matches":"b"}},
        "b":{"inside":{"stopBy":"neighbor","matches":"a"}}
    });
    let result = checked(
        json!({"matches":"a"}),
        utils,
        &[("empty.py".into(), "x = ((value))\n".into())],
    );
    assert_eq!(result["matches"], 0);
}

#[test]
fn relational_single_state_closures_use_the_same_termination_proof() {
    let sources = vec![("single.py".into(), "x = ((seed))\na = b + c\n".into())];
    for edge in ["inside", "has", "follows", "precedes"] {
        for seeded in [false, true] {
            let step = json!({edge:{"stopBy":"neighbor","matches":"a"}});
            let rule = if seeded {
                json!({"any":[{"regex":"^seed$"},step]})
            } else {
                step
            };
            let p = policy(json!({"matches":"a"}), json!({"a":rule}));
            let result = measure(&grammar_matchers(&p)[0], &sources);
            assert_eq!(result["matches"].as_u64().unwrap() > 0, seeded);
        }
    }
}

#[test]
fn relational_component_expansion_has_a_compile_time_bound() {
    use super::ir::{Edge, Program, Relation, Set};
    let mut program = Program::default();
    for id in 0..129 {
        program.relations.push(Relation {
            source: format!("state {id}"),
            domain: None,
            keys: None,
            expression: Set::Preimage(Edge::Parent, Box::new(Set::Reference((id + 1) % 129))),
        });
    }
    assert!(
        super::physical::plan(&program)
            .err()
            .unwrap()
            .contains("128 arms")
    );
}

#[test]
fn relational_components_compose_with_ordered_capture_answers() {
    let p = policy(
        json!({"all":[{"kind":"identifier","pattern":"$NAME"},
        {"inside":{"stopBy":"end","matches":"scope"}}]}),
        json!({
            "scope":{"kind":"function_definition","inside":{"stopBy":{"kind":"function_definition"},"matches":"producer"}},
            "producer":{"kind":"for_statement","has":{"stopBy":{"kind":"function_definition"},
                "kind":"identifier","pattern":"$NAME","matches":"slot_a"}},
            "slot_a":{"any":[
                {"inside":{"stopBy":"neighbor","kind":"assignment"},"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}},
                {"inside":{"stopBy":"neighbor","kind":"tuple_pattern","matches":"slot_b"}}]},
            "slot_b":{"any":[
                {"inside":{"stopBy":"neighbor","kind":"assignment"},"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}},
                {"inside":{"stopBy":"neighbor","kind":"tuple_pattern","matches":"slot_a"}}]}
        }),
    );
    let result = measure(
        &grammar_matchers(&p)[0],
        &[
            (
                "capture.py".into(),
                "for i in xs:\n (a, (b, c)) = values\n a = again\n def f():\n  return a, b, c\n"
                    .into(),
            ),
            (
                "other.py".into(),
                "for j in ys:\n (a, b) = values\n def g():\n  return a, b\n".into(),
            ),
        ],
    );
    assert_eq!(result["matches"], 5);
    assert!(
        result["sql"]
            .as_str()
            .unwrap()
            .contains("(state,file,node)")
    );
}

#[test]
fn relational_components_reject_unproved_recursion_before_sql() {
    let cases = [
        (
            json!({"inside":{"stopBy":"neighbor","not":{"matches":"b"}}}),
            json!({"inside":{"stopBy":"neighbor","matches":"a"}}),
            "recursive negation",
        ),
        (
            json!({"all":[{"inside":{"stopBy":"neighbor","matches":"b"}},{"inside":{"stopBy":"neighbor","matches":"b"}}]}),
            json!({"inside":{"stopBy":"neighbor","matches":"a"}}),
            "nonlinear",
        ),
        (
            json!({"inside":{"stopBy":"neighbor","matches":"b"}}),
            json!({"has":{"stopBy":"neighbor","matches":"a"}}),
            "preorder direction",
        ),
        (
            json!({"inside":{"stopBy":"end","matches":"b"}}),
            json!({"inside":{"stopBy":"neighbor","matches":"a"}}),
            "region dependency",
        ),
    ];
    for (a, b, expected) in cases {
        let p = policy(json!({"matches":"a"}), json!({"a":a,"b":b}));
        let Err(error) = super::compile(&grammar_matchers(&p)[0]) else {
            panic!("accepted {expected}")
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn relational_components_do_not_inline_or_specialize_one_member() {
    let p = policy(json!({"matches":"root"}), diamond());
    let grammar = &grammar_matchers(&p)[0];
    let mut program = super::lower::compile(grammar).unwrap();
    let original = program.relations[0].expression.clone();
    assert_eq!(super::demand::inline(&original, &program), original);
    let roots = program.relations.len();
    program.relations.push(super::ir::Relation {
        source: "test region".into(),
        expression: super::ir::Set::AllNodes,
        domain: None,
        keys: None,
    });
    assert_eq!(
        super::demand::bound(&original, roots, &mut program, &mut Default::default()),
        original
    );
}

#[test]
fn relational_components_scaling() {
    let mut samples = Vec::new();
    let mut utils = diamond();
    // Adjacent seed levels keep native budget charges polynomial. The SQL closure still has
    // exponentially many duplicate paths if its UNION is replaced with UNION ALL.
    utils["root"]["any"][0] =
        json!({"any":[{"kind":"assignment"},{"kind":"expression_statement"}]});
    for size in [16, 64, 128] {
        let source = format!("x = {}value{}\n", "(".repeat(size), ")".repeat(size));
        let mut result = checked(
            json!({"matches":"root"}),
            utils.clone(),
            &[("depth.py".into(), source)],
        );
        result["dimension"] = json!("diamond-depth");
        result["size"] = json!(size);
        samples.push(result);
    }
    for size in [16, 64, 256] {
        let source = "x = (((value)))\n".repeat(size);
        let mut result = checked(
            json!({"matches":"root"}),
            utils.clone(),
            &[("width.py".into(), source)],
        );
        result["dimension"] = json!("independent-regions");
        result["size"] = json!(size);
        samples.push(result);
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            let growth = pair[1]["size"].as_u64().unwrap() / pair[0]["size"].as_u64().unwrap();
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap()
                    <= pair[0]["vmSteps"].as_u64().unwrap() * (growth + 1),
                "superlinear component work"
            );
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_COMPONENT_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&json!({
            "scope":"Total, capture-free structural components; not the complete job or native resource-limit semantics",
            "samples":samples
        })).unwrap()).unwrap();
    }
}
