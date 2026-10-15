//! Port-shaped rules for ESLint's no-unused-vars, no-undef and no-redeclare and typescript-eslint's
//! no-unused-vars and no-redeclare, against where ESLint 10.11.0 and typescript-eslint 8.70.1
//! report them on the fixtures in `fixtures/ecma/scope/eslint-usage` under each option set
//! (`fixtures/ecma/scope/generate_eslint_usage.mjs`), and for ESLint's no-unused-private-class-members on
//! its own tests (`fixtures/ecma/scope/generate_eslint_private.mjs`). The engine owns what reads a variable; each option
//! is the port's, and these are the shapes that express them. Where the engine is conservative by
//! design (a direct `eval`, a `with`) the port reports less, and the test lists each range.
//!
//! `ESLINT_USAGE_ORACLE_ROOT` points the test at another directory's `eslint-usage.expected.json`
//! and its sources (the repository-wide rerun).

use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::test_support::*;
use serde_json::{Value, json};

/// Every node a name can be: identifiers, type names, enum members and shorthand patterns.
fn names() -> Value {
    json!([
        { "kind": "identifier" },
        { "kind": "type_identifier" },
        { "kind": "property_identifier" },
        { "kind": "shorthand_property_identifier_pattern" },
        { "kind": "shorthand_property_identifier" },
        { "kind": "undefined" },
    ])
}

/// ESLint's `isFunction(def.name.parent)`: a parameter that is a plain name, with no pattern,
/// default or rest.
fn simple_parameter() -> Value {
    let name = json!([{ "kind": "identifier" }, { "kind": "undefined" }]);
    json!({ "any": [
        { "kind": "required_parameter", "has": { "any": name }, "not": { "has": { "regex": "^=$" } } },
        { "kind": "optional_parameter", "has": { "any": name }, "not": { "has": { "regex": "^=$" } } },
        { "kind": "identifier", "inside": { "kind": "arrow_function" } },
    ] })
}

/// A name in an object pattern whose last element is a rest element (`{ a, ...rest }`).
fn rest_sibling() -> Value {
    let element = json!([
        { "kind": "pair_pattern" }, { "kind": "shorthand_property_identifier_pattern" },
        { "kind": "object_assignment_pattern" }, { "kind": "rest_pattern" },
    ]);
    let with_rest = json!({ "kind": "object_pattern",
        "has": { "kind": "rest_pattern", "not": { "precedes": { "any": element } } } });
    json!({ "any": [
        { "inside": with_rest },
        { "inside": { "kind": "pair_pattern", "inside": with_rest } },
    ] })
}

/// no-unused-vars at the node ESLint reports (`resolves.write: "last"`), under the rule's
/// options. `typescript` is typescript-eslint's, which reports a used ignored variable where it
/// reports an unused one; ESLint reports it at the first definition.
fn no_unused_vars(options: &Value, typescript: bool) -> Value {
    let option = |key: &str| options.get(key).and_then(Value::as_str);
    let mut scopes = vec![
        "module",
        "function",
        "class",
        "block",
        "namespace",
        "typeParameters",
    ];
    if option("vars") != Some("local") {
        scopes.push("global");
    }
    let anchor = |filter: Value| {
        let mut binding = json!({ "scope": scopes, "directives": "eslint" });
        binding
            .as_object_mut()
            .expect("object")
            .extend(filter.as_object().expect("filter").clone());
        json!({ "resolves": { "write": "last", "binding": binding } })
    };
    let first_definition = |filter: Value| {
        if typescript {
            return anchor(filter);
        }
        let mut binds = json!({ "scope": scopes });
        binds
            .as_object_mut()
            .expect("object")
            .extend(filter.as_object().expect("filter").clone());
        json!({ "all": [{ "binds": binds }, { "not": { "binds": { "redefines": {} } } }] })
    };
    let kinds = [
        (
            json!("parameter"),
            option("argsIgnorePattern"),
            option("args") == Some("none"),
        ),
        (
            json!("except"),
            option("caughtErrorsIgnorePattern"),
            option("caughtErrors") == Some("none"),
        ),
        (
            json!([
                "assignment",
                "for",
                "import",
                "definition",
                "typeParameter",
                "implicit",
                "typeAlias"
            ]),
            option("varsIgnorePattern"),
            false,
        ),
    ];
    let report_used = options.get("reportUsedIgnorePattern") == Some(&json!(true));
    let (mut terms, mut ignored) = (Vec::new(), Vec::new());
    for (form, pattern, skip) in kinds {
        if skip {
            continue;
        }
        let mut not = Vec::new();
        if let Some(pattern) = pattern {
            not.push(json!({ "regex": pattern }));
            if report_used {
                ignored.push(json!({ "all": [
                    first_definition(json!({ "form": form, "used": true })),
                    { "regex": pattern },
                    { "not": { "inside": { "any": [{ "kind": "enum_body" }, { "kind": "enum_assignment" }] } } },
                ] }));
            }
        }
        if form == "parameter" && option("args").unwrap_or("after-used") == "after-used" {
            // A plain parameter before one whose name is referenced.
            let referenced = names()
                .as_array()
                .expect("names")
                .iter()
                .map(|name| {
                    let mut name = name.clone();
                    name["any"] =
                        json!([{ "binds": { "referenced": true } }, { "binds": { "used": true } }]);
                    name
                })
                .collect::<Vec<_>>();
            let mut site = simple_parameter();
            site["precedes"] =
                json!({ "stopBy": "end", "has": { "stopBy": "end", "any": referenced } });
            not.push(json!({ "resolves": { "write": "last", "binding": { "form": "parameter", "site": site } } }));
        }
        if let Some(pattern) = option("destructuredArrayIgnorePattern") {
            // Some definition or reference of the variable stands in an array pattern.
            let in_array = json!({ "inside": { "kind": "array_pattern" } });
            not.push(json!({ "all": [
                { "regex": pattern },
                { "resolves": { "write": "last", "binding": { "occurrence": in_array } } },
            ] }));
            if report_used {
                ignored.push(json!({ "all": [
                    first_definition(json!({ "form": form, "used": true, "occurrence": in_array })),
                    { "regex": pattern },
                ] }));
            }
        }
        if options.get("ignoreRestSiblings") == Some(&json!(true)) {
            not.push(json!({ "resolves": { "write": "last", "binding": { "occurrence": rest_sibling() } } }));
        }
        if options.get("ignoreClassWithStaticInitBlock") == Some(&json!(true)) {
            not.push(json!({ "inside": {
                "any": [{ "kind": "class_declaration" }, { "kind": "class" }],
                "has": { "kind": "class_body", "has": { "kind": "class_static_block" } },
            } }));
        }
        let mut term = vec![anchor(
            json!({ "form": form, "used": false, "exported": false }),
        )];
        if !not.is_empty() {
            term.push(json!({ "not": { "any": not } }));
        }
        terms.push(json!({ "all": term }));
    }
    terms.extend(ignored);
    json!({ "any": names(), "all": [{ "any": terms }] })
}

/// typescript-eslint's declaration-file marks: a variable defined by a declaration that stands
/// (through `declare`) directly in the program or in a namespace body is used, unless that body
/// re-exports (`export {}`, `export *`, `export =`, `export default name`).
fn declaration_file_marks() -> Value {
    let direct = |kind: &str| json!({ "has": { "stopBy": "neighbor", "kind": kind } });
    let token = |text: &str| json!({ "has": { "stopBy": "neighbor", "regex": text } });
    let overriding = json!({ "kind": "export_statement", "any": [
        direct("export_clause"), direct("namespace_export"), token("^\\*$"), token("^=$"),
        { "all": [token("^default$"), direct("identifier")] },
    ] });
    let container = json!({
        "any": [
            { "kind": "program" },
            { "kind": "statement_block", "inside": { "stopBy": "neighbor", "any": [{ "kind": "internal_module" }, { "kind": "module" }] } },
        ],
        "not": { "has": { "stopBy": "neighbor", "all": [overriding] } },
    });
    let in_container = json!({ "any": [
        { "inside": { "stopBy": "neighbor", "all": [container] } },
        { "inside": {
            "stopBy": "neighbor",
            "any": [{ "kind": "ambient_declaration" }, { "kind": "expression_statement" }],
            "inside": { "stopBy": "neighbor", "all": [container] },
        } },
    ] });
    let declarations = [
        "interface_declaration",
        "type_alias_declaration",
        "class_declaration",
        "abstract_class_declaration",
        "function_signature",
        "enum_declaration",
        "internal_module",
    ]
    .iter()
    .map(|kind| json!({ "kind": kind }))
    .collect::<Vec<_>>();
    let site = json!({ "any": [
        { "any": declarations, "all": [in_container] },
        { "kind": "variable_declarator", "inside": {
            "stopBy": "neighbor",
            "any": [{ "kind": "lexical_declaration" }, { "kind": "variable_declaration" }],
            "all": [in_container],
        } },
    ] });
    json!({ "resolves": { "write": "last", "binding": { "site": site } } })
}

/// no-undef: a reference nothing binds and no declared global names; `typeof x` aside.
fn no_undef(options: &Value) -> Value {
    let mut rule =
        json!({ "any": names(), "resolves": { "unresolved": true, "directives": "eslint" } });
    if options.get("typeof") != Some(&json!(true)) {
        rule["not"] = json!({ "inside": {
            "kind": "unary_expression", "regex": "^typeof\\b",
            "stopBy": { "not": { "kind": "parenthesized_expression" } },
        } });
    }
    rule
}

/// The scopes no-redeclare visits: the program, functions (arrows too), blocks, loops, `switch`.
const REDECLARE_SCOPES: [&str; 4] = ["global", "module", "function", "block"];

/// ESLint's no-redeclare: each definition after a variable's first, and with `builtinGlobals` every
/// one of a configured global's.
fn no_redeclare(options: &Value) -> Value {
    let mut terms = vec![json!({ "binds": { "scope": REDECLARE_SCOPES, "redefines": {} } })];
    if options.get("builtinGlobals") != Some(&json!(false)) {
        terms.push(json!({ "binds": { "scope": REDECLARE_SCOPES, "builtin": true } }));
    }
    json!({ "any": names(), "all": [{ "any": terms }] })
}

/// typescript-eslint's no-redeclare: overload signatures are no definitions; a `/* global */`
/// comment is the first; with `ignoreDeclarationMerge` interfaces and namespaces merge, and so does
/// one class, function or enum with namespaces (a class with interfaces too), whose second class,
/// function or enum is still reported.
fn typescript_no_redeclare(options: &Value) -> Value {
    let builtin_globals = options.get("builtinGlobals") != Some(&json!(false));
    let merge = options.get("ignoreDeclarationMerge") != Some(&json!(false));
    let kinds = |kinds: &[&str]| {
        kinds
            .iter()
            .map(|kind| json!({ "kind": kind }))
            .collect::<Vec<_>>()
    };
    let signature = json!({ "kind": "function_signature" });
    let this_is = |these: &[&str]| json!({ "binds": { "site": { "any": kinds(these) } } });
    // Every definition is one of these (signatures aside).
    let all_of = |these: &[&str]| {
        let mut others = kinds(these);
        others.push(signature.clone());
        json!({ "all": [this_is(these), { "binds": { "redefines": { "every": true, "site": { "any": others } } } }] })
    };
    let earlier = |these: Option<&[&str]>| match these {
        Some(these) => json!({ "binds": { "redefines": { "site": { "any": kinds(these) } } } }),
        None => json!({ "binds": { "redefines": { "site": { "not": signature } } } }),
    };
    let builtin = json!({ "binds": { "builtin": true, "directives": "eslint" } });
    let reported = |earlier: Value| {
        if builtin_globals {
            json!({ "any": [earlier, builtin] })
        } else {
            earlier
        }
    };
    let base = json!({ "binds": { "scope": REDECLARE_SCOPES, "site": { "not": signature } } });
    let plain = reported(earlier(None));
    if !merge {
        return json!({ "any": names(), "all": [base, plain] });
    }
    // Other definitions than signatures: typescript-eslint merges only then.
    let others =
        json!({ "not": { "binds": { "redefines": { "every": true, "site": signature } } } });
    let classes: &[&str] = &["class_declaration", "abstract_class_declaration"];
    let functions: &[&str] = &["function_declaration", "generator_function_declaration"];
    let enums: &[&str] = &["enum_declaration"];
    let groups = [
        (
            [
                classes,
                &["interface_declaration", "internal_module", "module"][..],
            ]
            .concat(),
            classes,
        ),
        (
            [functions, &["internal_module", "module"][..]].concat(),
            functions,
        ),
        ([enums, &["internal_module", "module"][..]].concat(), enums),
    ];
    let merged = json!([
        all_of(&["interface_declaration"]),
        all_of(&["internal_module", "module"])
    ]);
    let grouped = groups
        .iter()
        .map(|(group, reported_kinds)| {
            json!({ "all": [
                all_of(group), { "not": { "any": merged } }, this_is(reported_kinds),
                reported(earlier(Some(reported_kinds))),
            ] })
        })
        .collect::<Vec<_>>();
    let mut in_group = merged.as_array().expect("merged").clone();
    in_group.extend(groups.iter().map(|(group, _)| all_of(group)));
    let mut cases = vec![
        json!({ "all": [others, { "not": { "any": in_group } }, plain] }),
        json!({ "all": [others, { "any": grouped }] }),
    ];
    if builtin_globals {
        cases.push(json!({ "all": [{ "not": others }, builtin] }));
    }
    json!({ "any": names(), "all": [base, { "any": cases }] })
}

/// The port for a case: its clauses, each a rule and a `file` condition, and the engine's
/// `languageOptions` for it.
fn port(case: &Value) -> (Vec<(Value, Option<Value>)>, Value) {
    let options = &case["options"];
    let typescript = case["flavor"] == "typescript-eslint";
    let clauses = match case["rule"].as_str().expect("rule") {
        // typescript-eslint reads a declaration file's name: one clause for each kind of file.
        "no-unused-vars" if typescript => {
            let rule = no_unused_vars(options, true);
            let mut marked = rule.clone();
            marked["all"]
                .as_array_mut()
                .expect("all")
                .push(json!({ "not": declaration_file_marks() }));
            vec![
                (rule, Some(json!({ "declaration": false }))),
                (marked, Some(json!({ "declaration": true }))),
            ]
        }
        "no-unused-vars" => vec![(no_unused_vars(options, false), None)],
        "no-undef" => vec![(no_undef(options), None)],
        "no-redeclare" if typescript => vec![(typescript_no_redeclare(options), None)],
        "no-redeclare" => vec![(no_redeclare(options), None)],
        other => panic!("no port for {other}"),
    };
    let mut language_options = case
        .get("languageOptions")
        .cloned()
        .unwrap_or_else(|| json!({}));
    // Base ESLint has no JSX pragma.
    if !typescript {
        language_options["jsxPragma"] = Value::Null;
    }
    (clauses, language_options)
}

/// Where a port reports on a source, as byte ranges.
fn ranges(case: &Value, source: &str) -> Result<Vec<(u64, u64)>, String> {
    let (clauses, language_options) = port(case);
    let file = case["file"].as_str().expect("file");
    let tsx = file.ends_with('x');
    let clauses = clauses
        .into_iter()
        .enumerate()
        .map(|(index, (rule, file))| {
            let mut ast = json!({ "language": "typescript", "languageOptions": language_options, "rule": rule });
            if let Some(file) = file {
                ast["file"] = file;
            }
            json!({
                "id": format!("port-{index}"),
                "forbid": { "ast": ast },
                "violation": { "anchor": "witness.span", "code": "PORT", "message": "port" },
            })
        })
        .collect::<Vec<_>>();
    let document = json!({ "format": "rerail.policy/v1", "id": "port", "revision": 1, "mode": "steer", "clauses": clauses });
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .map_err(|error| error.reason.to_string())?;
    // The fixture's own name: typescript-eslint reads a declaration file's.
    let language = if tsx { "tsx" } else { "typescript" };
    let result = evaluate_file(&compiled, language, file, source);
    if result["failures"] != json!([]) {
        return Err(result["failures"].to_string());
    }
    let mut ranges = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| {
            (
                violation["span"]["startByte"].as_u64().expect("start"),
                violation["span"]["endByte"].as_u64().expect("end"),
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.dedup();
    Ok(ranges)
}

/// Every difference between the ports and the oracle over `root`'s `eslint-usage.expected.json`,
/// as `file rule flavor options: only ours/theirs line:text`.
fn differences(root: &std::path::Path) -> Vec<String> {
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("eslint-usage.expected.json")).expect("oracle"),
    )
    .expect("oracle JSON");
    let mut report = Vec::new();
    for case in expected["cases"].as_array().expect("cases") {
        let file = case["file"].as_str().expect("file");
        let source = std::fs::read_to_string(root.join("eslint-usage").join(file))
            .or_else(|_| std::fs::read_to_string(root.join(file)))
            .expect("source");
        let label = format!(
            "{file} {} {} {} {}",
            case["rule"].as_str().expect("rule"),
            case["flavor"].as_str().expect("flavor"),
            case["options"],
            case.get("languageOptions").unwrap_or(&json!({})),
        );
        let ours = match ranges(case, &source) {
            Ok(ranges) => ranges,
            Err(failure) => {
                report.push(format!("{label}: {failure}"));
                continue;
            }
        };
        let theirs = case["reports"]
            .as_array()
            .expect("reports")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_u64().expect("start"),
                    pair[1].as_u64().expect("end"),
                )
            })
            .collect::<Vec<_>>();
        let text = |(start, end): &(u64, u64)| {
            let (start, end) = (*start as usize, *end as usize);
            format!(
                "{}:{}",
                source[..start].matches('\n').count() + 1,
                &source[start..end]
            )
        };
        for range in ours.iter().filter(|range| !theirs.contains(range)) {
            report.push(format!("{label}: only ours {}", text(range)));
        }
        for range in theirs.iter().filter(|range| !ours.contains(range)) {
            report.push(format!("{label}: only theirs {}", text(range)));
        }
    }
    report
}

#[test]
fn ports_report_where_eslint_and_typescript_eslint_do() {
    let root = std::env::var("ESLINT_USAGE_ORACLE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("ecma/scope")
        });
    let repository = std::env::var("ESLINT_USAGE_ORACLE_ROOT").is_ok();
    let report = differences(&root);
    // Conservative by design: a direct `eval` leaves the usedness of what its function sees
    // unknown, the script's global variables included, and so does a `with` (ESLint's
    // eslint-scope resolves nothing under the `eval`). A `/* global */` comment is no node a
    // violation can stand on, so no-redeclare's report inside one is out of reach.
    let dynamic = "dynamic.js no-unused-vars eslint {} {\"sourceType\":\"script\"}: only theirs";
    let redeclare = "redeclare.js no-redeclare eslint";
    let expected = if repository {
        Vec::new()
    } else {
        [
            "1:evaluated",
            "2:hidden",
            "3:alsoHidden",
            "6:scoped",
            "7:outside",
            "10:plain",
        ]
        .iter()
        .map(|name| format!("{dynamic} {name}"))
        .chain(
            [
                "{} {\"sourceType\":\"script\"}",
                "{\"builtinGlobals\":false} {\"sourceType\":\"script\"}",
                "{} {\"globals\":{\"top\":\"readonly\"},\"sourceType\":\"script\"}",
            ]
            .iter()
            .map(|options| format!("{redeclare} {options}: only theirs 13:declared")),
        )
        .collect()
    };
    assert_eq!(report, expected, "{}", report.join("\n"));
}

/// ESLint's no-unused-private-class-members: a private name no read in its class body uses, at
/// its last declaration (a getter and setter pair is one member, reported at the later one). A
/// field is `field`: tree-sitter-typescript's `public_field_definition`, tree-sitter-javascript's
/// `field_definition`.
fn no_unused_private_class_members(field: &str) -> Value {
    let members = json!([{ "kind": field }, { "kind": "method_definition" }]);
    json!({
        "kind": "private_property_identifier",
        "pattern": "$N",
        "binds": { "used": false },
        "not": { "inside": {
            "stopBy": "neighbor",
            "any": members,
            "precedes": { "stopBy": "end", "any": members,
                "has": { "stopBy": "neighbor", "kind": "private_property_identifier", "pattern": "$N" } },
        } },
    })
}

/// The port reports exactly where ESLint 10.11.0 does on the rule's own tests and the extra cases
/// (`fixtures/ecma/scope/generate_eslint_private.mjs`): every case in tree-sitter-typescript,
/// and those ESLint parses as JavaScript in tree-sitter-javascript too (whose fields key their
/// name `property`, not `name`).
#[test]
fn private_class_member_ports_report_where_eslint_does() {
    let expected: Value = serde_json::from_str(oracle!("ecma/scope/eslint-private.expected.json"))
        .expect("oracle JSON");

    let mut report = Vec::new();
    for (language, field, path) in [
        ("typescript", "public_field_definition", "a.ts"),
        ("javascript", "field_definition", "a.js"),
    ] {
        let document = clause_document(json!({
            "id": "port",
            "forbid": { "ast": { "language": language, "rule": no_unused_private_class_members(field) } },
            "violation": { "anchor": "witness.span", "code": "PORT", "message": "port" },
        }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        for case in expected["cases"].as_array().expect("cases") {
            if language == "javascript" && case["typescript"] == json!(true) {
                continue;
            }
            let code = case["code"].as_str().expect("code");
            let result = evaluate_file(&compiled, language, path, code);
            if result["failures"] != json!([]) {
                report.push(format!(
                    "{path} {}: {}",
                    case["id"], result["failures"][0]["code"]
                ));
                continue;
            }
            let mut ours = result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| {
                    json!([violation["span"]["startByte"], violation["span"]["endByte"]])
                })
                .collect::<Vec<_>>();
            ours.sort_by_key(|range| (range[0].as_u64(), range[1].as_u64()));
            ours.dedup();
            if json!(ours) != case["reports"] {
                report.push(format!(
                    "{path} {}: ours {ours:?}, ESLint {}",
                    case["id"], case["reports"]
                ));
            }
        }
    }
    // Both grammars end the field before a member named `instanceof` or a line starting `[`, as
    // ESLint does (Patch 8 in grammars/tree-sitter-javascript/PATCHES.md, Patch 12 in
    // grammars/tree-sitter-typescript/PATCHES.md), so every case reports where ESLint does.
    assert!(report.is_empty(), "{}", report.join("\n"));
    assert_eq!(expected["cases"].as_array().map(Vec::len), Some(83));
}
