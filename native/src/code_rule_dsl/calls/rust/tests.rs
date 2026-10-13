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
