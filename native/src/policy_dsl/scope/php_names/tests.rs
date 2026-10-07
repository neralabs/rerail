use super::{NameKind, PhpNames, Resolved};
use crate::policy_dsl::CompiledPolicyDsl;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::oracle;
use crate::policy_dsl::scope::BuildError;
use crate::policy_dsl::test_support::*;
use ast_grep_language::LanguageExt;
use napi::Result;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The file's names table, and the text of any syntax error tree-sitter recovered from.
fn build(source: &str) -> (PhpNames, Vec<String>) {
    let root = GrammarDialect::Php.ast_grep(source);
    let errors = root
        .root()
        .dfs()
        .filter(|node| node.is_error() || node.is_missing())
        .map(|node| format!("{:?} {:?}", node.range(), node.text()))
        .collect();
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a names table");
    (names, errors)
}

fn kind(name: &str) -> NameKind {
    match name {
        "class" => NameKind::Class,
        "function" => NameKind::Function,
        "constant" => NameKind::Constant,
        other => panic!("kind {other}"),
    }
}

/// Every name nikic/PHP-Parser's NameResolver resolves in the fixtures resolves alike here, and
/// every reference read here is one it resolves.
#[test]
fn names_resolve_as_php_parser_name_resolver() {
    let oracle: Value =
        serde_json::from_str(oracle!("php/scope/php/names.expected.json")).expect("oracle JSON");
    assert_eq!(oracle["php"], "8.5.11");
    assert_eq!(oracle["phpParser"], "v5.9.0");
    let directory =
        std::path::Path::new(crate::policy_dsl::fixtures_dir!()).join("php/scope/php/names");
    let mut failures = Vec::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for (file, records) in oracle["files"].as_object().expect("files") {
        let source = std::fs::read_to_string(directory.join(file)).expect("fixture");
        let (names, errors) = build(&source);
        assert!(errors.is_empty(), "{file} has syntax errors: {errors:?}");
        let mut seen = Vec::new();
        for record in records.as_array().expect("records") {
            let start = record["start"].as_u64().expect("start") as usize;
            let end = record["end"].as_u64().expect("end") as usize;
            let text = record["text"].as_str().expect("text");
            assert_eq!(&source[start..end], text, "{file}: oracle range");
            seen.push(start..end);
            let reference = names.reference(&(start..end));
            let expected_kind = kind(record["kind"].as_str().expect("kind"));
            let (expected, bucket) = if record.get("special").is_some() {
                (Resolved::Special, "special")
            } else if let Some(resolved) = record["resolved"].as_str() {
                (Resolved::Name(resolved.into()), "resolved")
            } else if record["declared"] == true {
                (
                    Resolved::Name(record["namespaced"].as_str().expect("namespaced").into()),
                    "fallback: declared",
                )
            } else if record["builtin"] == true {
                (Resolved::Name(text.into()), "fallback: builtin")
            } else {
                (
                    Resolved::Either {
                        namespaced: record["namespaced"].as_str().expect("namespaced").into(),
                        global: text.into(),
                    },
                    "fallback: unknown",
                )
            };
            *counts.entry(bucket).or_default() += 1;
            match reference {
                // `self::`, `static::` and `parent::` are scopes in the grammar, not names; they
                // resolve to nothing either way.
                None if expected == Resolved::Special => {}
                None => failures.push(format!("{file} {start}..{end} `{text}`: not a reference")),
                Some(reference) => {
                    if reference.kind != Some(expected_kind) || reference.resolved != expected {
                        failures.push(format!(
                            "{file} {start}..{end} `{text}`: {:?} {:?}, expected {expected_kind:?} {expected:?}",
                            reference.kind, reference.resolved
                        ));
                    }
                }
            }
        }
        for (range, reference) in names.references() {
            if !seen.contains(&range) {
                failures.push(format!(
                    "{file} {range:?} `{}`: {reference:?} is no name NameResolver resolves",
                    &source[range.clone()]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let total: usize = counts.values().sum();
    assert!(total >= 300, "{counts:?}");
    for bucket in [
        "resolved",
        "special",
        "fallback: declared",
        "fallback: builtin",
        "fallback: unknown",
    ] {
        assert!(
            counts.get(bucket).is_some_and(|&count| count > 0),
            "{counts:?}"
        );
    }
}

/// Every name a `use` item writes imports what nikic/PHP-Parser's UseItem says (a group's prefix
/// joined with the item's name), of the item's kind; and every import site read here is one.
#[test]
fn use_items_import_what_php_parser_reads() {
    let oracle: Value =
        serde_json::from_str(oracle!("php/scope/php/names.expected.json")).expect("oracle JSON");
    let directory =
        std::path::Path::new(crate::policy_dsl::fixtures_dir!()).join("php/scope/php/names");
    let mut failures = Vec::new();
    let mut total = 0;
    for (file, records) in oracle["imports"].as_object().expect("imports") {
        let source = std::fs::read_to_string(directory.join(file)).expect("fixture");
        let (names, _) = build(&source);
        let mut seen = Vec::new();
        for record in records.as_array().expect("records") {
            total += 1;
            let start = record["start"].as_u64().expect("start") as usize;
            let end = record["end"].as_u64().expect("end") as usize;
            let text = record["text"].as_str().expect("text");
            assert_eq!(&source[start..end], text, "{file}: oracle range");
            seen.push(start..end);
            let expected = (
                kind(record["kind"].as_str().expect("kind")),
                record["imported"].as_str().expect("imported"),
            );
            if names.import(&(start..end)) != Some(expected) {
                failures.push(format!(
                    "{file} {start}..{end} `{text}`: {:?}, expected {expected:?}",
                    names.import(&(start..end))
                ));
            }
        }
        for (range, kind, name) in names.imports() {
            if !seen.contains(&range) {
                failures.push(format!(
                    "{file} {range:?}: {kind:?} {name} is no use item PHP-Parser reads"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(total >= 40, "{total} use items");
}

#[test]
fn a_build_stops_at_its_node_budget() {
    let source = "<?php\nnamespace A;\nexec();\n";
    let root = GrammarDialect::Php.ast_grep(source);
    assert!(PhpNames::build(root.root(), 3, &|| false).is_err());
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a table");
    assert!(names.cost > 3);
}

#[test]
fn a_braced_namespace_body_is_charged_to_the_budget() {
    let source = "<?php\nnamespace A {\nexec();\n}\n";
    let root = GrammarDialect::Php.ast_grep(source);
    assert_eq!(
        PhpNames::build(root.root(), 3, &|| false).err(),
        Some(BuildError::Budget)
    );
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a table");
    assert!(names.cost > 3);
}

#[test]
fn a_braced_namespace_body_reads_the_deadline() {
    let source = format!("<?php\nnamespace A {{\n{}}}\n", "exec();\n".repeat(600));
    let root = GrammarDialect::Php.ast_grep(&source);
    assert_eq!(
        PhpNames::build(root.root(), usize::MAX, &|| true).err(),
        Some(BuildError::Deadline)
    );
}

fn php_rule(rule: Value) -> Result<CompiledPolicyDsl> {
    compile_document_with(
        check_document(json!({
            "id": "php-names",
            "forbid": { "ast": { "language": "php", "rule": rule } },
            "finding": finding(),
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn matches(rule: &Value, source: &str) -> Vec<String> {
    let compiled = php_rule(rule.clone()).unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "php", "a.php", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

fn calls_resolving_to(import: Value) -> Value {
    json!({ "kind": "function_call_expression", "has": { "field": "function", "resolves": { "import": import } } })
}

#[test]
fn a_call_matches_where_its_callee_resolves_to_a_listed_function() {
    let exec = calls_resolving_to(json!(["exec"]));
    assert_eq!(
        matches(
            &exec,
            "<?php\nexec('a');\n\\exec('b');\nEXEC('c');\nstrlen('d');\n"
        ),
        ["exec('a')", "\\exec('b')", "EXEC('c')"]
    );
    // In a namespace an unqualified call falls back to the builtin, unless the file declares the
    // namespaced function.
    assert_eq!(
        matches(
            &exec,
            "<?php\nnamespace App;\nexec('a');\nApp\\exec('b');\n"
        ),
        ["exec('a')"]
    );
    assert_eq!(
        matches(
            &exec,
            "<?php\nnamespace App;\nfunction exec() {}\nexec('a');\n\\exec('b');\n"
        ),
        ["\\exec('b')"]
    );
    // An alias names what it imports.
    let aliased = "<?php\nnamespace App;\nuse function Tools\\run as exec;\nexec('a');\n";
    assert_eq!(matches(&exec, aliased), Vec::<String>::new());
    assert_eq!(
        matches(&calls_resolving_to(json!(["\\Tools\\RUN"])), aliased),
        ["exec('a')"]
    );
}

#[test]
fn a_fallback_php_resolves_at_run_time_is_unknown_where_only_one_candidate_is_listed() {
    let source = "<?php\nnamespace App;\nhelper();\nother();\n";
    // `helper()` is `App\helper` if another file declares it, else `helper`.
    let listed = calls_resolving_to(json!(["helper"]));
    assert_eq!(matches(&listed, source), Vec::<String>::new());
    let negated = json!({ "kind": "function_call_expression", "has": { "field": "function", "not": { "resolves": { "import": ["helper"] } } } });
    assert_eq!(matches(&negated, source), ["other()"]);
    // Both candidates listed, it holds either way.
    let both = calls_resolving_to(json!(["helper", "App\\helper"]));
    assert_eq!(matches(&both, source), ["helper()"]);
    assert_eq!(
        matches(&calls_resolving_to(json!(true)), source),
        ["helper()", "other()"]
    );
}

#[test]
fn classes_and_namespaces_compare_case_insensitively_and_constant_names_do_not() {
    let classes = json!({ "kind": "object_creation_expression", "has": { "resolves": { "import": ["App\\Models"], "prefix": true } } });
    let source = "<?php\nnamespace App;\nuse App\\Models as M;\nnew models\\User();\nnew M\\Post();\nnew \\Apple\\Pie();\nnew self();\n";
    assert_eq!(
        matches(&classes, source),
        ["new models\\User()", "new M\\Post()"]
    );
    let constants = json!({ "resolves": { "import": ["PHP_EOL", "app\\LIMIT"] } });
    let source = "<?php\nnamespace App;\nconst LIMIT = 1;\necho PHP_EOL, php_eol, LIMIT, \\APP\\LIMIT, \\App\\limit;\n";
    assert_eq!(
        matches(&constants, source),
        ["PHP_EOL", "LIMIT", "\\APP\\LIMIT"]
    );
}

#[test]
fn imported_matches_the_names_use_clauses_import() {
    let source = "<?php\nnamespace App;\nuse Illuminate\\Support\\Facades\\DB, Illuminate\\Support\\Str as S;\nuse function Illuminate\\Support\\{collect, value as v};\nuse const App\\Limits\\MAX;\nnew DB();\n";
    let imported = |listed: Value, prefix: bool| {
        let mut resolves = json!({ "imported": listed });
        if prefix {
            resolves["prefix"] = json!(true);
        }
        matches(&json!({ "resolves": resolves }), source)
    };
    // The clause's name, not its alias; a function and a class compared case-insensitively.
    assert_eq!(
        imported(
            json!([
                "illuminate\\support\\facades\\db",
                "\\Illuminate\\Support\\Str"
            ]),
            false
        ),
        [
            "Illuminate\\Support\\Facades\\DB",
            "Illuminate\\Support\\Str"
        ]
    );
    assert_eq!(
        imported(json!(["Illuminate\\Support\\collect"]), false),
        ["collect"]
    );
    assert_eq!(
        imported(json!(["Illuminate\\Support"]), true),
        [
            "Illuminate\\Support\\Facades\\DB",
            "Illuminate\\Support\\Str",
            "collect",
            "value"
        ]
    );
    // Constants compare their own name case-sensitively.
    assert_eq!(
        imported(json!(["App\\Limits\\max"]), false),
        Vec::<String>::new()
    );
    assert_eq!(
        imported(json!(["app\\limits\\MAX"]), false),
        ["App\\Limits\\MAX"]
    );
    // `import` reads the references, `imported` the use clauses; either may match.
    assert_eq!(
        matches(
            &json!({ "resolves": { "import": ["Illuminate\\Support\\Facades\\DB"], "imported": ["App\\Limits\\MAX"] } }),
            source
        ),
        ["App\\Limits\\MAX", "DB"]
    );
}

#[test]
fn php_resolves_refuses_what_php_does_not_answer() {
    let at = "/checks/0/forbid/ast/rule/resolves";
    for (rule, message) in [
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["exec"] } }),
            format!(
                "{at}/global/0: `exec` is no PHP superglobal; expected one of GLOBALS, _SERVER, _GET, _POST, _FILES, _COOKIE, _SESSION, _REQUEST, _ENV"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["exec"], "relative": "text" } }),
            format!(
                "{at}/relative: a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference; php has none"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["Foo"], "seenBefore": true } }),
            format!(
                "{at}/seenBefore: Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "this": "global" } }),
            format!(
                "{at}/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "declared": false } }),
            format!(
                "{at}/declared: declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "directives": "eslint" } }),
            format!(
                "{at}/directives: /* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "scriptDeclarations": true } }),
            format!(
                "{at}/scriptDeclarations: scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "beforeLocal": true } }),
            format!(
                "{at}/beforeLocal: a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "imported": ["exec"], "global": ["_GET"] } }),
            format!(
                "{at}: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["os.path"] } }),
            format!("{at}/imported/0: `os.path` is a dotted name; PHP names join labels with `\\`"),
        ),
    ] {
        assert_eq!(rejection(php_rule(rule)), message);
    }
    let python = compile_document_with(
        check_document(json!({
            "id": "python-names",
            "forbid": { "ast": { "language": "python", "rule": { "kind": "identifier", "resolves": { "import": ["os\\path"] } } } },
            "finding": finding(),
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    );
    assert_eq!(
        rejection(python),
        "/checks/0/forbid/ast/rule/resolves/import/0: `os\\path` is a PHP name; Python imports are dotted names"
    );
}
