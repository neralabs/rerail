//! End fields are not ordinary exclusive descendant or continuous-field walks.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};

fn policy(rule: Value, utils: Value) -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"field-end","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
fn parity(rule: Value, source: &str) -> Value {
    let p = policy(rule, json!({}));
    let result =
        super::super::tests::measure(&grammar_matchers(&p)[0], &[("a.py".into(), source.into())]);
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
    result
}
#[test]
fn field_end_includes_selected_root_and_excludes_later_duplicate_fields() {
    for (rule, source, expected) in [
        (
            json!({"kind":"assignment","has":{"field":"right","stopBy":"end","kind":"integer","regex":"^1$"}}),
            "x = 1\n",
            1,
        ),
        (
            json!({"kind":"assignment","has":{"field":"right","stopBy":"end","kind":"integer","regex":"^1$"}}),
            "x = ((1))\n",
            1,
        ),
        (
            json!({"kind":"assignment","has":{"field":"right","stopBy":"end","kind":"integer","regex":"^2$"}}),
            "x = 1\ny = 2\n",
            1,
        ),
        (
            json!({"kind":"import_statement","has":{"field":"name","stopBy":"end","regex":"^b$"}}),
            "import a, b\n",
            0,
        ),
        (
            json!({"kind":"import_statement","has":{"field":"name","stopBy":"end","regex":"^a$"}}),
            "import a, b\n",
            1,
        ),
        // Python's operator field selects the anonymous '+' token.
        (
            json!({"kind":"binary_operator","has":{"field":"operator","stopBy":"end","regex":"^\\+$"}}),
            "x = 1 + 2\n",
            1,
        ),
    ] {
        assert_eq!(parity(rule, source)["matches"], expected);
    }
}
#[test]
fn field_end_ancestor_walk_crosses_ineligible_edges() {
    let source = "for (a, b) in xs:\n    pass\nfor c in ys:\n    pass\n";
    // a/b -> tuple/parentheses has no left field. The later ascent into
    // for_statement does; that earlier failed eligibility must not end the walk.
    assert_eq!(
        parity(
            json!({"kind":"identifier","regex":"^[abc]$","inside":{"field":"left","stopBy":"end","kind":"for_statement"}}),
            source
        )["matches"],
        3
    );
    assert_eq!(
        parity(
            json!({"kind":"identifier","regex":"^[abc]$","inside":{"field":"left","stopBy":"neighbor","kind":"for_statement"}}),
            source
        )["matches"],
        1
    );
}
#[test]
fn field_end_commits_first_capture_and_rolls_back_failed_candidates_and_branches() {
    for rule in [
        json!({"kind":"assignment","pattern":"$ROOT","has":{"field":"right","stopBy":"end","kind":"integer","pattern":"$X"}}),
        json!({"kind":"assignment","all":[{"pattern":"$ROOT"},{"any":[{"all":[{"has":{"field":"right","stopBy":"end","pattern":"$LEAK"}},{"regex":"^never$"}]},{"has":{"field":"right","stopBy":"end","kind":"integer","pattern":"$X"}}]}]}),
        json!({"kind":"assignment","pattern":"$ROOT","has":{"field":"right","stopBy":"end","all":[{"pattern":"$X"},{"regex":"^2$"}]}}),
        json!({"kind":"identifier","regex":"^a$","pattern":"$NAME","inside":{"field":"left","stopBy":"end","kind":"for_statement","pattern":"$PARENT"}}),
    ] {
        let result = parity(
            rule,
            "x = (1, 2, 3)\ny = (2, 4)\nfor (a, b) in xs:\n    pass\n",
        );
        assert!(result["matches"].as_u64().unwrap() > 0);
    }
}
#[test]
fn field_end_preserves_unknown_strictness_reached_work_and_missing_field_laziness() {
    for rule in [
        json!({"kind":"assignment","has":{"field":"right","stopBy":"end","kind":"integer","testUnknown":"^1$","value":{"gt":0}}}),
        json!({"kind":"assignment","has":{"field":"right","stopBy":"end","kind":"integer","pattern":"$X","testUnknown":"^1$","value":{"gt":0}}}),
        json!({"kind":"identifier","has":{"field":"body","stopBy":"end","value":{"gt":0}}}),
        json!({"kind":"identifier","regex":"^a$","inside":{"field":"left","stopBy":"end","kind":"for_statement","testUnknown":"^for","has":{"field":"right","stopBy":"end","kind":"integer","value":{"gt":0}}}}),
    ] {
        let p = policy(rule.clone(), json!({}));
        for work in [0, 1, 2, 8, 64] {
            for fault in [None, Some("2")] {
                check_work(
                    &p,
                    std::slice::from_ref(&rule),
                    &[
                        "x = (1, 2)\ny = 3\nfor (a, b) in (1, 2):\n    pass\n",
                        "z = 4\n",
                    ],
                    fault,
                    work,
                );
            }
        }
    }
    // A utility is charged only at eligible field ancestors and at candidates
    // admitted by the native two-valued kind filter (disabled in three-valued mode).
    for rule in [
        json!({"kind":"assignment","has":{"field":"right","stopBy":"end","matches":"u"}}),
        json!({"kind":"identifier","regex":"^a$","inside":{"field":"left","stopBy":"end","matches":"parent"}}),
    ] {
        let p = policy(
            rule.clone(),
            json!({"u":{"kind":"integer","regex":"^2$"},"parent":{"kind":"for_statement"}}),
        );
        for work in 0..=12 {
            check_work(
                &p,
                std::slice::from_ref(&rule),
                &["x = (1, 2, 3)\nfor (a, b) in xs:\n    pass\n"],
                None,
                work,
            );
        }
    }
}
#[test]
fn field_end_work_is_bounded_by_selected_subtree_or_ancestor_depth() {
    for rule in [
        json!({"kind":"assignment","regex":"^chosen","has":{"field":"right","stopBy":"end","kind":"integer","value":{"gt":0}}}),
        json!({"kind":"identifier","regex":"^a$","inside":{"field":"left","stopBy":"end","kind":"for_statement"}}),
    ] {
        let mut samples = Vec::new();
        for unrelated in [32, 128, 512] {
            let source = format!(
                "chosen = (1, 2)\nfor (a, b) in xs:\n    pass\n{}",
                "1\n".repeat(unrelated)
            );
            let result = parity(rule.clone(), &source);
            samples.push((
                result["vmSteps"].as_u64().unwrap(),
                result["nodeAdvances"].as_u64().unwrap(),
                result["valueRequests"].as_u64().unwrap(),
            ));
        }
        assert!(
            samples.windows(2).all(|pair| pair[0] == pair[1]),
            "unrelated sibling expressions must not add matching work: {samples:?}"
        );
    }
}

#[test]
fn field_end_first_selected_subtree_ignores_later_direct_siblings() {
    let rule = json!({"kind":"import_statement","has":{
        "field":"name","stopBy":"end","regex":"^a$"}});
    let mut samples = Vec::new();
    for siblings in [32, 128, 512] {
        let aliases = (0..siblings)
            .map(|i| format!("later{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("import a, {aliases}\n");
        let result = parity(rule.clone(), &source);
        assert_eq!(result["matches"], 1);
        samples.push((
            result["vmSteps"].as_u64().unwrap(),
            result["nodeAdvances"].as_u64().unwrap(),
        ));
    }
    assert!(
        samples.windows(2).all(|pair| pair[0] == pair[1]),
        "later direct siblings must not add matching VM or node advances: {samples:?}"
    );
}
