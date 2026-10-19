//! The semantic leaves whose sub-rules the SQL engine composes (`terminals::leaves`): both engines
//! must report the same findings, and each test pins the native answer so it exercises the path
//! it names.

use super::{same, snapshot};
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use serde_json::{Value, json};

/// Both engines' findings for `ast` over `files` (each `(path, source)` in `language`), which must
/// agree: each `path:line:text MESSAGE NAME=text…` (a list capture's nodes joined by `|`).
fn found(language: &str, mut ast: Value, message: &str, files: &[(&str, &str)]) -> Vec<String> {
    ast["language"] = json!(language);
    let compiled = compile_document_with(
        clause_document(json!({
            "id": "leaf",
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "X", "message": message },
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let input: Vec<(&str, &str, &str)> = files
        .iter()
        .map(|(path, source)| (language, *path, *source))
        .collect();
    let result = same(&compiled, &snapshot(&input));
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let path = violation["path"].as_str().expect("path");
            let source = files
                .iter()
                .find(|(name, _)| *name == path)
                .map(|(_, source)| *source)
                .expect("a file of the snapshot");
            let text = |span: &Value| {
                let start = span["startByte"].as_u64().expect("start") as usize;
                let end = span["endByte"].as_u64().expect("end") as usize;
                &source[start..end]
            };
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let line = source[..start].matches('\n').count() + 1;
            let mut parts = vec![format!("{path}:{line}:{}", text(&violation["span"]))];
            if message != "x" {
                parts.push(violation["message"].as_str().expect("message").to_owned());
            }
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

fn typescript(ast: Value, source: &str) -> Vec<String> {
    found("typescript", ast, "x", &[("a.ts", source)])
}

fn python(ast: Value, source: &str) -> Vec<String> {
    found("python", ast, "x", &[("a.py", source)])
}

fn php(ast: Value, source: &str) -> Vec<String> {
    found("php", ast, "x", &[("a.php", source)])
}

/// A clause's rule: `{ "rule": rule }`.
fn rule(rule: Value) -> Value {
    json!({ "rule": rule })
}

/// `Op::Seq` three-valued: a binding's site, value and annotation sub-rules are read in order, and
/// the first that is not true decides. A site whose sub-rule is unknown leaves the binding unknown
/// though its value's sub-rule is false (where a three-valued `and` would be false), so the
/// negation holds at neither reference of `a`; at `b` the site holds and the value fails.
#[test]
fn a_bindings_sub_rules_are_read_in_order_and_the_first_not_true_decides() {
    let source = "var a = 1;\nvar b = 2;\nf(a, b);\n";
    let negated = |binding: Value| {
        rule(
            json!({ "kind": "identifier", "inside": { "kind": "arguments" },
            "not": { "resolves": { "binding": binding } } }),
        )
    };
    assert_eq!(
        typescript(
            negated(json!({ "site": { "testUnknown": "a = 1" }, "value": { "kind": "string" } })),
            source
        ),
        ["a.ts:3:b"]
    );
    // The value's unknown sub-rule is never read where the site's fails.
    assert_eq!(
        typescript(
            negated(json!({ "site": { "regex": "^b" }, "value": { "testUnknown": "1" } })),
            source
        ),
        ["a.ts:3:a"]
    );
    // `binds` reads its candidate's sub-rules alike.
    let binds = |binds: Value| {
        rule(
            json!({ "kind": "identifier", "inside": { "kind": "variable_declarator" },
            "not": { "binds": binds } }),
        )
    };
    assert_eq!(
        typescript(
            binds(json!({ "site": { "testUnknown": "a = 1" }, "value": { "kind": "string" } })),
            source
        ),
        ["a.ts:2:b"]
    );
}

/// `Op::Every` three-valued: `redefines.every` holds where every other definition passes; one
/// whose sub-rule is unknown leaves it unknown unless another fails.
#[test]
fn redefines_every_is_unknown_at_an_unknown_definition_unless_another_fails() {
    let source = "var a = 1;\nvar a = 2;\nvar a = 3;\n";
    let every = |site: Value| {
        rule(
            json!({ "kind": "identifier", "inside": { "kind": "variable_declarator" },
            "not": { "binds": { "redefines": { "every": true, "site": site } } } }),
        )
    };
    // Each definition's others include `a = 2` (unknown) except `a = 2`'s own, whose others pass.
    assert_eq!(
        typescript(every(json!({ "testUnknown": "a = 2" })), source),
        Vec::<String>::new()
    );
    // `a = 3` fails `regex`. The others are read in order and the first not passing decides:
    // `a = 1`'s reach the unknown `a = 2` first, `a = 3`'s too; `a = 2`'s reach the failing one.
    assert_eq!(
        typescript(
            every(json!({ "testUnknown": "a = 2", "regex": "^a = [12]" })),
            source
        ),
        ["a.ts:2:a"]
    );
    // The earlier definitions: some one passes, past an unknown one.
    let earlier = |site: Value| {
        rule(
            json!({ "kind": "identifier", "inside": { "kind": "variable_declarator" },
            "not": { "all": [{ "binds": { "redefines": { "site": site } } }, { "regex": "^never$" }] } }),
        )
    };
    assert_eq!(
        typescript(earlier(json!({ "testUnknown": "a = 1" })), source),
        ["a.ts:1:a", "a.ts:2:a", "a.ts:3:a"]
    );
}

/// `resolves.binding`'s `occurrence` binds at the first occurrence where it matches, in the
/// variable's order, and `binds`' alike.
#[test]
fn an_occurrence_binds_at_the_first_occurrence_where_it_matches() {
    let source = "let _a, b;\nfoo(() => { [_a, b] = x; b; });\nlet _c;\nexport { b };\n";
    let captured = rule(json!({ "kind": "identifier", "resolves": { "binding": {
        "occurrence": { "pattern": "$N", "inside": { "kind": "array_pattern" } },
    } } }));
    assert_eq!(
        typescript(captured, source),
        [
            "a.ts:2:_a N=_a",
            "a.ts:2:b N=b",
            "a.ts:2:b N=b",
            "a.ts:4:b N=b"
        ]
    );
    let binds = rule(json!({ "kind": "identifier", "binds": {
        "occurrence": { "pattern": "$N", "inside": { "stopBy": "end", "kind": "statement_block" } },
    } }));
    assert_eq!(
        typescript(binds, source),
        ["a.ts:1:_a N=_a", "a.ts:1:b N=b"]
    );
}

/// `redefines` over TypeScript's other definitions, `every` included, and Python's shadowed
/// binding, whose sub-rules may bind.
#[test]
fn redefines_reads_the_other_definitions_or_the_shadowed_binding() {
    let source = "interface A {}\ninterface A {}\nclass B {}\ninterface B {}\nclass B {}\n";
    let redefines = |redefines: Value| {
        rule(json!({ "kind": "type_identifier", "binds": { "redefines": redefines } }))
    };
    assert_eq!(
        typescript(
            redefines(json!({ "site": { "kind": "class_declaration" } })),
            source
        ),
        ["a.ts:4:B", "a.ts:5:B"]
    );
    assert_eq!(
        typescript(
            redefines(json!({ "every": true, "site": { "kind": "interface_declaration" } })),
            source
        ),
        ["a.ts:1:A", "a.ts:2:A"]
    );
    let source = "x = 1\nx = 2\ny = 'a'\ny = 3\n";
    let shadowed = rule(
        json!({ "kind": "identifier", "binds": { "form": "assignment",
        "redefines": { "site": { "kind": "assignment", "has": { "pattern": "$OLD", "kind": "integer" } } } } }),
    );
    assert_eq!(python(shadowed, source), ["a.py:2:x OLD=1"]);
}

/// TypeScript's private members: one declaration read alone, its occurrences, and `redefines`
/// over the member's other declarations, `every` included.
#[test]
fn private_members_read_their_declarations_and_occurrences() {
    let source = "\
class A {
  #field = 1;
  #written;
  #method() {}
  get #pair() { return 1; }
  set #pair(v) {}
  #tested;
  #never: number;
  m(o) { this.#field; this.#written = 2; this.#written += 1; this.#method(); #tested in o; this.#pair = 1; }
}
";
    let run = |binds: Value| {
        typescript(
            rule(json!({ "kind": "private_property_identifier", "binds": binds })),
            source,
        )
    };
    assert_eq!(run(json!({ "redefines": {} })), ["a.ts:6:#pair"]);
    assert_eq!(
        run(json!({ "redefines": { "every": true, "site": { "kind": "method_definition" } } })),
        [
            "a.ts:2:#field",
            "a.ts:3:#written",
            "a.ts:4:#method",
            "a.ts:5:#pair",
            "a.ts:6:#pair",
            "a.ts:7:#tested",
            "a.ts:8:#never"
        ]
    );
    assert_eq!(
        run(json!({ "occurrence": { "inside": { "kind": "binary_expression" } } })),
        ["a.ts:7:#tested"]
    );
    assert_eq!(
        run(json!({ "site": { "pattern": "$S", "kind": "method_definition" } })),
        [
            "a.ts:4:#method S=#method() {}",
            "a.ts:5:#pair S=get #pair() { return 1; }",
            "a.ts:6:#pair S=set #pair(v) {}"
        ]
    );
    // `nestedUse` has no reading for a private name: unknown, so neither it nor its negation.
    assert_eq!(run(json!({ "nestedUse": false })), Vec::<String>::new());
    let source = "class C {\n  get #p() { return 1; }\n  set #p(v) {}\n  #p = 2;\n}\n";
    assert_eq!(
        typescript(
            rule(
                json!({ "kind": "private_property_identifier", "not": { "all": [
                { "binds": { "redefines": { "site": { "testUnknown": "^get" } } } },
                { "regex": "^never$" },
            ] } })
            ),
            source
        ),
        ["a.ts:2:#p", "a.ts:3:#p", "a.ts:4:#p"]
    );
}

/// PHP's `binds` reads its variables' flow: the bindings a binding overwrites, `every` included,
/// and every node naming the variable.
#[test]
fn php_binds_reads_the_flow() {
    const SOURCE: &str = "<?php
function f(bool $c, array $items) {
    global $config;
    $config = 1;
    $limit = 10;
    $limit = 20;
    [$a, $b] = [1, 2];
    echo $a, $b, $limit, $config;
    if ($c) {
        global $maybe;
    }
    $maybe = 1;
    $once = 5;
    echo $once;
    foreach ($items as $item) {
        $last = $item;
    }
    echo $last ?? null;
    $fn = fn() => $once + $item;
}
";
    let binds = |binds: Value| {
        php(
            rule(json!({ "kind": "variable_name", "binds": binds })),
            SOURCE,
        )
    };
    assert_eq!(
        binds(json!({ "redefines": { "used": false } })),
        ["a.php:6:$limit"]
    );
    assert_eq!(
        binds(json!({ "redefines": { "form": "global" } })),
        ["a.php:4:$config", "a.php:12:$maybe"]
    );
    assert_eq!(
        binds(
            json!({ "form": "assignment", "redefines": { "sameScope": true, "every": true, "form": "assignment" } })
        ),
        ["a.php:6:$limit", "a.php:16:$last"]
    );
    assert_eq!(
        binds(json!({ "redefines": { "site": { "pattern": "$OLD" } } })),
        [
            "a.php:4:$config OLD=global $config;",
            "a.php:6:$limit OLD=$limit = 10",
            "a.php:12:$maybe OLD=global $maybe;",
            "a.php:15:$item OLD=foreach ($items as $item) {\n        $last = $item;\n    }",
            "a.php:16:$last OLD=$last = $item"
        ]
    );
    assert_eq!(
        binds(
            json!({ "form": ["assignment", "for"], "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } } })
        ),
        ["a.php:13:$once", "a.php:15:$item"]
    );
}

/// `arguments`: a spread argument leaves a slot unknown (ESTree's positions aside), so neither the
/// test nor its negation holds there, and `bind` binds the argument's node.
#[test]
fn arguments_with_a_spread_are_unknown() {
    let source = "f(x, y);\nf(...x, y);\nf(x, ...y);\nf(...x);\nf`t${x}`;\n";
    let bind = |mode: &str| {
        rule(json!({ "kind": "call_expression",
            "arguments": { "signature": ["a", "b"], "bind": { "B": "b" }, "mode": mode } }))
    };
    assert_eq!(
        typescript(bind("estree"), source),
        [
            "a.ts:1:f(x, y) B=y",
            "a.ts:2:f(...x, y) B=y",
            "a.ts:3:f(x, ...y) B=...y"
        ]
    );
    assert_eq!(typescript(bind("language"), source), ["a.ts:1:f(x, y) B=y"]);
    let negated = |mode: &str| {
        rule(json!({ "kind": "call_expression",
            "not": { "arguments": { "signature": ["a", "b"], "bind": { "B": "b" }, "mode": mode } } }))
    };
    assert_eq!(
        typescript(negated("estree"), source),
        ["a.ts:4:f(...x)", "a.ts:5:f`t${x}`"]
    );
    // In the language's reading a spread (and the tagged template) leaves the slots unknown.
    assert_eq!(
        typescript(negated("language"), source),
        Vec::<String>::new()
    );
    let python_source = "f(1, *rest)\nf(1, 2)\nf(**kw)\n";
    let keyword = rule(json!({ "kind": "call", "not": {
        "arguments": { "signature": ["a", "b"], "absent": ["b"] } } }));
    assert_eq!(python(keyword, python_source), ["a.py:2:f(1, 2)"]);
}

/// `parse` exports a capture of the parsed tree to the host, as its text and host range; the trees
/// of several files are documents numbered past the batch's files.
#[test]
fn parse_exports_captures_of_trees_numbered_past_the_files() {
    let ast = rule(
        json!({ "kind": "string", "parse": { "grammar": "percentFormat",
        "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } }),
    );
    let files = [
        ("a.py", "print('%(a)s %(b)s' % d)\n"),
        ("b.py", "x = 1\n"),
        ("c.py", "'%(c)s' % e\n'%s' % f\n"),
    ];
    assert_eq!(
        found("python", ast, "key $K", &files),
        ["a.py:1:'%(a)s %(b)s' key a K=a", "c.py:1:'%(c)s' key c K=c"]
    );
}

/// `parse` of an annotation: the tree is the scope table's, and its names `resolves` in the
/// file's table.
#[test]
fn an_annotations_names_resolve_in_its_files_table() {
    let strings = json!([{ "kind": "string" }, { "kind": "concatenated_string" }]);
    let ast = rule(
        json!({ "any": strings, "parse": { "grammar": "annotation", "has": {
        "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N", "stopBy": "end" } } }),
    );
    let files = [
        (
            "a.py",
            "from typing import List\nx: \"List[Missing]\"\ny: \"List[\\x4dissing]\"\nz: \"List\" \"[Missing]\"\n",
        ),
        (
            "b.py",
            "def f(a: \"Known\", b: \"Gone\"):\n    pass\nclass Known:\n    pass\n",
        ),
    ];
    // An escaped or concatenated string's name spans the whole literal in the host.
    assert_eq!(
        found("python", ast, "$N", &files),
        [
            "a.py:2:\"List[Missing]\" Missing N=Missing",
            "a.py:3:\"List[\\x4dissing]\" Missing N=\"List[\\x4dissing]\"",
            "a.py:4:\"List\" \"[Missing]\" Missing N=\"List\" \"[Missing]\"",
            "b.py:1:\"Gone\" Gone N=Gone"
        ]
    );
}

/// `host` tests the literal a parse holds with a host rule, reading the tree's captures, inside
/// the tree's own search; over several files.
#[test]
fn a_host_rule_tests_each_candidate_item() {
    let declared = json!({ "inside": { "kind": "program", "has": { "stopBy": "end", "kind": "variable_declarator",
        "has": { "field": "name", "pattern": "$V" },
        "compare": { "left": "V", "op": "eq", "right": "N", "text": "name" } } } });
    let items = |item: Value| {
        rule(
            json!({ "kind": "comment", "parse": { "grammar": "eslintDirective",
            "value": { "regex": "^globals?$" }, "has": item } }),
        )
    };
    let files = [
        ("a.ts", "/*global b, a, c*/ var a, c;\n"),
        ("b.ts", "var q;\n"),
        ("c.ts", "/* global z */ var y;\n/* global y */\n"),
    ];
    assert_eq!(
        found(
            "typescript",
            items(json!({ "kind": "item", "capture": "N", "host": declared.clone() })),
            "$N",
            &files
        ),
        [
            "a.ts:1:/*global b, a, c*/ a N=a",
            "c.ts:2:/* global y */ y N=y"
        ]
    );
    assert_eq!(
        found(
            "typescript",
            items(json!({ "kind": "item", "capture": "N", "not": { "host": declared } })),
            "$N",
            &files
        ),
        [
            "a.ts:1:/*global b, a, c*/ b N=b",
            "c.ts:1:/* global z */ z N=z"
        ]
    );
}

/// `formatArgs`: `unused` binds the first argument no field consumes into the host; `missing`
/// searches the template's fields no argument fills and exports what it binds; an incomplete side
/// leaves a miss unknown.
#[test]
fn format_args_unused_and_missing() {
    let unused = rule(
        json!({ "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace",
        "string": "S", "args": "A", "unused": { "pattern": "$U" } } }),
    );
    assert_eq!(
        python(unused, "'{}'.format(1, 2, x)\n'{}'.format(1)\n"),
        ["a.py:1:'{}'.format(1, 2, x) A=1|2|x S='{}' U=2"]
    );
    let missing = rule(
        json!({ "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace",
        "string": "S", "args": "A", "missing": { "kind": "field", "capture": "F" } } }),
    );
    assert_eq!(
        python(
            missing,
            "'{}{}{}'.format(1)\n'{a}'.format(**kw)\n'{}'.format(1)\n"
        ),
        ["a.py:1:'{}{}{}'.format(1) A=1 F={} S='{}{}{}'"]
    );
    // A `**` splat may fill any field: unknown, so the negation does not hold there either.
    let negated = rule(
        json!({ "pattern": "$S.format($$$A)", "not": { "formatArgs": { "style": "brace",
        "string": "S", "args": "A", "missing": { "kind": "field" } } } }),
    );
    assert_eq!(
        python(
            negated,
            "'{}{}{}'.format(1)\n'{a}'.format(**kw)\n'{}'.format(1)\n"
        ),
        ["a.py:3:'{}'.format(1) A=1 S='{}'"]
    );
}

/// `calls` with `chain`: a call reaching blocking I/O through the workspace's call graph binds its
/// definition chain, read by the message.
#[test]
fn a_reaching_call_binds_its_chain() {
    let mut ast = rule(crate::code_rule_dsl::calls::tests::blocking_rule());
    ast["rule"]["has"]["calls"]["chain"] = json!("CHAIN");
    let files: Vec<(&str, &str)> = crate::code_rule_dsl::calls::tests::CHAIN
        .iter()
        .map(|(path, _, source)| (*path, *source))
        .collect();
    assert_eq!(
        found("python", ast, "blocks via $CHAIN", &files),
        [
            "app/api.py:3:async def handler():\n    fetch() blocks via app.services.impl.fetch -> app.util.pause CHAIN=fetch()"
        ]
    );
}
