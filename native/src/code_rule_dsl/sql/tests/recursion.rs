//! Recursive utilities and `violation.each`, one named test per shape: both engines must report
//! the same findings, and each test also pins the native answer so it exercises what it names.

use super::{same, snapshot};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use serde_json::{Value, json};

/// One clause in `language` with `ast` (`rule`, `utils`, `constraints`), enumerating `each`.
fn compile(language: &str, mut ast: Value, each: Option<&str>) -> CompiledCodeRuleDsl {
    ast["language"] = json!(language);
    let mut violation = json!({ "anchor": "witness.span", "code": "X", "message": "x" });
    if let Some(each) = each {
        violation["each"] = json!(each);
    }
    compile_document_with(
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn extension(language: &str) -> &'static str {
    match language {
        "python" => "py",
        "php" => "php",
        _ => "ts",
    }
}

/// Each violation's text and its captures' texts, `TEXT NAME=text…`.
fn texts(result: &Value, source: &str) -> Vec<String> {
    let text = |span: &Value| {
        let start = span["startByte"].as_u64().expect("start") as usize;
        let end = span["endByte"].as_u64().expect("end") as usize;
        &source[start..end]
    };
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let mut parts = vec![text(&violation["span"]).to_owned()];
            for (name, spans) in violation["captures"].as_object().expect("captures") {
                let joined = spans
                    .as_array()
                    .expect("spans")
                    .iter()
                    .map(text)
                    .collect::<Vec<_>>()
                    .join("|");
                parts.push(format!("{name}={joined}"));
            }
            parts.join(" ")
        })
        .collect()
}

/// Both engines' findings for `ast` over `source`, which must agree; their texts.
fn agreed(language: &str, ast: Value, each: Option<&str>, source: &str) -> Vec<String> {
    let compiled = compile(language, ast, each);
    let result = same(
        &compiled,
        &snapshot(&[(language, &format!("a.{}", extension(language)), source)]),
    );
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut found = texts(&result, source);
    found.sort();
    found
}

/// The SQL engine's refusal of `ast` over `source`.
fn refused(language: &str, ast: Value, source: &str) -> String {
    let compiled = compile(language, ast, None);
    let input = snapshot(&[(language, &format!("a.{}", extension(language)), source)]);
    compiled
        .evaluate(
            input.to_string(),
            "report".to_owned(),
            Some("sql".to_owned()),
        )
        .expect_err("the SQL engine refuses")
        .reason
}

#[test]
fn a_recursive_utility_descends_through_has() {
    // `tail` holds where a member or call chain ends at the identifier `x`, as deep as it goes.
    let ast = json!({
        "rule": { "kind": "call_expression", "matches": "tail" },
        "utils": { "tail": { "any": [
            { "kind": "identifier", "regex": "^x$" },
            { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
            { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
        ] } },
    });
    let source = "x.a.b();\ny.a();\nx();\nx.c().d();\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["x()", "x.a.b()", "x.c()", "x.c().d()"]
    );
}

#[test]
fn a_recursive_utility_ascends_through_inside() {
    let ast = json!({
        "rule": { "kind": "identifier", "regex": "^v$", "matches": "looped" },
        "utils": { "looped": { "any": [
            { "kind": "for_statement" },
            { "inside": { "matches": "looped" } },
        ] } },
    });
    let source = "for (;;) { f(v); }\ng(v);\n";
    assert_eq!(agreed("typescript", ast, None, source), ["v"]);
}

#[test]
fn a_recursive_utility_walks_siblings() {
    // A statement some later sibling of which returns, one sibling at a time.
    let ast = json!({
        "rule": { "kind": "expression_statement", "matches": "later" },
        "utils": { "later": { "any": [
            { "kind": "return_statement" },
            { "precedes": { "matches": "later" } },
        ] } },
    });
    let source = "function f() { a(); b(); return 1; }\nfunction g() { c(); }\n";
    assert_eq!(agreed("typescript", ast, None, source), ["a();", "b();"]);
    // And backwards, through `follows`.
    let ast = json!({
        "rule": { "kind": "expression_statement", "matches": "earlier" },
        "utils": { "earlier": { "any": [
            { "kind": "lexical_declaration" },
            { "follows": { "matches": "earlier" } },
        ] } },
    });
    let source = "function f() { a(); const x = 1; b(); c(); }\n";
    assert_eq!(agreed("typescript", ast, None, source), ["b();", "c();"]);
}

#[test]
fn negation_over_a_recursive_reference_is_stratified() {
    // A block every statement of which ends in a `return`, recursively through nested blocks.
    let ast = json!({
        "rule": { "kind": "statement_block", "matches": "returns" },
        "utils": { "returns": { "any": [
            { "kind": "return_statement" },
            { "kind": "statement_block", "has": { "kind": "return_statement" },
              "not": { "has": { "kind": "if_statement", "not": { "has": { "field": "consequence", "matches": "returns" } } } } },
        ] } },
    });
    let source = "function f() { if (a) { return 1; } return 2; }\n\
                  function g() { if (a) { b(); } return 2; }\n";
    let found = agreed("typescript", ast, None, source);
    assert_eq!(
        found,
        ["{ if (a) { return 1; } return 2; }", "{ return 1; }"]
    );
    // A count over a recursive reference.
    let ast = json!({
        "rule": { "kind": "statement_block", "count": { "has": { "matches": "deep", "stopBy": "end" }, "min": 2, "as": "N" } },
        "utils": { "deep": { "any": [
            { "kind": "return_statement" },
            { "kind": "if_statement", "has": { "field": "consequence", "matches": "deep" } },
            { "kind": "statement_block", "has": { "matches": "deep" } },
        ] } },
    });
    let found = agreed("typescript", ast, None, source);
    assert!(!found.is_empty());
}

#[test]
fn a_recursive_utility_binds_captures_in_its_frames() {
    // The innermost object of a member chain, bound by the recursion's deepest level.
    let ast = json!({
        "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "base" } },
        "utils": { "base": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "kind": "member_expression", "has": { "field": "object", "matches": "base" } },
        ] } },
    });
    let source = "x.a.b();\ny();\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["x.a.b() A=x", "y() A=y"]
    );
}

#[test]
fn a_recursive_utility_reads_a_capture_bound_before_it() {
    // A call whose arguments hold, at any depth, its own callee.
    let ast = json!({
        "rule": { "pattern": "$A($$$)", "has": { "field": "arguments", "matches": "same" } },
        "utils": { "same": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "has": { "matches": "same" } },
        ] } },
    });
    let source = "f(g(f));\nh(1);\nk([k]);\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["f(g(f)) A=f", "k([k]) A=k"]
    );
}

#[test]
fn a_recursive_parameterized_call_returns_through_its_frames() {
    let ast = json!({
        "rule": { "pattern": "$X($$$)", "has": { "field": "arguments", "matches": { "same": { "P": "$X" } } } },
        "utils": { "same": { "params": ["P"], "rule": { "any": [
            { "kind": "identifier", "pattern": "$P" },
            { "has": { "matches": { "same": { "P": "$P" } } } },
        ] } } },
    });
    let source = "f(g(f));\nh(1);\nk([k]);\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["f(g(f)) X=f", "k([k]) X=k"]
    );
    // Calls that recurse through each other, binding through either.
    for any in crate::code_rule_dsl::test_support::recursive_call_branch_orders() {
        let ast = json!({
            "rule": { "kind": "expression_statement", "any": any },
            "utils": crate::code_rule_dsl::test_support::mutually_recursive_calls(),
        });
        let source = "a;\nb;\n{ c; }\n";
        agreed("typescript", ast, None, source);
    }
}

#[test]
fn two_sites_of_one_recursion_run_its_loop_each_over_their_own_demand() {
    // `same` reads `$A`, so it is a unit, not a stage. Its first site demands each call's
    // arguments; its second, only behind the calls the first kept, the return statements of
    // their blocks: other keys, demanded after the first site's answers are in.
    let ast = json!({
        "rule": { "pattern": "$A($$$)", "all": [
            { "has": { "field": "arguments", "matches": "same" } },
            { "inside": { "stopBy": "end", "kind": "statement_block",
                          "has": { "stopBy": "end", "kind": "return_statement", "matches": "same" } } },
        ] },
        "utils": { "same": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "has": { "matches": "same" } },
        ] } },
    });
    let source = "function q() { f(g(f)); return [f]; }\nfunction r() { h(h); return x; }\n\
                  function s() { k(1); return k; }\nf(g(f));\n";
    assert_eq!(
        agreed("typescript", ast.clone(), None, source),
        ["f(g(f)) A=f"]
    );
    // The loop is emitted once and run again at the second site.
    assert_eq!(markers("typescript", ast), [1, 1, 1]);
}

/// How many loop blocks (`BLOCK`), loops (`LOOP`) and later runs of a block (`RUN`) the SQL
/// engine's statements of `ast` hold.
fn markers(language: &str, ast: Value) -> [usize; 3] {
    let compiled = compile(language, ast, None);
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &compiled.inner.clauses[0].kind
    else {
        panic!("an AST clause")
    };
    let program = crate::code_rule_dsl::sql::compile(&[&matchers[0]]).expect("a program");
    let count = |marker: &str| {
        program
            .statements()
            .filter(|statement| statement.starts_with(marker))
            .count()
    };
    [count("/*BLOCK*/"), count("/*LOOP*/"), count("/*RUN*/")]
}

#[test]
fn a_utility_referenced_at_three_sites_is_one_unit_each_site_demands() {
    // `named` reads `$F` and is not recursive: its three references are one unit, each demanding
    // its own nodes (the arguments, the enclosing block's returns, an enclosing class).
    let ast = json!({
        "rule": { "pattern": "$F($$$)", "all": [
            { "has": { "field": "arguments", "matches": "named" } },
            { "inside": { "stopBy": "end", "kind": "statement_block",
                          "has": { "stopBy": "end", "kind": "return_statement", "matches": "named" } } },
            { "not": { "inside": { "stopBy": "end", "kind": "class_declaration", "matches": "named" } } },
        ] },
        "utils": { "named": { "has": { "stopBy": "end", "kind": "identifier", "pattern": "$F" } } },
    });
    let source = "function q() { f(g(f)); return [f]; }\nfunction r() { h(h); return x; }\n\
                  class f { m() { f(f); return f; } }\nfunction s() { k([k]); return k; }\n";
    assert_eq!(
        agreed("typescript", ast.clone(), None, source),
        ["f(g(f)) F=f", "k([k]) F=k"]
    );
    assert_eq!(markers("typescript", ast), [1, 1, 2]);
}

#[test]
fn a_unit_reading_a_capture_it_does_not_hold_is_keyed_by_it() {
    // `other` compares its `$X` against the caller's `$F`, which it never binds: two references
    // make it one unit, whose answer at a node differs with `$F`, so `$F` is in its key.
    let ast = json!({
        "rule": { "pattern": "$F($$$)", "all": [
            { "has": { "field": "arguments", "has": { "stopBy": "end", "matches": "other" } } },
            { "has": { "field": "arguments", "has": { "stopBy": "end", "kind": "identifier", "matches": "other" } } },
        ] },
        "utils": { "other": { "kind": "identifier", "pattern": "$X",
                              "compare": { "left": "X", "op": "ne", "right": "F", "text": "node" } } },
    });
    let source = "function q() { f(g(f)); h(h); k(x); }\n";
    assert_eq!(
        agreed("typescript", ast.clone(), None, source),
        [
            "f(g(f)) F=f X=g",
            "g(f) F=g X=f",
            "h(h) F=h X=h",
            "k(x) F=k X=x"
        ]
    );
    assert_eq!(markers("typescript", ast), [1, 1, 1]);
}

#[test]
fn a_chain_of_units_runs_each_where_a_site_demands_new_keys() {
    // `d<k>` reads `d<k-1>` at two sites: run at every site of every pass above it, the chain's
    // loops would run 2^k times. A site that demands no new key runs nothing.
    let depth = 16;
    let mut utils = serde_json::Map::new();
    utils.insert("d0".to_owned(), json!({ "kind": "statement_block" }));
    for level in 1..=depth {
        let below = format!("d{}", level - 1);
        utils.insert(
            format!("d{level}"),
            json!({ "any": [
                { "kind": "statement_block", "matches": below },
                { "inside": { "stopBy": "end", "matches": below } },
            ] }),
        );
    }
    let ast = json!({
        "rule": { "kind": "identifier", "matches": format!("d{depth}") },
        "utils": utils,
    });
    let source = "function q() { if (a) { f(b); } }\nlet c = 1;\n";
    crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE.set(Some(Default::default()));
    assert_eq!(agreed("typescript", ast, None, source), ["a", "b", "f"]);
    let statements: usize = crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE
        .take()
        .unwrap_or_default()
        .values()
        .map(|(_, _, runs)| runs)
        .sum();
    assert!(statements < 5_000, "{statements} statements");
}

#[test]
fn a_parameterized_utility_called_at_two_sites_maps_its_parameters_per_site() {
    // `holds` is called with `P` bound to the callee at one site, and with `P` unbound at the
    // other, where it returns what it binds as `$Y`: one unit, keyed by `P`'s value.
    let ast = json!({
        "rule": { "pattern": "$F($$$)", "all": [
            { "has": { "field": "arguments", "matches": { "holds": { "P": "$F" } } } },
            { "inside": { "stopBy": "end", "kind": "return_statement",
                          "matches": { "holds": { "P": "$Y" } } } },
        ] },
        "utils": { "holds": { "params": ["P"], "rule": {
            "has": { "stopBy": "end", "kind": "identifier", "pattern": "$P" } } } },
    });
    let source = "function q() { return f(g(f)); }\nfunction r() { return h(1); }\n\
                  function s() { return [k(k)]; }\nm(m);\n";
    assert_eq!(
        agreed("typescript", ast.clone(), None, source),
        ["f(g(f)) F=f Y=f", "k(k) F=k Y=k"]
    );
    assert_eq!(markers("typescript", ast), [1, 1, 1]);
}

#[test]
fn mutually_recursive_utilities_meet_at_the_same_node() {
    // The data-layer policies' raising-branch utilities, in small: an `if` raises where its
    // consequence and every `elif`/`else` raise.
    let ast = json!({
        "rule": { "kind": "function_definition", "has": { "field": "body", "matches": "block-raises" } },
        "utils": {
            "block-raises": { "has": { "stopBy": "neighbor", "matches": "stmt-raises" } },
            "stmt-raises": { "any": [
                { "kind": "raise_statement" },
                { "kind": "if_statement", "matches": "if-raises" },
            ] },
            "if-raises": { "all": [
                { "has": { "field": "consequence", "matches": "block-raises" } },
                { "any": [
                    { "kind": "if_statement", "has": { "stopBy": "neighbor",
                        "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }],
                        "not": { "follows": { "stopBy": "end", "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }] } },
                        "matches": "orelse-raises" } },
                    { "kind": "elif_clause", "precedes": { "stopBy": "end",
                        "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }],
                        "matches": "orelse-raises" } },
                ] },
            ] },
            "orelse-raises": { "any": [
                { "kind": "elif_clause", "matches": "if-raises" },
                { "kind": "else_clause", "has": { "field": "body", "matches": "block-raises" } },
            ] },
        },
    });
    let source = "def f():\n    if a:\n        raise X\n    else:\n        raise Y\n\
                  def g():\n    if a:\n        raise X\n    elif b:\n        pass\n    else:\n        raise Y\n\
                  def h():\n    raise Z\n";
    let found = agreed("python", ast, None, source);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_recursion_whose_reads_loop_back_is_refused() {
    // From a node to its parent and back to its children: the node itself again.
    let ast = json!({
        "rule": { "kind": "identifier", "matches": "loop" },
        "utils": { "loop": { "any": [
            { "kind": "number" },
            { "inside": { "has": { "matches": "loop" } } },
        ] } },
    });
    let reason = refused("typescript", ast, "f(a, 1);\n");
    assert!(
        reason.contains("Unsupported(recursion whose reads loop back)"),
        "{reason}"
    );
}

/// `reach` walks from a call up to its function and down to the calls in it, its own among them,
/// as a function that calls itself is reached through its own definition: `rule` with `has` as
/// the walk down.
fn reach(down: Value) -> Value {
    json!({
        "rule": { "kind": "call", "matches": "reach" },
        "utils": { "reach": { "any": [
            { "pattern": "g($$$)" },
            { "inside": { "stopBy": "end", "kind": "function_definition", "has": down } },
        ] } },
    })
}

/// The matcher stops `has` at its first match, `g(x)`, before it reaches the self-call `f(x)`;
/// the set evaluation tests `f(x)` too, which reads `f(x)`'s own key. That read is from a
/// candidate past the first match, which the read graph leaves out (`emit::lineage`).
#[test]
fn a_self_call_past_the_first_match_reads_no_loop() {
    let down = json!({ "stopBy": "end", "kind": "call", "matches": "reach" });
    assert_eq!(
        agreed(
            "python",
            reach(down),
            None,
            "def f(x):\n    g(x)\n    f(x)\n"
        ),
        ["f(x)", "g(x)"]
    );
}

/// Where the self-call comes first, the matcher reads `f(x)`'s key evaluating it: a loop.
#[test]
fn a_self_call_before_the_first_match_is_refused() {
    let down = json!({ "stopBy": "end", "kind": "call", "matches": "reach" });
    let reason = refused("python", reach(down), "def f(x):\n    f(x)\n    g(x)\n");
    assert!(
        reason.contains("Unsupported(recursion whose reads loop back)"),
        "{reason}"
    );
}

/// A strict search (its rule binds `$C`) ends at its first unknown candidate, `u(x)`
/// (`testUnknown`), before the self-call: `f(x)` is unknown, not matched, and its own key is read
/// past the end of the search. Where the self-call comes first, the search reads it.
#[test]
fn a_strict_search_ending_unknown_before_a_self_call_reads_no_loop() {
    let down = json!({ "stopBy": "end", "any": [
        { "kind": "call", "pattern": "$C($$$)", "regex": "^u", "testUnknown": "^u" },
        { "kind": "call", "matches": "reach" },
    ] });
    assert!(
        crate::code_rule_dsl::test_support::grammar_matchers(&compile(
            "python",
            reach(down.clone()),
            None
        ))[0]
            .three_valued
    );
    assert_eq!(
        agreed(
            "python",
            reach(down.clone()),
            None,
            "def f(x):\n    u(x)\n    f(x)\ng(1)\n"
        ),
        ["g(1)"]
    );
    let reason = refused(
        "python",
        reach(down),
        "def f(x):\n    f(x)\n    u(x)\ng(1)\n",
    );
    assert!(
        reason.contains("Unsupported(recursion whose reads loop back)"),
        "{reason}"
    );
}

/// A `count` tests every candidate, an `nthChild` probe every named sibling, and a `not` decides
/// its row whole: past a candidate that matched, each still reads the self-call's key, as the
/// matcher does, so the loop is refused (the `not`'s answer flips every pass: it never settles).
#[test]
fn count_nth_child_and_not_over_a_self_call_still_read_its_loop() {
    let downs = [
        (
            json!({ "stopBy": "end", "kind": "block", "count": { "has": { "stopBy": "end", "kind": "call", "matches": "reach" }, "min": 1 } }),
            "def f(x):\n    g(x)\n    f(x)\n",
        ),
        (
            json!({ "stopBy": "end", "kind": "argument_list", "has": { "kind": "call", "nthChild": { "position": 1, "ofRule": { "matches": "reach" } } } }),
            "def f(x):\n    h(g(x), f(x))\n",
        ),
        (
            json!({ "stopBy": "end", "kind": "call", "not": { "matches": "reach" } }),
            "def f(x):\n    g(x)\n    f(x)\n",
        ),
    ];
    for (down, source) in downs {
        let reason = refused("python", reach(down.clone()), source);
        assert!(
            reason.contains("Unsupported(recursion whose reads loop back)")
                || reason.contains("Unsupported(recursion deeper than 256 levels)"),
            "{down}: {reason}"
        );
    }
}

/// `a` and `b` call each other, so `reach` loops between them, but only `main` is a root, and
/// the matcher stops `main`'s search at `h()`, which reaches `g`: it never evaluates `a(1)`, nor
/// `a` or `b`. The set evaluation demands their keys from that cut candidate; no needed row
/// reaches them, so their loop is not checked (the start set of `exec::acyclic`).
#[test]
fn a_loop_only_a_cut_candidate_reaches_is_not_checked() {
    let ast = json!({
        "rule": { "kind": "function_definition", "regex": "^def main", "matches": "reach" },
        "utils": {
            "reach": { "kind": "function_definition", "any": [
                { "has": { "stopBy": "end", "pattern": "g($$$)" } },
                { "has": { "stopBy": "end", "kind": "call", "matches": "edge" } },
            ] },
            "edge": { "kind": "call", "has": { "field": "function", "kind": "identifier",
                "resolves": { "binding": { "site": { "matches": "reach" } } } } },
        },
    });
    let source = "def a(x):\n    b(x)\n\ndef b(x):\n    a(x)\n\ndef h():\n    g(1)\n\ndef main():\n    h()\n    a(1)\n";
    let found = agreed("python", ast.clone(), None, source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("def main"), "{found:?}");
    // Where `main` calls `a` first, the matcher evaluates the loop, and leaves it where it comes
    // back to `a`'s site under the site's rule (`binding::FOLLOWING`): `a(1)` reaches nothing.
    let first = source.replace("    h()\n    a(1)\n", "    a(1)\n    h()\n");
    let found = agreed("python", ast, None, &first);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("def main"), "{found:?}");
}

/// `_compile_server_ref.py`'s shape (serverplan_state_tripwires' `ctx-reach`/`ctx-edge`):
/// `lower_scalar_server_ref` and `lower_scalar_source` call each other, through the site of the
/// binding each call resolves to, and only `resolve_ref` reads the context's materialized state.
/// The matcher leaves the loop where a call comes back to a site whose rule it is matching
/// (`binding::FOLLOWING`), and each function reaches `resolve_ref` another way; the set
/// evaluation keys each `reach` by the sites it is following and agrees. Both as a shared
/// utility and as a parameterized call (whose frame is keyed by its parameters).
#[test]
fn mutual_recursion_through_a_binding_site_follows_as_the_matcher() {
    let source = "\
def lower_scalar_server_ref(value, ctx):\n    if isinstance(value, Seed):\n        return lower_scalar_source(value, ctx)\n    return resolve_ref(value, ctx)\n\n\
def lower_scalar_source(value, ctx):\n    if is_ref(value):\n        return lower_scalar_server_ref(value, ctx)\n    return value\n\n\
def resolve_ref(value, ctx):\n    return getattr(ctx, \"materialized_pipeline\", None)\n\n\
def lower_row_cell(ref, ctx):\n    return lower_scalar_server_ref(ref, ctx)\n\n\
def unrelated(value):\n    return is_ref(value)\n";
    let sink = json!({ "kind": "call", "regex": "^getattr\\(\\s*ctx" });
    let edge = |reach: Value| {
        json!({ "kind": "call", "has": { "field": "function", "kind": "identifier",
            "resolves": { "binding": { "site": { "all": [
                { "kind": "function_definition" },
                { "matches": reach },
            ] } } } } })
    };
    let body = json!({ "kind": "function_definition", "any": [
        { "has": { "stopBy": "end", "matches": "sink" } },
        { "all": [
            { "not": { "has": { "stopBy": "end", "matches": "sink" } } },
            { "has": { "stopBy": "end", "kind": "call", "matches": "edge" } },
        ] },
    ] });
    let rule = |reach: Value| {
        json!({ "kind": "function_definition", "pattern": "$F",
            "not": { "has": { "stopBy": "end", "matches": "sink" } }, "matches": reach })
    };
    let shared = json!({
        "rule": rule(json!("reach")),
        "utils": { "sink": sink, "reach": body, "edge": edge(json!("reach")) },
    });
    let called = json!({
        "rule": rule(json!({ "reach": { "Z": "$Z" } })),
        "utils": {
            "sink": sink,
            "reach": { "params": ["Z"], "rule": body },
            "edge": edge(json!({ "reach": { "Z": "$Z" } })),
        },
    });
    for ast in [shared, called] {
        let found = agreed("python", ast.clone(), None, source);
        let names: Vec<&str> = found
            .iter()
            .map(|found| found.split(['(', ' ']).nth(1).expect("a def"))
            .collect();
        assert_eq!(
            names,
            [
                "lower_row_cell",
                "lower_scalar_server_ref",
                "lower_scalar_source"
            ],
            "{ast}"
        );
    }
}

/// The guard changes which candidate binds first, where a least fixpoint would not. `b()`'s
/// site is `b`, entered under `edge`'s site rule; in `b`, `a()` comes first, but `a` reaches the
/// sink only through `b()`, whose site the matcher is matching: `a` fails there, and `d()` binds
/// `$C`. A least fixpoint over the call graph, which knows no context, has `a` reach the sink
/// and binds `a`.
#[test]
fn the_following_guard_decides_which_candidate_binds_first() {
    let ast = json!({
        "rule": { "kind": "call", "regex": "^b\\(\\)$", "matches": "edge" },
        "utils": {
            "reach": { "params": ["C"], "rule": { "kind": "function_definition", "any": [
                { "has": { "stopBy": "end", "kind": "call", "regex": "^sink\\(" } },
                { "has": { "stopBy": "end", "kind": "call", "pattern": "$C($$$)", "matches": "edge" } },
            ] } },
            "edge": { "kind": "call", "has": { "field": "function", "kind": "identifier",
                "resolves": { "binding": { "site": { "matches": { "reach": { "C": "$D" } } } } } } },
        },
    });
    let source = "def a():\n    b()\n\ndef b():\n    a()\n    d()\n\ndef d():\n    sink()\n\ndef main():\n    b()\n";
    assert_eq!(agreed("python", ast, None, source), ["b() D=d", "b() D=d"]);
}

/// A loop that passes no binding sub-rule keeps its following context: its keys repeat, and it
/// is refused, as before, where a guarded loop in the same cycle is not.
#[test]
fn a_loop_outside_the_guarded_sub_rules_is_still_refused() {
    let ast = json!({
        "rule": { "kind": "call", "matches": "edge" },
        "utils": {
            "reach": { "kind": "function_definition", "any": [
                { "has": { "stopBy": "end", "kind": "call", "matches": "edge" } },
                { "has": { "stopBy": "end", "kind": "call", "regex": "^sink\\(" } },
            ] },
            "edge": { "kind": "call", "any": [
                { "has": { "field": "function", "kind": "identifier",
                    "resolves": { "binding": { "site": { "matches": "reach" } } } } },
                { "inside": { "stopBy": "end", "kind": "function_definition", "matches": "reach" } },
            ] },
        },
    });
    let reason = refused("python", ast, "def f():\n    g()\n");
    assert!(
        reason.contains("Unsupported(recursion whose reads loop back)"),
        "{reason}"
    );
}

/// A `matches` of a capture-free utility, recursive or not, kind-less or not, runs on the keys its
/// frame demands: no per-batch stage evaluates it over every node of its root kinds.
#[test]
fn a_capture_free_utility_runs_on_the_keys_its_references_demand() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let utils = json!({
        "tail": { "any": [
            { "kind": "identifier", "regex": "^x$" },
            { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
        ] },
        "top": { "not": { "inside": { "stopBy": "end", "kind": "function_declaration" } } },
    });
    let rules = [
        json!({ "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } }, "utils": utils.clone() }),
        json!({ "rule": { "kind": "call_expression", "matches": "top", "any": [
            { "has": { "field": "function", "matches": "top" } },
            { "has": { "field": "function", "matches": "tail" } },
        ] }, "utils": utils }),
    ];
    let source = "x.a();\nfunction f() { x.b(); y.c(); }\ny.d();\n";
    for rule in rules {
        let found = agreed("typescript", rule.clone(), None, source);
        assert!(!found.is_empty(), "{rule}");
        let rule = compiled(rule);
        let grammars = vec![&grammar_matchers(&rule)[0]];
        let (stages, _) = crate::code_rule_dsl::sql::compile(&grammars)
            .expect("compiles")
            .statement_count();
        assert_eq!(stages, 0, "a stage stands for a `matches`");
    }
}

/// A recursive utility of a three-valued clause returns its body's unknown verdicts through
/// `matches` (`testUnknown` is unknown where its regex matches): `u.a()` is neither matched nor
/// matched by the negation, both in a capture-free cycle and in one whose frames bind a capture
/// (units whose answers carry `unk`).
#[test]
fn a_three_valued_recursive_utility_returns_unknown_verdicts() {
    use crate::code_rule_dsl::sql::terminals::calls::UNKNOWN_VERDICTS;
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let source = "x.a();\nu.a();\ny.a();\nx.b.c();\nu.b.c();\n";
    for (leaf, held_found) in [
        (
            json!({ "kind": "identifier", "regex": "^[xu]$", "testUnknown": "^u" }),
            ["x.a()", "x.b.c()"],
        ),
        (
            json!({ "kind": "identifier", "regex": "^[xu]$", "pattern": "$X", "testUnknown": "^u" }),
            ["x.a() X=x", "x.b.c() X=x"],
        ),
    ] {
        let utils = json!({ "tail": { "any": [
            leaf,
            { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
            { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
        ] } });
        let held = json!({ "rule": { "kind": "call_expression", "matches": "tail" }, "utils": utils.clone() });
        assert!(grammar_matchers(&compile("typescript", held.clone(), None))[0].three_valued);
        UNKNOWN_VERDICTS.set(0);
        assert_eq!(agreed("typescript", held, None, source), held_found);
        assert!(UNKNOWN_VERDICTS.get() > 0, "a terminal answered unknown");
        let negated = json!({
            "rule": { "kind": "call_expression", "not": { "matches": "tail" } },
            "utils": utils,
        });
        assert_eq!(agreed("typescript", negated, None, source), ["y.a()"]);
    }
}

/// Three-valued `violation.each` (`emit/each.rs`): a strict search passes over a true candidate
/// whose binding was reported and stops at the unknown one after it (`testUnknown` is unknown
/// where its regex matches), which leaves the round's match unknown and ends the enumeration
/// before `b`; where the unknown comes last, `b` is enumerated first.
#[test]
fn each_passes_over_an_excluded_true_candidate_and_stops_at_an_unknown_one() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A", "testUnknown": "^u" } },
    } });
    assert!(grammar_matchers(&compile("typescript", ast.clone(), Some("A")))[0].three_valued);
    assert_eq!(
        agreed("typescript", ast.clone(), Some("A"), "f(a, u, b);\n"),
        ["f(a, u, b) A=a"]
    );
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b, u);\n"),
        ["f(a, b, u) A=a", "f(a, b, u) A=b"]
    );
}

/// An `any` branch whose true trial binds a reported range reads as false: the next branch
/// decides. An unknown branch that binds nothing then leaves the `any` unknown, so the strict
/// search over the identifiers stops at `a` in the second round; one that may bind ends the
/// `any` unknown though a later branch would bind `b`.
#[test]
fn each_reads_an_unknown_any_branch_after_an_excluded_one() {
    let source = "f(a, b);\n";
    let any = |branches: Value| {
        json!({ "rule": {
            "kind": "call_expression",
            "has": { "field": "arguments", "has": { "kind": "identifier", "any": branches } },
        } })
    };
    let decided = any(json!([{ "pattern": "$A" }, { "regex": "^zzz" }]));
    assert_eq!(
        agreed("typescript", decided, Some("A"), source),
        ["f(a, b) A=a", "f(a, b) A=b"]
    );
    let open = any(json!([{ "pattern": "$A" }, { "testUnknown": "^a" }]));
    assert_eq!(
        agreed("typescript", open, Some("A"), source),
        ["f(a, b) A=a"]
    );
    let binding = any(json!([
        { "pattern": "$A", "testUnknown": "^b" },
        { "pattern": "$A" },
    ]));
    assert_eq!(
        agreed("typescript", binding, Some("A"), source),
        ["f(a, b) A=a"]
    );
}

/// A round whose match is unknown leaves its root unknown: in round 1 the first term binds `u`
/// and the second is unknown there, so `f` reports `a` only; `g`'s round 0 is unknown already,
/// and an unknown root is not reported.
#[test]
fn an_unknown_each_round_leaves_its_root_unknown() {
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A", "testUnknown": "^u" } } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, u, b);\ng(u, a);\n"),
        ["f(a, u, b) A=a"]
    );
}

/// The enumeration of a root ends at a round whose match is unknown, even where the unknown row
/// holds a fresh binding: a constraint unknown at `u` leaves the round's `A = u` row unknown,
/// which is neither reported nor enumerated on to `b`.
#[test]
fn each_stops_at_a_round_whose_match_is_unknown() {
    let ast = json!({
        "rule": { "kind": "call_expression", "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        "constraints": { "A": { "testUnknown": "^u" } },
    });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, u, b);\ng(b, a);\n"),
        ["f(a, u, b) A=a", "g(b, a) A=a", "g(b, a) A=b"]
    );
}

/// Inside a candidate that binds a reported range, every choice point passes over its true
/// candidates, also in a capture-free rule (which reads the trial's binding): the walk up to the
/// call finds only `u`'s unknown, so the round's candidate `a` is unknown and ends the search
/// before `u` or `b`. The same capture-free rule as a relation's rule, as a shared utility and as
/// a capture-free recursive utility, none of which may be read from a stage matched without
/// exclusion. A kind-only `any` passes over its true branch too: false, it decides the candidate
/// false though an earlier term was unknown, and the search walks on to `u`.
#[test]
fn a_capture_free_rule_inside_an_excluded_candidate_passes_over_its_true_candidates() {
    let call = json!({ "kind": "call_expression", "has": { "stopBy": "end", "kind": "identifier", "testUnknown": "^u" } });
    let mut up = call.clone();
    up["stopBy"] = json!("end");
    let candidate = |term: (&str, Value)| {
        let mut rule = json!({ "kind": "identifier", "pattern": "$A" });
        rule[term.0] = term.1;
        json!({ "kind": "call_expression", "has": { "field": "arguments", "has": rule } })
    };
    let cases = [
        json!({ "rule": candidate(("inside", up.clone())) }),
        json!({
            "rule": candidate(("matches", json!("up"))),
            "utils": { "up": { "inside": { "stopBy": "end", "matches": "call" } }, "call": call.clone() },
        }),
        json!({
            "rule": candidate(("matches", json!("up"))),
            "utils": { "up": { "any": [call.clone(), { "inside": { "matches": "up" } }] } },
        }),
    ];
    for ast in cases {
        assert_eq!(
            agreed(
                "typescript",
                ast.clone(),
                Some("A"),
                "f(a, u, b);\ng(a, b);\n"
            ),
            ["f(a, u, b) A=a", "g(a, b) A=a", "g(a, b) A=b"],
            "{ast}"
        );
    }
    let kinds = json!({ "rule": { "kind": "call_expression", "has": { "field": "arguments", "has": { "all": [
        { "kind": "identifier", "pattern": "$A" },
        { "inside": up },
        { "any": [{ "kind": "identifier" }, { "kind": "number" }] },
    ] } } } });
    assert_eq!(
        agreed("typescript", kinds, Some("A"), "f(a, u, b);\n"),
        ["f(a, u, b) A=a", "f(a, u, b) A=b", "f(a, u, b) A=u"]
    );
}

/// Under `each`, a three-valued `nthChild` whose binding `ofRule` may be unknown is refused by
/// name: the matcher's excluding re-match of `ofRule` can leave `UNKNOWN` set on a true answer.
#[test]
fn three_valued_nth_child_with_an_unknowable_commit_is_refused_under_each() {
    let ast = json!({ "rule": {
        "kind": "identifier",
        "nthChild": { "position": 1, "ofRule": { "kind": "identifier", "pattern": "$A", "testUnknown": "^u" } },
    } });
    let compiled = compile("typescript", ast, Some("A"));
    let input = snapshot(&[("typescript", "a.ts", "f(a, u);\n")]);
    let reason = compiled
        .evaluate(
            input.to_string(),
            "report".to_owned(),
            Some("sql".to_owned()),
        )
        .expect_err("the SQL engine refuses")
        .reason;
    assert!(
        reason.contains("Unsupported(three-valued nthChild whose excluding commit may be unknown)"),
        "{reason}"
    );
}

#[test]
fn each_enumerates_every_range_a_relation_binds() {
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } },
    } });
    let source = "f(a, b, a);\ng();\n";
    assert_eq!(
        agreed("typescript", ast, Some("A"), source),
        ["f(a, b, a) A=a", "f(a, b, a) A=a", "f(a, b, a) A=b"]
    );
}

#[test]
fn each_stops_where_the_binding_repeats_or_goes_unbound() {
    // Bound at the root by its pattern: excluding it leaves the same binding, which ends it.
    let ast = json!({ "rule": { "pattern": "$A($$$)" } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a);\n"),
        ["f(a) A=f"]
    );
    // An unbound first match is visited once; a later unbound one ends the enumeration.
    let ast = json!({ "rule": { "kind": "call_expression", "any": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "kind": "call_expression" },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\ng();\n"),
        ["f(a, b) A=a", "f(a, b) A=b", "g()"]
    );
}

#[test]
fn each_follows_the_first_match_order_of_its_rounds() {
    // `matcher-semantics.md` §1F: a round binds the next candidate the first term accepts; where
    // a later term then rejects it, the round fails and the enumeration ends, though a later
    // candidate (`b`) would have passed.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "has": { "field": "arguments", "has": { "kind": "identifier", "regex": "^[ab]$", "pattern": "$A" } } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, c, b);\n"),
        ["f(a, c, b) A=a"]
    );
    // The other captures are those bound with the candidate each round accepts.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "has": { "field": "arguments", "has": { "kind": "number", "pattern": "$B" } } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, 1, b, 2);\n"),
        ["f(a, 1, b, 2) A=a B=1", "f(a, 1, b, 2) A=b B=1"]
    );
}

#[test]
fn each_excludes_nothing_inside_negations_counts_and_unpassed_calls() {
    // The `not` probe binds `$A` freely though `a` is excluded in the second round.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "not": { "has": { "field": "arguments", "has": { "kind": "number" } } } },
        { "count": { "has": { "stopBy": "end", "kind": "identifier", "pattern": "$A" }, "min": 1 } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\n"),
        ["f(a, b) A=a", "f(a, b) A=b"]
    );
    // A call the capture is passed to excludes it under its parameter; one it is not passed to
    // excludes nothing.
    let ast = json!({
        "rule": { "kind": "call_expression", "all": [
            { "matches": { "arg": { "P": "$A" } } },
            { "matches": { "arg": { "P": "$B" } } },
        ] },
        "utils": { "arg": { "params": ["P"], "rule": {
            "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$P" } },
        } } },
    });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\n"),
        ["f(a, b) A=a B=a", "f(a, b) A=b B=a"]
    );
}

#[test]
fn each_excludes_inside_a_recursive_utility() {
    let ast = json!({
        "rule": { "kind": "call_expression", "has": { "field": "arguments", "matches": "leaf" } },
        "utils": { "leaf": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "has": { "matches": "leaf" } },
        ] } },
    });
    let source = "f([a, [b]], c);\n";
    let found = agreed("typescript", ast, Some("A"), source);
    assert!(found.len() >= 2, "{found:?}");
}

#[test]
fn each_over_a_regex_group_reports_one_finding_per_group_match() {
    let ast = json!({ "rule": { "kind": "identifier", "regex": "(?P<PART>[a-z]+)_" } });
    let found = agreed("typescript", ast, Some("PART"), "const ab_cd_ef = 1;\n");
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn the_capacity_limits_are_far_above_real_patterns_and_calls() {
    // A pattern with 40 captures and a call mapping six parameters to one capture.
    let names: Vec<String> = (0..40).map(|index| format!("$V{index}")).collect();
    let ast = json!({ "rule": { "pattern": format!("f({})", names.join(", ")) } });
    let arguments: Vec<String> = (0..40).map(|index| format!("a{index}")).collect();
    let source = format!("f({});\n", arguments.join(", "));
    assert_eq!(agreed("typescript", ast, None, &source).len(), 1);
    let params: Vec<String> = (0..6).map(|index| format!("P{index}")).collect();
    let pattern = format!(
        "$P0({})",
        params[1..]
            .iter()
            .map(|param| format!("${param}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mapping: serde_json::Map<String, Value> = params
        .iter()
        .map(|param| (param.clone(), json!("$X")))
        .collect();
    let ast = json!({
        "rule": { "matches": { "all": mapping } },
        "utils": { "all": { "params": params, "rule": { "pattern": pattern } } },
    });
    assert_eq!(
        agreed(
            "typescript",
            ast,
            None,
            "x(x, x, x, x, x);\nx(x, y, x, x, x);\n"
        ),
        ["x(x, x, x, x, x) X=x"]
    );
}

#[test]
fn a_recursion_level_runs_a_bounded_number_of_statements_per_batch() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    // A capture-free recursion and one that binds a capture: both are frame loops over the keys
    // their references demand.
    let rules = [
        json!({
            "rule": { "kind": "call_expression", "matches": "tail" },
            "utils": { "tail": { "any": [
                { "kind": "identifier", "regex": "^x$" },
                { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
                { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
            ] } },
        }),
        json!({
            "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "base" } },
            "utils": { "base": { "any": [
                { "kind": "identifier", "pattern": "$A" },
                { "kind": "member_expression", "has": { "field": "object", "matches": "base" } },
            ] } },
        }),
    ];
    let mut passes = Vec::new();
    for rule in rules {
        let rule = compiled(rule);
        let grammars = vec![&grammar_matchers(&rule)[0]];
        let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
        passes.extend(program.loop_statements());
    }
    eprintln!("statements per recursion level: {passes:?}");
    // Each unit's tables include the dirty-key index (`emit::unit_tables`), one statement more
    // per demanding site inside the loop.
    assert_eq!(passes, [65, 44]);
}

/// Every table a statement creates has one column list: a three-valued `all`'s frames are not
/// named like a recursive unit's answer tables (`a{unit}`). The differential found the clash in
/// this rule, whose `all` frame `a3` and unit 3's answer table were one table.
#[test]
fn three_valued_frames_and_unit_tables_have_distinct_names() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let ast = json!({
        "rule": {
            "any": [
                { "count": { "as": "N", "has": { "kind": "elif_clause", "stopBy": "neighbor" }, "max": 1, "offset": 1 }, "matches": { "u0": { "P": "$C", "Q": "$C" } } },
                { "kind": "attribute", "pattern": "$A == 1.5" },
                { "count": { "max": 4, "min": 2, "precedes": { "stopBy": { "any": [{ "kind": "named_expression" }, { "kind": "pattern_list" }] }, "value": { "type": "str" } } } },
            ],
            "count": { "as": "N", "inside": { "kind": "none", "stopBy": "end" }, "min": 1, "offset": 0 },
            "matches": "r0",
            "pattern": "return 2",
        },
        "utils": {
            "r0": { "all": [
                { "inside": { "field": "subject", "pattern": "$A", "stopBy": { "kind": "for_in_clause" }, "testUnknown": "^doubled" } },
                { "follows": { "matches": "r1" } },
            ] },
            "r1": { "any": [
                { "precedes": { "kind": "class_pattern", "stopBy": "end", "testUnknown": "^6" } },
                { "kind": "escape_sequence", "matches": "r0" },
            ] },
            "u0": { "params": ["P", "Q"], "rule": {
                "matches": "r0",
                "not": { "layout": { "before": { "blank": { "max": 0 }, "breaks": { "min": 1 }, "comments": true, "to": "token" } }, "regex": "^1" },
                "pattern": "$P",
            } },
        },
    });
    let compiled = compile("python", ast.clone(), None);
    let grammar = &grammar_matchers(&compiled)[0];
    assert!(grammar.three_valued);
    let statement = crate::code_rule_dsl::sql::lower::ProgramBuilder::default()
        .clause_each(grammar, None)
        .expect("lowers");
    let mut tables = std::collections::HashMap::<String, String>::new();
    for sql in crate::code_rule_dsl::sql::emit::clause(&statement).statements {
        let Some(rest) = sql
            .strip_prefix("CREATE TEMP TABLE IF NOT EXISTS ")
            .or_else(|| sql.strip_prefix("CREATE TEMP TABLE "))
        else {
            continue;
        };
        let (name, columns) = rest.split_once('(').expect("a column list");
        let known = tables
            .entry(name.to_owned())
            .or_insert_with(|| columns.to_owned());
        assert_eq!(known, columns, "{name} is created with two column lists");
    }
    assert_eq!(
        agreed("python", ast, None, "def f(x):\n    return 2\n"),
        Vec::<String>::new()
    );
}
