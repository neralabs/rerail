//! `resolves.binding` and `value.follow`: rules over what a name's file-local binding holds.

use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::matcher::UtilLimit;
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

fn python(ast: Value) -> CompiledCodeRuleDsl {
    compile("python", ast).unwrap_or_else(|error| panic!("{}", error.reason))
}

fn typescript(ast: Value) -> CompiledCodeRuleDsl {
    compile("typescript", ast).unwrap_or_else(|error| panic!("{}", error.reason))
}

/// Each violation as `line:text` (1-based lines), with its single-node captures.
fn found(result: &Value, source: &str) -> Vec<String> {
    assert_eq!(result["failures"], json!([]), "{result}");
    let lines = matched_texts(result, source);
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .zip(lines)
        .map(|(violation, text)| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let line = source[..start].matches('\n').count() + 1;
            format!("{line}:{text}")
        })
        .collect()
}

fn python_found(ast: Value, source: &str) -> Vec<String> {
    found(
        &evaluate_file(&python(ast), "python", "a.py", source),
        source,
    )
}

fn typescript_found(ast: Value, source: &str) -> Vec<String> {
    found(
        &evaluate_file(&typescript(ast), "typescript", "a.ts", source),
        source,
    )
}

/// Ruff's SIM910 test for a dict receiver (`is_known_to_be_of_type_dict`): the binding the call
/// sees is a dict display or comprehension, a `dict` annotation, or a `**kwargs` parameter.
fn dict_receiver() -> Value {
    json!({ "any": [
        { "resolves": { "binding": { "value": { "any": [{ "kind": "dictionary" }, { "kind": "dictionary_comprehension" }] } } } },
        { "resolves": { "binding": { "annotation": { "regex": "^(dict|Dict)\\b" } } } },
        { "resolves": { "binding": { "site": { "any": [
            { "kind": "dictionary_splat_pattern" },
            { "kind": "typed_parameter", "has": { "kind": "dictionary_splat_pattern" } },
        ] } } } },
    ] })
}

#[test]
fn binding_sub_rules_match_the_site_value_and_annotation_the_read_sees() {
    let rule = json!({
        "rule": { "pattern": "$D.get($K, None)" },
        "constraints": { "D": dict_receiver() },
    });
    let source = "\
ages = {\"Tom\": 23}
ages.get(\"Cat\", None)
ages = [\"Tom\"]
ages.get(\"Cat\", None)
def f(**kwargs):
    kwargs.get(\"a\", None)
def g(some_dict: dict, other: object, plain, *args: int, **typed: str):
    some_dict.get(\"a\", None)
    other.get(\"a\", None)
    plain.get(\"a\", None)
    typed.get(\"a\", None)
unbound.get(\"a\", None)
a = b = {}
b.get(1, None)
(p) = {}
p.get(1, None)
q, r = {}, {}
q.get(1, None)
";
    assert_eq!(
        python_found(rule, source),
        [
            "2:ages.get(\"Cat\", None) D=ages K=\"Cat\"",
            "6:kwargs.get(\"a\", None) D=kwargs K=\"a\"",
            "8:some_dict.get(\"a\", None) D=some_dict K=\"a\"",
            "11:typed.get(\"a\", None) D=typed K=\"a\"",
            "14:b.get(1, None) D=b K=1",
            "16:p.get(1, None) D=p K=1",
            "18:q.get(1, None) D=q K=1",
        ]
    );
}

#[test]
fn binding_sub_rules_bind_captures() {
    let rule = json!({
        "rule": { "pattern": "$N.open()" },
        "constraints": { "N": { "resolves": { "binding": { "value": { "pattern": "$F($$$)" } } } } },
    });
    let source = "p = Path('x')\np.open()\nq = 'x'\nq.open()\n";
    assert_eq!(
        python_found(rule.clone(), source),
        ["2:p.open() F=Path N=p"]
    );
    // A later term compares what the binding bound.
    let rule = json!({
        "rule": { "pattern": "$N.open()" },
        "constraints": { "N": { "resolves": { "binding": { "value": { "pattern": "$F($$$)" } } },
                                "compare": { "left": "F", "op": "eq", "right": "N" } } },
    });
    assert_eq!(
        python_found(rule, "Path = Path('x')\nPath.open()\n"),
        ["2:Path.open() F=Path N=Path"]
    );
}

#[test]
fn aliases_resolve_through_a_recursive_util() {
    // ESLint no-obj-calls: calls through a variable initialized to the global.
    let rule = json!({
        "rule": { "pattern": "$F()" },
        "constraints": { "F": { "matches": "json" } },
        "utils": { "json": { "any": [
            { "kind": "identifier", "resolves": { "global": ["JSON"] } },
            { "kind": "parenthesized_expression", "has": { "matches": "json" } },
            { "kind": "identifier", "resolves": { "binding": { "value": { "matches": "json" } } } },
        ] } },
    });
    let source = "\
var foo = JSON; function f(foo) { foo(); }
var a = JSON, b = a; b();
var c = c; c();
var d = e, e = d; d();
let g = JSON; g = Math; g();
const h = (JSON); h();
JSON();
";
    assert_eq!(
        typescript_found(rule, source),
        ["2:b() F=b", "6:h() F=h", "7:JSON() F=JSON"]
    );
}

#[test]
fn a_binding_needs_a_file_local_binding_and_each_listed_node() {
    let rule = |binding: Value| {
        json!({ "rule": { "kind": "identifier", "regex": "^[a-z]$", "inside": { "kind": "expression_statement" },
                          "resolves": { "binding": binding } } })
    };
    let source = "x = 1\ny: int = 2\nfor z in []: pass\nx\ny\nz\nw\n";
    assert_eq!(python_found(rule(json!({})), source), ["4:x", "5:y", "6:z"]);
    assert_eq!(
        python_found(rule(json!({ "value": { "kind": "integer" } })), source),
        ["4:x", "5:y"]
    );
    assert_eq!(
        python_found(
            rule(json!({ "annotation": { "kind": "identifier" } })),
            source
        ),
        ["5:y"]
    );
    assert_eq!(
        python_found(rule(json!({ "site": { "kind": "for_statement" } })), source),
        ["6:z"]
    );
    // Negated: every name that is no such reference, a global included.
    assert_eq!(
        python_found(
            json!({ "rule": { "kind": "identifier", "regex": "^[a-z]$", "inside": { "kind": "expression_statement" },
                              "not": { "resolves": { "binding": { "value": { "kind": "integer" } } } } } }),
            source
        ),
        ["6:z", "7:w"]
    );
}

#[test]
fn follow_reads_values_through_bindings() {
    let rule = json!({
        "rule": { "pattern": "os.chmod($P, $M)" },
        "constraints": { "M": { "value": { "bitsAny": 2, "follow": true } } },
    });
    let source = "\
mode = 0o777
os.chmod(p, mode)
alias = mode
os.chmod(p, alias)
later = 0o700
def f():
    os.chmod(p, later)
later = 0o722
os.chmod(p, unknown)
x: int = 0o002
os.chmod(p, x)
y, z = 0o777, 0o777
os.chmod(p, y)
v, w = 0o777, 0o777, 0o777
os.chmod(p, v)
(s, *t), u = (0o777, 0o700), 0o777
os.chmod(p, s)
";
    assert_eq!(
        python_found(rule.clone(), source),
        [
            "2:os.chmod(p, mode) M=mode P=p",
            "4:os.chmod(p, alias) M=alias P=p",
            "7:os.chmod(p, later) M=later P=p",
            "11:os.chmod(p, x) M=x P=p",
            "13:os.chmod(p, y) M=y P=p",
            "17:os.chmod(p, s) M=s P=p",
        ]
    );
    // Without `follow` a name has no value.
    let rule = json!({
        "rule": { "pattern": "os.chmod($P, $M)" },
        "constraints": { "M": { "value": { "bitsAny": 2 } } },
    });
    assert_eq!(python_found(rule, source), Vec::<String>::new());
}

#[test]
fn follow_stops_after_sixteen_hops_and_at_cycles() {
    let rule = json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "value": { "eq": 1, "follow": true } } } });
    let chain = |length: usize| {
        let mut source = "n0 = 1\n".to_owned();
        for index in 1..length {
            source.push_str(&format!("n{index} = n{}\n", index - 1));
        }
        source.push_str(&format!("f(n{})\n", length - 1));
        source
    };
    assert_eq!(python_found(rule.clone(), &chain(16)).len(), 1);
    assert_eq!(python_found(rule.clone(), &chain(17)).len(), 0);
    // Unknown past the limit, so a negation does not hold either.
    let negated = json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "not": { "value": { "eq": 1, "follow": true } } } } });
    assert_eq!(python_found(negated, &chain(17)).len(), 0);
    let rule = json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "value": { "constant": false, "follow": true } } } });
    assert_eq!(
        typescript_found(rule, "var c = c; f(c);\nvar a = b, b = a; f(a);\n"),
        ["1:f(c) A=c", "2:f(a) A=a"]
    );
}

#[test]
fn follow_needs_an_effectively_constant_typescript_binding() {
    // ESLint no-implied-eval: a string argument, through `const`.
    let rule = json!({
        "rule": { "pattern": "setTimeout($S)" },
        "constraints": { "S": { "value": { "type": "string", "follow": true } } },
    });
    let source = "\
const s = 'alert(1)'; setTimeout(s);
let t = 'a'; t = 'b'; setTimeout(t);
let u = 'a' + 1; setTimeout(u);
var v = `x${s}`; setTimeout(v);
function f(p = 'a') { setTimeout(p); }
setTimeout(undefined);
";
    assert_eq!(
        typescript_found(rule, source),
        [
            "1:setTimeout(s) S=s",
            "3:setTimeout(u) S=u",
            "4:setTimeout(v) S=v"
        ]
    );
    // A mutated array or object has no value (`hasMutationInProperty`); a primitive keeps it.
    let rule = json!({
        "rule": { "pattern": "use($A)" },
        "constraints": { "A": { "value": { "type": ["array", "object", "number"], "follow": true } } },
    });
    let source = "\
const a = [1]; use(a);
const b = [1]; b.push(2); use(b);
const c = {}; c.x = 1; use(c);
const d = 1; use(d);
";
    assert_eq!(
        typescript_found(rule, source),
        ["1:use(a) A=a", "4:use(d) A=d"]
    );
}

/// A name a declaration destructures from an object or array literal carries the element it takes:
/// `resolves.binding.value` reads it. `value.follow` does not, as getStaticValue reads no
/// destructured binding.
#[test]
fn destructured_typescript_names_carry_the_literal_element_they_take() {
    let rule = json!({
        "rule": { "pattern": "use($A)" },
        "constraints": { "A": { "resolves": { "binding": { "value": { "pattern": "$V" } } } } },
    });
    let source = "\
const {a} = {a: 1}; use(a);
const [x] = [2]; use(x);
const {b: {c}, d: [, e]} = {b: {c: 3}, d: [4, 5]}; use(c); use(e);
const {f} = {f: 6, ...o}; use(f);
const {g} = {...o, g: 7}; use(g);
const {h = 8} = {h: 9}; use(h);
const [i, ...j] = [10, 11]; use(i); use(j);
const [k] = [...o, 12]; use(k);
const {'l': l, [`m`]: m} = ({l: 13, 'm': 14}); use(l); use(m);
const {n} = {[key]: 15}; use(n);
const {p} = {p: 16, p: 17}; use(p);
const {q} = {get q() { return 18; }}; use(q);
let {r} = {r: 19}; r = 20; use(r);
const [s] = [, 21]; use(s);
";
    assert_eq!(
        typescript_found(rule, source),
        [
            "1:use(a) A=a V=1",
            "2:use(x) A=x V=2",
            "3:use(c) A=c V=3",
            "3:use(e) A=e V=5",
            "5:use(g) A=g V=7",
            "7:use(i) A=i V=10",
            "9:use(l) A=l V=13",
            "9:use(m) A=m V=14",
            "11:use(p) A=p V=17",
        ]
    );
    let follow = json!({
        "rule": { "pattern": "use($A)" },
        "constraints": { "A": { "value": { "type": "number", "follow": true } } },
    });
    assert_eq!(
        typescript_found(
            follow,
            "const {a} = {a: 1}; use(a);\nconst [x] = [2]; use(x);\nconst y = 3; use(y);\n"
        ),
        ["3:use(y) A=y"]
    );
}

/// A hole or a defaulted target stands at a position of the initializer like any other: where
/// the initializer has a spread there, no later name takes an element, as their positions are
/// past the spread (`const [, b] = [...xs, 5]` gives `b` `xs[0]`, not `5`).
#[test]
fn array_holes_and_defaults_take_no_element_past_an_initializer_spread() {
    let rule = json!({
        "rule": { "pattern": "use($A)" },
        "constraints": { "A": { "resolves": { "binding": { "value": { "pattern": "$V" } } } } },
    });
    let source = "\
const [, b] = [...xs, 5]; use(b);
const [c = 1, d] = [...ys, 6]; use(d);
const [e, , f] = [7, ...zs, 8]; use(e); use(f);
const [, g] = [, 9]; use(g);
";
    assert_eq!(
        typescript_found(rule, source),
        ["3:use(e) A=e V=7", "4:use(g) A=g V=9"]
    );
}

/// `destructuredValue: false` gives a destructured name no value, as ESLint's `ReferenceTracker`
/// and `getStaticValue` read none (no-obj-calls: `const {j} = {j: JSON}; j()` is valid), on the
/// reference and the `write: "last"` paths; a plain declarator keeps its initializer.
#[test]
fn destructured_value_false_gives_destructured_names_no_value() {
    let rule = |binding: Value| {
        json!({
            "rule": { "pattern": "$A()" },
            "constraints": { "A": { "resolves": { "binding": binding } } },
        })
    };
    let source = "const {j} = {j: JSON}; j();\nconst [k] = [Math]; k();\nconst m = Math; m();\n";
    let value = json!({ "kind": "identifier", "pattern": "$V" });
    assert_eq!(
        typescript_found(rule(json!({ "value": value })), source),
        ["1:j() A=j V=JSON", "2:k() A=k V=Math", "3:m() A=m V=Math"]
    );
    assert_eq!(
        typescript_found(
            rule(json!({ "destructuredValue": false, "value": value })),
            source
        ),
        ["3:m() A=m V=Math"]
    );
    // The binding still resolves: only its value is gone.
    assert_eq!(
        typescript_found(
            rule(json!({ "destructuredValue": false, "site": { "kind": "variable_declarator" } })),
            source
        ),
        ["1:j() A=j", "2:k() A=k", "3:m() A=m"]
    );
    let last = json!({ "rule": { "kind": "identifier", "resolves": { "write": "last", "binding": {
        "destructuredValue": false, "value": { "pattern": "$V" } } } } });
    assert_eq!(
        typescript_found(last, "const {a} = {a: 1};\nconst b = 2;\n"),
        ["2:b V=2"]
    );
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "destructuredValue": false } } } }),
        )),
        "/clauses/0/forbid/ast/rule/resolves/binding/destructuredValue: destructured values are TypeScript's; a Python unpacked name keeps its element"
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "destructuredValue": 0 } } } }),
        )),
        "/clauses/0/forbid/ast/rule/resolves/binding/destructuredValue must be a boolean"
    );
}

#[test]
fn followed_python_containers_keep_only_their_kind() {
    let rule = |test: Value| json!({ "rule": { "pattern": "use($A)" }, "constraints": { "A": { "value": test } } });
    let source = "x = []\nx.append(1)\nuse(x)\nt = ()\nuse(t)\n";
    assert_eq!(
        python_found(rule(json!({ "type": "list", "follow": true })), source),
        ["3:use(x) A=x"]
    );
    assert_eq!(
        python_found(rule(json!({ "truthy": false, "follow": true })), source),
        ["5:use(t) A=t"]
    );
}

#[test]
fn binding_rules_are_source_incomplete_in_files_with_syntax_errors() {
    for ast in [
        json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "value": { "kind": "integer" } } } } }),
        json!({ "rule": { "kind": "identifier", "value": { "eq": 1, "follow": true } } }),
    ] {
        let rule = python(ast);
        assert!(!grammar_matchers(&rule)[0].sound_under_errors);
        let result = evaluate_file(&rule, "python", "a.py", "x = 1\nx\ndef (:\n");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }
    let plain = python(json!({ "rule": { "kind": "integer", "value": { "eq": 1 } } }));
    assert!(grammar_matchers(&plain)[0].sound_under_errors);
}

#[test]
fn binding_recursion_spends_the_step_budget() {
    let document = clause_document(json!({
        "id": "ast-match",
        "forbid": { "ast": {
            "language": "typescript",
            "rule": { "kind": "identifier", "matches": "alias", "inside": { "kind": "arguments" } },
            "utils": { "alias": { "any": [
                { "kind": "identifier", "resolves": { "global": ["JSON"] } },
                { "kind": "identifier", "resolves": { "binding": { "value": { "matches": "alias" } } } },
            ] } },
        } },
        "violation": violation(),
    }));
    let chain = |length: usize, calls: usize| {
        let mut source = "const n0 = JSON;\n".to_owned();
        for index in 1..length {
            source.push_str(&format!("const n{index} = n{};\n", index - 1));
        }
        for _ in 0..calls {
            source.push_str(&format!("f(n{});\n", length - 1));
        }
        source
    };
    let failure = |limit: UtilLimit| json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": limit.message(), "path": "a.ts" }]);
    let rule = compile_document_with(
        document.clone(),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    assert_violations(
        &evaluate_file(&rule, "typescript", "a.ts", &chain(30, 1)),
        1,
    );
    // Each hop is a utility call: a chain past the depth limit fails the clause, never reads false.
    let result = evaluate_file(&rule, "typescript", "a.ts", &chain(300, 1));
    assert_eq!(result["failures"], failure(UtilLimit::Depth), "{result}");
    let capabilities =
        capabilities_with(|capabilities| capabilities["limits"]["maxVisitedNodes"] = json!(3_000));
    let rule = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&rule, "typescript", "a.ts", &chain(30, 200));
    assert_eq!(result["failures"], failure(UtilLimit::Work), "{result}");
}

#[test]
fn binding_objects_are_checked() {
    let reject = |resolves: Value| {
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": resolves } }),
        ))
    };
    assert_eq!(
        reject(json!({ "binding": { "values": {} } })),
        "unknown field `values` at /clauses/0/forbid/ast/rule/resolves/binding"
    );
    assert_eq!(
        reject(json!({ "binding": [] })),
        "/clauses/0/forbid/ast/rule/resolves/binding must be an object"
    );
    assert_eq!(
        reject(json!({ "binding": {}, "global": ["x"] })),
        "/clauses/0/forbid/ast/rule/resolves/global: `binding` asks about the name's own file-local binding; write other `resolves` terms in a resolves of their own"
    );
    assert_eq!(
        reject(json!({ "binding": { "value": { "kinds": "integer" } } })),
        "unknown field `kinds` at /clauses/0/forbid/ast/rule/resolves/binding/value"
    );
    let reason = reject(json!({ "binding": { "value": { "kind": "no_such_kind" } } }));
    assert!(
        reason.starts_with("/clauses/0/forbid/ast/rule/resolves/binding/value/kind: "),
        "{reason}"
    );
    // Constraints may name captures only a binding's sub-rule binds.
    python(json!({
        "rule": { "kind": "identifier", "resolves": { "binding": { "value": { "pattern": "$F($$$)" } } } },
        "constraints": { "F": { "kind": "identifier" } },
    }));
}

#[test]
fn follow_carries_big_ints_and_named_escapes_through_aliases() {
    let rule = |test: Value| json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "value": test } } });
    // Python ints to 4,096 bits and `\N{...}` escapes decode at the binding and carry through
    // every hop; past 4,096 bits, or with a name CPython does not know, the value stays unknown.
    let source = "\
big = 2 ** 200
alias = big
f(alias)
huge = 2 ** 4096
f(huge)
s = '\\N{LATIN SMALL LETTER A}'
t = s
f(t)
bad = '\\N{NOT A NAME}'
f(bad)
";
    assert_eq!(
        python_found(rule(json!({ "gt": 1e60, "follow": true })), source),
        ["3:f(alias) A=alias"]
    );
    assert_eq!(
        python_found(rule(json!({ "eq": "a", "follow": true })), source),
        ["8:f(t) A=t"]
    );
    let negated = |test: Value| json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "not": { "value": test } } } });
    assert_eq!(
        python_found(
            negated(json!({ "type": ["int", "str"], "follow": true })),
            source
        ),
        Vec::<String>::new()
    );
    // TypeScript BigInts to 128 bits carry through a `const` alias; wider ones stay unknown.
    let source = "const b = 1n << 100n; const c = b; f(c);\nconst w = 1n << 200n; f(w);\n";
    assert_eq!(
        typescript_found(rule(json!({ "type": "bigint", "follow": true })), source),
        ["1:f(c) A=c"]
    );
    assert_eq!(
        typescript_found(negated(json!({ "type": "bigint", "follow": true })), source),
        Vec::<String>::new()
    );
}

#[test]
fn all_goes_on_past_an_unknown_followed_value_binding_or_argument_that_binds_nothing() {
    // Each first term is unknown on `f(n, *a)` (`n` has no known value; `*a` may fill `y`) and
    // binds nothing, so the false second term decides and the negation holds.
    let unknown_then_false = |first: Value| json!({ "rule": { "kind": "call", "pattern": "f($$$)", "not": { "all": [first, { "regex": "^never" }] } } });
    let source = "n = g()\nf(n, *a)\n";
    for first in [
        json!({ "has": { "stopBy": "end", "kind": "identifier", "regex": "^n$", "value": { "eq": 1, "follow": true } } }),
        json!({ "has": { "stopBy": "end", "kind": "identifier", "regex": "^n$",
                         "resolves": { "binding": { "value": { "value": { "eq": 1 } } } } } }),
        json!({ "arguments": { "signature": ["x", "y"], "absent": ["y"] } }),
    ] {
        assert_eq!(
            python_found(unknown_then_false(first.clone()), source),
            ["2:f(n, *a)"],
            "{first}"
        );
    }
    // A binding's sub-rule or an `arguments` that binds a capture withholds it: a later term that
    // names none of it still decides; one that reads it stays open.
    for first in [
        json!({ "has": { "stopBy": "end", "kind": "identifier", "regex": "^n$",
                         "resolves": { "binding": { "value": { "pattern": "$V", "value": { "eq": 1 } } } } } }),
        json!({ "arguments": { "signature": ["x", "y"], "bind": { "Y": "y" } } }),
    ] {
        assert_eq!(
            python_found(unknown_then_false(first.clone()), source),
            ["2:f(n, *a)"],
            "{first}"
        );
    }
    let reads_bound = json!({ "rule": { "kind": "call", "pattern": "f($$$)", "not": { "all": [
        { "arguments": { "signature": ["x", "y"], "bind": { "Y": "y" } } },
        { "has": { "stopBy": "end", "pattern": "$Y", "regex": "^never" } },
    ] } } });
    assert_eq!(python_found(reads_bound, source), Vec::<String>::new());
}

#[test]
fn structural_compare_reads_names_as_text_not_their_values() {
    // `compare`'s structural modes read the captured syntax; a name is its text, whatever its
    // binding holds, as Ruff's `ComparableExpr` does.
    let rule = json!({
        "rule": { "pattern": "f($A, $B)", "compare": { "left": "A", "op": "eq", "right": "B", "text": "structure" } },
    });
    let source = "x = 1\ny = 1\nf(x, y)\nf(x, x)\nf(1, 1.0)\n";
    assert_eq!(python_found(rule, source), ["4:f(x, x) A=x B=x"]);
}

/// `stopAtGlobal`: Ruff's `lookup_symbol` (`find_assigned_value`, `is_bound_to_tuple`) stops at a
/// scope's `global` binding, which has no value, where `resolve_name` continues in the module.
#[test]
fn stop_at_global_reads_the_scopes_global_binding() {
    let source = "\
import os
MODE = os.P_WAIT
async def f():
    global MODE
    os.spawnl(MODE, \"a\")
async def g():
    os.spawnl(MODE, \"a\")
def h():
    global MODE
    MODE = os.P_NOWAIT
    os.spawnl(MODE, \"a\")
def k():
    global LATE
    os.spawnl(LATE, \"a\")
";
    let rule = |binding: Value| {
        json!({ "rule": { "kind": "identifier", "inside": { "stopBy": "neighbor", "kind": "argument_list" },
                          "resolves": { "binding": binding } } })
    };
    let value = json!({ "value": { "kind": "attribute" } });
    assert_eq!(
        python_found(rule(value.clone()), source),
        ["5:MODE", "7:MODE", "11:MODE"]
    );
    let mut stopping = value;
    stopping["stopAtGlobal"] = json!(true);
    assert_eq!(python_found(rule(stopping), source), ["7:MODE", "11:MODE"]);
    // The `global` binding itself: its site is the statement.
    let site = json!({ "stopAtGlobal": true, "site": { "kind": "global_statement" } });
    assert_eq!(python_found(rule(site), source), ["5:MODE", "14:LATE"]);
    let site = json!({ "site": { "kind": "global_statement" } });
    assert_eq!(python_found(rule(site), source), ["14:LATE"]);
    // `false` is the default reading.
    let off = json!({ "stopAtGlobal": false, "value": { "kind": "attribute" } });
    assert_eq!(
        python_found(rule(off), source),
        ["5:MODE", "7:MODE", "11:MODE"]
    );
}

/// `only`: Ruff's `only_binding`, the binding the read resolves to shadows no earlier binding of
/// its name in its scope, whatever form each binding takes.
#[test]
fn only_holds_where_the_binding_shadows_no_earlier_one_in_its_scope() {
    let source = "\
import httpx
from pathlib import Path
a = Path()
a.exists()
b = 1
b = Path()
b.exists()
c = Path()
c.exists()
c = 1
def walrus():
    if (d := 1):
        pass
    d = Path()
    d.exists()
def comprehension():
    [(e := 1) for _ in range(1)]
    e = Path()
    e.exists()
def handler():
    try:
        pass
    except E as g:
        pass
    g = Path()
    g.exists()
def matched(v):
    match v:
        case h:
            pass
    h = Path()
    h.exists()
def looped():
    for i in []:
        pass
    i = Path()
    i.exists()
def within():
    with x as j:
        pass
    j = Path()
    j.exists()
def imported():
    import k
    k = Path()
    k.exists()
def defined():
    def m(): pass
    m = Path()
    m.exists()
    class n: pass
    n = Path()
    n.exists()
def parameter(p):
    p = Path()
    p.exists()
def deferred():
    return q.exists()
q = Path()
q = Path()
def nested():
    r = Path()
    def inner():
        nonlocal r
        r = 1
    r.exists()
def declared():
    global s
    s = Path()
    s.exists()
def augmented():
    t = Path()
    t += 1
    t.exists()
def fine():
    u = Path()
    u.exists()
";
    let rule = |binding: Value| {
        json!({ "rule": { "pattern": "$X.exists()" },
                "constraints": { "X": { "kind": "identifier", "resolves": { "binding": binding } } } })
    };
    let lines = |found: Vec<String>| {
        found
            .iter()
            .map(|found| found.split(':').next().expect("line").to_owned())
            .collect::<Vec<_>>()
    };
    let path = json!({ "value": { "kind": "call" } });
    assert_eq!(
        lines(python_found(rule(path), source)),
        [
            "4", "7", "9", "15", "19", "26", "32", "37", "42", "46", "50", "53", "56", "58", "66",
            "70", "77"
        ]
    );
    let only = json!({ "only": true, "value": { "kind": "call" } });
    // `c` reads its first binding; `r` its first too, as `nonlocal` rebinds after the body.
    assert_eq!(
        lines(python_found(rule(only), source)),
        ["4", "9", "66", "77"]
    );
    // Alone, `only` asks just that.
    assert_eq!(
        lines(python_found(rule(json!({ "only": true })), source)),
        ["4", "9", "66", "77"]
    );
}

/// TypeScript's `only` is a single declaration of the name in its scope.
#[test]
fn only_counts_typescript_declarations_in_the_scope() {
    let source = "var a = 1; f(a);\nvar b = 1; var b = 2; f(b);\nlet c = 1; { let c = 2; f(c); }\nfunction d() {} f(d);\n";
    let rule = json!({ "rule": { "pattern": "f($X)" },
        "constraints": { "X": { "resolves": { "binding": { "only": true } } } } });
    assert_eq!(
        typescript_found(rule, source),
        ["1:f(a) X=a", "3:f(c) X=c", "4:f(d) X=d"]
    );
}

#[test]
fn binding_flags_are_checked() {
    let reject = |language: &str, resolves: Value| {
        rejection(compile(
            language,
            json!({ "rule": { "kind": "identifier", "resolves": resolves } }),
        ))
    };
    assert_eq!(
        reject("python", json!({ "binding": { "only": 1 } })),
        "/clauses/0/forbid/ast/rule/resolves/binding/only must be a boolean"
    );
    assert_eq!(
        reject("python", json!({ "binding": { "stopAtGlobal": "yes" } })),
        "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal must be a boolean"
    );
    assert_eq!(
        reject("typescript", json!({ "binding": { "stopAtGlobal": true } })),
        "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal: `global` statements are Python's; TypeScript has none"
    );
}

/// M10b's own scopes for a class's inner name and a named function expression's name, read
/// through M9b's `only`, `bindings` and the TypeScript value fixes: each name binds alone in its
/// scope, with the class or function as its site and no value, so `value.follow` and `key` read
/// nothing through it.
#[test]
fn class_and_function_expression_names_bind_alone_without_a_value() {
    let source = "\
const k = 'a'; f(k);
const g = function k() { f(k); };
const h = function k() { var k = 'b'; f(k); };
class C { m() { f(C); } }
var E = 1; var E = 2; const D = class E { m() { f(E); } }; f(E);
";
    let binding = |binding: Value| {
        typescript_found(
            json!({ "rule": { "pattern": "f($X)" },
                    "constraints": { "X": { "resolves": { "binding": binding } } } }),
            source,
        )
    };
    let alone = [
        "1:f(k) X=k",
        "2:f(k) X=k",
        "3:f(k) X=k",
        "4:f(C) X=C",
        "5:f(E) X=E",
    ];
    assert_eq!(binding(json!({ "only": true })), alone);
    assert_eq!(binding(json!({ "bindings": { "max": 1 } })), alone);
    assert_eq!(
        binding(json!({ "only": true, "bindings": { "min": 2 } })),
        Vec::<String>::new()
    );
    assert_eq!(binding(json!({ "bindings": { "min": 2 } })), ["5:f(E) X=E"]);
    assert_eq!(
        binding(json!({ "scope": "functionName", "site": { "kind": "function_expression" } })),
        ["2:f(k) X=k"]
    );
    assert_eq!(
        binding(json!({ "scope": "class", "site": { "kind": "class_declaration" } })),
        ["4:f(C) X=C"]
    );
    assert_eq!(
        binding(json!({ "scope": "class", "site": { "kind": "class" }, "only": true })),
        ["5:f(E) X=E"]
    );
    // Only the declarators carry a value; a class or function name has none.
    assert_eq!(
        typescript_found(
            json!({ "rule": { "pattern": "f($X)" },
                    "constraints": { "X": { "value": { "type": "string", "follow": true } } } }),
            source,
        ),
        ["1:f(k) X=k", "3:f(k) X=k"]
    );
    assert_eq!(
        binding(json!({ "value": { "kind": "string" }, "only": true })),
        ["1:f(k) X=k", "3:f(k) X=k"]
    );
    // A computed key named by the function's own name has no static name, whatever it binds.
    let named = json!({ "compare": { "left": "P", "op": "eq", "right": "P", "text": "key" } });
    let key = |extra: Value| {
        let mut rule = json!({ "kind": "pair", "pattern": "$P",
            "has": { "kind": "computed_property_name", "has": { "kind": "identifier",
                "resolves": { "binding": { "only": true, "site": { "kind": "function_expression" } } } } } });
        rule.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        typescript_found(
            json!({ "rule": rule }),
            "const k = 'x';\nconst g = function k() { return { [k]: 1, x: 2 }; };\n",
        )
    };
    assert_eq!(key(json!({})), ["2:[k]: 1 P=[k]: 1"]);
    assert_eq!(key(json!({ "all": [named.clone()] })), Vec::<String>::new());
    assert_eq!(key(json!({ "not": named })), ["2:[k]: 1 P=[k]: 1"]);
}

/// M9's destructured values beside M10b's class-name and function-expression scopes: a class's
/// inner name and a named function expression's name hide an outer destructured name of the same
/// spelling and carry no value; outside them the destructured element stays the binding's value,
/// a class or function expression element included.
#[test]
fn destructured_values_stop_at_class_and_function_expression_names() {
    let source = "\
const {C} = {C: 1}; f(C);
const D = class C { m() { inner(C); } };
const [g] = [2]; f(g);
const h = function g() { inner(g); };
const {K} = {K: class K { m() { inner(K); } }}; f(K);
const [F] = [function F() { inner(F); }]; f(F);
";
    let binding = |binding: Value| {
        typescript_found(
            json!({ "rule": { "pattern": "$G($X)" },
                    "constraints": { "X": { "resolves": { "binding": binding } } } }),
            source,
        )
    };
    assert_eq!(
        binding(json!({ "value": { "pattern": "$V" } })),
        [
            "1:f(C) G=f V=1 X=C",
            "3:f(g) G=f V=2 X=g",
            "5:f(K) G=f V=class K { m() { inner(K); } } X=K",
            "6:f(F) G=f V=function F() { inner(F); } X=F",
        ]
    );
    assert_eq!(
        binding(json!({ "scope": ["class", "functionName"], "only": true })),
        [
            "2:inner(C) G=inner X=C",
            "4:inner(g) G=inner X=g",
            "5:inner(K) G=inner X=K",
            "6:inner(F) G=inner X=F",
        ]
    );
    // `value.follow` reads neither: the destructured element is `resolves.binding.value`'s only.
    assert_eq!(
        typescript_found(
            json!({ "rule": { "pattern": "f($X)" },
                    "constraints": { "X": { "value": { "type": "number", "follow": true } } } }),
            source,
        ),
        Vec::<String>::new()
    );
}

/// M10a's fix round's `beforeLocal` beside M9b's `resolves.binding` terms: the read resolves to the
/// binding the function's own later one shadows, so `only` and the sub-rules read that outer
/// binding, and a `global` declaration (which `stopAtGlobal` reads) never makes a `beforeLocal`.
#[test]
fn before_local_reads_meet_only_and_stop_at_global() {
    let source = "\
x = 0
def f():
    print(x)
    x = 1
y = 0
y = 1
def g():
    print(y)
    y = 2
def h():
    global w
    print(w)
    w = 1
w = 0
";
    let before = json!({ "resolves": { "beforeLocal": true } });
    let rule = |binding: Value| json!({ "rule": { "kind": "identifier", "all": [before.clone(), { "resolves": { "binding": binding } }] } });
    assert_eq!(
        python_found(
            json!({ "rule": { "kind": "identifier", "resolves": { "beforeLocal": true } } }),
            source
        ),
        ["3:x", "8:y"]
    );
    assert_eq!(
        python_found(
            rule(json!({ "value": { "kind": "integer", "regex": "^0$" } })),
            source
        ),
        ["3:x"]
    );
    assert_eq!(python_found(rule(json!({ "only": true })), source), ["3:x"]);
    // `y`'s read sees the module's latest binding, `y = 1`, which is not `only`; `false` is the
    // default reading.
    assert_eq!(
        python_found(rule(json!({ "value": { "regex": "^1$" } })), source),
        ["8:y"]
    );
    assert_eq!(
        python_found(
            rule(json!({ "only": false, "value": { "regex": "^1$" } })),
            source
        ),
        ["8:y"]
    );
    assert_eq!(
        python_found(
            rule(json!({ "stopAtGlobal": true, "site": { "kind": "global_statement" } })),
            source
        ),
        Vec::<String>::new()
    );
    // `print(w)` in `h` reads the module's `w` through the `global` statement, before `local` ever.
    let read = json!({ "rule": { "kind": "identifier", "regex": "^w$", "inside": { "kind": "call", "stopBy": "end" },
                                 "resolves": { "binding": { "stopAtGlobal": true, "site": { "kind": "global_statement" } } } } });
    assert_eq!(python_found(read, source), ["12:w"]);
}

/// `locals()` leaves usedness unknown in its function but no value: `value.follow` and
/// `resolves.binding` still read the bindings there.
#[test]
fn locals_hides_usedness_not_values() {
    let source = "\
import os
def f(p):
    mode = 0o777
    alias = mode
    os.chmod(p, alias)
    unused = 1
    return locals()
";
    let follow = json!({
        "rule": { "pattern": "os.chmod($P, $M)" },
        "constraints": { "M": { "value": { "bitsAny": 2, "follow": true } } },
    });
    assert_eq!(
        python_found(follow, source),
        ["5:os.chmod(p, alias) M=alias P=p"]
    );
    let binding = json!({ "rule": { "kind": "identifier", "regex": "^alias$", "inside": { "kind": "argument_list" },
                                    "resolves": { "binding": { "value": { "kind": "identifier" }, "only": true } } } });
    assert_eq!(python_found(binding, source), ["5:alias"]);
    let unused = json!({ "rule": { "kind": "identifier", "binds": { "form": "assignment", "used": false } } });
    assert_eq!(python_found(unused, source), Vec::<String>::new());
}

/// `destructuredValue` beside M10a's scope keys: Python refuses it next to `beforeLocal` and
/// `seenBefore` as it does alone, and an unpacked Python name keeps its element.
#[test]
fn destructured_value_meets_before_local_and_seen_before() {
    let message = "/clauses/0/forbid/ast/rule/resolves/binding/destructuredValue: destructured values are TypeScript's; a Python unpacked name keeps its element";
    for (key, scope) in [
        (
            "beforeLocal",
            json!({ "resolves": { "beforeLocal": true } }),
        ),
        (
            "seenBefore",
            json!({ "resolves": { "imported": ["os"], "seenBefore": true } }),
        ),
    ] {
        let rule = json!({ "kind": "identifier", "all": [scope, { "resolves": { "binding": { "destructuredValue": false } } }] });
        assert_eq!(
            rejection(compile("python", json!({ "rule": rule }))),
            message.replace("/rule/resolves", "/rule/all/1/resolves"),
            "{key}"
        );
    }
    // A Python unpacked name keeps its element under M10a's reading of the name.
    let source = "a, b = 1, 2\nprint(b)\n";
    assert_eq!(
        python_found(
            json!({ "rule": { "kind": "identifier", "inside": { "kind": "call", "stopBy": "end" },
                              "resolves": { "binding": { "value": { "pattern": "$V" } } } } }),
            source
        ),
        ["2:b V=2"]
    );
}

/// m10fix2's aliased builtin calls beside M11a's `value.regex` backreferences: a call through an
/// alias of `String` folds to the decoded string, which a backreference pattern reads.
#[test]
fn aliased_builtin_calls_fold_for_backreference_patterns() {
    let rule = json!({
        "rule": { "pattern": "use($A)" },
        "constraints": { "A": { "value": { "regex": "^(.)\\1$", "follow": true } } },
    });
    let source = "\
const f = String;
const g = f;
use(f('\\x61a'));
use(g('ab'));
use(String('bb'));
const {h} = {h: String};
use(h('cc'));
";
    assert_eq!(
        typescript_found(rule, source),
        [
            "3:use(f('\\x61a')) A=f('\\x61a')",
            "5:use(String('bb')) A=String('bb')"
        ]
    );
}
