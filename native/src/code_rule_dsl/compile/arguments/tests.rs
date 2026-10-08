use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::*;
use napi::Result;
use serde_json::{Value, json};

fn compile(language: &str, ast: Value) -> Result<CompiledCodeRuleDsl> {
    let mut ast = ast;
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation() }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

/// Each violation as `line:text` (1-based lines), with its single-node captures.
fn found(language: &str, ast: Value, source: &str) -> Vec<String> {
    let compiled = compile(language, ast).unwrap_or_else(|error| panic!("{}", error.reason));
    let path = if language == "python" { "a.py" } else { "a.ts" };
    let result = evaluate_file(&compiled, language, path, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let lines = matched_texts(&result, source);
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .zip(lines)
        .map(|(violation, text)| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            format!("{}:{text}", source[..start].matches('\n').count() + 1)
        })
        .collect()
}

#[test]
fn absent_holds_where_no_argument_can_fill_the_slot() {
    // Bandit B113: an HTTP call without a timeout.
    let rule = json!({ "rule": { "pattern": "requests.get($$$)",
        "arguments": { "signature": ["url", "params", "*", "timeout"], "absent": ["timeout"] } } });
    let source = "\
requests.get(url)
requests.get(url, timeout=5)
requests.get(url, None, timeout=None)
requests.get(url, **kw)
requests.get(url, **{'timeout': 5})
requests.get(url, **{'verify': False})
requests.get(*args)
requests.get(url, timeout)
requests.get(url, # comment
             timeout=1)
";
    assert_eq!(
        found("python", rule, source),
        [
            "1:requests.get(url)",
            "6:requests.get(url, **{'verify': False})",
            "7:requests.get(*args)",
            "8:requests.get(url, timeout)"
        ]
    );
    // Negated, an unknown slot is still unknown: `**kw` may or may not pass it.
    let passed = json!({ "rule": { "pattern": "requests.get($$$)",
        "not": { "arguments": { "signature": ["url", "params", "*", "timeout"], "absent": ["timeout"] } } } });
    assert_eq!(
        found("python", passed, source),
        [
            "2:requests.get(url, timeout=5)",
            "3:requests.get(url, None, timeout=None)",
            "5:requests.get(url, **{'timeout': 5})",
            "9:requests.get(url, # comment\n             timeout=1)"
        ]
    );
}

#[test]
fn bind_captures_the_argument_in_a_slot_by_position_or_keyword() {
    // Bandit B610: Django's QuerySet.extra(select, where, params, tables, order_by, select_params).
    let rule = json!({
        "rule": { "pattern": "$Q.extra($$$)",
            "arguments": { "signature": ["select", "where", "params", "tables", "order_by", "select_params"], "bind": { "W": "where" } } },
        "constraints": { "W": { "not": { "kind": "list" } } },
    });
    let source = "\
qs.extra(where=['a'])
qs.extra(None, 'raw')
qs.extra(select={'a': b}, where=x)
qs.extra(*args)
qs.extra()
qs.extra(None, ['a'], where='b')
qs.extra(**{'where': w})
";
    assert_eq!(
        found("python", rule, source),
        [
            "2:qs.extra(None, 'raw') Q=qs W='raw'",
            "3:qs.extra(select={'a': b}, where=x) Q=qs W=x",
            "7:qs.extra(**{'where': w}) Q=qs W=w"
        ]
    );
}

#[test]
fn positional_only_and_keyword_only_slots() {
    let rule = |arguments: Value| json!({ "rule": { "kind": "call", "arguments": arguments } });
    let source = "f(1, 2)\nf(a=1, b=2)\nf(1, b=2, c=3)\nf(x for x in y)\n";
    assert_eq!(
        found(
            "python",
            rule(json!({ "signature": ["a", "/", "b", "*", "c"], "bind": { "A": "a" } })),
            source
        ),
        [
            "1:f(1, 2) A=1",
            "3:f(1, b=2, c=3) A=1",
            "4:f(x for x in y) A=(x for x in y)"
        ]
    );
    assert_eq!(
        found(
            "python",
            rule(json!({ "signature": ["a", "/", "b", "*", "c"], "absent": ["a"] })),
            source
        ),
        ["2:f(a=1, b=2)"]
    );
    assert_eq!(
        found(
            "python",
            rule(json!({ "signature": ["a", "/", "b", "*", "c"], "bind": { "C": "c", "B": "b" } })),
            source
        ),
        ["3:f(1, b=2, c=3) B=2 C=3"]
    );
    // A slot passed twice is a TypeError: unknown, so neither bound nor absent.
    let twice = "f(1, a=2)\n";
    assert!(
        found(
            "python",
            rule(json!({ "signature": ["a"], "bind": { "A": "a" } })),
            twice
        )
        .is_empty()
    );
    assert!(
        found(
            "python",
            rule(json!({ "signature": ["a"], "absent": ["a"] })),
            twice
        )
        .is_empty()
    );
}

#[test]
fn typescript_arguments_are_positional() {
    let rule = |arguments: Value| json!({ "rule": { "any": [{ "kind": "call_expression" }, { "kind": "new_expression" }], "arguments": arguments } });
    let source = "setTimeout(code, 10);\nsetTimeout(...args);\nsetTimeout(code, ...rest);\nnew Foo;\nnew Foo(a);\ntag`x${a}`;\n";
    assert_eq!(
        found(
            "typescript",
            rule(json!({ "signature": ["handler", "timeout"], "bind": { "H": "handler" } })),
            source
        ),
        [
            "1:setTimeout(code, 10) H=code",
            "3:setTimeout(code, ...rest) H=code",
            "5:new Foo(a) H=a"
        ]
    );
    assert_eq!(
        found(
            "typescript",
            rule(json!({ "signature": ["handler", "timeout"], "absent": ["timeout"] })),
            source
        ),
        ["4:new Foo", "5:new Foo(a)"]
    );
}

#[test]
fn a_non_call_has_no_arguments() {
    let rule = json!({ "rule": { "kind": "identifier", "not": { "arguments": { "signature": ["a"], "absent": ["a"] } } } });
    assert_eq!(found("python", rule, "x\n"), ["1:x"]);
}

/// `mode: "syntactic"` is Ruff's `find_argument` / `find_keyword`: a keyword by its literal name
/// first, else the positional argument at the slot's position before any starred one. Nothing is
/// unknown: `**kw` and `**{...}` pass nothing, a starred argument ends the positions.
#[test]
fn syntactic_mode_finds_arguments_as_ruff_does() {
    let seconds = json!({ "rule": { "kind": "call", "regex": "^trio\\.sleep\\(",
        "arguments": { "signature": ["seconds"], "bind": { "S": "seconds" }, "mode": "syntactic" } } });
    let source = "\
trio.sleep(0)
trio.sleep(seconds=0)
trio.sleep(**{\"seconds\": 0})
trio.sleep(0, **kw)
trio.sleep(*a)
trio.sleep(*a, 0)
trio.sleep(0, seconds=1)
trio.sleep(x for x in y)
trio.sleep(**{\"seconds\": 0, **k})
trio.sleep((0))
";
    assert_eq!(
        found("python", seconds, source),
        [
            "1:trio.sleep(0) S=0",
            "2:trio.sleep(seconds=0) S=0",
            "4:trio.sleep(0, **kw) S=0",
            "7:trio.sleep(0, seconds=1) S=1",
            "8:trio.sleep(x for x in y) S=(x for x in y)",
            "10:trio.sleep((0)) S=(0)",
        ]
    );
    let strict = |mode: &str| {
        json!({ "rule": { "pattern": "zip($$$)",
            "arguments": { "signature": ["*", "strict"], "absent": ["strict"], "mode": mode } } })
    };
    let source = "\
zip(a, b)
zip(a, b, strict=True)
zip(a, b, **kw)
zip(a, b, **{\"strict\": True})
zip(a, b, **{})
";
    assert_eq!(
        found("python", strict("syntactic"), source),
        [
            "1:zip(a, b)",
            "3:zip(a, b, **kw)",
            "4:zip(a, b, **{\"strict\": True})",
            "5:zip(a, b, **{})",
        ]
    );
    // The default binds as Python does: `**kw` may pass `strict`, `**{...}` does.
    assert_eq!(
        found("python", strict("language"), source),
        ["1:zip(a, b)", "5:zip(a, b, **{})"]
    );
    let slot = |slot: &str| {
        json!({ "rule": { "pattern": "g($$$)",
            "arguments": { "signature": ["a", "b", "/", "c", "*", "d"], "bind": { "X": slot }, "mode": "syntactic" } } })
    };
    let source = "\
g(1, 2, 3)
g(1, 2, c=3)
g(1, 2, *r)
g(*r, c=3)
g(1, 2, 3, c=4)
g(a=1, b=2, c=3)
g(1, 2, d=3)
g(1, *(2, 3))
g(1, 2, 3, 4)
";
    assert_eq!(
        found("python", slot("c"), source),
        [
            "1:g(1, 2, 3) X=3",
            "2:g(1, 2, c=3) X=3",
            "4:g(*r, c=3) X=3",
            "5:g(1, 2, 3, c=4) X=4",
            "6:g(a=1, b=2, c=3) X=3",
            "9:g(1, 2, 3, 4) X=3",
        ]
    );
    // Positional-only slots take no keyword; keyword-only ones no position.
    assert_eq!(
        found("python", slot("a"), source),
        [
            "1:g(1, 2, 3) X=1",
            "2:g(1, 2, c=3) X=1",
            "3:g(1, 2, *r) X=1",
            "5:g(1, 2, 3, c=4) X=1",
            "7:g(1, 2, d=3) X=1",
            "8:g(1, *(2, 3)) X=1",
            "9:g(1, 2, 3, 4) X=1",
        ]
    );
    assert_eq!(found("python", slot("d"), source), ["7:g(1, 2, d=3) X=3"]);
}

#[test]
fn arguments_objects_are_checked() {
    let reject = |language: &str, arguments: Value| {
        rejection(compile(
            language,
            json!({ "rule": { "kind": "call", "arguments": arguments } }),
        ))
    };
    let python = |arguments: Value| reject("python", arguments);
    for (arguments, message) in [
        (
            json!({ "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature is required",
        ),
        (
            json!({ "signature": "a", "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature must be an array of slot names",
        ),
        (
            json!({ "signature": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments must `bind` a slot or list `absent` ones",
        ),
        (
            json!({ "signature": ["a", "a"], "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature/1: `a` is listed twice",
        ),
        (
            json!({ "signature": ["*", "a", "/"], "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature/2: `/` may appear once, before `*`",
        ),
        (
            json!({ "signature": ["a", "*", "*"], "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature/2: `*` may appear once",
        ),
        (
            json!({ "signature": ["a-b"], "absent": ["a-b"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature/0: `a-b` is not a slot name (an identifier), `/` or `*`",
        ),
        (
            json!({ "signature": ["/"], "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/signature must name a slot",
        ),
        (
            json!({ "signature": ["a"], "absent": ["b"] }),
            "/clauses/0/forbid/ast/rule/arguments/absent/0: `b` is not a slot of the signature",
        ),
        (
            json!({ "signature": ["a"], "absent": [] }),
            "/clauses/0/forbid/ast/rule/arguments/absent must not be empty",
        ),
        (
            json!({ "signature": ["a"], "bind": { "$A": "a" } }),
            "/clauses/0/forbid/ast/rule/arguments/bind: `$A` is not a metavariable name (A-Z first, then A-Z, 0-9, _, without `$`)",
        ),
        (
            json!({ "signature": ["a"], "bind": { "A": "a" }, "absent": ["a"] }),
            "/clauses/0/forbid/ast/rule/arguments/absent/0: `a` is bound, so it is passed",
        ),
        (
            json!({ "signature": ["a"], "bind": {} }),
            "/clauses/0/forbid/ast/rule/arguments/bind must bind a capture",
        ),
        (
            json!({ "signature": ["a"], "absent": ["a"], "present": ["a"] }),
            "unknown field `present` at /clauses/0/forbid/ast/rule/arguments",
        ),
        (
            json!({ "signature": ["a"], "absent": ["a"], "mode": "ruff" }),
            "/clauses/0/forbid/ast/rule/arguments/mode must be \"language\", \"syntactic\" or \"estree\"",
        ),
    ] {
        assert_eq!(python(arguments.clone()), message, "{arguments}");
    }
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "call_expression", "arguments": { "signature": ["a", "*", "b"], "absent": ["b"] } } })
        )),
        "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only"
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "call_expression", "arguments": { "signature": ["a"], "absent": ["a"], "mode": "syntactic" } } })
        )),
        "/clauses/0/forbid/ast/rule/arguments/mode: `syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords"
    );
    // A constraint may name a capture only `bind` defines.
    compile(
        "python",
        json!({ "rule": { "kind": "call", "arguments": { "signature": ["a"], "bind": { "A": "a" } } },
                "constraints": { "A": { "kind": "integer" } } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
}

/// `mode: "estree"` is ESLint's `node.arguments[i]`: every argument is a position, a spread
/// included, so nothing is unknown; a tagged template is no call.
#[test]
fn estree_mode_counts_spreads_as_positions() {
    let second = |mode: &str| {
        json!({ "rule": { "kind": "call_expression",
            "arguments": { "signature": ["a", "b"], "bind": { "B": "b" }, "mode": mode } } })
    };
    let source = "f(x, y);\nf(...x, y);\nf(x, ...y);\nf(...x);\nf`t${x}`;\n";
    assert_eq!(
        found("typescript", second("estree"), source),
        ["1:f(x, y) B=y", "2:f(...x, y) B=y", "3:f(x, ...y) B=...y"]
    );
    assert_eq!(
        found("typescript", second("language"), source),
        ["1:f(x, y) B=y"]
    );
    let absent = |mode: &str| {
        json!({ "rule": { "kind": "call_expression",
            "arguments": { "signature": ["a", "b"], "absent": ["b"], "mode": mode } } })
    };
    assert_eq!(found("typescript", absent("estree"), source), ["4:f(...x)"]);
    assert!(found("typescript", absent("language"), source).is_empty());
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "call", "arguments": { "signature": ["a"], "absent": ["a"], "mode": "estree" } } })
        )),
        "/clauses/0/forbid/ast/rule/arguments/mode: `estree` reads ESLint's `node.arguments`; use language or syntactic for Python"
    );
}

/// ESTree has no parenthesized expression, so under `mode: "estree"` `super((a))` passes `a`; a
/// spread stays the spread element (`...(a)` is a SpreadElement of `a`). `language` keeps the
/// parentheses. `compare` `name` reads the captured node itself, so no-useless-constructor's
/// pass-through test (`superArgs[i].name === params[i].name`) holds for `super((a))` only through
/// `estree`, as in ESLint.
#[test]
fn estree_mode_unwraps_parenthesized_arguments() {
    let first = |mode: &str| {
        json!({ "rule": { "kind": "call_expression",
            "arguments": { "signature": ["a"], "bind": { "A": "a" }, "mode": mode } } })
    };
    let source = "f((a));\nf(((b)), c);\nf(...(d));\n";
    assert_eq!(
        found("typescript", first("estree"), source),
        ["1:f((a)) A=a", "2:f(((b)), c) A=b", "3:f(...(d)) A=...(d)"]
    );
    assert_eq!(
        found("typescript", first("language"), source),
        ["1:f((a)) A=(a)", "2:f(((b)), c) A=((b))"]
    );
    let pass_through = |mode: &str| {
        json!({ "rule": { "kind": "method_definition",
            "has": { "kind": "formal_parameters", "stopBy": "end", "has": { "kind": "required_parameter",
                "has": { "kind": "identifier", "pattern": "$P" } } },
            "all": [{ "has": { "stopBy": "end", "kind": "call_expression", "has": { "kind": "super" },
                "arguments": { "signature": ["a"], "bind": { "A": "a" }, "mode": mode },
                "compare": { "left": "A", "op": "eq", "right": "P", "text": "name" } } }] } })
    };
    let source = "class A extends B { constructor(a) { super((a)); } }\n\
                  class C extends B { constructor(a) { super(b); } }\n";
    let estree = found("typescript", pass_through("estree"), source);
    assert_eq!(estree.len(), 1, "{estree:?}");
    assert!(estree[0].starts_with("1:constructor(a)"), "{estree:?}");
    assert!(found("typescript", pass_through("language"), source).is_empty());
}

#[test]
fn arguments_are_sound_under_syntax_errors() {
    let compiled = compile(
        "python",
        json!({ "rule": { "kind": "call", "not": { "arguments": { "signature": ["a"], "absent": ["a"] } } } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    assert!(grammar_matchers(&compiled)[0].sound_under_errors);
    let result = evaluate_file(&compiled, "python", "a.py", "f(1)\ndef (:\n");
    assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
    assert_violations_texts(&result, "f(1)\ndef (:\n", &["f(1)"]);
}

fn assert_violations_texts(result: &Value, source: &str, expected: &[&str]) {
    let texts = matched_texts(result, source);
    assert_eq!(texts, expected, "{result}");
}
