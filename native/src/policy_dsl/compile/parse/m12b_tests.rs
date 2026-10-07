//! M12b's keys over M11a's parsed literals: `finding.each` and `fix.range` read a parsed tree's
//! exported captures by their host ranges, a `transform` reads them as text, and `commute` stays
//! with `tokens`.

use crate::policy_dsl::CompiledPolicyDsl;
use crate::policy_dsl::test_support::*;
use napi::Result;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn compile_check(language: &str, ast: Value, finding: Value) -> Result<CompiledPolicyDsl> {
    let mut ast = ast;
    ast["language"] = json!(language);
    compile_document_with(
        check_document(json!({ "id": "m12b", "forbid": { "ast": ast }, "finding": finding })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn findings(ast: Value, finding: Value, source: &str) -> Vec<Value> {
    let compiled =
        compile_check("python", ast, finding).unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["findings"].as_array().expect("findings").clone()
}

/// A finding's span as source text.
fn spanned<'s>(finding: &Value, source: &'s str) -> &'s str {
    let at = |name: &str| finding["span"][name].as_u64().expect(name) as usize;
    &source[at("startByte")..at("endByte")]
}

fn anchored(each: Option<&str>, anchor: &str, message: &str) -> Value {
    let mut finding =
        json!({ "anchor": anchor, "code": "X", "message": message, "severity": "error" });
    if let Some(each) = each {
        finding["each"] = json!(each);
    }
    finding
}

/// Each finding's anchored text, and the number of distinct ids.
fn each_texts(ast: Value, each: Option<&str>, anchor: &str, source: &str) -> (Vec<String>, usize) {
    let found = findings(ast, anchored(each, anchor, "m"), source);
    let mut texts = found
        .iter()
        .map(|finding| spanned(finding, source).to_owned())
        .collect::<Vec<_>>();
    texts.sort();
    let ids = found
        .iter()
        .map(|finding| finding["id"].as_str().expect("id").to_owned())
        .collect::<BTreeSet<_>>();
    (texts, ids.len())
}

#[test]
fn each_enumerates_a_format_strings_placeholders_by_host_range() {
    // `parse` with `capture`: one finding per placeholder, anchored at it, each with its own id.
    let placeholders = json!({ "rule": { "kind": "string", "parse": { "grammar": "percentFormat",
        "has": { "kind": "placeholder", "stopBy": "end", "capture": "F" } } } });
    let source = "'%s and %(k)d or %r' % x\n";
    assert_eq!(
        each_texts(placeholders.clone(), Some("F"), "capture.F", source),
        (
            vec!["%(k)d".to_owned(), "%r".to_owned(), "%s".to_owned()],
            3
        )
    );
    assert_eq!(
        each_texts(placeholders, None, "capture.F", source),
        (vec!["%s".to_owned()], 1)
    );
    // Escapes decode in the tree; the enumeration reads the placeholders' spans in the host.
    let escaped = json!({ "rule": { "kind": "string", "parse": { "grammar": "braceFormat",
        "has": { "kind": "field", "capture": "F" } } } });
    let source = "'{a}\\x7bb} {c}'\n";
    assert_eq!(
        each_texts(escaped, Some("F"), "capture.F", source),
        (
            vec!["\\x7bb}".to_owned(), "{a}".to_owned(), "{c}".to_owned()],
            3
        )
    );
    // `formatArgs`' `missing` exports each field no argument fills, `unused` binds each argument
    // no field consumes: both choose among candidates, so both enumerate.
    let missing = json!({ "rule": { "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace",
        "string": "S", "args": "A", "missing": { "kind": "field", "capture": "F" } } } });
    let source = "'{}{}{}'.format(1)\n";
    assert_eq!(
        each_texts(missing.clone(), Some("F"), "capture.F", source),
        (vec!["{}".to_owned(), "{}".to_owned()], 2)
    );
    let found = findings(missing, anchored(Some("F"), "capture.F", "m"), source);
    let starts = found
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<BTreeSet<_>>();
    assert_eq!(starts, BTreeSet::from([3, 5]));
    let unused = json!({ "rule": { "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace",
        "string": "S", "args": "A", "unused": { "pattern": "$U" } } } });
    assert_eq!(
        each_texts(unused, Some("U"), "capture.U", "'{}'.format(1, 2, x)\n"),
        (vec!["2".to_owned(), "x".to_owned()], 2)
    );
}

#[test]
fn a_fix_range_replaces_an_exported_capture_in_the_host() {
    let ast = json!({ "rule": { "kind": "string", "parse": { "grammar": "percentFormat",
        "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } } });
    let fix = json!({ "anchor": "witness.span", "code": "X", "message": "key $K", "severity": "error",
        "fix": { "template": "b", "range": "capture.K" } });
    // The key's host range, escapes and all, inside the string.
    for (source, span, message) in [
        ("'%(a)s' % d\n", (3, 4), "key a"),
        ("'%(\\x61)s' % d\n", (3, 7), "key a"),
    ] {
        let found = findings(ast.clone(), fix.clone(), source);
        assert_eq!(found.len(), 1, "{found:?}");
        let fix = &found[0]["fix"];
        assert_eq!(fix["text"], "b");
        assert_eq!(
            (
                fix["span"]["startByte"].as_u64(),
                fix["span"]["endByte"].as_u64()
            ),
            (Some(span.0), Some(span.1))
        );
        assert_eq!(found[0]["message"], message);
    }
    // With `each`, every key gets its own fix.
    let mut each = fix;
    each["each"] = json!("K");
    let found = findings(ast, each, "'%(a)s %(bb)s' % d\n");
    let mut spans = found
        .iter()
        .map(|finding| {
            (
                finding["fix"]["span"]["startByte"].as_u64(),
                finding["fix"]["span"]["endByte"].as_u64(),
            )
        })
        .collect::<Vec<_>>();
    spans.sort();
    assert_eq!(spans, [(Some(3), Some(4)), (Some(9), Some(11))]);
}

/// A capture a `parse` exports through another `parse` in a sibling `any` branch of a recursive
/// utility, or through a chain of utilities each holding one `parse`, reaches the host at its own
/// range: the anchor, `each` and `fix.range` read it there.
#[test]
fn captures_exported_through_nested_parses_in_utilities_keep_their_host_range() {
    let recursive = json!({
        "rule": { "kind": "string", "matches": "names" },
        "utils": { "names": { "any": [
            { "parse": { "grammar": "annotation", "has": { "stopBy": "end", "kind": "identifier",
                         "regex": "^(foo|baz)$", "capture": "N" } } },
            { "parse": { "grammar": "annotation", "has": { "stopBy": "end", "kind": "string", "matches": "names" } } },
        ] } },
    });
    let source = "x: \"list['foo', 'baz']\"\n";
    assert_eq!(
        each_texts(recursive.clone(), None, "capture.N", source),
        (vec!["foo".to_owned()], 1)
    );
    assert_eq!(
        each_texts(recursive.clone(), Some("N"), "capture.N", source),
        (vec!["baz".to_owned(), "foo".to_owned()], 2)
    );
    let fix = json!({ "anchor": "witness.span", "code": "X", "message": "name $N", "severity": "error",
        "each": "N", "fix": { "template": "qux", "range": "capture.N" } });
    let mut fixes = findings(recursive, fix, source)
        .iter()
        .map(|finding| {
            (
                spanned(&finding["fix"], source).to_owned(),
                finding["fix"]["text"].as_str().expect("text").to_owned(),
                finding["message"].as_str().expect("message").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    fixes.sort();
    assert_eq!(
        fixes,
        [
            ("baz".to_owned(), "qux".to_owned(), "name baz".to_owned()),
            ("foo".to_owned(), "qux".to_owned(), "name foo".to_owned()),
        ]
    );
    // Three utilities, each a string's `parse` reaching the next: the first is compiled before
    // the others, whose exports it passes on.
    let parse = |inner: Value| json!({ "parse": { "grammar": "annotation", "has": inner } });
    let chain = json!({
        "rule": { "kind": "string", "matches": "a" },
        "utils": {
            "a": parse(json!({ "stopBy": "end", "kind": "string", "matches": "b" })),
            "b": parse(json!({ "stopBy": "end", "kind": "string", "matches": "c" })),
            "c": parse(json!({ "stopBy": "end", "kind": "identifier", "regex": "^foo$", "capture": "N" })),
        },
    });
    let source = "x: \"\"\"list['list[\"foo\"]']\"\"\"\n";
    assert_eq!(
        each_texts(chain, Some("N"), "capture.N", source),
        (vec!["foo".to_owned()], 1)
    );
}

#[test]
fn a_transform_reads_format_args_and_parse_exports_as_text() {
    // A missing field's text, unwrapped by a transform, in the message.
    let ast = json!({
        "rule": { "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "S",
            "args": "A", "missing": { "kind": "field", "capture": "F" } } },
        "transform": { "N": { "replace": { "source": "$F", "replace": "^\\{|\\}$", "by": "" } },
                       "U": { "convert": { "source": "$N", "toCase": "upperCase" } } },
    });
    let source = "'{name} {0}'.format(1)\n";
    let found = findings(ast, anchored(None, "capture.F", "missing $N ($U)"), source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["message"], "missing name (NAME)");
    assert_eq!(spanned(&found[0], source), "{name}");
    // An unused argument's transform, as a host capture would be.
    let ast = json!({
        "rule": { "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "S",
            "args": "A", "unused": { "kind": "keyword_argument", "has": { "kind": "identifier", "pattern": "$K" } } } },
        "transform": { "T": { "substring": { "source": "$K", "endChar": 1 } } },
    });
    let found = findings(
        ast,
        anchored(None, "capture.K", "unused $K ($T)"),
        "'{a}'.format(a=1, bee=2)\n",
    );
    assert_eq!(found[0]["message"], "unused bee (b)");
    // A comparison inside a parsed tree reads a transform of that tree's capture.
    let ast = json!({
        "rule": { "kind": "string", "parse": { "grammar": "percentFormat", "all": [
            { "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } },
            { "has": { "kind": "mapping_key", "stopBy": "end", "capture": "L",
                       "compare": { "left": "L", "op": "eq", "right": "U", "text": "source" } } },
        ] } },
        "transform": { "U": { "convert": { "source": "$K", "toCase": "upperCase" } } },
    });
    let source = "'%(a)s %(A)s' % d\n'%(a)s %(b)s' % d\n";
    let found = findings(ast, anchored(None, "witness.span", "m"), source);
    assert_eq!(
        found
            .iter()
            .map(|finding| spanned(finding, source))
            .collect::<Vec<_>>(),
        ["'%(a)s %(A)s'"]
    );
    // There, as at the host, a transform variable compares as text only.
    let refused = compile_check(
        "python",
        json!({
            "rule": { "kind": "string", "parse": { "grammar": "percentFormat",
                "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K",
                         "compare": { "left": "K", "op": "eq", "right": "U", "text": "value" } } } },
            "transform": { "U": { "convert": { "source": "$K", "toCase": "upperCase" } } },
        }),
        finding(),
    );
    assert!(
        rejection(refused)
            .contains("a transform variable is text; compare it as source or unquoted")
    );
}

/// `field` names a grammar's fields, which a parsed tree's rule does not read: refused there.
#[test]
fn field_is_refused_inside_a_parsed_tree() {
    let reason = rejection(compile_check(
        "python",
        json!({ "rule": { "kind": "string", "parse": { "grammar": "percentFormat",
            "has": { "kind": "placeholder", "field": "key" } } } }),
        finding(),
    ));
    assert_eq!(
        reason,
        "/checks/0/forbid/ast/rule/parse/has/field: field is not supported in a parsed tree's rule"
    );
}

/// `commute` reorders tokens; ESLint's `isSameReference` and Ruff's structural equalities have no
/// operand order to relax, so `commute` is refused with them.
#[test]
fn commute_applies_to_tokens_only() {
    for (language, text) in [
        ("typescript", "reference"),
        ("typescript", "reference-no-static-keys"),
        ("python", "structure"),
        ("python", "structure-hash"),
    ] {
        let reason = rejection(compile_check(
            language,
            json!({ "rule": { "pattern": "f($A, $B)",
                "compare": { "left": "A", "op": "eq", "right": "B", "text": text, "commute": ["||"] } } }),
            finding(),
        ));
        assert!(
            reason.ends_with("commute: commute applies to text tokens only"),
            "{text}: {reason}"
        );
    }
}
