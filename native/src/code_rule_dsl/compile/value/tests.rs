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

/// Each violation as `line:text` (1-based lines).
fn found(result: &Value, source: &str) -> Vec<String> {
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let end = violation["span"]["endByte"].as_u64().expect("end") as usize;
            let line = source[..start].matches('\n').count() + 1;
            format!("{line}:{}", &source[start..end])
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

#[test]
fn eq_compares_decoded_values_whatever_the_spelling() {
    // Bandit B104: binding to every interface.
    let rule = json!({
        "rule": { "pattern": "$S.bind(($HOST, $PORT))" },
        "constraints": { "HOST": { "value": { "eq": "0.0.0.0" } } },
    });
    let source = "\
s.bind(('0.0.0.0', 80))
s.bind((\"0.0.\" \"0.0\", 80))
s.bind(('\\x30.0.0.0', 80))
s.bind(('127.0.0.1', 80))
s.bind((host, 80))
s.bind((b'0.0.0.0', 80))
";
    assert_eq!(
        python_found(rule, source),
        [
            "1:s.bind(('0.0.0.0', 80))",
            "2:s.bind((\"0.0.\" \"0.0\", 80))",
            "3:s.bind(('\\x30.0.0.0', 80))"
        ]
    );
    // Python `==`: 1 == 1.0 == True; JavaScript `Object.is`.
    let one =
        json!({ "rule": { "kind": "expression_statement", "has": { "value": { "eq": 1 } } } });
    assert_eq!(
        python_found(one, "1\n1.0\nTrue\n0x1\n'1'\n2 - 1\n"),
        ["1:1", "2:1.0", "3:True", "4:0x1", "6:2 - 1"]
    );
    let one =
        json!({ "rule": { "kind": "expression_statement", "has": { "value": { "eq": 1 } } } });
    assert_eq!(
        typescript_found(one, "1;\n1.0;\ntrue;\n0x1;\n'1';\n1n;\n"),
        ["1:1;", "2:1.0;", "4:0x1;"]
    );
    let null =
        json!({ "rule": { "kind": "expression_statement", "has": { "value": { "eq": null } } } });
    assert_eq!(
        typescript_found(null, "null;\nundefined;\nvoid 0;\n"),
        ["1:null;"]
    );
}

#[test]
fn predicates_read_types_prefixes_patterns_bits_bounds_and_truthiness() {
    let at = |value: Value| json!({ "rule": { "kind": "expression_statement", "has": { "value": value } } });
    let source = "\
1
2.5
'a'
b'a'
(1, 2)
[x]
None
x
";
    assert_eq!(
        python_found(at(json!({ "type": ["int", "tuple", "list"] })), source),
        ["1:1", "5:(1, 2)", "6:[x]"]
    );
    // `constant` asks whether the whole value is known: `[x]` is a list, not a constant.
    assert_eq!(
        python_found(
            at(json!({ "type": ["list", "int"], "constant": true })),
            source
        ),
        ["1:1"]
    );
    assert_eq!(
        python_found(at(json!({ "constant": false })), source),
        ["6:[x]", "8:x"]
    );
    // B108: temporary directories.
    let tmp = at(json!({ "startsWith": ["/tmp", "/var/tmp"] }));
    let paths = "'/tmp/x'\n'/var/tmp'\n'/home'\nb'/tmp'\nf'/tmp/{x}'\n";
    assert_eq!(python_found(tmp, paths), ["1:'/tmp/x'", "2:'/var/tmp'"]);
    let pow = json!({ "rule": { "kind": "string", "value": { "regex": "^p[o0]w$" } } });
    assert_eq!(
        typescript_found(pow, "'pow';\n'p\\u006Fw';\n`p0w`;\n'powder';\n"),
        ["1:'pow'", "2:'p\\u006Fw'"]
    );
    // B103: a world- or group-writable mode.
    let writable = json!({
        "rule": { "pattern": "os.chmod($P, $MODE)" },
        "constraints": { "MODE": { "value": { "bitsAny": 18 } } },
    });
    let modes = "os.chmod(p, 0o777)\nos.chmod(p, 0o755)\nos.chmod(p, 0o644 | 0o020)\nos.chmod(p, stat.S_IWOTH)\nos.chmod(p, 0x1ff)\n";
    assert_eq!(
        python_found(writable, modes),
        [
            "1:os.chmod(p, 0o777)",
            "3:os.chmod(p, 0o644 | 0o020)",
            "5:os.chmod(p, 0x1ff)"
        ]
    );
    let long = at(json!({ "gt": 86400 }));
    assert_eq!(
        python_found(long, "86401\n86400\n1e5\n24 * 3600 + 1\n'90000'\nTrue\n"),
        ["1:86401", "3:1e5", "4:24 * 3600 + 1"]
    );
    let bounded = at(json!({ "ge": 1, "lt": 2 }));
    assert_eq!(
        typescript_found(bounded, "1;\n1.5;\n2;\n1n;\n'1';\n"),
        ["1:1;", "2:1.5;", "4:1n;"]
    );
    let truthy = at(json!({ "truthy": true }));
    assert_eq!(
        python_found(truthy, "0\n'a'\n[]\n[x]\n{**d}\nx\n...\n"),
        ["2:'a'", "4:[x]", "7:..."]
    );
    let falsy = at(json!({ "truthy": false }));
    assert_eq!(
        typescript_found(falsy, "0;\n'';\nNaN;\n[];\n({});\nnull;\nx;\n"),
        ["1:0;", "2:'';", "3:NaN;", "6:null;"]
    );
}

/// A regex literal is a RegExp object, as getStaticValue reads it: of type `object`, which
/// `regexp` refines, truthy, and equal to nothing but itself.
#[test]
fn a_regex_literal_is_a_regexp_object() {
    let source = "x = /a/;\nx = /b/gu;\nx = 'a';\nx = [];\nx = {};\n";
    let rule = |value: Value| json!({ "rule": { "any": [{ "kind": "regex" }, { "kind": "string" }, { "kind": "array" }, { "kind": "object" }], "value": value } });
    assert_eq!(
        typescript_found(rule(json!({ "type": "object" })), source),
        ["1:/a/", "2:/b/gu", "5:{}"]
    );
    assert_eq!(
        typescript_found(rule(json!({ "type": "regexp" })), source),
        ["1:/a/", "2:/b/gu"]
    );
    assert_eq!(
        typescript_found(
            json!({ "rule": { "kind": "regex", "value": { "truthy": true, "constant": true } } }),
            source
        ),
        ["1:/a/", "2:/b/gu"]
    );
    // Known, so its negations hold: a regex is not a string and equals no JSON value.
    assert_eq!(
        typescript_found(
            json!({ "rule": { "kind": "regex", "not": { "value": { "type": "string" } } } }),
            source
        ),
        ["1:/a/", "2:/b/gu"]
    );
    assert_eq!(
        typescript_found(
            json!({ "rule": { "kind": "regex", "not": { "value": { "eq": "/a/" } } } }),
            source
        ),
        ["1:/a/", "2:/b/gu"]
    );
    // `typeof`, `!` and the short circuits read it; conversions read its string (`/a/gi` in
    // `flags`' order), as an array's do not.
    let folded = "x = typeof /a/;\nx = !/a/;\nx = /a/ || 1;\nx = /a/ig + '';\nx = [/a/] + '';\nx = /a/ <= 1;\n";
    assert_eq!(
        typescript_found(
            json!({ "rule": { "kind": "assignment_expression", "has": { "stopBy": "end", "any": [{ "kind": "unary_expression" }, { "kind": "binary_expression" }], "value": { "constant": true } } } }),
            folded
        ),
        [
            "1:x = typeof /a/",
            "2:x = !/a/",
            "3:x = /a/ || 1",
            "4:x = /a/ig + ''",
            "6:x = /a/ <= 1"
        ]
    );
    assert_eq!(
        typescript_found(
            json!({ "rule": { "kind": "binary_expression", "any": [{ "value": { "eq": "/a/gi" } }, { "value": { "eq": false } }] } }),
            folded
        ),
        ["4:/a/ig + ''", "6:/a/ <= 1"]
    );
}

/// A JSX attribute string is raw: no JavaScript escapes, but HTML character references decoded,
/// as typescript-estree reads its `Literal.value`. ESLint's no-script-url fires on the second.
#[test]
fn a_jsx_attribute_string_is_raw_with_character_references() {
    let rule =
        typescript(json!({ "rule": { "kind": "string", "value": { "regex": "^javascript:" } } }));
    let source = "const a = <a href='\\x6Aavascript:' />;\nconst b = <a href='&#106;avascript:' />;\n\
                  const c = <a href=\"&#x6A;ava&#115;cript&colon;\" />;\nconst d = <a href={'\\x6Aavascript:'} />;\n";
    let result = evaluate_file(&rule, "tsx", "a.tsx", source);
    assert_eq!(
        found(&result, source),
        ["2:'&#106;avascript:'", "4:'\\x6Aavascript:'"]
    );
    let rule = typescript(
        json!({ "rule": { "kind": "string", "value": { "eq": "a\\nb & <c> é \u{1F600}" } } }),
    );
    let source = "<a b='a\\nb &amp; &lt;c&gt; &eacute; &#x1F600;' />;\n";
    let result = evaluate_file(&rule, "tsx", "a.tsx", source);
    assert_eq!(found(&result, source).len(), 1, "{result}");
}

#[test]
fn an_unknown_value_satisfies_and_fails_nothing() {
    // `not` of an unknown value is unknown too: `x` is neither 1 nor not 1.
    let not_one = json!({ "rule": { "kind": "expression_statement", "has": { "kind": "identifier", "not": { "value": { "eq": 1 } } } } });
    assert_eq!(python_found(not_one, "x\n"), Vec::<String>::new());
    let not_one = json!({ "rule": { "kind": "expression_statement", "not": { "has": { "value": { "eq": 1 } } } } });
    assert_eq!(typescript_found(not_one, "2;\nx;\n1;\n"), ["1:2;"]);
    // An unknown operand leaves a sum unknown, but not a short-circuit it never reaches.
    let two =
        json!({ "rule": { "kind": "expression_statement", "has": { "value": { "eq": 2 } } } });
    assert_eq!(python_found(two, "x + 1\n2 or x\nx or 2\n"), ["2:2 or x"]);
}

#[test]
fn builtin_names_are_values_only_where_nothing_shadows_them() {
    let undefined = json!({ "rule": { "kind": "expression_statement", "has": { "value": { "type": "undefined" } } } });
    let source = "undefined;\nfunction f(undefined) {\n  undefined;\n}\nvoid 1;\n";
    assert_eq!(
        typescript_found(undefined, source),
        ["1:undefined;", "5:void 1;"]
    );
    let empty = json!({ "rule": { "kind": "expression_statement", "has": { "value": { "truthy": false } } } });
    let source = "list()\ndef f(list):\n    list()\nset()\nset = frozenset\nset()\n";
    // Module code reads the builtin until the binding; a function body sees every module binding.
    assert_eq!(python_found(empty, source), ["1:list()", "4:set()"]);
    // Language options: a global turned off is no builtin.
    let nan = typescript(json!({
        "rule": { "kind": "expression_statement", "has": { "value": { "type": "number" } } },
        "languageOptions": { "globals": { "NaN": "off" } },
    }));
    let source = "NaN;\nInfinity;\n";
    assert_eq!(
        found(&evaluate_file(&nan, "typescript", "a.ts", source), source),
        ["2:Infinity;"]
    );
}

/// Builtins' calls and member reads fold as getStaticValue folds them, and only where the global
/// they read is unshadowed and declared under the query's language options.
#[test]
fn builtin_calls_fold_only_through_the_unshadowed_global() {
    let rule = |options: Value| {
        let mut ast = json!({ "rule": { "kind": "expression_statement", "has": { "value": { "type": ["string", "number", "bigint"] } } } });
        if !options.is_null() {
            ast["languageOptions"] = options;
        }
        typescript(ast)
    };
    let source = "String('x');\n'POW'.toLowerCase();\nMath.max(1, 2);\n'a'.length;\nBigInt(1);\n\
                  function f(String) {\n  String(1);\n}\nfoo('x');\nMath.random();\nMath.sin(0);\n";
    let lines = |options: Value| {
        found(
            &evaluate_file(&rule(options), "typescript", "a.ts", source),
            source,
        )
    };
    assert_eq!(
        lines(Value::Null),
        [
            "1:String('x');",
            "2:'POW'.toLowerCase();",
            "3:Math.max(1, 2);",
            "4:'a'.length;",
            "5:BigInt(1);"
        ]
    );
    // `BigInt` is no global of ES2015; `Math` turned off is no global at all.
    assert_eq!(
        lines(json!({ "ecmaVersion": 2015, "globals": { "Math": "off" } })),
        ["1:String('x');", "2:'POW'.toLowerCase();", "4:'a'.length;"]
    );
    // The ESLint suite's no-implied-eval invalid/5: the first argument is a string.
    let implied = typescript(
        json!({ "rule": { "pattern": "setTimeout($S, $$$)" }, "constraints": { "S": { "value": { "type": "string" } } } }),
    );
    let source =
        "setTimeout(String('x=1'), 100);\nsetTimeout(String, 100);\nsetTimeout(f('x=1'), 100);\n";
    assert_eq!(
        found(
            &evaluate_file(&implied, "typescript", "a.ts", source),
            source
        ),
        ["1:setTimeout(String('x=1'), 100)"]
    );
}

#[test]
fn a_value_rule_stays_sound_in_a_file_with_syntax_errors() {
    let rule = python(
        json!({ "rule": { "kind": "expression_statement", "has": { "value": { "truthy": false } } } }),
    );
    let source = "list()\n0\ndef f(:\n    pass\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    // `0` is a literal; `list()` needs the bindings, which an error region may hide.
    assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
    let lines = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| violation["span"]["startLine"].clone())
        .collect::<Vec<_>>();
    // Zero-based: the `0` on the second line.
    assert_eq!(lines, [json!(1)], "{result}");
}

#[test]
fn decoding_spends_the_visited_node_budget() {
    let document = clause_document(json!({
        "id": "ast-match",
        "violation": violation(),
        "forbid": { "ast": {
            "language": "python",
            "rule": { "kind": "binary_operator", "value": { "gt": 1000 } },
        } },
    }));
    let capabilities =
        capabilities_with(|capabilities| capabilities["limits"]["maxVisitedNodes"] = json!(3_000));
    let rule = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    // Every `+` is a root and decodes the sum to its left again: quadratic in the terms.
    let source = format!("{}\n", vec!["1"; 200].join(" + "));
    let result = evaluate_file(&rule, "python", "a.py", &source);
    assert_eq!(
        result["failures"],
        json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Value.message(), "path": "a.py" }]),
        "{result}"
    );
    let result = evaluate_file(&rule, "python", "a.py", "1000 + 1\n");
    assert_violations(&result, 1);
}

#[test]
fn value_objects_are_checked() {
    let reject = |value: Value| {
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "string", "value": value } }),
        ))
    };
    assert_eq!(
        reject(json!({ "eq": 511, "follow": 1 })),
        "/clauses/0/forbid/ast/rule/value/follow must be true or false"
    );
    assert_eq!(
        reject(json!({ "follow": true })),
        "/clauses/0/forbid/ast/rule/value must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy"
    );
    assert_eq!(
        reject(json!({})),
        "/clauses/0/forbid/ast/rule/value must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy"
    );
    assert_eq!(
        reject(json!({ "eq": [1] })),
        "/clauses/0/forbid/ast/rule/value/eq must be a string, number, boolean or null"
    );
    assert_eq!(
        reject(json!({ "type": "string" })),
        "/clauses/0/forbid/ast/rule/value/type/0: `string` is not a Python type name; expected one of int, float, complex, str, bytes, bool, NoneType, ellipsis, tuple, list, set, dict"
    );
    assert_eq!(
        reject(json!({ "bitsAny": -1 })),
        "/clauses/0/forbid/ast/rule/value/bitsAny must be a positive integer"
    );
    assert_eq!(
        reject(json!({ "gt": "1" })),
        "/clauses/0/forbid/ast/rule/value/gt must be a number"
    );
    assert_eq!(
        reject(json!({ "regex": "(" })),
        "/clauses/0/forbid/ast/rule/value/regex: Parsing text matcher fails."
    );
    assert_eq!(
        reject(json!({ "startsWith": [] })),
        "/clauses/0/forbid/ast/rule/value/startsWith must not be empty"
    );
    assert_eq!(
        reject(json!({ "truthy": 1 })),
        "/clauses/0/forbid/ast/rule/value/truthy must be true or false"
    );
    assert_eq!(
        reject(json!({ "equals": 1 })),
        "unknown field `equals` at /clauses/0/forbid/ast/rule/value"
    );
    let reason = rejection(compile(
        "typescript",
        json!({ "rule": { "kind": "string", "value": { "type": "str" } } }),
    ));
    assert!(
        reason.contains("`str` is not a TypeScript type name"),
        "{reason}"
    );
}

/// Folding beside M10b's `languageOptions.lib`: the callee is getStaticValue's global when a
/// configured global or a lib variable declares it, whichever namespace the lib gives it
/// (eslint-utils' `findVariable` reads the name, not the namespace), so a lib keeps a builtin a
/// configured `off` drops and adds one the ECMAScript version lacks. A file-local binding still
/// hides it. Each case was checked against ESLint 10.11.0 with typescript-eslint's parser 8.70.1.
#[test]
fn builtin_folding_reads_lib_globals() {
    let lines = |options: Value, source: &str| {
        let rule = typescript(json!({
            "rule": { "kind": "expression_statement", "has": { "value": { "type": ["string", "bigint"] } } },
            "languageOptions": options,
        }));
        found(&evaluate_file(&rule, "typescript", "a.ts", source), source)
    };
    let source = "String(1);\nBigInt(1);\n";
    assert_eq!(
        lines(json!({ "globals": { "String": "off" } }), source),
        ["2:BigInt(1);"]
    );
    // `es5` declares `String` a value, `es2015.core` only a type: either keeps it.
    for lib in ["es5", "es2015.core"] {
        assert_eq!(
            lines(
                json!({ "lib": [lib], "globals": { "String": "off" } }),
                source
            ),
            ["1:String(1);", "2:BigInt(1);"],
            "{lib}"
        );
    }
    // `BigInt` is no global of ES2019; `es2020` declares it, even over a configured `off`.
    assert_eq!(
        lines(json!({ "ecmaVersion": 2019 }), source),
        ["1:String(1);"]
    );
    for options in [
        json!({ "ecmaVersion": 2019, "lib": ["es2020"] }),
        json!({ "ecmaVersion": 2019, "lib": ["es2020"], "globals": { "BigInt": "off" } }),
    ] {
        assert_eq!(
            lines(options.clone(), source),
            ["1:String(1);", "2:BigInt(1);"],
            "{options}"
        );
    }
    assert_eq!(
        lines(
            json!({ "ecmaVersion": 5, "sourceType": "script", "lib": ["es2015.core"] }),
            source
        ),
        ["1:String(1);"]
    );
    // A script's own `var String` merges into the lib variable as a definition: not the builtin.
    assert_eq!(
        lines(
            json!({ "sourceType": "script", "lib": ["es5"] }),
            "var String = 1;\nString(1);\n"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn name_tests_what_a_node_names_escapes_decoded() {
    // ESLint's id-length counts the graphemes of `Identifier.name`: `\u{61}` is the one-letter
    // `a`, which the raw text (six characters) is not. A binding, a property and a private name
    // have names but no value.
    let short = json!({ "name": true, "regex": "^(?:\\P{M}\\p{M}*){1,2}$" });
    let rule = |kind: &str| json!({ "rule": { "kind": kind, "value": short.clone() } });
    let source =
        "const \\u{61} = 1, abc = 2, \\u0062c = 3;\nclass K { #x = 1; yy() {} }\nlet 𠮷 = 4;\n";
    assert_eq!(
        typescript_found(rule("identifier"), source),
        ["1:\\u{61}", "1:\\u0062c", "3:𠮷"]
    );
    assert_eq!(
        typescript_found(rule("property_identifier"), source),
        ["2:yy"]
    );
    assert_eq!(
        typescript_found(rule("private_property_identifier"), source),
        ["2:#x"]
    );
    // eq and startsWith read the name too; a string literal names its value; other nodes none.
    let eq = json!({ "rule": { "any": [{ "kind": "identifier" }, { "kind": "string" }, { "kind": "number" }],
        "value": { "name": true, "eq": "ab" } } });
    assert_eq!(
        typescript_found(eq, "a\\u0062; 'a\\x62'; ab; abc; 1;\n"),
        ["1:a\\u0062", "1:'a\\x62'", "1:ab"]
    );
    let prefix =
        json!({ "rule": { "kind": "identifier", "value": { "name": true, "startsWith": "é" } } });
    assert_eq!(
        typescript_found(prefix, "\\u00e9t\\u00e9; et;\n"),
        ["1:\\u00e9t\\u00e9"]
    );
    for (value, expected) in [
        (
            json!({ "name": true, "truthy": true }),
            "/clauses/0/forbid/ast/rule/value/truthy: `name: true` reads a name, a string, which only eq, startsWith and regex test",
        ),
        (
            json!({ "name": "yes", "eq": "a" }),
            "/clauses/0/forbid/ast/rule/value/name must be true or false",
        ),
        (
            json!({ "name": true }),
            "/clauses/0/forbid/ast/rule/value must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy",
        ),
    ] {
        let error = compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "value": value } }),
        )
        .err()
        .expect("refused");
        assert_eq!(error.reason, expected);
    }
    let error = compile(
        "python",
        json!({ "rule": { "kind": "identifier", "value": { "name": true, "eq": "a" } } }),
    )
    .err()
    .expect("refused");
    assert!(error.reason.contains("/value/name"), "{}", error.reason);
}
