//! PHP bindings beyond definedness and usage, against their oracles: the binding sites and
//! occurrences of every variable (nikic/PHP-Parser), destructured values (the PHP runtime), and
//! what reaches each binding and each scope's end, and which bindings write an outer variable (the
//! PHP runtime, tracing every binding of runnable fixtures).

use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::scope::usage::{BindingId, Usage};
use crate::policy_dsl::scope::{Form, Reading, ScopeTable};
use crate::policy_dsl::value::{Context, Value, php};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn oracle_root() -> PathBuf {
    PathBuf::from(crate::policy_dsl::fixtures_dir!()).join("php/scope/php")
}

fn oracle(name: &str) -> Json {
    let oracle: Json =
        serde_json::from_str(&std::fs::read_to_string(oracle_root().join(name)).expect("oracle"))
            .expect("oracle JSON");
    assert_eq!(oracle["php"], "8.5.11");
    oracle
}

fn form_name(form: Form) -> &'static str {
    match form {
        Form::Assignment => "assignment",
        Form::Augmented => "augmented",
        Form::For => "for",
        Form::Except => "except",
        Form::Parameter => "parameter",
        Form::Global => "global",
        Form::Static => "static",
        other => panic!("{other:?} is no PHP form"),
    }
}

/// A variable: its name, its bindings (name node start, form) in order, and its occurrences.
type Variable = (String, Vec<(u64, String)>, Vec<u64>);

/// Every variable's bindings, in the order PHP evaluates them and with their forms, and every
/// node that names it, against nikic/PHP-Parser's tree (`generate_bindings.php`): what `only`,
/// `bindings` and `occurrence` read.
#[test]
fn php_bindings_and_occurrences_agree_with_php_parser() {
    let oracle = oracle("bindings.expected.json");
    assert_eq!(oracle["phpParser"], "v5.9.0");
    let mut failures = Vec::new();
    let mut variables = 0;
    for (file, records) in oracle["files"].as_object().expect("files") {
        let source = std::fs::read_to_string(oracle_root().join(file)).expect("fixture");
        let root = GrammarDialect::Php.ast_grep(&source);
        let table = ScopeTable::build_with(
            root.root(),
            GrammarDialect::Php,
            usize::MAX,
            &|| false,
            Reading::default(),
        )
        .expect("table builds");
        let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
        let mut rerail = BTreeMap::<u64, Variable>::new();
        for (index, scope) in table.scopes.iter().enumerate() {
            for (name, bindings) in &scope.bindings {
                let sites: Vec<_> = bindings
                    .iter()
                    .map(|binding| {
                        let start = binding.name.expect("a name node").start;
                        (u64::from(start), form_name(binding.site.form).to_owned())
                    })
                    .collect();
                let id = BindingId {
                    scope: index as u32,
                    name,
                    index: 0,
                };
                let occurrences = usage
                    .occurrences(id)
                    .iter()
                    .map(|node| u64::from(node.start))
                    .collect();
                rerail.insert(sites[0].0, (name.to_string(), sites, occurrences));
            }
        }
        let mut expected = BTreeMap::<u64, Variable>::new();
        for record in records.as_array().expect("records") {
            let sites: Vec<_> = record["bindings"]
                .as_array()
                .expect("bindings")
                .iter()
                .map(|site| {
                    (
                        site["start"].as_u64().expect("start"),
                        site["form"].as_str().expect("form").to_owned(),
                    )
                })
                .collect();
            let occurrences = record["occurrences"]
                .as_array()
                .expect("occurrences")
                .iter()
                .map(|start| start.as_u64().expect("start"))
                .collect();
            let name = record["name"].as_str().expect("name").to_owned();
            expected.insert(sites[0].0, (name, sites, occurrences));
        }
        variables += expected.len();
        let line = |start: u64| source[..start as usize].matches('\n').count() + 1;
        for (start, variable) in &expected {
            match rerail.get(start) {
                Some(found) if found == variable => {}
                found => failures.push(format!(
                    "{file}:{} {}: Rerail {found:?}, PHP-Parser {variable:?}",
                    line(*start),
                    variable.0
                )),
            }
        }
        for (start, variable) in &rerail {
            if !expected.contains_key(start) {
                failures.push(format!(
                    "{file}:{} {}: no variable PHP-Parser reads ({variable:?})",
                    line(*start),
                    variable.0
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(variables >= 200, "{variables} variables");
}

/// The value of each argument of the last `var_dump(...)` call, following variables through
/// their bindings (`value.follow`).
fn followed_arguments(source: &str) -> Vec<Value> {
    let root = GrammarDialect::Php.ast_grep(source);
    let table = ScopeTable::build_with(
        root.root(),
        GrammarDialect::Php,
        usize::MAX,
        &|| false,
        Reading::default(),
    )
    .expect("table builds");
    let call = root
        .root()
        .dfs()
        .filter(|node| {
            node.kind() == "function_call_expression"
                && node
                    .field("function")
                    .is_some_and(|callee| callee.text() == "var_dump")
        })
        .last()
        .unwrap_or_else(|| panic!("{source:?} calls var_dump"));
    let arguments: Vec<_> = call
        .field("arguments")
        .expect("arguments")
        .children()
        .filter(|child| child.kind() == "argument")
        .filter_map(|argument| argument.children().find(|child| child.is_named()))
        .collect();
    arguments
        .iter()
        .map(|argument| {
            let mut spend = || true;
            let mut binding = |range: std::ops::Range<usize>, name: &str| {
                let reference = table.reference(&range)?;
                let (binding, value) = table.binding(reference, name)?;
                Some(binding.followed(value?))
            };
            let mut context = Context::new(&mut spend).following(&mut binding);
            php::decode(argument, &mut context)
        })
        .collect()
}

/// Every case of the destructuring oracle (`generate_destructuring.php`): a destructured variable
/// whose value Rerail knows through its binding holds the value PHP gives it, and Rerail knows each
/// one unless the case says why not.
#[test]
fn destructured_variables_hold_the_elements_php_gives_them() {
    let oracle = oracle("destructuring.expected.json");
    let mut failures = Vec::new();
    let mut known = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let code = case["code"].as_str().expect("code");
        let reads: Vec<_> = case["read"]
            .as_array()
            .expect("read")
            .iter()
            .map(|name| name.as_str().expect("name"))
            .collect();
        let arguments = reads
            .iter()
            .map(|name| format!("${name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source =
            format!("<?php\nfunction t() {{\n    {code}\n    var_dump({arguments});\n}}\n");
        for (name, value) in reads.iter().zip(followed_arguments(&source)) {
            let expected = &case["values"][*name];
            let unknown = case["unknown"].get(*name).is_some();
            match value {
                Value::Unknown | Value::Container { .. } if unknown => {}
                Value::Unknown | Value::Container { .. } => {
                    failures.push(format!(
                        "{code}: ${name} is not known; PHP gives {expected}"
                    ));
                }
                value if php::tests::agrees(&value, expected) => known += 1,
                value => failures.push(format!(
                    "{code}: ${name} is {value:?}; PHP gives {expected}"
                )),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(known >= 50, "{known} values known");
}

/// The traced runs of the flow fixtures (`generate_flow.php`) against the flow: for every binding,
/// the bindings that reach it (what `redefines` reads), whether it reaches its scope's normal end
/// (`latest`) and whether it writes the global variable of its name (`outer`; a top-level binding
/// never does). The fixtures' runs take every path the flow reads, so the two agree exactly, but
/// in `exceptions.php`: the flow lets every statement of a `try` body throw, so its reaching
/// bindings and ends there hold the runs' and may hold more.
#[test]
fn php_flow_agrees_with_traced_runs() {
    let oracle = oracle("flow.expected.json");
    assert_eq!(oracle["phpParser"], "v5.9.0");
    let mut failures = Vec::new();
    let mut bindings = 0;
    for (file, records) in oracle["files"].as_object().expect("files") {
        let exact = !file.starts_with("exceptions");
        let source =
            std::fs::read_to_string(oracle_root().join("flow").join(file)).expect("fixture");
        let root = GrammarDialect::Php.ast_grep(&source);
        let table = ScopeTable::build_with(
            root.root(),
            GrammarDialect::Php,
            usize::MAX,
            &|| false,
            Reading::default(),
        )
        .expect("table builds");
        let flow = table.php.as_deref().expect("a PHP flow");
        // Each binding by its name node's start: its scope, and its index among the scope's.
        let mut starts = BTreeMap::new();
        for (scope, owner) in table.scopes.iter().enumerate() {
            for (name, chain) in &owner.bindings {
                for (index, binding) in chain.iter().enumerate() {
                    let start = u64::from(binding.name.expect("a name node").start);
                    starts.insert(start, (scope as u32, name.clone(), index));
                }
            }
        }
        let line = |start: u64| source[..start as usize].matches('\n').count() + 1;
        let mut traced = BTreeSet::new();
        for record in records.as_array().expect("records") {
            bindings += 1;
            let start = record["start"].as_u64().expect("start");
            traced.insert(start);
            let name = record["name"].as_str().expect("name");
            let at = format!("{file}:{} {name}", line(start));
            let Some((scope, _, _)) = starts.get(&start) else {
                failures.push(format!("{at}: no binding here"));
                continue;
            };
            let reach = flow.reach(start as u32).expect("a reached binding");
            let mut reaching: BTreeSet<i64> = reach
                .bindings
                .iter()
                .map(|&(scope, index)| {
                    let binding = &table.scopes[scope as usize].bindings[name][index];
                    i64::from(binding.name.expect("a name node").start)
                })
                .collect();
            if reach.undefined {
                reaching.insert(-1);
            }
            let ran: BTreeSet<i64> = record["reaching"]
                .as_array()
                .expect("reaching")
                .iter()
                .map(|tag| tag.as_i64().expect("tag"))
                .collect();
            if reach.unknown || (exact && reaching != ran) || !reaching.is_superset(&ran) {
                failures.push(format!(
                    "{at}: reached by {reaching:?} (unknown: {}), runs {ran:?}",
                    reach.unknown
                ));
            }
            let ended = record["end"].as_bool().expect("end");
            let latest = flow.latest(*scope, start as u32);
            if latest.is_none()
                || (exact && latest != Some(ended))
                || (ended && latest != Some(true))
            {
                failures.push(format!(
                    "{at}: latest {latest:?}, runs ended with it: {ended}"
                ));
            }
            let outer = flow.outer(start as u32);
            let expected = match record.get("outer").and_then(Json::as_array) {
                // The global scope's own variables are no outer ones.
                None if *scope == 0 => Some(false),
                None => continue,
                Some(runs) => match runs.as_slice() {
                    [only] => only.as_bool(),
                    _ => None,
                },
            };
            if outer != expected {
                failures.push(format!("{at}: outer {outer:?}, runs {expected:?}"));
            }
        }
        for (start, (_, name, _)) in &starts {
            if !traced.contains(start) {
                failures.push(format!("{file}:{} {name}: never traced", line(*start)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(bindings >= 120, "{bindings} bindings traced");
}

/// A `break` or `continue` out of a `try` runs its `finally` first: what follows reads what the
/// `finally` assigned, not what the jump carried.
#[test]
fn jumps_out_of_try_run_its_finally() {
    for (open, jump, close) in [
        ("for (;;) {", "break", "}"),
        ("do {", "continue", "} while (false);"),
    ] {
        let source = format!(
            "<?php\nfunction t() {{\n    $x = 1;\n    {open}\n        try {{\n            {jump};\n        }} finally {{\n            $x = 2;\n        }}\n    {close}\n    var_dump($x);\n}}\n"
        );
        assert_eq!(
            followed_arguments(&source),
            [Value::Int(2.into())],
            "{source}"
        );
    }
}

mod policies {
    use crate::policy_dsl::CompiledPolicyDsl;
    use crate::policy_dsl::test_support::*;
    use napi::Result;
    use serde_json::{Value, json};

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

    fn compile(rule: Value) -> Result<CompiledPolicyDsl> {
        compile_document_with(
            check_document(json!({
                "id": "php-bindings",
                "forbid": { "ast": { "language": "php", "rule": rule } },
                "violation": finding(),
            })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    /// The lines of the variables a rule matches in `SOURCE`, with their text.
    fn run(rule: Value) -> Vec<String> {
        let compiled = compile(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "f.php", SOURCE);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
                let end = finding["span"]["endByte"].as_u64().expect("end") as usize;
                let line = SOURCE[..start].matches('\n').count() + 1;
                format!("{line}:{}", &SOURCE[start..end])
            })
            .collect()
    }

    fn binds(filter: Value) -> Vec<String> {
        run(json!({ "kind": "variable_name", "binds": filter }))
    }

    fn binding(binding: Value) -> Vec<String> {
        run(json!({ "kind": "variable_name", "resolves": { "binding": binding } }))
    }

    #[test]
    fn outer_bindings_write_a_global_variable_on_every_path() {
        // `$maybe = 1` writes the global only where `$c` held: neither outer nor not.
        assert_eq!(
            binds(json!({ "outer": true })),
            ["3:$config", "4:$config", "10:$maybe"]
        );
        assert!(!binds(json!({ "outer": false })).contains(&"12:$maybe".to_owned()));
        assert!(binds(json!({ "outer": false })).contains(&"5:$limit".to_owned()));
    }

    #[test]
    fn latest_bindings_reach_their_scope_end() {
        assert_eq!(
            binds(json!({ "latest": false })),
            ["3:$config", "5:$limit", "10:$maybe"]
        );
    }

    #[test]
    fn redefines_reads_the_bindings_a_binding_overwrites() {
        // `$limit = 20` overwrites `$limit = 10`, which nothing read. `$config = 1` overwrites
        // the global's binding, whose use another function may make: unknown.
        assert_eq!(
            binds(json!({ "redefines": { "used": false } })),
            ["6:$limit"]
        );
        assert_eq!(
            binds(json!({ "redefines": { "form": "global" } })),
            ["4:$config", "12:$maybe"]
        );
        // The loop's `$last` overwrites itself on a second iteration.
        assert_eq!(
            binds(
                json!({ "form": "assignment", "redefines": { "sameScope": true, "every": true, "form": "assignment" } })
            ),
            ["6:$limit", "16:$last"]
        );
    }

    #[test]
    fn occurrences_are_every_node_naming_the_variable() {
        // The arrow function reads `$once` and `$item` of `f`, which it copies.
        assert_eq!(
            binds(
                json!({ "form": ["assignment", "for"], "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } } })
            ),
            ["13:$once", "15:$item"]
        );
        // The read in the arrow function is defined on some paths only: its binding is unknown.
        assert_eq!(
            binding(
                json!({ "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } }, "form": "for" })
            ),
            ["16:$item"]
        );
    }

    #[test]
    fn only_and_destructured_values_read_the_binding() {
        assert_eq!(
            binding(json!({ "only": true, "scope": "function" })),
            [
                "8:$a",
                "8:$b",
                "9:$c",
                "14:$once",
                "15:$items",
                "16:$item",
                "19:$once"
            ]
        );
        assert_eq!(
            binding(json!({ "value": { "kind": "integer" } })),
            ["8:$a", "8:$b", "8:$limit", "14:$once", "19:$once"]
        );
        assert_eq!(
            binding(json!({ "destructuredValue": false, "value": { "kind": "integer" } })),
            ["8:$limit", "14:$once", "19:$once"]
        );
        assert_eq!(
            run(json!({ "kind": "variable_name", "value": { "follow": true, "eq": 2 } })),
            ["8:$b"]
        );
        // PHP's reads stop at `global $x` whether asked or not.
        assert_eq!(
            binding(json!({ "stopAtGlobal": true, "form": "assignment", "outer": true })),
            ["8:$config"]
        );
    }

    /// What `resolves.this` would ask has PHP's answer in `unresolved`: `$this` outside an object
    /// context is undefined.
    #[test]
    fn this_outside_an_object_is_unresolved() {
        let source = "<?php\nfunction g() { return $this; }\nclass A {\n    static function s() { return $this; }\n    function m() { return $this; }\n}\n";
        let compiled =
            compile(json!({ "kind": "variable_name", "resolves": { "unresolved": true } }))
                .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "g.php", source);
        let lines: Vec<_> = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
                source[..start].matches('\n').count() + 1
            })
            .collect();
        assert_eq!(lines, [2, 4]);
    }

    #[test]
    fn what_php_has_no_counterpart_for_is_refused_with_why() {
        let at = "/clauses/0/forbid/ast/rule";
        for (rule, message) in [
            (
                json!({ "kind": "variable_name", "binds": { "exported": false } }),
                format!(
                    "{at}/binds/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "builtin": true } }),
                format!(
                    "{at}/binds/builtin: `builtin` reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal); php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "directives": "eslint" } }),
                format!(
                    "{at}/binds/directives: `directives` reads ESLint's /* exported */ comments; PHP exports no variables; php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "redefines": { "sameImport": true } } }),
                format!(
                    "{at}/binds/redefines/sameImport: qualified import names are Python's; a PHP variable is never bound by an import"
                ),
            ),
            (
                json!({ "kind": "variable_name", "resolves": { "write": "last" } }),
                format!(
                    "{at}/resolves/write: ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which `binds: {{ used: false }}` matches"
                ),
            ),
            (
                json!({ "kind": "variable_name", "resolves": { "binding": { "stopAtGlobal": false } } }),
                format!(
                    "{at}/resolves/binding/stopAtGlobal: a php read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so php has no lookup through it"
                ),
            ),
        ] {
            assert_eq!(rejection(compile(rule)), message);
        }
    }
}
