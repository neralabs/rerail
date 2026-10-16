//! Three-valued clauses (`plan`'s contract): both engines must report the same findings, and each
//! test pins the native answer so it exercises the case it names. `testUnknown` is unknown where
//! its regex matches the node's text and true elsewhere.

use super::{same, snapshot};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use serde_json::{Value, json};

fn compile(language: &str, mut ast: Value) -> CompiledCodeRuleDsl {
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(json!({
            "id": "ast-match",
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

/// The texts of the violations both engines agree on for `rule` over the TypeScript `source`.
fn agreed(rule: Value, source: &str) -> Vec<String> {
    agreed_in("typescript", "a.ts", json!({ "rule": rule }), source)
}

fn agreed_in(language: &str, path: &str, ast: Value, source: &str) -> Vec<String> {
    let result = same(
        &compile(language, ast),
        &snapshot(&[(language, path, source)]),
    );
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let span = &violation["span"];
            let start = span["startByte"].as_u64().expect("start") as usize;
            let end = span["endByte"].as_u64().expect("end") as usize;
            source[start..end].to_owned()
        })
        .collect()
}

/// `f(u1)`'s argument is unknown: an `all` binding `$X` from it is unknown, and so withholds the
/// later term that reads `$X`. Under `not`, a false `all` matches and an unknown one does not.
#[test]
fn an_unknown_term_withholds_the_later_terms_that_mention_what_it_would_bind() {
    let first = json!({ "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X", "testUnknown": "^u" } } });
    let source = "f(u1);\ng(a);\n";
    // The second term mentions `$X`: withheld, so the `all` stays unknown for `f(u1)`; `g(a)`
    // binds `$X = a`, which no identifier `zzz` is, so its `all` is false.
    let withheld = json!({
        "kind": "call_expression",
        "not": { "all": [
            first.clone(),
            { "has": { "stopBy": "end", "kind": "identifier", "regex": "^zzz", "pattern": "$X" } },
        ] },
    });
    assert_eq!(agreed(withheld, source), ["g(a)"]);
    // The same term without `$X` is not withheld: it is false, and so is the `all`.
    let decided = json!({
        "kind": "call_expression",
        "not": { "all": [
            first,
            { "has": { "stopBy": "end", "kind": "identifier", "regex": "^zzz" } },
        ] },
    });
    assert_eq!(agreed(decided, source), ["f(u1)", "g(a)"]);
}

/// A withheld term's own bindings are withheld too: a third term that reads what the withheld
/// one binds is withheld in turn, where a term reading nothing withheld still decides.
#[test]
fn a_withheld_term_withholds_what_it_binds() {
    let rule = json!({
        "kind": "call_expression",
        "not": { "all": [
            { "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X", "testUnknown": "^u" } } },
            { "has": { "kind": "identifier", "pattern": "$Y", "not": { "pattern": "$X" } } },
            { "has": { "stopBy": "end", "regex": "^zzz", "pattern": "$Y" } },
        ] },
    });
    assert_eq!(agreed(rule, "f(u1);\ng(a);\n"), ["g(a)"]);
}

/// An unknown `any` branch that may bind ends the `any` unknown: a later branch must not bind in
/// its place. One that binds nothing lets the later branches decide.
#[test]
fn an_unknown_any_branch_ends_the_search_only_where_it_may_bind() {
    let source = "f(u1);\ng(a);\n";
    let binding = json!({
        "kind": "call_expression",
        "any": [
            { "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X", "testUnknown": "^u" } } },
            { "pattern": "$F($$$)" },
        ],
    });
    assert_eq!(agreed(binding, source), ["g(a)"]);
    let plain = json!({
        "kind": "call_expression",
        "any": [
            { "has": { "kind": "arguments", "has": { "kind": "identifier", "testUnknown": "^u" } } },
            { "pattern": "$F($$$)" },
        ],
    });
    assert_eq!(agreed(plain, source), ["f(u1)", "g(a)"]);
    // Unknown branches that bind nothing and no true one: the `any` is unknown, not false.
    let open = json!({
        "kind": "call_expression",
        "not": { "any": [
            { "has": { "kind": "arguments", "has": { "kind": "identifier", "testUnknown": "^u" } } },
            { "regex": "^zzz" },
        ] },
    });
    assert_eq!(agreed(open, "f(u1);\ng(1);\n"), ["g(1)"]);
}

/// A relation whose rule may bind stops at its first unknown candidate; one that binds nothing
/// walks on to a later match.
#[test]
fn a_strict_relation_stops_at_its_first_unknown_candidate() {
    let source = "f(u1, a);\ng(a, u1);\n";
    let strict = json!({
        "kind": "arguments",
        "has": { "kind": "identifier", "pattern": "$X", "testUnknown": "^u" },
    });
    assert_eq!(agreed(strict, source), ["(a, u1)"]);
    let open = json!({
        "kind": "arguments",
        "has": { "kind": "identifier", "testUnknown": "^u" },
    });
    assert_eq!(agreed(open, source), ["(u1, a)", "(a, u1)"]);
    // A stop rule unknown at a node ends the walk there and leaves it open.
    let stopped = json!({
        "kind": "program",
        "not": { "has": { "kind": "identifier", "regex": "^zzz", "stopBy": { "kind": "expression_statement", "testUnknown": "^u" } } },
    });
    assert_eq!(agreed(stopped.clone(), "f(a);\n"), ["f(a);\n"]);
    assert_eq!(agreed(stopped, "u(a);\n"), Vec::<String>::new());
}

/// `count` over unknown candidates is an interval: true where every count in it holds, false
/// where none does, unknown otherwise (read here through `not`, true only where it is false).
#[test]
fn count_reads_unknown_candidates_as_an_interval() {
    let count = |bounds: Value| {
        let mut count = json!({ "has": { "kind": "identifier", "testUnknown": "^u" } });
        for (key, value) in bounds.as_object().expect("bounds") {
            count[key] = value.clone();
        }
        json!({ "kind": "arguments", "not": { "count": count } })
    };
    let source = "f(a, b, u1);\ng(a, u1);\nh(u1, u2);\n";
    // At least 2: `(a, b, u1)` is 2..=3, true; `(a, u1)` 1..=2 and `(u1, u2)` 0..=2, unknown.
    assert_eq!(
        agreed(count(json!({ "min": 2 })), source),
        Vec::<String>::new()
    );
    // At least 3: 2..=3 is open; 1..=2 and 0..=2 fail.
    assert_eq!(
        agreed(count(json!({ "min": 3 })), source),
        ["(a, u1)", "(u1, u2)"]
    );
    // At most 1: 2..=3 fails; 1..=2 and 0..=2 are open.
    assert_eq!(agreed(count(json!({ "max": 1 })), source), ["(a, b, u1)"]);
    // Exactly 4: 2..=3, 1..=2 and 0..=2 all fail.
    assert_eq!(
        agreed(count(json!({ "min": 4, "max": 4 })), source),
        ["(a, b, u1)", "(a, u1)", "(u1, u2)"]
    );
    // A bound count needs its exact value: unknown wherever a candidate is.
    let bound = json!({
        "kind": "arguments",
        "count": { "has": { "kind": "identifier", "testUnknown": "^u" }, "min": 1, "as": "N" },
    });
    assert_eq!(agreed(bound, "f(a, b);\ng(a, u1);\n"), ["(a, b)"]);
}

/// An unknown root does not match, and neither does its negation.
#[test]
fn an_unknown_root_does_not_match() {
    let source = "f(a, u1);\n";
    assert_eq!(
        agreed(json!({ "kind": "identifier", "testUnknown": "^u" }), source),
        ["f", "a"]
    );
    assert_eq!(
        agreed(
            json!({ "kind": "identifier", "not": { "testUnknown": "^u" } }),
            source
        ),
        Vec::<String>::new()
    );
}

/// `nthChild` is unknown where a sibling's `ofRule` probe is, whatever the position; a call is
/// unknown where its utility's body is.
#[test]
fn nth_child_and_calls_read_unknown_probes() {
    let source = "f(a, u1, b);\ng(a, b);\n";
    let nth = json!({
        "kind": "identifier",
        "inside": { "kind": "arguments" },
        "not": { "nthChild": { "position": 1, "ofRule": { "kind": "identifier", "testUnknown": "^u" } } },
    });
    assert_eq!(agreed(nth, source), ["b"]);
    let called = json!({
        "rule": { "kind": "call_expression", "not": { "matches": { "unknown-arg": { "P": "$F" } } } },
        "utils": { "unknown-arg": { "params": ["P"], "rule": { "has": { "kind": "arguments", "has": { "kind": "identifier", "testUnknown": "^u" } } } } },
    });
    assert_eq!(
        agreed_in("typescript", "a.ts", called, source),
        Vec::<String>::new()
    );
}

/// `value`, `compare` and `layout` evaluate natively behind the `fact` terminal, once per
/// statement over the distinct keys of its frame.
#[test]
fn semantic_leaves_answer_through_the_fact_terminal() {
    use crate::code_rule_dsl::sql::terminals::calls::FACT_FILTERS;
    let source = "const n = 1;\nconst s = 'x';\nf(n, s, m, 2);\n";
    FACT_FILTERS.set(0);
    let numbers = json!({
        "kind": "identifier",
        "inside": { "kind": "arguments" },
        "value": { "type": "number", "follow": true },
    });
    assert_eq!(agreed(numbers, source), ["n"]);
    assert_eq!(FACT_FILTERS.get(), 1);
    // `m` has no binding: its type is unknown, so `not` does not hold there either.
    let others = json!({
        "kind": "identifier",
        "inside": { "kind": "arguments" },
        "not": { "value": { "type": "number", "follow": true } },
    });
    assert_eq!(agreed(others, source), ["s"]);
    let ordered = json!({
        "pattern": "f($A, $B, $$$)",
        "compare": { "left": "A", "op": "lt", "right": "B", "text": "source", "collation": "fold-upper" },
    });
    assert_eq!(agreed(ordered, source), ["f(n, s, m, 2)"]);
    let laid = json!({
        "kind": "lexical_declaration",
        "layout": { "after": { "to": "sibling", "breaks": { "eq": 1 } } },
    });
    assert_eq!(agreed(laid, source), ["const n = 1;", "const s = 'x';"]);
}

/// Withholding is not bounded by one mask word: three unknown terms bind 90 captures, and the
/// term reading the last of them (a bit of the second `wh` column) is withheld.
#[test]
fn withholding_spans_several_mask_columns() {
    let source = format!("f({});\n", vec!["a"; 30].join(", "));
    let unknown = |prefix: &str| {
        let names: Vec<String> = (0..30).map(|index| format!("${prefix}{index}")).collect();
        json!({ "pattern": format!("f({})", names.join(", ")), "testUnknown": "^f" })
    };
    let rule = |last: Value| {
        json!({
            "kind": "call_expression",
            "not": { "all": [unknown("A"), unknown("B"), unknown("C"), last] },
        })
    };
    let withheld = json!({ "has": { "stopBy": "end", "regex": "^zzz", "pattern": "$C29" } });
    assert_eq!(agreed(rule(withheld), &source), Vec::<String>::new());
    let decided = json!({ "has": { "stopBy": "end", "regex": "^zzz" } });
    assert_eq!(
        agreed(rule(decided), &source),
        [source.trim_end().trim_end_matches(';')]
    );
}

/// A batch builds each file's scope table once, however many statements read values through it:
/// the fact terminal parks the file's scopes between statements.
#[test]
fn a_batch_builds_a_files_scopes_once() {
    use crate::code_rule_dsl::matcher::TABLE_BUILDS;
    let compiled = compile(
        "typescript",
        json!({ "rule": {
            "kind": "identifier",
            "inside": { "kind": "arguments" },
            "value": { "type": "number", "follow": true },
            "not": { "has": { "stopBy": "end", "value": { "type": "string", "follow": true } } },
        } }),
    );
    let input = snapshot(&[(
        "typescript",
        "a.ts",
        "const n = 1;\nconst s = 'x';\nf(n, s, m, 2);\n",
    )]);
    TABLE_BUILDS.set(0);
    let result = super::evaluate(&compiled, &input, "sql");
    assert_eq!(result["violations"].as_array().map(Vec::len), Some(1));
    assert_eq!(TABLE_BUILDS.get(), 1);
}
