use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::*;
use napi::Result;
use serde_json::{Value, json};

fn compile_clause(language: &str, ast: Value, violation: Value) -> Result<CompiledCodeRuleDsl> {
    let mut ast = ast;
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(
            json!({ "id": "format", "forbid": { "ast": ast }, "violation": violation }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn found(rule: Value, source: &str) -> Vec<String> {
    let compiled = compile_clause("python", json!({ "rule": rule }), violation())
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

fn format_call(test: Value) -> Value {
    json!({ "pattern": "$S.format($$$A)", "formatArgs": test })
}

#[test]
fn unused_arguments_bind_and_missing_fields_export() {
    let source = "'{a}'.format(a=1, b=2)\n'{}{}'.format(1)\n'{0}'.format(1, x)\n";
    // `unused` binds from the first unused argument it matches, in the host environment.
    let rule = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "keyword_argument", "has": { "kind": "identifier", "pattern": "$K" } } }));
    assert_eq!(found(rule, source), ["'{a}'.format(a=1, b=2) K=b S='{a}'"]);
    // `missing` matches the template's fields and exports its captures.
    let rule = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "missing": { "kind": "field", "capture": "F" } }));
    assert_eq!(found(rule, source), ["'{}{}'.format(1) A=1 F={} S='{}{}'"]);
    let rule = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "identifier" } }));
    assert_eq!(found(rule, source), ["'{0}'.format(1, x) S='{0}'"]);
}

#[test]
fn splats_leave_what_they_may_pass_unknown() {
    // With `**kw`, an unpassed keyword may be passed and a passed one may be the only one used:
    // neither is known, so neither holds, negated or not. Positions stay known (Ruff's F523).
    let source = "'{a}'.format(b=1, **kw)\n'{}'.format(1, 2, **kw)\n'{a}'.format(*args)\n";
    let unused = |rule: Value| {
        format_call(json!({ "style": "brace", "string": "S", "args": "A", "unused": rule }))
    };
    assert_eq!(
        found(unused(json!({ "kind": "keyword_argument" })), source),
        Vec::<String>::new()
    );
    assert_eq!(
        found(
            json!({ "pattern": "$S.format($$$A)", "not": { "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "kind": "keyword_argument" } } } }),
            source
        ),
        [
            "'{}'.format(1, 2, **kw) S='{}'",
            "'{a}'.format(*args) A=*args S='{a}'"
        ]
    );
    assert_eq!(
        found(unused(json!({ "kind": "integer" })), source),
        ["'{}'.format(1, 2, **kw) S='{}'"]
    );
    let missing = format_call(
        json!({ "style": "brace", "string": "S", "args": "A", "missing": { "kind": "field" } }),
    );
    assert_eq!(found(missing, source), Vec::<String>::new());
}

#[test]
fn percent_operands_are_read_as_ruff_reads_them() {
    let source = "\
'%(a)s' % {'a': 1, 'b': 2}
'%(a)s %(c)s' % {'a': 1, k: 2}
'%s %s' % (1,)
'%s' % (1, 2)
'%s %s' % 42
'%s %s' % x
'' % x
'%s' % {'a': 1}
";
    let rule = |test: Value| {
        let mut test = test;
        test["style"] = json!("percent");
        test["string"] = json!("S");
        test["args"] = json!("R");
        json!({ "pattern": "$S % $R", "formatArgs": test })
    };
    // A dict display's pairs against the keys; a tuple's elements and a known type's one value
    // against the positions; with no placeholder any operand but a tuple is unused.
    assert_eq!(
        found(rule(json!({ "unused": { "pattern": "$U" } })), source),
        [
            "'%(a)s' % {'a': 1, 'b': 2} R={'a': 1, 'b': 2} S='%(a)s' U='b': 2",
            "'%s' % (1, 2) R=(1, 2) S='%s' U=2",
            "'' % x R=x S='' U=x",
        ]
    );
    // A dynamic key leaves the missing keys unknown; an operand of unknown type the positions.
    assert_eq!(
        found(rule(json!({ "missing": { "capture": "M" } })), source),
        [
            "'%s %s' % (1,) M=%s R=(1,) S='%s %s'",
            "'%s %s' % 42 M=%s R=42 S='%s %s'",
        ]
    );
}

/// Ruff's F507 types the operand (`ResolvedPythonType`) before it falls back to "one value": an
/// operand it types as a tuple (`(1,) + (2,)`) has a length it cannot count, so nothing is known
/// unused or missing, whatever the placeholders; a starred tuple display neither.
#[test]
fn tuple_typed_operands_are_counted_as_ruff_counts_them() {
    let source = "\
'hello' % ((1,) + (2,))
'%s' % ((1,) + (2,))
'%s %s' % (() + ())
'hello' % (*a,)
'hello' % ((1,) * 2)
'hello' % [*a]
'%s %s' % ([1] + [2])
";
    let rule = |test: Value| {
        let mut test = test;
        test["style"] = json!("percent");
        test["string"] = json!("S");
        test["args"] = json!("R");
        json!({ "pattern": "$S % $R", "formatArgs": test })
    };
    assert_eq!(
        found(rule(json!({ "unused": { "pattern": "$U" } })), source),
        [
            "'hello' % ((1,) * 2) R=((1,) * 2) S='hello' U=(1,) * 2",
            "'hello' % [*a] R=[*a] S='hello' U=[*a]",
        ]
    );
    assert_eq!(
        found(rule(json!({ "missing": { "capture": "M" } })), source),
        ["'%s %s' % ([1] + [2]) M=%s R=([1] + [2]) S='%s %s'"]
    );
    // Unknown, not false: negated, the tuple-typed operands do not hold either.
    let negated = json!({ "pattern": "$S % $R", "not": { "formatArgs": { "style": "percent",
        "string": "S", "args": "R", "unused": { "pattern": "$U" } } } });
    assert_eq!(
        found(negated, source),
        ["'%s %s' % ([1] + [2]) R=([1] + [2]) S='%s %s'"]
    );
}

/// Ruff's AST has no parentheses, so `string` is read through them, at any depth; and Ruff's F50x
/// and F52x read a plain `str` literal only, so an f-string's format is unknown, as bytes are.
#[test]
fn the_string_is_read_through_parentheses_and_only_as_a_plain_literal() {
    let percent = json!({ "pattern": "$S % $R", "formatArgs": { "style": "percent", "string": "S",
        "args": "R", "unused": { "kind": "pair" } } });
    let source = "\
('%(a)s') % {'b': 1}
(('%(a)s')) % {'b': 1}
((('%(a)s' '%(c)s'))) % {'b': 1}
'%(a)s' % ({'b': 1})
f'%(a)s' % {'b': 1}
(f'%(a)s') % {'b': 1}
('%(a)s' + x) % {'b': 1}
";
    assert_eq!(
        found(percent, source),
        [
            "('%(a)s') % {'b': 1} R={'b': 1} S=('%(a)s')",
            "(('%(a)s')) % {'b': 1} R={'b': 1} S=(('%(a)s'))",
            "((('%(a)s' '%(c)s'))) % {'b': 1} R={'b': 1} S=((('%(a)s' '%(c)s')))",
            "'%(a)s' % ({'b': 1}) R=({'b': 1}) S='%(a)s'",
        ]
    );
    let negated = json!({ "pattern": "$S % $R", "not": { "formatArgs": { "style": "percent",
        "string": "S", "args": "R", "unused": { "kind": "pair" } } } });
    assert_eq!(
        found(negated, source),
        ["('%(a)s' + x) % {'b': 1} R={'b': 1} S=('%(a)s' + x)"]
    );
    let brace = format_call(
        json!({ "style": "brace", "string": "S", "args": "A", "missing": { "kind": "field" } }),
    );
    assert_eq!(
        found(brace, "(('{a}')).format(b=1)\nf'{{}}'.format()\n"),
        ["(('{a}')).format(b=1) A=b=1 S=(('{a}'))"]
    );
}

#[test]
fn format_args_are_checked() {
    for (rule, expected) in [
        (
            json!({ "formatArgs": { "style": "brace", "string": "S", "args": "A" } }),
            "/clauses/0/forbid/ast/rule/formatArgs must hold unused, missing or both",
        ),
        (
            json!({ "formatArgs": { "style": "c", "string": "S", "args": "A", "unused": { "kind": "identifier" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/style: unknown value `c`; expected brace, percent, printf, scanf or rust",
        ),
        (
            json!({ "formatArgs": { "style": "brace", "args": "A", "unused": { "kind": "identifier" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/string is required",
        ),
        (
            json!({ "formatArgs": { "style": "brace", "string": "$S", "args": "A", "unused": { "kind": "identifier" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/string: `$S` is not a capture name (A-Z, 0-9, _, without `$`)",
        ),
        (
            json!({ "formatArgs": { "style": "brace", "string": "S", "args": "A", "missing": { "kind": "placeholder" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/missing/kind: `placeholder` is not a braceFormat kind; expected one of format, literal, field, field_name, auto, index, name, attribute, element_index, conversion, format_spec, fill, align, sign, alternate, zero, width, grouping, precision, type, spec_error, error",
        ),
        (
            json!({ "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "capture": "U" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/unused/capture: capture binds a node of a parsed tree; use it inside parse (a pattern's metavariables capture host nodes)",
        ),
        (
            json!({ "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "kind": "identifier" }, "extra": 1 } }),
            "unknown field `extra` at /clauses/0/forbid/ast/rule/formatArgs",
        ),
    ] {
        assert_eq!(
            rejection(compile_clause(
                "python",
                json!({ "rule": rule.clone() }),
                violation()
            )),
            expected,
            "{rule}"
        );
    }
    for (rule, expected) in [
        (
            json!({ "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "T", "args": "A", "unused": { "kind": "identifier" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/string: `$T` is not bound by the rule, its utils, or its constraints",
        ),
        (
            json!({ "pattern": "$S.format($A)", "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "kind": "identifier" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/args: a `.format` call's arguments are a list; capture them with `$$$A`",
        ),
        (
            json!({ "pattern": "$$$S % $R", "formatArgs": { "style": "percent", "string": "S", "args": "R", "missing": { "kind": "placeholder" } } }),
            "/clauses/0/forbid/ast/rule/formatArgs/string: `$$$S` captures a node list; the format string is one node",
        ),
        (
            json!({ "pattern": "$S % $R", "has": { "formatArgs": { "style": "percent", "string": "S", "args": "Q", "missing": { "kind": "placeholder" } } } }),
            "/clauses/0/forbid/ast/rule/has/formatArgs/args: `$Q` is not bound by the rule, its utils, or its constraints",
        ),
    ] {
        assert_eq!(
            rejection(compile_clause(
                "python",
                json!({ "rule": rule.clone() }),
                violation()
            )),
            expected,
            "{rule}"
        );
    }
    assert_eq!(
        rejection(compile_clause(
            "typescript",
            json!({ "rule": { "pattern": "f($$$A)", "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "kind": "identifier" } } } }),
            violation()
        )),
        "/clauses/0/forbid/ast/rule/formatArgs: `formatArgs` is not available for typescript policies"
    );
}

/// `unused` is a host rule: a `value` term inside it folds the argument as M9's value rounds fold
/// Python (`1 + 1` is `2`, `"a" * 2` is `"aa"`), and a `compare` reads what `formatArgs` bound.
#[test]
fn unused_arguments_fold_their_values() {
    let source =
        "'{}'.format(1, 'a' * 2)\n'{}'.format(1, 'a' * 3)\n'%s' % (1, 1 + 1)\n'%s' % (1, 3)\n";
    let brace = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "value": { "eq": "aa" } } }));
    assert_eq!(found(brace, source), ["'{}'.format(1, 'a' * 2) S='{}'"]);
    let percent = json!({ "pattern": "$S % $R", "formatArgs": { "style": "percent", "string": "S", "args": "R",
        "unused": { "value": { "eq": 2 }, "pattern": "$U" } } });
    assert_eq!(
        found(percent, source),
        ["'%s' % (1, 1 + 1) R=(1, 1 + 1) S='%s' U=1 + 1"]
    );
    // A comparison reads the unused argument `formatArgs` bound: its value against the statement
    // before the call's.
    let compared = json!({ "compare": { "left": "U", "op": "eq", "right": "V", "text": "value" },
        "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "S", "args": "A",
        "unused": { "pattern": "$U" } },
        "inside": { "kind": "expression_statement", "follows": { "kind": "expression_statement",
            "has": { "pattern": "$V" } } } });
    let source = "2\n'{}'.format(1, 1 + 1)\n3\n'{}'.format(1, 1 + 1)\n";
    assert_eq!(
        found(compared, source),
        ["'{}'.format(1, 1 + 1) S='{}' U=1 + 1 V=2"]
    );
}

/// `formatArgs` and `arguments` read one call each their own way: `arguments`' syntactic mode ends
/// the positions at a starred argument (Ruff's `find_argument`), while `formatArgs` keeps reading
/// positions by syntactic index past it (Ruff's F523).
#[test]
fn format_args_and_arguments_read_one_call() {
    let source = "'{0}'.format(1, *a, 2)\n'{a}'.format(a=1, sep=2)\n";
    let rule = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "integer", "pattern": "$U" } }));
    let mut both = rule.clone();
    both["arguments"] =
        json!({ "signature": ["first", "second"], "bind": { "B": "second" }, "mode": "syntactic" });
    // The first call's `2` is unused; `arguments` binds no second slot there (the star ends
    // the positions), so the rule does not hold.
    assert_eq!(found(rule, source), ["'{0}'.format(1, *a, 2) S='{0}' U=2"]);
    assert_eq!(found(both, source), Vec::<String>::new());
    let keyword = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "keyword_argument", "has": { "kind": "integer", "pattern": "$U" } } }));
    let mut keyword = keyword;
    keyword["arguments"] =
        json!({ "signature": ["*", "sep"], "bind": { "B": "sep" }, "mode": "syntactic" });
    keyword["compare"] = json!({ "left": "U", "op": "eq", "right": "B", "text": "source" });
    assert_eq!(
        found(keyword, source),
        ["'{a}'.format(a=1, sep=2) B=2 S='{a}' U=2"]
    );
    // M9's ESLint readings stay TypeScript's beside `formatArgs`: `estree` arguments and `name`
    // comparisons are refused on Python.
    let mut estree = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "integer" } }));
    estree["arguments"] = json!({ "signature": ["first"], "absent": ["first"], "mode": "estree" });
    assert!(
        rejection(compile_clause("python", json!({ "rule": estree }), violation()))
            .ends_with("/arguments/mode: `estree` reads ESLint's `node.arguments`; use language or syntactic for Python")
    );
    let mut name = format_call(json!({ "style": "brace", "string": "S", "args": "A",
        "unused": { "kind": "keyword_argument", "pattern": "$U" } }));
    name["compare"] = json!({ "left": "U", "op": "eq", "right": "S", "text": "name" });
    assert!(
        rejection(compile_clause("python", json!({ "rule": name }), violation()))
            .ends_with("/compare/text: `name` reads the names JavaScript and PHP identifiers and strings write; use unquoted for Python")
    );
}
