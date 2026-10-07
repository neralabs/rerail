//! `formatArgs` with style `printf` against where PHPStan 2.2.16's `PrintfParametersRule` and
//! `PrintfArrayParametersRule` report on fixtures (`fixtures/php/format_args`).

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const ORACLE: &str = oracle!("php/format_args/phpstan.expected.json");
const FIXTURES: &[(&str, &str)] = &[
    ("calls.php", oracle!("php/format_args/phpstan/calls.php")),
    (
        "constants.php",
        oracle!("php/format_args/phpstan/constants.php"),
    ),
    (
        "defines.php",
        oracle!("php/format_args/phpstan/defines.php"),
    ),
    ("follow.php", oracle!("php/format_args/phpstan/follow.php")),
    (
        "heredocs.php",
        oracle!("php/format_args/phpstan/heredocs.php"),
    ),
    ("shapes.php", oracle!("php/format_args/phpstan/shapes.php")),
];

/// The calls `formatArgs` leaves unknown, by the fixture line each starts on (the only call that
/// starts there), besides a literal format PHP refuses. Those PHPStan does not read either: an
/// `...` unpacking, an interpolated literal, values that are no array literal or an array literal
/// with a key not known, a parameter, `static::X` of a class a subclass may redeclare it in, and
/// a constant or class the file does not declare. Those PHPStan reads and Rerail does not know: a
/// variable whose value differs by path (PHPStan reads the most placeholders any value has), a
/// compound assignment (`.=`), an interpolated string in a variable, `self::class`, a constant
/// PHP defines (`PHP_EOL`), a `define` that may run or not, and a format PHP refuses with a
/// `ValueError` through a variable (PHPStan's "invalid placeholder"; `parse` reads a literal).
const UNDECIDED: &[(&str, &[u64])] = &[
    ("calls.php", &[41, 42, 43, 64, 65, 79, 80]),
    ("constants.php", &[52, 53, 54, 95, 117]),
    ("defines.php", &[21]),
    ("follow.php", &[21, 22, 59, 65, 68, 70, 72, 73]),
    ("heredocs.php", &[19]),
    ("shapes.php", &[41, 43]),
];

/// A node a rule finds: the line (from 1) it starts on, its start byte, its source.
type Found<'s> = (u64, usize, &'s str);

/// The nodes a PHP rule finds in `source`.
fn found<'s>(ast: Value, source: &'s str) -> BTreeSet<Found<'s>> {
    let mut ast = ast;
    ast["language"] = json!("php");
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(
        check_document(json!({ "id": "printf", "forbid": { "ast": ast }, "finding": finding() })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "php", "f.php", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            let at = |key: &str| finding["span"][key].as_u64().expect(key) as usize;
            let (start, end) = (at("startByte"), at("endByte"));
            let line = source[..start].matches('\n').count() as u64 + 1;
            (line, start, &source[start..end])
        })
        .collect()
}

/// The lines (from 1) a PHP rule finds in `source`.
fn lines(ast: Value, source: &str) -> BTreeSet<u64> {
    found(ast, source)
        .into_iter()
        .map(|(line, _, _)| line)
        .collect()
}

/// `formatArgs` over the variadic calls (`sprintf`, `printf`; `$$$A` matches no node after a
/// comma, so a call with the format alone is a pattern of its own) and the array ones
/// (`vsprintf`, `vprintf`), with `side` (`unused` or `missing`) holding `rule`.
fn format_args(side: &str, variadic: Value, array: Value) -> Value {
    let [variadic, array] = format_args_terms(side, variadic, array);
    json!({ "rule": { "any": [variadic, array] } })
}

/// The calls `format_args` reads, each with `formatArgs` negated: those whose format and values
/// are known and have no field or value `side` holding `rule`.
fn format_args_negated(side: &str, variadic: Value, array: Value) -> Value {
    let [mut variadic, mut array] = format_args_terms(side, variadic, array);
    for term in [&mut variadic, &mut array] {
        let test = term["formatArgs"].take();
        let call = term["any"].take();
        *term = json!({ "all": [{ "any": call }, { "not": { "formatArgs": test } }] });
    }
    json!({ "rule": { "any": [variadic, array] } })
}

fn format_args_terms(side: &str, variadic: Value, array: Value) -> [Value; 2] {
    let test = |args: &str, rule: Value| {
        let mut test = json!({ "style": "printf", "string": "F", "args": args });
        test[side] = rule;
        test
    };
    [
        json!({
            "any": [
                { "pattern": "sprintf($F)" }, { "pattern": "printf($F)" },
                { "pattern": "sprintf($F, $$$A)" }, { "pattern": "printf($F, $$$A)" },
            ],
            "formatArgs": test("A", variadic),
        }),
        json!({
            "any": [{ "pattern": "vsprintf($F, $V)" }, { "pattern": "vprintf($F, $V)" }],
            "formatArgs": test("V", array),
        }),
    ]
}

#[test]
fn printf_arguments_match_phpstan() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle");
    let unused = format_args(
        "unused",
        json!({ "kind": "argument" }),
        json!({ "kind": "array_element_initializer" }),
    );
    let decided = format_args_negated(
        "unused",
        json!({ "kind": "argument" }),
        json!({ "kind": "array_element_initializer" }),
    );
    let calls = json!({ "rule": { "any": [
        { "pattern": "sprintf($$$A)" }, { "pattern": "printf($$$A)" },
        { "pattern": "vsprintf($$$A)" }, { "pattern": "vprintf($$$A)" },
    ] } });
    let fields = json!({ "any": [{ "kind": "placeholder" }, { "kind": "star" }] });
    let missing = format_args("missing", fields.clone(), fields);
    let invalid = json!({
        "rule": { "any": [
            { "pattern": "sprintf($F)" }, { "pattern": "printf($F)" },
            { "pattern": "sprintf($F, $$$A)" }, { "pattern": "printf($F, $$$A)" },
            { "pattern": "vsprintf($F, $$$A)" }, { "pattern": "vprintf($F, $$$A)" },
        ] },
        // `$F` captures the argument; `parse` holds on the literal it passes.
        "constraints": { "F": { "has": {
            "parse": { "grammar": "printfFormat", "has": { "kind": "error" } },
        } } },
    });
    let mut reports = 0;
    for (name, source) in FIXTURES {
        let all = found(calls.clone(), source);
        // PHPStan reports a call on the line it starts on. A line reported or undecided starts
        // one call, the one each finding must be: a finding on a call around it or inside it, or
        // an undecided call beside a decided one, is no match.
        let call_on = |line: u64| {
            let on: Vec<_> = all.iter().filter(|call| call.0 == line).collect();
            match on.as_slice() {
                [call] => **call,
                _ => panic!("{name}:{line}: one call starts on the line, not {on:?}"),
            }
        };
        let unknown: BTreeSet<u64> = UNDECIDED
            .iter()
            .filter(|(fixture, _)| fixture == name)
            .flat_map(|(_, lines)| lines.iter().copied())
            .collect();
        let mut expected = [BTreeSet::new(), BTreeSet::new(), BTreeSet::new()];
        for report in oracle[name].as_array().expect("reports") {
            reports += 1;
            let line = report[0].as_u64().expect("line");
            if unknown.contains(&line) {
                continue;
            }
            let side = match (&report[1], &report[2]) {
                (Value::String(_), _) => 2,
                (placeholders, values) if placeholders.as_u64() > values.as_u64() => 1,
                _ => 0,
            };
            expected[side].insert(call_on(line));
        }
        assert_eq!(found(unused.clone(), source), expected[0], "{name}: unused");
        assert_eq!(
            found(missing.clone(), source),
            expected[1],
            "{name}: missing"
        );
        assert_eq!(
            found(invalid.clone(), source),
            expected[2],
            "{name}: invalid"
        );
        // Every other call is decided: its format and values are known, or its format is no
        // string.
        let decided = &found(unused.clone(), source) | &found(decided.clone(), source);
        let undecided = &(&all - &decided) - &expected[2];
        let unknown: BTreeSet<_> = unknown.into_iter().map(call_on).collect();
        assert_eq!(undecided, unknown, "{name}: undecided calls");
    }
    assert!(reports >= 60, "{reports} reports");
}

/// Compiles one ast query in `language`.
fn compile(language: &str, rule: Value) -> napi::Result<crate::policy_dsl::CompiledPolicyDsl> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    compile_document_with(
        check_document(json!({
            "id": "printf",
            "forbid": { "ast": { "language": language, "rule": rule } },
            "finding": { "anchor": "witness.span", "code": "X", "message": "x", "severity": "error" },
        })),
        &capabilities,
    )
}

#[test]
fn a_format_tree_maps_placeholders_to_their_source() {
    let source = "<?php\necho sprintf(\"a \\x25s %d\", $x, $y);\necho sprintf(\"%s $x\", $y);\n";
    let compiled = compile(
        "php",
        json!({
            "kind": "encapsed_string",
            "parse": { "grammar": "printfFormat", "has": { "kind": "placeholder", "value": { "eq": 0 }, "capture": "P" } },
        }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    // The interpolated string's format is unknown: no finding.
    assert_eq!(
        matched_texts(&evaluate_file(&compiled, "php", "f.php", source), source),
        ["\"a \\x25s %d\" P=\\x25s"]
    );
}

#[test]
fn styles_and_grammars_are_their_familys() {
    let format_args = |style: &str| json!({ "pattern": "f($S, $$$A)", "formatArgs": { "style": style, "string": "S", "args": "A", "unused": { "kind": "argument" } } });
    let parse =
        |grammar: &str| json!({ "kind": "string", "parse": { "grammar": grammar, "regex": "." } });
    for (language, rule, expected) in [
        (
            "php",
            parse("percentFormat"),
            "parse/grammar: `percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
        ),
        (
            "php",
            parse("braceFormat"),
            "parse/grammar: `braceFormat` parses Python literals; PHP has no str.format templates (its printf formats are `printfFormat`)",
        ),
        (
            "php",
            parse("annotation"),
            "parse/grammar: `annotation` parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
        ),
        (
            "php",
            parse("eslintDirective"),
            "parse/grammar: `eslintDirective` parses TypeScript comments",
        ),
        (
            "typescript",
            parse("phpDoc"),
            "parse/grammar: `phpDoc` parses PHP doc comments",
        ),
        (
            "python",
            parse("printfFormat"),
            "parse/grammar: `printfFormat` parses PHP strings; Python's printf formats are `percentFormat`",
        ),
        (
            "typescript",
            parse("printfFormat"),
            "parse/grammar: `printfFormat` parses PHP strings",
        ),
        (
            "php",
            format_args("percent"),
            "formatArgs/style: `percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
        ),
        (
            "php",
            format_args("brace"),
            "formatArgs/style: `brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
        ),
        (
            "python",
            format_args("printf"),
            "formatArgs/style: `printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
        ),
    ] {
        assert_eq!(
            rejection(compile(language, rule)),
            format!("/checks/0/forbid/ast/rule/{expected}"),
            "{language}"
        );
    }
}

/// `scanf`: with variables passed, PHP 8.5.11 throws where a placeholder numbers a variable past
/// the last passed or a passed variable is assigned by none (`literal::scanf::tests` holds the
/// runtime oracle); with none, the call returns an array. A format through a variable, a
/// concatenation or a constant throws alike (`php -r`: "Different numbers of variable names and
/// field specifiers" on line 8, "Variable is not assigned by any conversion specifiers" on 9
/// and 11).
#[test]
fn scanf_variables_are_missing_or_unused_as_php_throws() {
    let source = "<?php\nsscanf($s, '%d %d', $a);\nsscanf($s, '%d', $a, $b);\nsscanf($s, '%2$s', $a, $b);\nsscanf($s, '%d %d', $a, $b);\nsscanf($s, '%d %d');\n$f = '%d %d';\nsscanf($s, $f, $a);\nsscanf($s, '%d' . ' %d', $a, $b, $c);\nconst F = '%d';\nsscanf($s, F, $a, $b);\n";
    let rule = |side: &str, rule: Value| {
        let mut test = json!({ "style": "scanf", "string": "F", "args": "A" });
        test[side] = rule;
        json!({ "rule": { "any": [{ "pattern": "sscanf($S, $F)" }, { "pattern": "sscanf($S, $F, $$$A)" }], "formatArgs": test } })
    };
    assert_eq!(
        lines(rule("missing", json!({ "kind": "placeholder" })), source),
        BTreeSet::from([2, 8])
    );
    assert_eq!(
        lines(rule("unused", json!({ "kind": "argument" })), source),
        BTreeSet::from([3, 4, 9, 11])
    );
}
