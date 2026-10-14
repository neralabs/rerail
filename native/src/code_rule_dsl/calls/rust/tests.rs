//! The call resolver against the recorded rust-analyzer resolution of the fixture crate
//! (`fixtures/rust/calls`): every call it answers must be the target rust-analyzer recorded.

use super::{Callee, Known, callees, definitions, module_of, trait_names};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::scope::ScopeTable;
use crate::code_rule_dsl::scope::rust::build_for_test;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The fixture crate's files by crate-relative path.
const FILES: &[(&str, &str)] = &[
    ("src/lib.rs", oracle!("rust/calls/crate/src/lib.rs")),
    ("src/util.rs", oracle!("rust/calls/crate/src/util.rs")),
    ("src/net/mod.rs", oracle!("rust/calls/crate/src/net/mod.rs")),
    ("src/net/tcp.rs", oracle!("rust/calls/crate/src/net/tcp.rs")),
];

/// The recorded resolution.
fn recording() -> Value {
    serde_json::from_str(oracle!("rust/calls/calls.expected.json")).expect("oracle JSON")
}

/// The byte offset of a one-based line and code-point column.
fn offset_of(source: &str, line: u64, column: u64) -> usize {
    let start: usize = source
        .split_inclusive('\n')
        .take(line as usize - 1)
        .map(str::len)
        .sum();
    let rest = &source[start..];
    let within = rest
        .char_indices()
        .nth(column as usize - 1)
        .map_or(rest.len(), |(index, _)| index);
    start + within
}

/// One file read: its source's table and module.
struct Read {
    path: &'static str,
    source: &'static str,
    table: ScopeTable,
    module: Vec<String>,
}

/// Reads every file of the fixture crate.
fn crate_files() -> Vec<Read> {
    FILES
        .iter()
        .map(|(path, source)| Read {
            path,
            source,
            table: build_for_test(source),
            module: module_of(path).expect("a module under src/"),
        })
        .collect()
}

/// Each definition's qualified name with its file and name offset, and the crate's `Known`.
fn crate_definitions(files: &[Read]) -> (Vec<(String, &'static str, usize)>, Known) {
    let mut defined = Vec::new();
    let mut traits = Vec::new();
    for file in files {
        let root = GrammarDialect::Rust.ast_grep(file.source);
        for (name, offset) in definitions(&root.root(), &file.module, &file.table) {
            defined.push((name, file.path, offset));
        }
        traits.extend(trait_names(&root.root(), &file.module, &file.table));
    }
    let known = Known::new(defined.iter().map(|(name, _, _)| name.clone()), traits);
    (defined, known)
}

#[test]
fn definitions_are_recorded_definitions() {
    let recording = recording();
    let files = crate_files();
    let (defined, _) = crate_definitions(&files);
    let recorded: Vec<(String, u64, u64)> = recording["definitions"]
        .as_array()
        .expect("definitions")
        .iter()
        .map(|row| {
            (
                row["path"].as_str().expect("path").to_owned(),
                row["line"].as_u64().expect("line"),
                row["column"].as_u64().expect("column"),
            )
        })
        .collect();
    let mut found = 0;
    for (name, path, offset) in &defined {
        let source = FILES.iter().find(|(file, _)| file == path).expect("file").1;
        let hit = recorded.iter().any(|(recorded_path, line, column)| {
            recorded_path == path && offset_of(source, *line, *column) == *offset
        });
        assert!(
            hit,
            "{name} in {path} at {offset} is no recorded definition"
        );
        found += 1;
    }
    eprintln!("definitions agreed: {found} of {}", recorded.len());
}

#[test]
fn calls_agree_with_rust_analyzer() {
    let recording = recording();
    let files = crate_files();
    let (defined, known) = crate_definitions(&files);
    let by_name: HashMap<&str, (&str, usize)> = defined
        .iter()
        .map(|(name, path, offset)| (name.as_str(), (*path, *offset)))
        .collect();
    let mut entries: HashMap<(&str, usize), Callee> = HashMap::new();
    for file in &files {
        let root = GrammarDialect::Rust.ast_grep(file.source);
        for (offset, callee) in callees(&root.root(), &file.module, &file.table, &known) {
            entries.insert((file.path, offset), callee);
        }
    }
    let mut path_agreed = 0;
    let mut external_agreed = 0;
    let mut constructors = 0;
    let mut open: BTreeMap<String, usize> = BTreeMap::new();
    let mut wrong = Vec::new();
    let mut total = 0;
    for call in recording["calls"].as_array().expect("calls") {
        let kind = call["kind"].as_str().expect("kind");
        if kind.starts_with("m:") {
            continue;
        }
        total += 1;
        let path = call["path"].as_str().expect("path");
        let source = FILES
            .iter()
            .find(|(file, _)| *file == path)
            .expect("file")
            .1;
        let offset = offset_of(
            source,
            call["line"].as_u64().expect("line"),
            call["column"].as_u64().expect("column"),
        );
        let target = &call["target"];
        let describe = format!(
            "{path}:{}:{} {}",
            call["line"], call["column"], call["text"]
        );
        match entries.get(&(path, offset)) {
            None => {
                let symbol = call["symbol"].as_str().unwrap_or("");
                if symbol.ends_with('#') {
                    constructors += 1;
                } else {
                    wrong.push(format!("{describe}: no entry for a function call"));
                }
            }
            Some(Callee::Open) => *open.entry(kind.to_owned()).or_default() += 1,
            Some(Callee::Path(name)) => {
                let Some((def_path, def_offset)) = by_name.get(name.as_str()) else {
                    wrong.push(format!("{describe}: {name} is no definition"));
                    continue;
                };
                let def_source = FILES
                    .iter()
                    .find(|(file, _)| file == def_path)
                    .expect("file")
                    .1;
                let agrees = target["path"].as_str() == Some(def_path)
                    && offset_of(
                        def_source,
                        target["line"].as_u64().unwrap_or(0).max(1),
                        target["column"].as_u64().unwrap_or(0).max(1),
                    ) == *def_offset;
                if agrees {
                    path_agreed += 1;
                } else {
                    wrong.push(format!("{describe}: Path({name}) but oracle says {target}"));
                }
            }
            Some(Callee::External(name)) => {
                if target.get("external").is_some() {
                    external_agreed += 1;
                } else {
                    wrong.push(format!(
                        "{describe}: External({name}) but oracle says {target}"
                    ));
                }
            }
        }
    }
    eprintln!(
        "calls outside macros: {total}; path agreed {path_agreed}; external agreed {external_agreed}; \
         constructors without entry {constructors}; open by oracle kind {open:?}"
    );
    assert!(wrong.is_empty(), "wrong answers:\n{}", wrong.join("\n"));
    assert!(path_agreed > 0);
}

#[test]
fn module_paths_follow_the_file_layout() {
    let paths: Vec<(&str, Option<Vec<String>>)> = FILES
        .iter()
        .map(|(path, _)| (*path, module_of(path)))
        .collect();
    let recording = recording();
    let recorded: Vec<&str> = recording["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|file| file.as_str())
        .collect();
    assert_eq!(paths.len(), recorded.len());
    for (path, module) in &paths {
        assert!(recorded.contains(path));
        assert!(module.is_some());
    }
    assert_eq!(module_of("tests/it.rs"), None);
    assert_eq!(module_of("src/bin/tool.rs"), None);
}

/// What each call of a one-file crate (`src/lib.rs`) calls, by the call's source text.
fn calls_of(source: &str) -> Vec<(String, Option<Callee>)> {
    let root = GrammarDialect::Rust.ast_grep(source);
    let table = build_for_test(source);
    let module = Vec::new();
    let known = Known::new(
        definitions(&root.root(), &module, &table)
            .into_iter()
            .map(|(name, _)| name),
        trait_names(&root.root(), &module, &table),
    );
    let entries: HashMap<usize, Callee> = callees(&root.root(), &module, &table, &known)
        .into_iter()
        .collect();
    root.root()
        .dfs()
        .filter(|node| node.kind() == "call_expression")
        .map(|call| {
            let function = call.field("function").expect("callee");
            let at = function
                .field("name")
                .unwrap_or_else(|| function.clone())
                .range()
                .start;
            (call.text().to_string(), entries.get(&at).cloned())
        })
        .collect()
}

#[test]
fn a_local_binding_shadows_a_function_item() {
    // rustc: the call reads the closure `let f` binds, not the function.
    assert_eq!(
        calls_of("fn f() {}\nfn main() { let f = || {}; f(); }\n"),
        [("f()".to_owned(), Some(Callee::Open))]
    );
}

#[test]
fn use_path_heads_resolve_through_their_lexical_bindings() {
    let modules = "mod a { pub fn f() {} }\nmod b { pub fn f() {} }\n";
    // `a` in `use a::f` is the alias the block's first `use` binds: `f` is `b::f`.
    assert_eq!(
        calls_of(&format!(
            "{modules}fn main() {{ use crate::b as a; use a::f; f(); }}\n"
        )),
        [(
            "f()".to_owned(),
            Some(Callee::Path("crate::b::f".to_owned()))
        )]
    );
    // Through a chain of aliases, and from an outer scope.
    assert_eq!(
        calls_of(&format!(
            "{modules}use crate::b as c;\nfn main() {{ use c as a; {{ use a::f; f(); }} }}\n"
        )),
        [(
            "f()".to_owned(),
            Some(Callee::Path("crate::b::f".to_owned()))
        )]
    );
    // Aliases naming each other end open, not in a loop.
    assert_eq!(
        calls_of("use b as a;\nuse a as b;\nuse a::f;\nfn main() { f(); }\n"),
        [("f()".to_owned(), Some(Callee::Open))]
    );
}

#[test]
fn prelude_constructors_a_glob_may_shadow_stay_open() {
    // `use m::*` brings `m::Some`, which shadows the prelude's variant: the call calls it.
    assert_eq!(
        calls_of("mod m { pub fn Some() {} }\nuse m::*;\nfn main() { Some(); }\n"),
        [("Some()".to_owned(), Some(Callee::Open))]
    );
    // Without a glob, `Some(1)` builds a value.
    assert_eq!(
        calls_of("fn main() { Some(1); }\n"),
        [("Some(1)".to_owned(), None)]
    );
}

#[test]
fn a_path_attribute_leaves_the_crates_links_open() {
    use super::super::closure::Graph;
    use super::super::names::Name;
    use super::summary;
    use crate::code_rule_dsl::language::Family;
    use std::sync::Arc;
    let summarize = |path: &str, source: &str| {
        let root = GrammarDialect::Rust.ast_grep(source);
        let table = build_for_test(source);
        Arc::new(summary(&root.root(), path, Some(&table), false))
    };
    let caller = ("src/c.rs", "pub fn run() { crate::a::hit(); }\n");
    let files = |lib: &str| {
        [
            ("src/lib.rs", lib),
            ("src/a.rs", "pub fn hit() {}\n"),
            ("src/b.rs", "pub fn hit() {}\n"),
            caller,
        ]
        .into_iter()
        .map(|(path, source)| (Arc::<str>::from(path), summarize(path, source)))
        .collect::<Vec<_>>()
    };
    let plain = "mod a;\nmod b;\nmod c;\n";
    // `crate::a` is `src/b.rs`: its file name does not tell it.
    let swapped = "#[path = \"b.rs\"]\nmod a;\n#[path = \"a.rs\"]\nmod b;\nmod c;\n";
    let hit = Name::new(Family::Rust, "crate::a::hit");
    let mut graph = Graph::link(files(plain));
    let (defs, open) = graph.link_of(&hit);
    assert!(!open);
    assert_eq!(
        defs.iter()
            .map(|def| graph.path(def.file()).expect("path").to_string())
            .collect::<Vec<_>>(),
        ["src/a.rs"]
    );
    // Declaring the swap relinks the unchanged caller's call.
    graph.update([(
        Arc::from("src/lib.rs"),
        Some(summarize("src/lib.rs", swapped)),
    )]);
    let (defs, open) = graph.link_of(&hit);
    assert!(open && defs.is_empty());
    let graph = Graph::link(files(swapped));
    let (defs, open) = graph.link_of(&hit);
    assert!(open && defs.is_empty());
    // An attribute a `cfg_attr` applies counts too.
    let graph = Graph::link(files(
        "#[cfg_attr(unix, path = \"b.rs\")]\nmod a;\nmod b;\nmod c;\n",
    ));
    let (defs, open) = graph.link_of(&hit);
    assert!(open && defs.is_empty());
}

/// Whether a call of `a` in `main` (`src/lib.rs`, with `source` defining `a`) must reach a
/// definition that calls `sink`, evaluated as a rule: `Some(true)` where it reaches, `Some(false)`
/// where it does not, `None` where that is unknown (neither claim reports).
fn a_reaches_sink(source: &str) -> Option<bool> {
    use crate::code_rule_dsl::test_support::*;
    let sink = serde_json::json!({ "has": {
        "stopBy": { "kind": "function_item" }, "kind": "call_expression",
        "has": { "field": "function", "regex": "^sink$" },
    } });
    let call_of_a = |calls: Value| {
        serde_json::json!({
            "kind": "call_expression",
            "has": { "field": "function", "regex": "^a$" },
            "all": [calls],
        })
    };
    let source = format!("{source}\nfn sink() {{}}\npub fn main() {{ a(); }}\n");
    let reported = |rule: Value| {
        let document = clause_document(serde_json::json!({
            "id": "reach",
            "forbid": { "ast": { "language": "rust", "rule": rule } },
            "violation": violation(),
        }));
        let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
        let compiled = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let snapshot = serde_json::json!({
            "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0,
            "files": [{ "language": "rust", "path": "src/lib.rs", "source": source }],
        });
        let output: Value = serde_json::from_str(
            &compiled
                .evaluate(snapshot.to_string(), "report".to_owned())
                .expect("evaluation succeeds"),
        )
        .expect("evaluation is JSON");
        assert_eq!(output["failures"], serde_json::json!([]), "{output}");
        !output["violations"]
            .as_array()
            .expect("violations")
            .is_empty()
    };
    let reaches = reported(call_of_a(serde_json::json!({ "calls": sink })));
    let absent = reported(call_of_a(serde_json::json!({ "not": { "calls": sink } })));
    match (reaches, absent) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
        (true, true) => panic!("a call both reaches and does not reach: {source}"),
    }
}

#[test]
fn a_call_in_a_macros_tokens_is_open_never_absent() {
    let callees = "fn b() -> i32 { sink(); 1 }\nmacro_rules! m { () => { b() }; }\n";
    // The control: a call outside a macro links, and a body that calls nothing reaches nothing.
    assert_eq!(
        a_reaches_sink(&format!("pub fn a() {{ b(); }}\n{callees}")),
        Some(true)
    );
    assert_eq!(
        a_reaches_sink(&format!("pub fn a() {{ let _ = 1; }}\n{callees}")),
        Some(false)
    );
    // rustc 1.99.0: each of these calls `b`, so `a` reaches `sink`; read from tokens it is unknown.
    for body in [
        r#"println!("{}", b());"#,
        "assert!(b() == 1);",
        "let _ = vec![b()];",
        r#"std::println!("{}", format!("{}", b()));"#,
        "assert_eq!(1, Some(1).map(|_| b()).unwrap_or(0));",
        "m!();",
        r#"println!("{}", m!());"#,
        "let _ = vec![b::<>()];",
    ] {
        assert_eq!(
            a_reaches_sink(&format!("pub fn a() {{ {body} }}\n{callees}")),
            None,
            "{body}"
        );
    }
    // The standard library's macros whose tokens hold no call call nothing of the crate.
    for body in [
        r#"println!("{} {x}", 1, x = 2);"#,
        "assert!(true);",
        "let _ = vec![1, 2];",
        r#"assert_eq!(1, 1, "{}", "same");"#,
        "let _ = matches!(1, 0..=2);",
    ] {
        assert_eq!(
            a_reaches_sink(&format!("pub fn a() {{ {body} }}\n{callees}")),
            Some(false),
            "{body}"
        );
    }
}

/// A tree deeper than the scope table's nesting bound: the summary is open, and reading it uses
/// no stack per level (a 256 KiB thread of a debug build).
#[test]
fn a_deep_file_summarizes_open_on_a_small_stack() {
    use super::summary;
    let terms = vec!["1"; 3000].join(" + ");
    let parens = format!("{}1{}", "(".repeat(2000), ")".repeat(2000));
    let source = format!("pub fn f() {{ let _ = {terms}; let _ = {parens}; }}\n");
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(move || {
            let root = GrammarDialect::Rust.ast_grep(&source);
            let summary = summary(&root.root(), "src/lib.rs", None, false);
            assert!(summary.open);
            assert_eq!(summary.defs.len(), 1);
        })
        .expect("thread")
        .join()
        .expect("no stack overflow");
}
