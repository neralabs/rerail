//! Actual compiler/runner coverage across phase boundaries, with full native output parity.

use super::{same, snapshot};
use crate::code_rule_dsl::sql::exec::{LOOP_PROFILE, STATEMENT_PROFILE};
use crate::code_rule_dsl::test_support::compiled;
use serde_json::{Value, json};

fn utility() -> Value {
    json!({ "pick": { "params": ["P"], "rule": { "any": [
        { "kind": "identifier", "regex": "^winner$", "pattern": "$P" },
        { "kind": "member_expression", "has": { "field": "object", "matches": { "pick": { "P": "$P" } } } },
    ] } } })
}

fn arguments(winner: Option<usize>) -> String {
    let names: Vec<_> = (0..513)
        .map(|i| {
            if winner == Some(i) {
                "winner".to_owned()
            } else {
                format!("miss{i}")
            }
        })
        .collect();
    format!("f({});", names.join(", "))
}

#[test]
fn recursive_searches_keep_first_captures_without_sql_phase_drivers() {
    let rule = compiled(json!({
        "rule": { "kind": "arguments", "has": { "stopBy": "end", "kind": "identifier", "matches": { "pick": { "P": "$Y" } } } },
        "utils": utility(),
    }));
    for winner in [Some(0), Some(31), Some(32), Some(64), Some(512), None] {
        STATEMENT_PROFILE.set(Some(Default::default()));
        LOOP_PROFILE.set(Some(Default::default()));
        let source = arguments(winner);
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        let statements = STATEMENT_PROFILE.take().unwrap();
        let loops = LOOP_PROFILE.take().unwrap();
        let found = result["violations"].as_array().unwrap();
        assert_eq!(
            found.len(),
            usize::from(winner.is_some()),
            "{winner:?}: {result}"
        );
        if winner.is_some() {
            let span = &found[0]["captures"]["Y"][0];
            assert_eq!(
                &source[span["startByte"].as_u64().unwrap() as usize
                    ..span["endByte"].as_u64().unwrap() as usize],
                "winner"
            );
        }
        let phase_passes: usize = loops
            .loops
            .iter()
            .filter(|(sql, _)| sql.starts_with(crate::code_rule_dsl::sql::emit::FRONTIER))
            .map(|(_, (_, passes, _))| passes)
            .sum();
        assert_eq!(
            phase_passes, 0,
            "SQL must not schedule native matcher phases"
        );
        assert_eq!(
            statements
                .values()
                .map(|(_, _, count)| count)
                .sum::<usize>(),
            1
        );
    }
}

#[test]
fn nested_sibling_frontiers_keep_answers_across_multiple_files() {
    let rule = compiled(
        json!({ "rule": { "precedes": { "stopBy": "end", "precedes": {
        "stopBy": "end", "inside": { "stopBy": "end", "nthChild": 3, "layout": { "start": { "max": 82 } } }
    } } } }),
    );
    let paths = [
        "ecma/scope/typescript/bindings.ts",
        "ecma/scope/typescript/constants.ts",
        "ecma/value/eslint/no-dupe-keys.ts",
    ];
    let sources: Vec<_> = paths
        .iter()
        .map(|p| {
            std::fs::read_to_string(format!("{}/../fixtures/{p}", env!("CARGO_MANIFEST_DIR")))
                .unwrap()
        })
        .collect();
    let files: Vec<_> = paths
        .iter()
        .zip(&sources)
        .map(|(p, s)| ("typescript", *p, s.as_str()))
        .collect();
    let result = same(&rule, &snapshot(&files));
    assert_eq!(result["violations"].as_array().unwrap().len(), 12);
}

#[test]
fn changed_recursive_answers_reopen_an_earlier_prefix_and_replace_its_capture() {
    let rule = compiled(json!({
        "rule": { "kind":"call_expression", "regex":"^f\\(", "matches": { "reach": { "P":"$Y" } } },
        "utils": { "reach": { "params":["P"], "rule": { "any":[
            { "kind":"identifier", "regex":"^winner$", "pattern":"$P" },
            { "kind":"call_expression", "has": { "field":"arguments", "has": { "matches": { "reach": { "P":"$P" } } } } },
        ] } } },
    }));
    let mut names: Vec<_> = (0..130).map(|i| format!("miss{i}")).collect();
    names[40] = "g(h(winner))".to_owned();
    names[129] = "winner".to_owned();
    let source = format!("f({});", names.join(", "));
    let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
    let found = result["violations"].as_array().unwrap();
    assert_eq!(found.len(), 1);
    let span = &found[0]["captures"]["Y"][0];
    assert_eq!(
        span["startByte"].as_u64().unwrap() as usize,
        source.find("winner").unwrap()
    );
}

#[test]
fn a_closed_parent_keeps_its_prefix_while_another_parent_exhausts_its_domain() {
    let rule = compiled(json!({
        "rule": { "kind":"arguments", "has": { "stopBy":"end", "kind":"identifier", "matches": { "pick": { "P":"$Y" } } } },
        "utils": utility(),
    }));
    let source = format!("{}\n{}", arguments(Some(0)), arguments(None));
    STATEMENT_PROFILE.set(Some(Default::default()));
    let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
    assert_eq!(result["violations"].as_array().unwrap().len(), 1);
    let demands: usize = STATEMENT_PROFILE
        .take()
        .unwrap()
        .iter()
        .filter(|(sql, _)| sql.starts_with("INSERT INTO c") && sql.contains("(file, node"))
        .map(|(_, (_, rows, _))| rows)
        .sum();
    assert!(
        demands <= 580,
        "closed parent expanded again: {demands} keys"
    );
}

#[test]
fn unknown_events_and_counts_cross_phase_boundaries() {
    // Fillers fail. The unknown event occurs just past the first phase, before the true event.
    let source = format!(
        "f({}, unknown, {}, winner);",
        (0..32)
            .map(|i| format!("m{i}"))
            .collect::<Vec<_>>()
            .join(", "),
        (32..96)
            .map(|i| format!("m{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let cases = [
        (
            json!({"kind":"identifier", "any":[{"regex":"^unknown$", "testUnknown":"unknown"}, {"regex":"^winner$"}]}),
            true,
        ),
        (
            json!({"kind":"identifier", "pattern":"$X", "any":[{"regex":"^unknown$", "testUnknown":"unknown"}, {"regex":"^winner$"}]}),
            false,
        ),
    ];
    for (sub, expected) in cases {
        let rule = compiled(json!({ "rule": { "kind":"arguments", "has": sub } }));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        assert_eq!(
            result["violations"].as_array().unwrap().len(),
            usize::from(expected)
        );
    }
    for (bounds, expected) in [
        (json!({"min": 1}), true),
        (json!({"min": 2}), false),
        (json!({"max": 0}), false),
        (json!({"min": 1, "max": 1}), false),
        (json!({"min": 1, "as": "N"}), false),
    ] {
        let mut count = bounds;
        count["has"] = json!({ "kind":"identifier", "any":[{"regex":"^unknown$", "testUnknown":"unknown"}, {"regex":"^winner$"}] });
        let rule = compiled(json!({ "rule": { "kind":"arguments", "count": count } }));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        assert_eq!(
            result["violations"].as_array().unwrap().len(),
            usize::from(expected)
        );
    }
}
