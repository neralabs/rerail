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
