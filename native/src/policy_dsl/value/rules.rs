//! Port-shaped rules for Ruff's value and binding rules, against where Ruff 0.16.9 itself reports
//! them on the fixtures in `fixtures/python/value/ruff` (`fixtures/python/value/generate_ruff.py`); and for Bandit's call-argument
//! rules, against where Bandit 1.8.6 reports them on its own examples in `fixtures/python/value/bandit`
//! (`fixtures/python/value/generate_bandit.py`).

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("python/value/ruff.expected.json");

/// Where a Python query reports on a file: each finding's start byte, sorted.
fn starts(ast: Value, anchor: &str, source: &str) -> Vec<u64> {
    let mut ast = ast;
    ast["language"] = json!("python");
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": ast },
        "finding": { "anchor": anchor, "code": "PORT", "message": "port", "severity": "error" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "fixture.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut starts = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
}

fn expected(code: &str) -> Vec<u64> {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["ruff"], "0.16.9");
    oracle[code]
        .as_array()
        .expect("code")
        .iter()
        .map(|start| start.as_u64().expect("start"))
        .collect()
}

/// A string literal that is not an f-string (Ruff's f-strings are not literals).
fn plain_string() -> Value {
    json!({ "any": [
        { "kind": "string", "not": { "regex": "^[A-Za-z]*[fFtT]" } },
        { "kind": "concatenated_string", "not": { "has": { "kind": "string", "regex": "^[A-Za-z]*[fFtT]" } } },
    ] })
}

/// Ruff's `is_literal_expr`: strings and bytes, numbers, booleans, `None` and `...`.
fn literal() -> Value {
    json!({ "any": [
        plain_string(),
        { "kind": "integer" }, { "kind": "float" }, { "kind": "true" }, { "kind": "false" },
        { "kind": "none" }, { "kind": "ellipsis" },
    ] })
}

fn pair(key: &str, value: &str) -> Value {
    json!({ "context": format!("{{{key}: {value}}}"), "selector": "pair" })
}

#[test]
fn f601_repeated_literal_keys_by_hash() {
    // A literal key (tuples and signed numbers too) equal to an earlier key by Python `==`.
    // `count` searches past keys whose values are unknown (`x`, `**other`).
    let key = json!({ "any": [literal(), { "kind": "tuple" }, { "kind": "unary_operator" }] });
    let ast = json!({
        "rule": {
            "pattern": pair("$K", "$V"),
            "count": { "follows": {
                "stopBy": "end",
                "pattern": pair("$J", "$W"),
                "has": { "pattern": "$J", "all": [key.clone()] },
                "compare": { "left": "J", "op": "eq", "right": "K", "text": "value" },
            }, "min": 1 },
        },
        "constraints": { "K": key },
    });
    let source = oracle!("python/value/ruff/F601.py");
    assert_eq!(starts(ast, "capture.K", source), expected("F601"));
}

#[test]
fn b033_duplicate_literal_set_items_by_hash() {
    let ast = json!({
        "rule": {
            "all": [literal(), { "pattern": "$E" }],
            "inside": { "kind": "set" },
            "count": { "follows": {
                "stopBy": "end",
                "all": [literal(), { "pattern": "$F" }],
                "compare": { "left": "F", "op": "eq", "right": "E", "text": "value" },
            }, "min": 1 },
        },
    });
    let source = oracle!("python/value/ruff/B033.py");
    assert_eq!(starts(ast, "witness.span", source), expected("B033"));
}

#[test]
fn pie796_duplicate_enum_values_structurally() {
    // Ruff compares enum values structurally: a literal repeats one of the same type and value
    // (`1` and `0x1`, not `1` and `1.0` or `1 + 1`), anything else one with the same tokens.
    let same = |condition: Value| {
        json!({
            "kind": "expression_statement",
            "has": { "pattern": "$T = $V", "has": { "pattern": "$V", "all": [condition.clone()] } },
            "count": { "follows": {
                "stopBy": "end",
                "kind": "expression_statement",
                "has": { "pattern": "$S = $U", "has": { "pattern": "$U", "all": [condition] } },
            }, "min": 1 },
        })
    };
    let typed = |name: &str| {
        let mut rule = same(json!({ "all": [literal(), { "value": { "type": name } }] }));
        rule["count"]["follows"]["has"]["compare"] =
            json!({ "left": "U", "op": "eq", "right": "V", "text": "value" });
        rule
    };
    // Anything else, signed numbers and folded expressions included.
    let mut structural = same(json!({ "not": literal() }));
    structural["count"]["follows"]["has"]["compare"] =
        json!({ "left": "U", "op": "eq", "right": "V", "text": "tokens" });
    let mut alternatives = [
        "int", "float", "complex", "str", "bytes", "bool", "NoneType", "ellipsis",
    ]
    .map(typed)
    .to_vec();
    alternatives.push(structural);
    let ast = json!({
        "rule": {
            "any": alternatives,
            // Only the members of a class based on `enum.Enum`; `auto()` values are unique.
            "inside": { "kind": "block", "inside": { "kind": "class_definition", "has": {
                "kind": "argument_list",
                "has": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": ["enum.Enum"] } },
            } } },
            "not": { "has": { "kind": "assignment", "has": { "kind": "call", "has": { "resolves": { "import": ["enum.auto"] } } } } },
        },
    });
    let source = oracle!("python/value/ruff/PIE796.py");
    assert_eq!(starts(ast, "witness.span", source), expected("PIE796"));
}

/// SIM222 (`or`, truthy) and SIM223 (`and`, falsy): a chain that a constant operand decides. The
/// first operand decides anywhere; a later one only in a boolean test, and only after operands
/// without calls. Ruff's truthiness is syntactic: literals, displays, `lambda`, generators, and
/// the builtin iterable constructors; `1 + 1`, `-1` and `bool(0)` are unknown to it.
fn short_circuit(operator: &str, truthy: bool) -> Value {
    let mut constants = vec![
        json!({ "all": [plain_string(), { "value": { "truthy": truthy } }] }),
        json!({ "any": [{ "kind": "integer" }, { "kind": "float" }, { "kind": "true" }, { "kind": "false" },
                  { "kind": "none" }, { "kind": "ellipsis" }, { "kind": "list" }, { "kind": "set" },
                  { "kind": "tuple" }, { "kind": "dictionary" }],
          "value": { "truthy": truthy } }),
        json!({ "kind": "call", "regex": "^(list|set|tuple|dict|frozenset)\\(", "value": { "truthy": truthy } }),
        json!({ "kind": "parenthesized_expression", "has": { "matches": "constant" } }),
    ];
    // A lambda or a generator is always truthy; `value` knows neither.
    if truthy {
        constants
            .push(json!({ "any": [{ "kind": "lambda" }, { "kind": "generator_expression" }] }));
    }
    let constant = json!({ "any": constants });
    let before = json!({ "precedes": { "regex": format!("^{operator}$") } });
    let after = json!({ "follows": { "regex": format!("^{operator}$") } });
    let chain = json!({ "kind": "boolean_operator", "has": { "regex": format!("^{operator}$") } });
    json!({
        "rule": {
            "all": [chain.clone()],
            "not": { "inside": chain.clone() },
            "any": [
                { "matches": "first-decides" },
                { "matches": "later-decides", "inside": {
                    "stopBy": { "not": { "any": [{ "kind": "parenthesized_expression" }, { "kind": "boolean_operator" }, { "kind": "not_operator" }] } },
                    "any": [{ "kind": "if_statement" }, { "kind": "elif_clause" }, { "kind": "while_statement" },
                            { "kind": "assert_statement" }, { "kind": "conditional_expression" }],
                } },
            ],
        },
        "utils": {
            "constant": constant,
            // The leftmost operand of the chain is the constant.
            "first-decides": { "all": [chain.clone()], "has": { "all": [before.clone()], "any": [
                { "matches": "constant" },
                { "matches": "first-decides" },
            ] } },
            // Some operand after the first is the constant, and no operand before it calls.
            "later-decides": { "all": [chain], "any": [
                { "has": { "all": [after], "matches": "constant" },
                  "not": { "has": { "all": [before.clone()], "any": [{ "kind": "call" }, { "has": { "kind": "call", "stopBy": "end" } }] } } },
                { "has": { "all": [before], "matches": "later-decides" } },
            ] },
        },
    })
}

#[test]
fn sim222_or_with_a_truthy_constant() {
    let source = oracle!("python/value/ruff/SIM222.py");
    assert_eq!(
        starts(short_circuit("or", true), "witness.span", source),
        expected("SIM222")
    );
}

#[test]
fn sim223_and_with_a_falsy_constant() {
    let source = oracle!("python/value/ruff/SIM223.py");
    assert_eq!(
        starts(short_circuit("and", false), "witness.span", source),
        expected("SIM223")
    );
}

#[test]
fn b005_strip_with_a_repeated_character() {
    // Ruff's `value.chars().count() > 1 && !value.chars().all_unique()`: some character of the
    // decoded value repeats, a backreference (M11a), non-ASCII included.
    let ast = json!({
        "rule": { "pattern": "$O.$M($A)" },
        "constraints": {
            "M": { "regex": "^(strip|lstrip|rstrip)$" },
            "A": { "all": [plain_string(), { "not": { "regex": "^[A-Za-z]*[bB]" } }], "value": { "regex": "(?s)(.).*\\1" } },
        },
    });
    let source = oracle!("python/value/ruff/B005.py");
    assert_port("B005", source, ast, "witness.span");
}

/// Where a structural port reports on its fixture `fixtures/python/value/ruff/<stem>.py`, against Ruff.
fn assert_port(stem: &str, source: &str, ast: Value, anchor: &str) {
    assert_eq!(starts(ast, anchor, source), expected(stem), "{stem}");
}

/// A class based on `enum.Enum`, as PIE796 reads it.
fn enum_member_block() -> Value {
    json!({ "kind": "block", "inside": { "kind": "class_definition", "has": {
        "kind": "argument_list",
        "has": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": ["enum.Enum"] } },
    } } })
}

#[test]
fn pie796_structure_compares_enum_values_as_comparable_expr() {
    // One comparison for every value: Ruff's ComparableExpr, type-strict and spelling-blind.
    let ast = json!({
        "rule": {
            "kind": "expression_statement",
            "has": { "pattern": "$T = $V" },
            "inside": enum_member_block(),
            "count": { "follows": {
                "stopBy": "end",
                "kind": "expression_statement",
                "has": { "pattern": "$S = $U" },
                "compare": { "left": "U", "op": "eq", "right": "V", "text": "structure" },
            }, "min": 1 },
        },
    });
    let source = oracle!("python/value/ruff/PIE796-structure.py");
    assert_port("PIE796-structure", source, ast, "witness.span");
}

/// Ruff's `is_literal_key` (F601): strings, f-strings, bytes, numbers, booleans, `None`, `...`,
/// tuples, signed numbers, and a real number plus or minus an imaginary literal.
fn literal_key() -> Value {
    json!({ "any": [
        { "kind": "string", "not": { "regex": "^[A-Za-z]*[tT]" } },
        { "kind": "concatenated_string", "not": { "has": { "kind": "string", "regex": "^[A-Za-z]*[tT]" } } },
        { "kind": "integer" }, { "kind": "float" }, { "kind": "true" }, { "kind": "false" },
        { "kind": "none" }, { "kind": "ellipsis" }, { "kind": "tuple" },
        { "kind": "unary_operator", "regex": "^[-+]", "has": { "stopBy": "neighbor", "matches": "signed" } },
        { "kind": "binary_operator", "has": {
            "stopBy": "neighbor", "regex": "^[-+]$",
            "follows": { "stopBy": "neighbor", "matches": "signed", "not": { "regex": "[jJ]\\)*$" } },
            "precedes": { "stopBy": "neighbor", "any": [{ "kind": "integer" }, { "kind": "float" }], "regex": "[jJ]$" },
        } },
    ] })
}

#[test]
fn f601_structure_hash_compares_keys_as_hashable_expr() {
    // A literal key equal to an earlier key of any kind, by Ruff's HashableExpr.
    let ast = json!({
        "rule": {
            "pattern": "$KEY",
            "not": { "kind": "parenthesized_expression" },
            "all": [{ "matches": "literal-key" }, { "matches": "key-slot" }],
            "inside": {
                "stopBy": { "not": { "kind": "parenthesized_expression" } },
                "kind": "pair",
                "count": { "follows": {
                    "stopBy": "end",
                    "kind": "pair",
                    "has": { "stopBy": "neighbor", "not": { "follows": { "stopBy": "neighbor", "regex": "(?s)." } }, "pattern": "$J" },
                    "compare": { "left": "J", "op": "eq", "right": "KEY", "text": "structure-hash" },
                }, "min": 1 },
            },
        },
        "utils": {
            "literal-key": literal_key(),
            "signed": { "any": [
                { "kind": "integer" }, { "kind": "float" },
                { "kind": "unary_operator", "regex": "^[-+]", "has": { "stopBy": "neighbor", "matches": "signed" } },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "signed" } },
            ] },
            // The key of a pair, possibly inside parentheses.
            "key-slot": { "any": [
                { "not": { "follows": { "stopBy": "neighbor", "regex": "(?s)." } }, "inside": { "stopBy": "neighbor", "kind": "pair" } },
                { "inside": { "stopBy": "neighbor", "kind": "parenthesized_expression", "matches": "key-slot" } },
            ] },
        },
    });
    let source = oracle!("python/value/ruff/F601-structure.py");
    assert_port("F601-structure", source, ast, "capture.KEY");
}

#[test]
fn b033_structure_hash_compares_set_items_as_hashable_expr() {
    // A literal item of a set (parentheses aside) equal to an earlier literal item.
    let ast = json!({
        "rule": {
            "all": [{ "matches": "item-v" }],
            "inside": { "stopBy": "neighbor", "kind": "set" },
            "count": { "follows": {
                "stopBy": "end",
                "matches": "item-f",
                "compare": { "left": "F", "op": "eq", "right": "V", "text": "structure-hash" },
            }, "min": 1 },
        },
        "utils": {
            "literal": literal(),
            "item-v": { "any": [
                { "matches": "literal", "pattern": "$V" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "item-v" } },
            ] },
            "item-f": { "any": [
                { "matches": "literal", "pattern": "$F" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "item-f" } },
            ] },
        },
    });
    let source = oracle!("python/value/ruff/B033-structure.py");
    assert_port("B033-structure", source, ast, "capture.V");
}

#[test]
fn sim116_structure_parts_counts_literals_as_comparable_literal() {
    // An if/elif chain testing one name against at least three distinct literals, each branch a
    // single return. Implicit concatenations compare part by part (`"a" "b"` is not `"ab" ""`).
    let ast = json!({
        "rule": {
            "kind": "if_statement",
            "all": [
                { "has": { "stopBy": "neighbor", "follows": { "stopBy": "neighbor", "regex": "^if$" }, "matches": "eq-test" } },
                { "has": { "stopBy": "neighbor", "kind": "block", "matches": "return-block" } },
                { "not": { "has": { "stopBy": "neighbor", "kind": "elif_clause", "not": { "all": [
                    { "has": { "stopBy": "neighbor", "follows": { "stopBy": "neighbor", "regex": "^elif$" }, "matches": "eq-test" } },
                    { "has": { "stopBy": "neighbor", "kind": "block", "matches": "return-block" } },
                ] } } } },
                { "not": { "has": { "stopBy": "neighbor", "kind": "else_clause", "not": { "has": { "stopBy": "neighbor", "kind": "block", "matches": "return-block" } } } } },
            ],
            // A literal counts when no later elif tests an equal one.
            "count": { "has": { "stopBy": "end", "matches": "test-literal", "pattern": "$L", "not": { "inside": {
                "stopBy": "end",
                "any": [
                    { "kind": "elif_clause" },
                    { "any": [{ "kind": "comparison_operator" }, { "kind": "parenthesized_expression" }], "inside": { "stopBy": "neighbor", "kind": "if_statement" } },
                ],
                "precedes": { "stopBy": "end", "kind": "elif_clause", "has": {
                    "stopBy": "end", "matches": "test-literal", "pattern": "$M",
                    "compare": { "left": "M", "op": "eq", "right": "L", "text": "structure-parts" },
                } },
            } } }, "min": 3 },
        },
        "utils": {
            "literal": { "any": [literal(), { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "literal" } }] },
            "eq-test": { "any": [
                { "kind": "comparison_operator", "pattern": "$N == $_LIT",
                  "has": { "stopBy": "neighbor", "follows": { "stopBy": "neighbor", "regex": "^==$" }, "matches": "literal" },
                  "all": [{ "has": { "stopBy": "neighbor", "kind": "identifier", "precedes": { "stopBy": "neighbor", "regex": "^==$" } } }] },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "eq-test" } },
            ] },
            "return-block": { "has": { "stopBy": "neighbor", "kind": "return_statement" }, "not": { "has": { "stopBy": "neighbor", "not": { "kind": "return_statement" } } } },
            "test-slot": { "any": [
                { "follows": { "stopBy": "neighbor", "regex": "^(if|elif)$" } },
                { "inside": { "stopBy": "neighbor", "kind": "parenthesized_expression", "matches": "test-slot" } },
            ] },
            "test-literal": { "matches": "literal", "follows": { "stopBy": "neighbor", "regex": "^==$" },
                "inside": { "stopBy": "neighbor", "kind": "comparison_operator", "matches": "test-slot" } },
        },
    });
    let source = oracle!("python/value/ruff/SIM116.py");
    assert_port("SIM116", source, ast, "witness.span");
}

#[test]
fn sim101_structure_compares_adjacent_isinstance_targets() {
    // An `or` chain where two adjacent operands are isinstance calls with equal targets.
    let chain =
        json!({ "kind": "boolean_operator", "has": { "stopBy": "neighbor", "regex": "^or$" } });
    let right =
        json!({ "stopBy": "neighbor", "follows": { "stopBy": "neighbor", "regex": "^or$" } });
    let left =
        json!({ "stopBy": "neighbor", "precedes": { "stopBy": "neighbor", "regex": "^or$" } });
    let with = |relation: &Value, rule: Value| {
        let mut relation = relation.clone();
        relation
            .as_object_mut()
            .expect("object")
            .extend(rule.as_object().expect("rule").clone());
        relation
    };
    let ast = json!({
        "rule": {
            "all": [chain.clone()],
            "not": { "inside": chain.clone() },
            "any": [{ "matches": "pair" }, { "has": { "stopBy": { "not": chain.clone() }, "matches": "pair" } }],
        },
        "utils": {
            "chain": chain,
            "isinstance-a": { "any": [
                { "kind": "call", "pattern": "isinstance($A, $_TA)" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "isinstance-a" } },
            ] },
            "isinstance-b": { "any": [
                { "kind": "call", "pattern": "isinstance($B, $_TB)" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "isinstance-b" } },
            ] },
            // A chain node whose right operand and the operand before it are both isinstance calls.
            "pair": {
                "all": [
                    { "matches": "chain" },
                    { "has": with(&right, json!({ "matches": "isinstance-a" })) },
                    { "any": [
                        { "has": with(&left, json!({ "matches": "isinstance-b" })) },
                        { "has": with(&left, json!({ "matches": "chain", "has": with(&right, json!({ "matches": "isinstance-b" })) })) },
                    ] },
                ],
                "compare": { "left": "A", "op": "eq", "right": "B", "text": "structure" },
            },
        },
    });
    let source = oracle!("python/value/ruff/SIM101.py");
    assert_port("SIM101", source, ast, "witness.span");
}

#[test]
fn sim114_structure_compares_consecutive_if_arms_as_comparable_stmt() {
    // An if or elif branch whose body equals the next elif's (an else is not a branch here).
    let ast = json!({
        "rule": {
            "any": [
                { "kind": "if_statement",
                  "has": { "stopBy": "neighbor", "kind": "block", "pattern": "$X" },
                  "all": [{ "has": { "stopBy": "neighbor", "kind": "elif_clause",
                      "not": { "follows": { "stopBy": "end", "kind": "elif_clause" } },
                      "has": { "stopBy": "neighbor", "kind": "block", "pattern": "$Y" } } }] },
                { "kind": "elif_clause",
                  "has": { "stopBy": "neighbor", "kind": "block", "pattern": "$X" },
                  "precedes": { "stopBy": { "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }] }, "kind": "elif_clause",
                      "has": { "stopBy": "neighbor", "kind": "block", "pattern": "$Y" } } },
            ],
            "compare": { "left": "X", "op": "eq", "right": "Y", "text": "structure" },
        },
    });
    let source = oracle!("python/value/ruff/SIM114.py");
    assert_port("SIM114", source, ast, "witness.span");
}

/// Ruff's `is_dict` over a name's binding (`typing::check_type` with the dict checker): its value is
/// a dict display, comprehension or `dict(...)` call; its annotation names `dict` or `typing.Dict`
/// (subscripted or not); or it is a `**kwargs` parameter.
fn dict_binding_utils() -> Value {
    json!({
        "dict-name": { "any": [
            { "kind": "identifier", "resolves": { "global": ["dict"], "import": ["typing.Dict", "builtins.dict"] } },
            { "kind": "attribute", "resolves": { "import": ["typing.Dict", "builtins.dict"] } },
        ] },
        "dict-type": { "any": [
            { "matches": "dict-name" },
            { "kind": "generic_type", "has": { "stopBy": "neighbor", "matches": "dict-name" } },
            { "kind": "subscript", "has": { "stopBy": "neighbor", "matches": "dict-name" } },
        ] },
        "dict-bound": { "kind": "identifier", "any": [
            { "resolves": { "binding": { "value": { "any": [
                { "kind": "dictionary" },
                { "kind": "dictionary_comprehension" },
                { "kind": "call", "has": { "stopBy": "neighbor", "matches": "dict-name" } },
            ] } } } },
            { "resolves": { "binding": { "annotation": { "matches": "dict-type" } } } },
            { "resolves": { "binding": { "site": { "kind": "dictionary_splat_pattern" } } } },
        ] },
    })
}

#[test]
fn sim910_get_with_a_none_default_on_a_dict_binding() {
    let source = oracle!("python/value/ruff/SIM910.py");
    let ast = json!({
        "rule": { "pattern": "$D.get($K, None)" },
        "constraints": { "D": { "any": [
            { "kind": "dictionary" },
            { "kind": "dictionary_comprehension" },
            { "matches": "dict-bound" },
        ] } },
        "utils": dict_binding_utils(),
    });
    assert_eq!(starts(ast, "witness.span", source), expected("SIM910"));
}

#[test]
fn sim911_zip_of_a_dict_bindings_keys_and_values() {
    let source = oracle!("python/value/ruff/SIM911.py");
    let ast = json!({
        "rule": { "kind": "call", "any": [
            { "pattern": "$Z($D.keys(), $D.values())" },
            { "pattern": "$Z($D.keys(), $D.values(), strict=$S)" },
        ] },
        "constraints": {
            "Z": { "kind": "identifier", "resolves": { "global": ["zip"] } },
            "D": { "matches": "dict-bound" },
        },
        "utils": dict_binding_utils(),
    });
    assert_eq!(starts(ast, "witness.span", source), expected("SIM911"));
}

/// ASYNC230's port: Ruff's `find_binding_value` reads an assignment's or walrus's value (an
/// unpacked target's element of a display), or a `with` item's context expression for a plain
/// name; not an annotation or a parameter.
fn async230_port() -> Value {
    json!({
        "rule": {
            "pattern": "$X.open($$$)",
            "inside": { "stopBy": { "any": [{ "kind": "function_definition" }, { "kind": "lambda" }, { "kind": "class_definition" }] },
                        "kind": "function_definition", "regex": "^async\\s" },
        },
        "constraints": { "X": { "any": [
            { "matches": "path-call" },
            { "kind": "identifier", "resolves": { "binding": { "value": { "matches": "path-call" } } } },
            { "kind": "identifier", "resolves": { "binding": { "site": { "kind": "as_pattern", "all": [
                { "has": { "stopBy": "neighbor", "matches": "path-call" } },
                { "has": { "stopBy": "neighbor", "kind": "as_pattern_target", "has": { "stopBy": "neighbor", "matches": "plain-target" } } },
            ] } } } },
        ] } },
        "utils": {
            "plain-target": { "any": [
                { "kind": "identifier" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "plain-target" } },
            ] },
            "path-call": { "kind": "call", "has": { "stopBy": "neighbor", "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                                                    "resolves": { "import": ["pathlib.Path"] } } },
        },
    })
}

#[test]
fn async230_open_on_a_path_binding_in_an_async_function() {
    let source = oracle!("python/value/ruff/ASYNC230.py");
    assert_eq!(
        starts(async230_port(), "witness.span", source),
        expected("ASYNC230")
    );
}

#[test]
fn async230_open_on_an_unpacked_path_binding() {
    let source = oracle!("python/value/ruff/ASYNC230-unpacking.py");
    assert_eq!(
        starts(async230_port(), "witness.span", source),
        expected("ASYNC230-unpacking")
    );
}

#[test]
fn b019_cache_on_a_method_of_a_class_without_an_enum_base() {
    // Ruff's `any_qualified_base_class` follows a base name to its class definition's own bases;
    // an alias assignment is not followed.
    let source = oracle!("python/value/ruff/B019.py");
    let ast = json!({
        "rule": {
            "kind": "decorator",
            "has": { "stopBy": "neighbor", "any": [
                { "matches": "cache" },
                { "kind": "call", "has": { "stopBy": "neighbor", "matches": "cache" } },
            ] },
            "inside": { "stopBy": "neighbor", "kind": "decorated_definition",
                "has": { "stopBy": "neighbor", "kind": "function_definition" },
                "inside": { "stopBy": "neighbor", "kind": "block",
                    "inside": { "stopBy": "neighbor", "kind": "class_definition", "not": { "matches": "enum-bases" } } } },
        },
        "utils": {
            "cache": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                       "resolves": { "import": ["functools.cache", "functools.lru_cache"] } },
            "enum-bases": { "has": { "stopBy": "neighbor", "kind": "argument_list",
                                     "has": { "stopBy": "neighbor", "matches": "enum-class" } } },
            "enum-class": { "any": [
                { "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                  "resolves": { "import": ["enum.Enum", "enum.IntEnum", "enum.Flag", "enum.IntFlag", "enum.StrEnum", "enum.ReprEnum"] } },
                { "kind": "identifier", "resolves": { "binding": { "site": { "kind": "class_definition", "matches": "enum-bases" } } } },
            ] },
        },
    });
    assert_eq!(starts(ast, "witness.span", source), expected("B019"));
}

const BANDIT: &str = oracle!("python/value/bandit.expected.json");

/// Where Bandit 1.8.6 reports `code` on its own example (`fixtures/python/value/generate_bandit.py`).
fn bandit(code: &str) -> Vec<u64> {
    let oracle: Value = serde_json::from_str(BANDIT).expect("oracle JSON");
    assert_eq!(oracle["bandit"], "1.8.6");
    oracle[code]
        .as_array()
        .expect("code")
        .iter()
        .map(|start| start.as_u64().expect("start"))
        .collect()
}

/// A callee (name or attribute chain) that resolves to one of `names`.
fn callee(names: &[&str]) -> Value {
    json!({ "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
            "resolves": { "import": names } })
}

#[test]
fn b113_requests_without_a_timeout_or_with_none() {
    let verbs = ["get", "options", "head", "post", "put", "patch", "delete"];
    let requests = verbs.map(|verb| format!("requests.{verb}"));
    let httpx = verbs
        .iter()
        .chain(&["request", "stream", "Client", "AsyncClient"])
        .map(|attribute| format!("httpx.{attribute}"))
        .collect::<Vec<_>>();
    let requests = requests.iter().map(String::as_str).collect::<Vec<_>>();
    let httpx = httpx.iter().map(String::as_str).collect::<Vec<_>>();
    // Bandit reads `timeout` as a keyword only.
    let signature = json!(["*", "timeout"]);
    let ast = json!({
        "rule": { "kind": "call", "any": [
            { "has": callee(&requests), "arguments": { "signature": signature, "absent": ["timeout"] } },
            { "has": { "any": [callee(&requests), callee(&httpx)] },
              "arguments": { "signature": signature, "bind": { "T": "timeout" } } },
        ] },
        "constraints": { "T": { "kind": "none" } },
    });
    let source = oracle!("python/value/bandit/requests-missing-timeout.py");
    assert_eq!(starts(ast, "witness.span", source), bandit("B113"));
}

#[test]
fn b506_yaml_load_without_a_safe_loader() {
    // `Loader` by keyword or second position, safe when it names SafeLoader or CSafeLoader.
    let signature = json!(["stream", "Loader"]);
    let ast = json!({
        "rule": { "kind": "call", "has": callee(&["yaml.load"]), "any": [
            { "arguments": { "signature": signature, "absent": ["Loader"] } },
            { "arguments": { "signature": signature, "bind": { "L": "Loader" } } },
        ] },
        "constraints": { "L": { "not": { "any": [
            { "kind": "identifier", "regex": "^C?SafeLoader$" },
            { "kind": "attribute", "regex": "\\.C?SafeLoader$" },
        ] } } },
    });
    let source = oracle!("python/value/bandit/yaml_load.py");
    assert_eq!(starts(ast, "witness.span", source), bandit("B506"));
}

#[test]
fn b505_weak_key_sizes_by_keyword_or_position() {
    let module = "cryptography.hazmat.primitives.asymmetric";
    let key = |function: String, signature: Value, slot: &str| json!({ "has": callee(&[function.as_str()]), "arguments": { "signature": signature, "bind": { "K": slot } } });
    let ast = json!({
        "rule": { "kind": "call", "any": [
            key(format!("{module}.dsa.generate_private_key"), json!(["key_size", "backend"]), "key_size"),
            key(format!("{module}.rsa.generate_private_key"), json!(["public_exponent", "key_size", "backend"]), "key_size"),
            { "has": callee(&[&format!("{module}.ec.generate_private_key")]),
              "arguments": { "signature": ["curve", "backend"], "bind": { "C": "curve" } } },
            { "has": callee(&["Crypto.PublicKey.DSA.generate", "Crypto.PublicKey.RSA.generate",
                              "Cryptodome.PublicKey.DSA.generate", "Cryptodome.PublicKey.RSA.generate"]),
              "arguments": { "signature": ["bits"], "bind": { "K": "bits" } } },
        ] },
        // Below the medium sizes: 2048 bits for DSA and RSA, 224 for a curve.
        "constraints": {
            "K": { "value": { "lt": 2048 } },
            "C": { "any": [
                { "kind": "attribute", "regex": "\\.(SECP192R1|SECT163K1|SECT163R2)$" },
                { "kind": "identifier", "regex": "^(SECP192R1|SECT163K1|SECT163R2)$" },
            ] },
        },
    });
    let source = oracle!("python/value/bandit/weak_cryptographic_key_sizes.py");
    assert_eq!(starts(ast, "witness.span", source), bandit("B505"));
}

#[test]
fn b610_extra_with_a_non_literal_where_tables_or_select() {
    let signature = json!([
        "select",
        "where",
        "params",
        "tables",
        "order_by",
        "select_params"
    ]);
    // A slot's argument, found again by its capture: any node with the same text has the same
    // syntax, so the same verdict.
    let insecure = |capture: &str, slot: &str, secure: &str| {
        json!({ "arguments": { "signature": signature, "bind": { capture: slot } },
                "has": { "stopBy": "end", "pattern": format!("${capture}"), "not": { "matches": secure } } })
    };
    let ast = json!({
        "rule": { "any": [{ "pattern": "extra($$$)" }, { "pattern": "$R.extra($$$)" }],
                  "all": [{ "any": [insecure("W", "where", "strings"), insecure("T", "tables", "strings"),
                                    insecure("S", "select", "string-pairs")] }] },
        "utils": {
            "string": { "any": [
                { "kind": "string", "not": { "regex": "^[A-Za-z]*[fFbBtT]" } },
                { "kind": "concatenated_string", "not": { "has": { "kind": "string", "regex": "^[A-Za-z]*[fFbBtT]" } } },
            ] },
            "strings": { "kind": "list", "not": { "has": { "stopBy": "neighbor",
                "not": { "any": [{ "matches": "string" }, { "regex": "^[\\[\\],]$" }, { "kind": "comment" }] } } } },
            "string-pairs": { "kind": "dictionary", "not": { "has": { "stopBy": "neighbor",
                "not": { "any": [
                    { "kind": "pair", "not": { "has": { "stopBy": "neighbor",
                        "not": { "any": [{ "matches": "string" }, { "regex": "^:$" }, { "kind": "comment" }] } } } },
                    { "regex": "^[{},]$" }, { "kind": "comment" },
                ] } } } },
        },
    });
    let source = oracle!("python/value/bandit/django_sql_injection_extra.py");
    assert_eq!(starts(ast, "witness.span", source), bandit("B610"));
}

/// ASYNC220's `os.spawn*` branch: the mode is given and is not `os.P_WAIT` (Ruff's `is_p_wait`),
/// directly or as a name whose value is, read through `find_assigned_value`'s `lookup_symbol`,
/// which stops at a `global` binding.
fn async220_spawn_port(arguments: Value) -> Value {
    let spawn =
        ["l", "le", "lp", "lpe", "v", "ve", "vp", "vpe"].map(|suffix| format!("os.spawn{suffix}"));
    json!({
        "rule": {
            "kind": "call",
            "pattern": "$F($$$)",
            "inside": { "stopBy": { "any": [{ "kind": "function_definition" }, { "kind": "lambda" }, { "kind": "class_definition" }] },
                        "kind": "function_definition", "regex": "^async\\s" },
            "arguments": arguments,
        },
        "constraints": {
            "F": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": spawn } },
            "M": { "not": { "any": [
                { "matches": "p-wait" },
                { "kind": "identifier", "resolves": { "binding": { "stopAtGlobal": true, "value": { "matches": "p-wait" } } } },
            ] } },
        },
        "utils": {
            "p-wait": { "any": [
                { "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": ["os.P_WAIT"] } },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "p-wait" } },
            ] },
        },
    })
}

#[test]
fn async220_spawn_mode_read_through_a_global_stops_there() {
    let source = oracle!("python/value/ruff/ASYNC220-global.py");
    let port = async220_spawn_port(json!({ "signature": ["mode"], "bind": { "M": "mode" } }));
    assert_eq!(
        starts(port, "capture.F", source),
        expected("ASYNC220-global")
    );
}

#[test]
fn async240_path_method_on_the_only_binding_of_a_name() {
    // Ruff's `only_binding` then `check_type`: the name's binding shadows no earlier binding in its
    // scope, and its value is a `Path(...)` call or its annotation names `Path`.
    let source = oracle!("python/value/ruff/ASYNC240-only.py");
    let ast = json!({
        "rule": {
            "pattern": "$X.exists()",
            "inside": { "stopBy": { "any": [{ "kind": "function_definition" }, { "kind": "lambda" }, { "kind": "class_definition" }] },
                        "kind": "function_definition", "regex": "^async\\s" },
        },
        "constraints": { "X": { "any": [
            { "matches": "path-call" },
            { "kind": "identifier", "any": [
                { "resolves": { "binding": { "only": true, "value": { "matches": "path-call" } } } },
                { "resolves": { "binding": { "only": true, "annotation": { "matches": "path" } } } },
            ] },
        ] } },
        "utils": {
            "path": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": ["pathlib.Path"] } },
            "path-call": { "kind": "call", "has": { "stopBy": "neighbor", "matches": "path" } },
        },
    });
    assert_eq!(
        starts(ast, "witness.span", source),
        expected("ASYNC240-only")
    );
}

/// An argument of an argument list, keyword or not: no comment or punctuation.
fn any_argument() -> Value {
    json!({ "not": { "any": [{ "kind": "comment" }, { "regex": "^[(),]$" }] } })
}

#[test]
fn b905_zip_without_strict_finds_the_keyword_syntactically() {
    // Ruff's `find_keyword("strict")`: `**kw` and `**{"strict": ...}` name no keyword.
    let source = oracle!("python/value/ruff/B905-syntactic.py");
    let positional = json!({ "all": [any_argument(), { "not": { "any": [{ "kind": "keyword_argument" }, { "kind": "dictionary_splat" }] } }] });
    let ast = json!({
        "rule": {
            "kind": "call",
            "has": { "stopBy": "neighbor", "kind": "identifier", "resolves": { "global": ["zip"] } },
            "arguments": { "signature": ["*", "strict"], "absent": ["strict"], "mode": "syntactic" },
            "all": [{ "has": { "stopBy": "neighbor", "kind": "argument_list", "any": [
                { "count": { "has": { "stopBy": "neighbor", "matches": "positional" }, "min": 2 } },
                { "has": { "stopBy": "neighbor", "kind": "list_splat" } },
            ] } }],
        },
        "utils": { "positional": positional },
    });
    assert_eq!(
        starts(ast, "witness.span", source),
        expected("B905-syntactic")
    );
}

#[test]
fn async115_zero_sleep_finds_the_argument_syntactically() {
    // Ruff's `find_argument_value("seconds", 0)` on a call with exactly one argument of any kind.
    let source = oracle!("python/value/ruff/ASYNC115-syntactic.py");
    let ast = json!({
        "rule": {
            "kind": "call",
            "has": { "stopBy": "neighbor", "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                     "resolves": { "import": ["trio.sleep"] } },
            "arguments": { "signature": ["seconds"], "bind": { "S": "seconds" }, "mode": "syntactic" },
            "all": [{ "has": { "stopBy": "neighbor", "kind": "argument_list",
                               "count": { "has": { "stopBy": "neighbor", "matches": "argument" }, "min": 1, "max": 1 } } }],
        },
        "constraints": { "S": { "matches": "zero" } },
        "utils": {
            "argument": any_argument(),
            "zero": { "any": [
                { "kind": "integer", "value": { "eq": 0 } },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "zero" } },
            ] },
        },
    });
    assert_eq!(
        starts(ast, "witness.span", source),
        expected("ASYNC115-syntactic")
    );
}

/// SIM113: `N += 1` in a `for` body, where `N`'s binding is `N = 0` (unpacked too) at the loop's
/// own level, `N`'s scope binds it once in the loop (`binds.bindings`: a walrus in a
/// comprehension, a match capture and a `del` count, a comprehension target, a keyword argument
/// and a nested function's local do not), and no load of `N` outside the loop reads the initial
/// binding (Ruff's inherited references) or one made in the loop. Two identical loops in one
/// scope are told apart by node identity (`text: "node"`).
fn sim113_port(identity: &str) -> Value {
    let same = |capture: &str, other: &str| json!({ "left": capture, "op": "eq", "right": other, "text": identity });
    json!({
        "rule": {
            "kind": "augmented_assignment",
            "all": [
                { "has": { "stopBy": "neighbor", "kind": "identifier", "pattern": "$N",
                           "precedes": { "stopBy": "neighbor", "regex": "^\\+=$" } } },
                { "has": { "stopBy": "neighbor", "kind": "integer", "value": { "eq": 1 },
                           "follows": { "stopBy": "neighbor", "regex": "^\\+=$" } } },
                { "has": { "stopBy": "neighbor", "kind": "identifier", "pattern": "$N",
                           "precedes": { "stopBy": "neighbor", "regex": "^\\+=$" },
                           "resolves": { "binding": {
                               "value": { "kind": "integer", "value": { "eq": 0 } },
                               "site": { "kind": "assignment", "pattern": "$INIT",
                                         "inside": { "stopBy": "neighbor", "kind": "expression_statement",
                                                     "inside": { "stopBy": "neighbor", "pattern": "$AB" } } } } } } },
                { "inside": { "stopBy": "neighbor", "kind": "expression_statement",
                    "inside": { "stopBy": "neighbor", "kind": "block",
                        "inside": { "stopBy": "neighbor", "kind": "for_statement", "pattern": "$LOOP",
                            "inside": { "stopBy": "neighbor", "pattern": "$FB" },
                            "not": { "any": [
                                { "regex": "^async" },
                                { "has": { "stopBy": { "any": [{ "kind": "function_definition" }, { "kind": "class_definition" }] },
                                           "kind": "continue_statement" } },
                                { "matches": "later-load" },
                            ] } } } } },
                { "has": { "stopBy": "neighbor", "kind": "identifier", "pattern": "$N",
                           "precedes": { "stopBy": "neighbor", "regex": "^\\+=$" },
                           "binds": { "form": "augmented", "bindings": { "within": "LOOP", "max": 1 } } } },
            ],
            "compare": same("AB", "FB"),
        },
        "utils": {
            "later-load": { "inside": { "stopBy": "end", "any": [{ "kind": "function_definition" }, { "kind": "module" }],
                "has": { "stopBy": "end", "kind": "identifier", "pattern": "$N",
                         "not": { "inside": { "stopBy": "end", "kind": "for_statement", "pattern": "$L2", "compare": same("L2", "LOOP") } },
                         "resolves": { "binding": { "site": { "any": [
                             { "pattern": "$S", "compare": same("S", "INIT") },
                             { "inside": { "stopBy": "end", "kind": "for_statement", "pattern": "$L3", "compare": same("L3", "LOOP") } },
                         ] } } } } } },
        },
    })
}

#[test]
fn sim113_counter_tells_identical_loops_apart_by_node() {
    let source = oracle!("python/value/ruff/SIM113.py");
    assert_eq!(
        starts(sim113_port("node"), "witness.span", source),
        expected("SIM113")
    );
    // By text, a load in the second of two identical loops reads as inside the first.
    let by_text = starts(sim113_port("source"), "witness.span", source);
    assert_ne!(by_text, expected("SIM113"));
}

/// Where a Python query reports with `finding.each`: every finding's start byte, repeats kept.
fn each_starts(ast: Value, each: Option<&str>, source: &str) -> Vec<u64> {
    each_starts_at(ast, "witness.span", each, source)
}

/// `each_starts` with the finding anchored at `anchor`.
fn each_starts_at(ast: Value, anchor: &str, each: Option<&str>, source: &str) -> Vec<u64> {
    let mut ast = ast;
    ast["language"] = json!("python");
    let mut finding =
        json!({ "anchor": anchor, "code": "PORT", "message": "port", "severity": "error" });
    if let Some(each) = each {
        finding["each"] = json!(each);
    }
    let document =
        check_document(json!({ "id": "port", "forbid": { "ast": ast }, "finding": finding }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "fixture.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut starts = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts
}

/// The Ruff suite's E712 utilities (`rerail/conformance/ruff/rules/E712.json`), verbatim.
const E712_UTILS: &str = r#"{"constant":{"any":[{"kind":"string","not":{"has":{"stopBy":"end","kind":"interpolation"}}},{"kind":"concatenated_string","not":{"has":{"stopBy":"end","kind":"interpolation"}}},{"kind":"integer"},{"kind":"float"},{"kind":"none"},{"kind":"true"},{"kind":"false"},{"kind":"ellipsis"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"constant"}},{"kind":"tuple","not":{"has":{"stopBy":"neighbor","not":{"any":[{"matches":"constant"},{"regex":"^[(),]$"}]}}}}]},"singleton":{"any":[{"kind":"none"},{"kind":"true"},{"kind":"false"},{"kind":"ellipsis"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"singleton"}}]},"constant-non-singleton":{"all":[{"matches":"constant"},{"not":{"matches":"singleton"}}]},"is-true":{"any":[{"kind":"true"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"is-true"}}]},"is-false":{"any":[{"kind":"false"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"is-false"}}]},"flagged-bool-operand":{"all":[{"any":[{"matches":"is-true"},{"matches":"is-false"}]},{"any":[{"all":[{"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}},{"precedes":{"stopBy":"neighbor","regex":"^(==|!=)$","precedes":{"stopBy":"neighbor","not":{"matches":"constant-non-singleton"}}}}]},{"all":[{"follows":{"stopBy":"neighbor","regex":"^(==|!=)$","follows":{"stopBy":"neighbor","not":{"matches":"constant-non-singleton"}}}},{"not":{"any":[{"all":[{"matches":"is-true"},{"follows":{"stopBy":"neighbor","regex":"^==$","follows":{"stopBy":"neighbor","matches":"is-true"}}}]},{"all":[{"matches":"is-false"},{"follows":{"stopBy":"neighbor","regex":"^==$","follows":{"stopBy":"neighbor","matches":"is-false"}}}]}]}}]}]}]}}"#;

/// E712 (M12b): Ruff reports each offending `True`/`False` operand of a chained comparison, each
/// spanning the whole comparison. The suite's port binds the operand (`$OP`) and reports it with
/// `each: "OP"`; without `each` it reported the comparison once.
#[test]
fn e712_reports_each_offending_operand() {
    let utils: Value = serde_json::from_str(E712_UTILS).expect("utils");
    let ast = |binds: bool| {
        let operand = if binds {
            json!({ "stopBy": "neighbor", "matches": "flagged-bool-operand", "pattern": "$OP" })
        } else {
            json!({ "stopBy": "neighbor", "matches": "flagged-bool-operand" })
        };
        json!({ "utils": utils, "rule": { "kind": "comparison_operator", "has": operand } })
    };
    let source = oracle!("python/value/ruff/E712-each.py");
    let mut once = expected("E712-each");
    once.dedup();
    assert_eq!(each_starts(ast(false), None, source), once);
    assert_eq!(
        each_starts(ast(true), Some("OP"), source),
        expected("E712-each")
    );
}

/// PIE810 (M12b): Ruff reports an `or` chain once per group of two or more `N.startswith(x)` (or
/// `.endswith`) operands with the same receiver name and method, each spanning the chain. The
/// port binds each group's first call (`$FIRST`: a later operand shares its key, no earlier one
/// does) and reports it with `each: "FIRST"`. A shape with the call's argument read as any single
/// positional argument; the suite's port also reads whether a name is bound to a tuple.
#[test]
fn pie810_reports_each_group_of_calls() {
    let or = json!({ "kind": "boolean_operator", "has": { "field": "operator", "regex": "^or$" } });
    let spine = json!({ "not": { "matches": "or" } });
    let call = |receiver: &str, method: &str, argument: &str| {
        json!({
            "kind": "call",
            "pattern": format!("${receiver}.${method}(${argument})"),
            "has": { "field": "function", "has": { "field": "object", "kind": "identifier" } },
            "not": { "has": { "field": "arguments", "has": { "kind": "keyword_argument" } } },
        })
    };
    let same = |receiver: &str, method: &str| {
        json!({ "all": [
            { "compare": { "left": receiver, "op": "eq", "right": "R" } },
            { "compare": { "left": method, "op": "eq", "right": "M" } },
        ] })
    };
    let later = json!({ "all": [
        call("R2", "M2", "A2"),
        same("R2", "M2"),
        { "pattern": "$SECOND" },
        { "compare": { "left": "SECOND", "op": "ne", "right": "FIRST", "text": "node" } },
    ] });
    let earlier = json!({ "all": [call("R3", "M3", "A3"), same("R3", "M3")] });
    let ast = json!({
        "utils": { "or": or },
        "rule": {
            "matches": "or",
            "not": { "inside": { "field": "left", "matches": "or" } },
            "all": [
                // The group's first call: an operand of this chain (reached through `or`s only)
                // with no operand of the same key before it in the chain.
                { "has": { "stopBy": spine, "all": [
                    call("R", "M", "A"),
                    { "pattern": "$FIRST" },
                    { "not": { "follows": { "stopBy": "end", "any": [
                        earlier.clone(),
                        { "matches": "or", "has": { "stopBy": spine, "all": [earlier] } },
                    ] } } },
                ] } },
                // Another operand of the same key.
                { "has": { "stopBy": spine, "all": [later] } },
            ],
        },
        "constraints": { "M": { "regex": "^(startswith|endswith)$" } },
    });
    let source = oracle!("python/value/ruff/PIE810-each.py");
    let mut once = expected("PIE810-each");
    once.dedup();
    assert_eq!(each_starts(ast.clone(), None, source), once);
    assert_eq!(
        each_starts(ast, Some("FIRST"), source),
        expected("PIE810-each")
    );
    // The Ruff suite's shape: one utility binds a call's key (`$R`, `$M`), and the negation beside
    // the `matches` that binds it names the same utility, so it reads the key rather than binding
    // a fresh one. Written in any order, the negation runs after the `matches`.
    let key = call("R", "M", "_");
    let ast = json!({
        "utils": { "or": or, "call": key },
        "rule": {
            "matches": "or",
            "not": { "inside": { "field": "left", "matches": "or" } },
            "all": [
                { "has": { "stopBy": spine, "all": [
                    { "not": { "follows": { "stopBy": "end", "any": [
                        { "matches": "call" },
                        { "matches": "or", "has": { "stopBy": spine, "matches": "call" } },
                    ] } } },
                ], "pattern": "$FIRST", "matches": "call" } },
                { "has": { "stopBy": spine, "matches": "call", "pattern": "$SECOND",
                    "compare": { "left": "SECOND", "op": "ne", "right": "FIRST", "text": "node" } } },
            ],
        },
        "constraints": { "M": { "regex": "^(startswith|endswith)$" } },
    });
    assert_eq!(each_starts(ast.clone(), None, source), once);
    assert_eq!(
        each_starts(ast, Some("FIRST"), source),
        expected("PIE810-each")
    );
}

/// W605 in an f-string's format spec (M12b's fix round): tree-sitter keeps the spec's text in one
/// node, so the Ruff suite's port finds an invalid escape there with a regex group (`BS`). Ruff
/// reports each invalid escape; with `each` naming the group, a regex whose successive matches
/// walk the spec's escapes reports one finding per match that binds it.
#[test]
fn w605_reports_each_invalid_escape_in_a_format_spec() {
    let valid = "\\n\\r\\\\'\"abfnrtv0-7xNuU";
    let raw = json!({ "stopBy": "neighbor", "kind": "string_start", "regex": "[rR]" });
    let ast = json!({ "rule": {
        "kind": "format_specifier",
        "regex": format!("^(?:[^\\\\]|\\\\[\\s\\S])*?(?<BS>\\\\)(?:[^{valid}]|$)"),
        "all": [{ "regex": format!("\\\\[{valid}]|(?<BS>\\\\)(?:[^{valid}]|$)") }],
        "inside": { "stopBy": "neighbor", "kind": "interpolation",
            "inside": { "stopBy": "neighbor", "kind": "string", "not": { "has": raw } } },
    } });
    let source = oracle!("python/value/ruff/W605-each.py");
    // Without `each`, the first invalid escape of each spec.
    let firsts = each_starts_at(ast.clone(), "capture.BS", None, source);
    assert_eq!(firsts, [5, 17, 40, 55, 61, 78, 90, 100]);
    assert_eq!(
        each_starts_at(ast, "capture.BS", Some("BS"), source),
        expected("W605-each")
    );
}
