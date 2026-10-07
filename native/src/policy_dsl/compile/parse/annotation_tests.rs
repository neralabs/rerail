//! `parse: annotation`: string type definitions as the scope table classifies and reads them.

use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

fn compiled(rule: Value, anchor: &str) -> crate::policy_dsl::CompiledPolicyDsl {
    compile_document_with(
        check_document(
            json!({ "id": "annotation", "forbid": { "ast": { "language": "python", "rule": rule } },
            "finding": { "anchor": anchor, "code": "X", "message": "x", "severity": "error" } }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

/// Each finding's anchored text.
fn anchored(rule: Value, anchor: &str, source: &str) -> Vec<String> {
    let result = evaluate_file(&compiled(rule, anchor), "python", "a.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
            let end = finding["span"]["endByte"].as_u64().expect("end") as usize;
            source[start..end].to_owned()
        })
        .collect()
}

fn strings() -> Value {
    json!([{ "kind": "string" }, { "kind": "concatenated_string" }])
}

#[test]
fn only_strings_ruff_reads_as_type_definitions_parse() {
    let source = "\
import typing
from typing import List, Literal, cast
from other import cast as cast2
a: \"int\" = 1
b = \"int\"
c: Literal[\"int\"]
d = List[\"int\"]
e = cast(\"int\", 1)
f = cast2(\"int\", 1)
g = typing.cast(\"int\", 1)
h = print(\"int\")
def i(x: \"int\") -> \"int\": ...
";
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "has": { "kind": "identifier", "regex": "^int$", "stopBy": "end" } } });
    // `a`, `d` (a typing generic), `e` and `g` (typing's `cast`), `i`'s parameter and return.
    assert_eq!(anchored(rule, "witness.span", source).len(), 6);
    let rule = json!({ "any": strings(), "not": { "parse": { "grammar": "annotation", "kind": "expression_statement" } }, "regex": "int" });
    // A string that is no type definition is no annotation: its parse is false, not unknown.
    assert_eq!(
        anchored(rule, "witness.span", source),
        ["\"int\"", "\"int\"", "\"int\"", "\"int\""]
    );
}

#[test]
fn names_map_to_their_source_in_simple_strings_and_to_the_string_otherwise() {
    let source = "from typing import List\nx: \"List[Missing]\"\ny: \"List[\\x4dissing]\"\nz: \"List\" \"[Missing]\"\n";
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "has": {
        "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N", "stopBy": "end" } } });
    assert_eq!(
        anchored(rule, "capture.N", source),
        ["Missing", "\"List[\\x4dissing]\"", "\"List\" \"[Missing]\""]
    );
}

#[test]
fn names_are_forward_references_read_where_the_string_is() {
    let source = "\
import os
def f(value: \"os.PathLike\") -> \"Later\":
    local = 1
    inner: \"local\"
    shadowed: \"os\"
    os = 2
    return value
class Later:
    pass
";
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "has": {
        "kind": "identifier", "resolves": { "import": ["os"] }, "capture": "N", "stopBy": "end" } } });
    // The module's `os` first, even under the local one.
    assert_eq!(anchored(rule, "capture.N", source), ["os", "os"]);
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "has": {
        "kind": "identifier", "resolves": { "unresolved": true }, "stopBy": "end" } } });
    assert_eq!(anchored(rule, "witness.span", source), Vec::<String>::new());
}

#[test]
fn nested_strings_parse_and_export_their_captures() {
    let source = "from typing import List\nx: \"List['Missing']\"\n";
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "has": {
        "any": strings(), "stopBy": "end", "parse": { "grammar": "annotation", "has": {
            "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N", "stopBy": "end" } } } } });
    assert_eq!(anchored(rule, "capture.N", source), ["Missing"]);
}

#[test]
fn a_string_ruff_refuses_is_an_error_root() {
    let source = "x: \"List[\"\ny: \"int\"\n";
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "kind": "ERROR" } });
    assert_eq!(anchored(rule, "witness.span", source), ["\"List[\""]);
}

#[test]
fn annotations_read_the_file_so_are_not_evaluated_under_syntax_errors() {
    let rule = json!({ "any": strings(), "parse": { "grammar": "annotation", "kind": "expression_statement" } });
    let result = evaluate_file(
        &compiled(rule, "witness.span"),
        "python",
        "a.py",
        "x: \"int\"\ndef g(:\n",
    );
    assert_eq!(result["findings"], json!([]), "{result}");
    assert_eq!(
        result["failures"][0]["code"], "source-incomplete",
        "{result}"
    );
}

#[test]
fn annotation_rules_are_checked() {
    for (rule, expected) in [
        (
            json!({ "parse": { "grammar": "annotation", "has": { "kind": "identifier", "resolves": { "binding": { "form": "import" } } } } }),
            "/checks/0/forbid/ast/rule/parse/has/resolves/binding: a parsed annotation's names are read, but their bindings' nodes are the host's",
        ),
        (
            json!({ "parse": { "grammar": "annotation", "has": { "kind": "string", "parse": { "grammar": "braceFormat", "kind": "field" } } } }),
            "/checks/0/forbid/ast/rule/parse/has/parse/grammar: a string inside an annotation parses as an annotation",
        ),
        (
            json!({ "parse": { "grammar": "annotation", "has": { "kind": "identifier", "binds": { "used": false } } } }),
            "/checks/0/forbid/ast/rule/parse/has/binds: `binds` is not supported inside parse (annotation trees)",
        ),
    ] {
        let result = compile_document_with(
            check_document(
                json!({ "id": "c", "forbid": { "ast": { "language": "python", "rule": rule.clone() } }, "finding": finding() }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        );
        assert_eq!(rejection(result), expected, "{rule}");
    }
}

/// Ruff-shaped F821 (a name or a string type definition's name, unresolved), F841 and F842 on a
/// file at `path`: each finding as `code:line:column`.
fn reported(path: &str, source: &str) -> Vec<String> {
    let name = json!({ "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N" });
    let checks = [
        (
            "F821",
            "capture.N",
            json!({ "any": [
            { "kind": "identifier", "resolves": { "unresolved": true }, "pattern": "$N" },
            { "any": strings(), "parse": { "grammar": "annotation", "has": { "stopBy": "end", "any": [name,
                { "any": strings(), "parse": { "grammar": "annotation", "has": { "stopBy": "end",
                  "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N" } } }] } } },
        ] }),
        ),
        (
            "F841",
            "witness.span",
            json!({ "kind": "identifier",
            "binds": { "form": "assignment", "scope": "function", "latest": true, "used": false } }),
        ),
        (
            "F842",
            "witness.span",
            json!({ "kind": "identifier",
            "binds": { "form": "annotation", "scope": "function", "latest": true, "used": false } }),
        ),
    ];
    let mut found = Vec::new();
    for (code, anchor, rule) in checks {
        let result = evaluate_file(&compiled(rule, anchor), "python", path, source);
        assert_eq!(result["failures"], json!([]), "{result}");
        for finding in result["findings"].as_array().expect("findings") {
            let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
            let line = source[..start].matches('\n').count() + 1;
            let column = start - source[..start].rfind('\n').map_or(0, |at| at + 1) + 1;
            found.push(format!("{code}:{line}:{column}"));
        }
    }
    found.sort();
    found
}
fn reported_with_python_version(path: &str, source: &str, python_version: &str) -> Vec<String> {
    let name = json!({ "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N" });
    let rule = json!({ "any": [
        { "kind": "identifier", "resolves": { "unresolved": true }, "pattern": "$N" },
        { "any": strings(), "parse": { "grammar": "annotation", "has": { "stopBy": "end", "any": [
            name,
            { "any": strings(), "parse": { "grammar": "annotation", "has": {
                "stopBy": "end", "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N"
            } } },
        ] } } },
    ] });
    let compiled = compile_document_with(
        check_document(json!({
            "id": "annotation",
            "forbid": { "ast": {
                "language": "python",
                "languageOptions": { "pythonVersion": python_version },
                "rule": rule,
            } },
            "finding": { "anchor": "capture.N", "code": "F821", "message": "x", "severity": "error" },
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", path, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut found = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
            let line = source[..start].matches('\n').count() + 1;
            let column = start - source[..start].rfind('\n').map_or(0, |at| at + 1) + 1;
            format!("F821:{line}:{column}")
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

/// In a `.pyi` stub a bare annotation is a binding loads resolve to, and a read written before a
/// `del` sees the binding it removes: a string type definition's names read both as a stub's
/// names do, placed at their string. Each list is Ruff 0.16.9's (F821, F841, F842).
#[test]
fn string_type_definitions_in_stubs_read_as_stub_names_do() {
    let source = "\
from typing import List


def f():
    x: int
    y: \"x\"
    print(x)


class A:
    b: int
    c: \"b\"


def h():
    t: List[\"Later\"] = []
    return t


class Later: ...
";
    assert_eq!(
        reported("a.py", source),
        ["F821:12:9", "F821:6:9", "F821:7:11", "F842:6:5"]
    );
    assert_eq!(reported("a.pyi", source), ["F842:6:5"]);
    let source = "v = 1\nu: \"v\"\nw: v\ndel v\n";
    assert_eq!(reported("b.py", source), ["F821:2:5"]);
    assert_eq!(reported("b.pyi", source), Vec::<String>::new());
    // After the `del`, a stub's string reads the deletion too.
    let source = "v = 1\ndel v\nu: \"v\"\n";
    assert_eq!(reported("c.pyi", source), ["F821:3:5"]);
}
/// Python 3.13 removed TypedDict's legacy keyword fields. The string classifier follows the same
/// version gate as scope construction, so an ignored legacy value is not parsed as an annotation.
#[test]
fn py313_typed_dict_legacy_keyword_strings_are_not_type_definitions() {
    let source = "\
from typing import TypedDict
TD = TypedDict(\"TD\", None, legacy=\"Missing\")
";
    assert_eq!(
        reported_with_python_version("typed_dict.pyi", source, "3.13"),
        Vec::<String>::new()
    );
}

/// A string type definition is a forward reference whether or not the module defers annotations
/// (PEP 563, 3.14, a stub): its names read the module's binding first, so the locals they spell
/// are unused; type parameters' bounds and `type` values are read after the module. Each list
/// is Ruff 0.16.9's.
#[test]
fn string_type_definitions_read_the_module_first_under_every_reading() {
    let body = "\
import os
import sys
from typing import List


def f():
    os = 1
    x: \"os.PathLike\"
    sys = 2
    y: List[\"sys.Any\"]
    return x, y


class C:
    Inner = 1
    a: \"Inner\"
    b: Inner


def g[T: \"Later\"](t: T) -> T:
    return t


type Alias = \"Later\"


class Later: ...
";
    let future = format!("from __future__ import annotations\n\n{body}");
    assert_eq!(
        reported("c.py", &future),
        ["F821:13:12", "F821:13:15", "F841:11:5", "F841:9:5"]
    );
    assert_eq!(reported("c.pyi", &future), ["F841:11:5", "F841:9:5"]);
    assert_eq!(
        reported("d.py", body),
        ["F821:11:12", "F821:11:15", "F841:7:5", "F841:9:5"]
    );
}
