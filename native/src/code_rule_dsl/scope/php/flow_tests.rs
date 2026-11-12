//! PHP bindings beyond definedness and usage, against their oracles: the binding sites and
//! occurrences of every variable (nikic/PHP-Parser), destructured values (the PHP runtime), and
//! what reaches each binding and each scope's end, and which bindings write an outer variable (the
//! PHP runtime, tracing every binding of runnable fixtures).

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
use crate::code_rule_dsl::scope::{Form, Reading, ScopeTable};
use crate::code_rule_dsl::value::{Context, Value, php};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn oracle_root() -> PathBuf {
    PathBuf::from(crate::code_rule_dsl::fixtures::fixtures_dir!()).join("php/scope/php")
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
        let flow = table.flow.as_deref().and_then(crate::code_rule_dsl::scope::php::Flow::of).expect("a PHP flow");
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

// cratify: `code_rules` is rerail-engine's (it mounts it)
