use super::{build_for_test, resolve_index};
use crate::code_rule_dsl::oracle;
use serde_json::Value;
use std::collections::HashMap;
use std::ops::Range;

fn source(path: &str) -> &'static str {
    match path.rsplit('/').next().unwrap() {
        "closures.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/closures.rs"),
        "generics.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/generics.rs"),
        "imports.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/imports.rs"),
        "items.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/items.rs"),
        "lets.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/lets.rs"),
        "lib.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/lib.rs"),
        "macros.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/macros.rs"),
        "patterns.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/patterns.rs"),
        "unused.rs" => include_str!("../../../../../fixtures/rust/scope/crate/src/unused.rs"),
        other => panic!("unknown fixture {other}"),
    }
}

fn offset(text: &str, line: usize, column: usize) -> usize {
    let line_start = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    line_start
        + text[line_start..]
            .chars()
            .take(column - 1)
            .map(char::len_utf8)
            .sum::<usize>()
}

/// References whose binding only macro expansion makes, by `path:byte`.
const DIVERGENCES: &[(&str, &str)] = &[(
    "scope/crate/src/macros.rs:565",
    "`generated` is a function `make_fn!(generated)` defines: only expanding the macro binds it",
)];

/// What the oracle reference at a node is to a syntax-level reader.
enum Position {
    /// A lexical reference the table must hold.
    Lexical,
    /// Inside a macro token tree or an attribute: opaque.
    Opaque,
    /// A later path segment, a field, a method or an associated item: resolved through the
    /// first segment's or the receiver's members, not lexically.
    Member,
}

fn position<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> Position {
    if node.ancestors().any(|ancestor| {
        matches!(
            ancestor.kind().as_ref(),
            "token_tree" | "attribute_item" | "inner_attribute_item"
        )
    }) {
        return Position::Opaque;
    }
    if matches!(
        node.kind().as_ref(),
        "field_identifier" | "shorthand_field_identifier"
    ) {
        return Position::Member;
    }
    match node.parent() {
        // `mod m;` names the module's file; rust-analyzer reads it as a reference to it.
        Some(parent)
            if parent.kind() == "mod_item"
                && parent
                    .field("name")
                    .is_some_and(|name| name.range() == node.range()) =>
        {
            Position::Member
        }
        Some(parent)
            if matches!(
                parent.kind().as_ref(),
                "scoped_identifier"
                    | "scoped_type_identifier"
                    | "scoped_use_list"
                    | "use_as_clause"
            ) && parent
                .field("path")
                .is_none_or(|path| path.range() != node.range()) =>
        {
            Position::Member
        }
        // `use a::b`: segments of a use path name modules and crates, not lexical names.
        _ if node
            .ancestors()
            .any(|ancestor| ancestor.kind() == "use_declaration") =>
        {
            Position::Member
        }
        _ => Position::Lexical,
    }
}

/// Every reference rust-analyzer 1.99.0 reports in the scope fixtures
/// (`fixtures/rust/scope/generate.py`), read as Rerail's table reads it: a lexical reference to a
/// name defined in the same file resolves to rust-analyzer's definition; one to a name defined
/// elsewhere (another file, std) resolves to nothing or to the `use` that imports it; a reference in
/// a macro token tree or attribute is not recorded, and a member (a later path segment, a field, a
/// method) is not a lexical reference.
#[test]
fn rust_references_resolve_as_rust_analyzer_resolves_them() {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::scope::{Form, Import};
    use ast_grep_core::tree_sitter::LanguageExt;
    let oracle: Value = serde_json::from_str(oracle!("rust/scope/scope.expected.json")).unwrap();
    let symbols = oracle["symbols"].as_object().unwrap();
    let mut tables = HashMap::new();
    let mut roots = HashMap::new();
    for path in oracle["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
    {
        tables.insert(path.to_owned(), build_for_test(source(path)));
        roots.insert(path.to_owned(), GrammarDialect::Rust.ast_grep(source(path)));
    }
    // SCIP's `local N` symbols are per document: each file's definition of one.
    let mut locals: HashMap<(String, String), Value> = HashMap::new();
    for occurrence in oracle["occurrences"].as_array().unwrap() {
        let symbol = occurrence["symbol"].as_str().unwrap();
        if symbol.starts_with("local ") && occurrence["definition"].as_bool() == Some(true) {
            let path = occurrence["path"].as_str().unwrap();
            locals.insert((path.to_owned(), symbol.to_owned()), occurrence.clone());
        }
    }
    let (mut checked, mut agreed, mut opaque, mut members, mut divergent) = (0, 0, 0, 0, 0);
    let mut failures = Vec::new();
    for occurrence in oracle["occurrences"].as_array().unwrap() {
        if occurrence["definition"].as_bool() == Some(true) {
            continue;
        }
        let path = occurrence["path"].as_str().unwrap();
        let symbol = occurrence["symbol"].as_str().unwrap();
        let text = source(path);
        let at = |line: &str, column: &str, from: &Value| {
            offset(
                text,
                from[line].as_u64().unwrap() as usize,
                from[column].as_u64().unwrap() as usize,
            )
        };
        let start = at("line", "column", occurrence);
        let end = at("end_line", "end_column", occurrence);
        let name = &text[start..end];
        // Operator-trait and tuple-index occurrences (`+`, `*`, `.0`) name no lexical binding.
        let identifier = name.trim_start_matches('\'');
        if !identifier.starts_with(|c: char| c == '_' || c.is_alphabetic()) {
            continue;
        }
        let root = roots[path].root();
        let Some(node) = root
            .dfs()
            .filter(|node| node.range() == (start..end))
            .last()
        else {
            failures.push(format!("{path}:{start}: no node for {name}"));
            continue;
        };
        match position(&node) {
            Position::Opaque => {
                opaque += 1;
                continue;
            }
            Position::Member => {
                members += 1;
                continue;
            }
            Position::Lexical => {}
        }
        checked += 1;
        let table = &tables[path];
        let Some(reference) = table.reference(&Range { start, end }) else {
            failures.push(format!("{path}:{start}: {name} not recorded"));
            continue;
        };
        let found = resolve_index(table, reference, name)
            .map(|(scope, index)| &table.scopes[scope as usize].bindings[name][index]);
        let local = locals.get(&(path.to_owned(), symbol.to_owned()));
        let definition = match local {
            Some(local) => local.as_object(),
            None if symbol.starts_with("local ") => None,
            None => symbols[symbol]["definition"].as_object(),
        };
        // A name a `use` of this file imports resolves to that `use`, whatever it imports; `Self`
        // to the impl or trait it is in (rust-analyzer's impl symbol has no name to point at).
        let imported = found.is_some_and(|binding| {
            binding.site.form == Form::Import
                && matches!(&binding.import, Some(Import::Qualified(target)) if target.rsplit("::").next() == Some(name))
        });
        let own_self =
            name == "Self" && found.is_some_and(|binding| binding.site.form == Form::Implicit);
        if DIVERGENCES
            .iter()
            .any(|(site, _)| *site == format!("{path}:{start}"))
        {
            divergent += 1;
            continue;
        }
        match definition.filter(|definition| definition["path"].as_str() == Some(path)) {
            Some(definition) => {
                let expected = at("line", "column", &Value::Object(definition.clone()));
                let field = root.dfs().any(|node| {
                    node.range().start == expected && node.kind() == "field_identifier"
                });
                if field {
                    // A shorthand `Self { value }` reads the local and names the field: the
                    // oracle records the field.
                    members += 1;
                    checked -= 1;
                } else if imported
                    || own_self
                    || found.is_some_and(|binding| {
                        binding
                            .name
                            .is_some_and(|name| name.start as usize == expected)
                    })
                {
                    agreed += 1;
                } else {
                    failures.push(format!(
                        "{path}:{start}: {name} resolves to {:?}, rust-analyzer to byte {expected}",
                        found
                            .and_then(|binding| binding.name)
                            .map(|name| name.start)
                    ));
                }
            }
            None => {
                if own_self || found.is_none_or(|binding| binding.site.form == Form::Import) {
                    agreed += 1;
                } else {
                    failures.push(format!(
                        "{path}:{start}: {name} is defined elsewhere ({symbol}) but resolves to a local binding"
                    ));
                }
            }
        }
    }
    println!(
        "Rust scope oracle: {checked} lexical references, {agreed} agree, {divergent} divergent; {opaque} in token trees, {members} members"
    );
    assert!(checked > 150, "only {checked} references checked");
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The name start of the binding the reference at byte `at` of `source` (spelling `name`)
/// resolves to.
fn resolved_at(source: &str, at: usize, name: &str) -> Option<usize> {
    let table = build_for_test(source);
    let reference = table
        .reference(&Range {
            start: at,
            end: at + name.len(),
        })
        .expect("a recorded reference");
    let (scope, index) = resolve_index(&table, reference, name)?;
    table.scopes[scope as usize].bindings[name][index]
        .name
        .map(|name| name.start as usize)
}

#[test]
fn a_pattern_binds_over_a_function_name() {
    let source = "fn f() {}\nfn main() { let f = || {}; f(); }\n";
    let local = source.find("let f").expect("let") + 4;
    assert_eq!(
        resolved_at(source, source.rfind("f()").expect("call"), "f"),
        Some(local)
    );
    // A unit struct's name is a path the pattern reads.
    let source = "struct S;\nfn main() { let S = S; S; }\n";
    assert_eq!(
        resolved_at(source, source.rfind('S').expect("read"), "S"),
        Some(7)
    );
}

#[test]
fn import_paths_follow_use_aliases() {
    use super::import_path;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;
    let path_at = |source: &str, call: &str| {
        let root = GrammarDialect::Rust.ast_grep(source);
        let table = build_for_test(source);
        let at = source.rfind(call).expect("call");
        let node = root
            .root()
            .dfs()
            .find(|node| {
                node.range().start == at
                    && matches!(node.kind().as_ref(), "identifier" | "scoped_identifier")
                    && node.text() == call
            })
            .expect("node");
        import_path(&table, &node, Some("src/lib.rs"))
    };
    let modules = "mod a { pub fn f() {} }\nmod b { pub fn f() {} }\n";
    assert_eq!(
        path_at(
            &format!("{modules}fn main() {{ use crate::b as a; use a::f; f(); }}\n"),
            "f"
        )
        .as_deref(),
        Some("crate::b::f")
    );
    assert_eq!(
        path_at(
            &format!("{modules}use crate::b as c;\nfn main() {{ c::f(); }}\n"),
            "c::f"
        )
        .as_deref(),
        Some("crate::b::f")
    );
    // An `extern crate` alias names the crate.
    assert_eq!(
        path_at(
            "extern crate serde as s;\nuse s::de;\nfn main() { de(); }\n",
            "de"
        )
        .as_deref(),
        Some("serde::de")
    );
    // `use log::log;`: the head is not the `use` itself.
    assert_eq!(
        path_at("use log::log;\nfn main() { log(); }\n", "log").as_deref(),
        Some("log::log")
    );
    // Aliases naming each other name no path.
    assert_eq!(
        path_at("use b as a;\nuse a as b;\nfn main() { a::f(); }\n", "a::f"),
        None
    );
}

/// The names `resolves: { unresolved: true }` reports in the Rust file `source`.
fn unresolved_names(source: &str) -> Vec<String> {
    use crate::code_rule_dsl::test_support::*;
    let document = clause_document(serde_json::json!({
        "id": "unresolved",
        "forbid": { "ast": { "language": "rust", "rule": {
            "any": [{ "kind": "identifier" }, { "kind": "type_identifier" }, { "kind": "lifetime" }],
            "resolves": { "unresolved": true },
        } } },
        "violation": violation(),
    }));
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "rust", "src/lib.rs", source);
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    matched_texts(&result, source)
}

/// rustc 1.99.0 compiles `BOUND` (as a library): every name it reads is bound, by an associated
/// type, an `extern` block, a higher-ranked lifetime, or a macro's expansion.
const BOUND: &str = "pub trait Stream {\n    type Item: Clone;\n    type Pair<'a>: Clone where Self: 'a;\n    fn next(&mut self) -> Option<Self::Item>;\n}\n\
unsafe extern \"C\" {\n    fn abs(x: i32) -> i32;\n    static ERRNO: i32;\n}\n\
pub fn call() -> i32 { unsafe { abs(-1) + ERRNO } }\n\
pub fn hr<F>(f: F) where for<'a> F: Fn(&'a u8) -> &'a u8 { let _ = f; }\n\
pub fn hr2(f: &dyn for<'b> Fn(&'b u8) -> &'b u8) { let _ = f; }\n\
pub type Ptr = for<'c> fn(&'c u8) -> &'c u8;\n\
thread_local! { static COUNT: u8 = 0; }\n\
pub fn tl() -> u8 { COUNT.with(|c| *c) }\n\
macro_rules! make { ($n:ident) => { pub fn $n() {} }; }\n\
make!(generated);\n\
pub fn uses() { generated(); }\n";

#[test]
fn names_bound_outside_plain_items_are_not_unresolved() {
    assert_eq!(unresolved_names(BOUND), Vec::<String>::new());
    // A module's macros define nothing in a module inside it, nor does a block's in another.
    let control = format!(
        "{BOUND}mod inner {{\n    pub fn f() {{ missing(); }}\n    \
         pub fn g() {{ {{ make!(local); }} {{ local(); }} }}\n}}\n"
    );
    assert_eq!(unresolved_names(&control), ["missing", "local"]);
    // A standard expression macro defines nothing.
    assert_eq!(
        unresolved_names("pub fn f() { println!(\"x\"); missing(); }\n"),
        ["missing"]
    );
}
