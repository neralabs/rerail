//! Ruff-shaped ports of the rules string type definitions change (F632, F722, F821, F842), against
//! where Ruff 0.16.9 reports them on its own fixtures (`rerail-fixtures/python/literal/ruff`,
//! `rerail-fixtures/python/literal/generate_ruff.py`). Where the port differs, the test names each range and why.

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("python/literal/ruff.expected.json");

/// Ruff's default `dummy-variable-rgx`.
const DUMMY: &str = "^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$";

/// F821 at the host: a name with no binding, builtin or star import, unless a `try` around it
/// handles `NameError` (M10a's shape).
fn undefined_host() -> Value {
    json!({ "any": [{ "kind": "identifier" }, { "kind": "string" }], "resolves": { "unresolved": true } })
}

/// A name in a string type definition, or in a string nested in one, with no binding: captured.
fn undefined_in_annotation() -> Value {
    let name = json!({ "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N" });
    json!({ "any": [{ "kind": "string" }, { "kind": "concatenated_string" }], "parse": { "grammar": "annotation", "has": { "stopBy": "end", "any": [
        name,
        { "any": [{ "kind": "string" }, { "kind": "concatenated_string" }], "parse": { "grammar": "annotation", "has": { "stopBy": "end",
            "kind": "identifier", "resolves": { "unresolved": true }, "capture": "N" } } },
    ] } } })
}

fn f821() -> Value {
    json!({ "rule": {
        "any": [undefined_host(), undefined_in_annotation()],
        "not": { "inside": { "kind": "try_statement", "stopBy": "end",
            "has": { "kind": "except_clause", "has": { "kind": "identifier", "regex": "^NameError$" } } } },
    } })
}

/// Ruff's `is_constant_non_singleton` or `is_mutable_iterable_initializer` operand.
fn literal_operand() -> Value {
    let prefixed = "^[A-Za-z]*[fFtT][A-Za-z]*['\"]";
    json!({ "any": [
        { "kind": "string", "not": { "regex": prefixed } },
        { "kind": "concatenated_string", "not": { "has": { "kind": "string", "regex": prefixed } } },
        { "kind": "integer" },
        { "kind": "float" },
        { "kind": "tuple" },
        { "kind": "list" },
        { "kind": "set" },
        { "kind": "dictionary" },
        { "kind": "list_comprehension" },
        { "kind": "set_comprehension" },
        { "kind": "dictionary_comprehension" },
    ] })
}

/// F632: `is` or `is not` beside a literal operand.
fn is_literal() -> Value {
    json!({ "kind": "comparison_operator", "has": { "all": [
        { "regex": "^is(\\s|\\\\)*(not)?$", "not": { "kind": "identifier" } },
        { "any": [{ "precedes": literal_operand() }, { "follows": literal_operand() }] },
    ] } })
}

fn f632() -> Value {
    let mut in_annotation = is_literal();
    in_annotation["capture"] = json!("C");
    in_annotation["stopBy"] = json!("end");
    json!({ "rule": { "any": [
        is_literal(),
        { "any": [{ "kind": "string" }, { "kind": "concatenated_string" }], "parse": { "grammar": "annotation", "has": in_annotation } },
    ] } })
}

/// Ruff's `report_type_diagnostic`: nothing inside a function decorated `@no_type_check`.
fn no_type_check() -> Value {
    json!({ "inside": { "stopBy": "end", "kind": "decorated_definition", "all": [
        { "has": { "kind": "decorator", "regex": "no_type_check" } },
        { "has": { "kind": "function_definition" } },
    ] } })
}

/// F722 from Ruff's parser: the literal it refuses, reported on the string.
fn f722() -> Value {
    json!({ "rule": {
        "any": [{ "kind": "string" }, { "kind": "concatenated_string" }],
        "parse": { "grammar": "annotation", "kind": "ERROR" },
        "not": no_type_check(),
    } })
}

/// F722 from Ruff's checker: a semantic syntax error in a literal it parsed, reported at the
/// offending node (an `ERROR` node after the expression), anchored at `capture.E`.
fn f722_semantic() -> Value {
    json!({ "rule": {
        "any": [{ "kind": "string" }, { "kind": "concatenated_string" }],
        "parse": { "grammar": "annotation", "has": { "kind": "ERROR", "capture": "E" } },
        "not": no_type_check(),
    } })
}

fn f842() -> Value {
    json!({ "rule": {
        "kind": "identifier",
        "binds": { "form": "annotation", "scope": "function", "latest": true, "used": false },
        "not": { "regex": DUMMY },
    } })
}

/// Where a Python query reports on a source at `path`, which the host places at a dotted module
/// path, anchored at `anchor`, one finding per binding of `each` if given: each finding's byte range.
fn ranges_in(
    ast: Value,
    anchor: &str,
    each: Option<&str>,
    source: &str,
    path: &str,
    module: Option<&str>,
) -> Vec<[u64; 2]> {
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
    let mut file = json!({ "language": "python", "path": path, "source": source });
    if let Some(module) = module {
        file["module"] = json!(module);
    }
    let snapshot = json!({ "facts": [], "files": [file], "format": "rerail.repository-snapshot/v1", "revision": 0 });
    let output = compiled
        .evaluate(snapshot.to_string(), "diagnostic".to_owned())
        .expect("evaluation succeeds");
    let result: Value = serde_json::from_str(&output).expect("evaluation is JSON");
    let mut ranges: Vec<[u64; 2]> = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            [
                finding["span"]["startByte"].as_u64().expect("start"),
                finding["span"]["endByte"].as_u64().expect("end"),
            ]
        })
        .collect();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

/// The port's ranges against Ruff's for one fixture: the ranges only one reports, as `line:text`.
fn differences(
    code: &str,
    name: &str,
    source: &str,
    port: Value,
    anchor: &str,
) -> (Vec<String>, Vec<String>) {
    differences_in(code, name, source, port, anchor, None, None)
}

/// `differences` for a file at a dotted module path; `name` may name a settings variant
/// (`fixture:test`).
fn differences_in(
    code: &str,
    name: &str,
    source: &str,
    port: Value,
    anchor: &str,
    each: Option<&str>,
    module: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle");
    let mut theirs: Vec<[u64; 2]> = oracle[name][code]
        .as_array()
        .unwrap_or_else(|| panic!("{name} {code}"))
        .iter()
        .map(|range| {
            [
                range[0].as_u64().expect("start"),
                range[1].as_u64().expect("end"),
            ]
        })
        .collect();
    theirs.dedup();
    let stub = name
        .split(':')
        .next()
        .is_some_and(|file| file.ends_with(".pyi"));
    let path = if stub { "fixture.pyi" } else { "fixture.py" };
    let ours = ranges_in(port, anchor, each, source, path, module);
    let label = |[start, end]: &[u64; 2]| {
        let (start, end) = (*start as usize, *end as usize);
        format!(
            "{}:{}",
            source[..start].matches('\n').count() + 1,
            &source[start..end]
        )
    };
    (
        ours.iter()
            .filter(|range| !theirs.contains(range))
            .map(label)
            .collect(),
        theirs
            .iter()
            .filter(|range| !ours.contains(range))
            .map(label)
            .collect(),
    )
}

macro_rules! fixture {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!(
                env!("RERAIL_FIXTURES_DIR"),
                "/python/literal/ruff/",
                $name
            )),
        )
    };
}

/// The two F722 checks together: Ruff's parse errors on the string, its semantic errors inside.
#[test]
fn f722_forward_annotation_syntax_errors() {
    for (name, source) in [
        fixture!("F722.py"),
        fixture!("F722_1.py"),
        fixture!("F722_semantic.py"),
    ] {
        let (parse_ours, parse_theirs) = differences("F722", name, source, f722(), "witness.span");
        let (semantic_ours, semantic_theirs) =
            differences("F722", name, source, f722_semantic(), "capture.E");
        // Each check's findings are the other's "only Ruff".
        let only_port: Vec<&String> = parse_ours.iter().chain(&semantic_ours).collect();
        let only_ruff: Vec<&String> = parse_theirs
            .iter()
            .filter(|range| semantic_theirs.contains(range))
            .collect();
        assert_eq!(only_port, Vec::<&String>::new(), "{name}: only the port");
        // A rule has one witness per host node: `(yield *y)` holds two errors at two ranges, and
        // the port reports the first (`*y`).
        let expected: &[&str] = if name == "F722_semantic.py" {
            &["20:yield *y"]
        } else {
            &[]
        };
        assert_eq!(only_ruff, expected, "{name}: only Ruff");
    }
    // The names inside such a string are read: F821 on the fixture as Ruff reports it, but for
    // the second undefined name of one string (one witness per host node).
    let (name, source) = fixture!("F722_semantic.py");
    let (ours, theirs) = differences("F821", name, source, f821(), "capture.N");
    assert_eq!(ours, Vec::<String>::new(), "F821: only the port");
    assert_eq!(theirs, ["6:y", "7:y"], "F821: only Ruff");
}

/// The Ruff suite's F722 semantic-error check, recursive through strings nested in the string: a
/// utility whose one `parse` captures the `ERROR` node and whose other `parse` reaches a nested
/// string through the utility itself. Each error is reported at its host range, through every
/// level of nesting, once per error with `each`.
fn f722_nested() -> Value {
    json!({
        "rule": {
            "any": [{ "kind": "string" }, { "kind": "concatenated_string" }],
            "matches": "semantic-error",
            "not": no_type_check(),
        },
        "utils": { "semantic-error": { "any": [
            { "parse": { "grammar": "annotation", "has": { "kind": "ERROR", "capture": "E" } } },
            { "parse": { "grammar": "annotation", "has": { "stopBy": "end", "kind": "string", "matches": "semantic-error" } } },
        ] } },
    })
}

#[test]
fn f722_semantic_errors_in_nested_strings_anchor_at_the_error() {
    let (name, source) = fixture!("F722_nested.py");
    let (ours, theirs) = differences_in(
        "F722",
        name,
        source,
        f722_nested(),
        "capture.E",
        Some("E"),
        None,
    );
    assert_eq!(ours, Vec::<String>::new(), "only the port");
    assert_eq!(theirs, Vec::<String>::new(), "only Ruff");
    // Without `each`, one finding per host string: its first error.
    let (ours, theirs) = differences("F722", name, source, f722_nested(), "capture.E");
    assert_eq!(ours, Vec::<String>::new(), "only the port");
    assert_eq!(theirs, ["4:await w"], "only Ruff");
}

#[test]
fn f632_literal_comparisons_in_string_annotations() {
    let (name, source) = fixture!("F632.py");
    let (ours, theirs) = differences("F632", name, source, f632(), "capture.C");
    assert_eq!(ours, Vec::<String>::new(), "only the port");
    assert_eq!(theirs, Vec::<String>::new(), "only Ruff");
}

#[test]
fn f842_unused_annotations() {
    let (name, source) = fixture!("F842.py");
    let (ours, theirs) = differences("F842", name, source, f842(), "witness.span");
    assert_eq!(ours, Vec::<String>::new(), "only the port");
    assert_eq!(theirs, Vec::<String>::new(), "only Ruff");
}

/// F401 and F841 read string type definitions' names as reads (M10a's shapes, unchanged): the
/// usage index no longer leaves a name a string annotation's text holds unknown.
#[test]
fn f401_and_f841_read_string_type_definitions() {
    use crate::policy_dsl::compile::binds::ruff::{f401, f841};
    for (code, (name, source), port) in [
        ("F401", fixture!("F401_strings.py"), f401()),
        ("F841", fixture!("F841_strings.py"), f841()),
    ] {
        let (ours, theirs) =
            differences(code, name, source, json!({ "rule": port }), "witness.span");
        assert_eq!(ours, Vec::<String>::new(), "{code}: only the port");
        assert_eq!(theirs, Vec::<String>::new(), "{code}: only Ruff");
    }
    let (name, source) = fixture!("F821_strings.py");
    let (ours, theirs) = differences("F821", name, source, f821(), "capture.N");
    assert_eq!(ours, Vec::<String>::new(), "F821: only the port");
    assert_eq!(theirs, Vec::<String>::new(), "F821: only Ruff");
}

#[test]
fn f821_names_in_string_type_definitions() {
    let fixtures = [
        fixture!("F821_0.py"),
        fixture!("F821_1.py"),
        fixture!("F821_2.py"),
        fixture!("F821_3.py"),
        fixture!("F821_4.py"),
        fixture!("F821_5.py"),
        fixture!("F821_6.py"),
        fixture!("F821_7.py"),
        fixture!("F821_9.py"),
        fixture!("F821_10.py"),
        fixture!("F821_11.py"),
        fixture!("F821_12.py"),
        fixture!("F821_13.py"),
        fixture!("F821_14.py"),
        fixture!("F821_15.py"),
        fixture!("F821_16.py"),
        fixture!("F821_18.py"),
        fixture!("F821_19.py"),
        fixture!("F821_20.py"),
        fixture!("F821_21.py"),
        fixture!("F821_23.py"),
        fixture!("F821_24.py"),
        fixture!("F821_25.py"),
        fixture!("F821_26.py"),
        fixture!("F821_27.py"),
        fixture!("F821_28.py"),
        fixture!("F821_29.py"),
        fixture!("F821_30.py"),
        fixture!("F821_31.py"),
        fixture!("F821_33.py"),
        fixture!("F821_34.py"),
    ];
    let mut report = Vec::new();
    for (name, source) in fixtures {
        let (ours, theirs) = differences("F821", name, source, f821(), "capture.N");
        if !ours.is_empty() || !theirs.is_empty() {
            report.push(format!(
                "{name}: only the port {ours:?}; only Ruff {theirs:?}"
            ));
        }
    }
    // Not a string type definition's: Ruff reads names under NFKC (F821_28), as M10a records. A
    // walrus in an annotation (F821_19) and a `type` statement's value (F821_20) are read after the
    // module, as M10a's fix round reads them. F821_17 is left out: tree-sitter-python 0.23 cannot
    // parse its PEP 696 defaults.
    let expected: &[&str] = &[
        "F821_28.py: only the port [\"5:C\", \"6:C\", \"7:C\", \"7:𝑪\", \"7:𝓒\", \"7:𝕮\"]; only Ruff []",
    ];
    assert_eq!(report, expected, "\n{}", report.join("\n"));
}

/// Ruff's `lint.typing-modules` and `lint.pyflakes.extend-generics` as `languageOptions`, on the
/// fixtures Ruff's own tests read under them (`rules/pyflakes/mod.rs`), and without them.
#[test]
fn typing_modules_and_extend_generics_read_as_ruff_reads_them() {
    use crate::policy_dsl::compile::binds::ruff::f401;
    let with = |port: Value, options: Value| {
        let mut port = port;
        port["languageOptions"] = options;
        port
    };
    let typical = json!({ "typingModules": ["foo.typical"] });
    let settings = json!({ "typingModules": ["compat"], "extendGenerics": ["lib.Box"] });
    let cases = [
        (
            "F821",
            fixture!("typing_modules.py"),
            None,
            vec![
                ("typing_modules.py", json!({})),
                (
                    "typing_modules.py:extra_typing_modules",
                    json!({ "typingModules": ["airflow.typing_compat"] }),
                ),
            ],
        ),
        (
            "F821",
            fixture!("project/foo/bar.py"),
            Some("foo.bar"),
            vec![
                ("project/foo/bar.py", json!({})),
                ("project/foo/bar.py:relative_typing_module", typical.clone()),
            ],
        ),
        (
            "F821",
            fixture!("project/foo/bop/baz.py"),
            Some("foo.bop.baz"),
            vec![
                ("project/foo/bop/baz.py", json!({})),
                (
                    "project/foo/bop/baz.py:nested_relative_typing_module",
                    typical.clone(),
                ),
            ],
        ),
        (
            "F821",
            fixture!("F821_typing_settings.py"),
            None,
            vec![
                ("F821_typing_settings.py", json!({})),
                ("F821_typing_settings.py:settings", settings.clone()),
            ],
        ),
        (
            "F821",
            fixture!("F821_typing_settings.pyi"),
            None,
            vec![
                ("F821_typing_settings.pyi", json!({})),
                ("F821_typing_settings.pyi:settings", settings.clone()),
            ],
        ),
        (
            "F401",
            fixture!("F401_15.py"),
            None,
            vec![
                ("F401_15.py", json!({})),
                (
                    "F401_15.py:extend_immutable_calls",
                    json!({ "extendGenerics": ["django.db.models.ForeignKey"] }),
                ),
            ],
        ),
    ];
    let mut report = Vec::new();
    for (code, (_, source), module, variants) in cases {
        for (name, options) in variants {
            let (port, anchor) = match code {
                "F821" => (f821(), "capture.N"),
                _ => (json!({ "rule": f401() }), "witness.span"),
            };
            let (ours, theirs) = differences_in(
                code,
                name,
                source,
                with(port, options),
                anchor,
                None,
                module,
            );
            if !ours.is_empty() || !theirs.is_empty() {
                report.push(format!(
                    "{name}: only the port {ours:?}; only Ruff {theirs:?}"
                ));
            }
        }
    }
    assert_eq!(report, Vec::<String>::new(), "\n{}", report.join("\n"));
}

/// Ruff's `match_typing_expr`: under `lint.typing-modules`, a typing module's member is typing's
/// own wherever Ruff asks for one, so `resolves.import` lists it as `typing.<member>`. F811's
/// overload exemption (`std.overload`) and F722's `no_type_check` exemption, with and without the
/// setting; a module that is no typing module (`other`) is never typing's.
#[test]
fn a_typing_modules_member_resolves_as_typing_s_own() {
    let typing = |member: &str| {
        json!({ "any": [{ "kind": "identifier" }, { "kind": "attribute" }], "resolves": { "import": [
            format!("typing.{member}"), format!("_typeshed.{member}"), format!("typing_extensions.{member}"),
        ] } })
    };
    let f811 = json!({
        "rule": { "kind": "identifier", "binds": { "form": "definition", "redefines": {
            "form": "definition", "used": false, "site": { "not": { "matches": "overloaded" } } } } },
        "utils": { "overloaded": { "kind": "function_definition", "inside": { "kind": "decorated_definition",
            "has": { "kind": "decorator", "has": typing("overload") } } } },
    });
    let f722 = json!({ "rule": {
        "any": [{ "kind": "string" }, { "kind": "concatenated_string" }],
        "parse": { "grammar": "annotation", "kind": "ERROR" },
        "not": { "inside": { "stopBy": "end", "kind": "decorated_definition", "all": [
            { "has": { "kind": "decorator", "has": typing("no_type_check") } },
            { "has": { "kind": "function_definition" } },
        ] } },
    } });
    let std = json!({ "typingModules": ["std"] });
    let mut report = Vec::new();
    for (code, (file, source), port) in [
        ("F811", fixture!("F811_typing_modules.py"), f811),
        ("F722", fixture!("F722_typing_modules.py"), f722),
    ] {
        for (name, options) in [
            (file.to_owned(), None),
            (format!("{file}:typing_modules"), Some(std.clone())),
        ] {
            let mut port = port.clone();
            if let Some(options) = options {
                port["languageOptions"] = options;
            }
            let (ours, theirs) =
                differences_in(code, &name, source, port, "witness.span", None, None);
            if !ours.is_empty() || !theirs.is_empty() {
                report.push(format!(
                    "{name}: only the port {ours:?}; only Ruff {theirs:?}"
                ));
            }
        }
    }
    assert_eq!(report, Vec::<String>::new(), "\n{}", report.join("\n"));
}
