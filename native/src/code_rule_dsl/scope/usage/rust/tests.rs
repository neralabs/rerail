use super::super::Steps;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::scope::rust::build_for_test;
use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
use crate::code_rule_dsl::scope::{Form, ScopeKind};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value;

fn source(path: &str) -> &'static str {
    match path.rsplit('/').next().unwrap_or_default() {
        "closures.rs" => {
            include_str!("../../../../../../fixtures/rust/scope/crate/src/closures.rs")
        }
        "generics.rs" => {
            include_str!("../../../../../../fixtures/rust/scope/crate/src/generics.rs")
        }
        "imports.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/imports.rs"),
        "items.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/items.rs"),
        "lets.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/lets.rs"),
        "lib.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/lib.rs"),
        "macros.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/macros.rs"),
        "patterns.rs" => {
            include_str!("../../../../../../fixtures/rust/scope/crate/src/patterns.rs")
        }
        "unused.rs" => include_str!("../../../../../../fixtures/rust/scope/crate/src/unused.rs"),
        other => panic!("unknown fixture {other}"),
    }
}

/// Each binding rustc's unused lints judge in the scope fixtures (`fixtures/rust/scope`): rustc
/// 1.99.0 reports it unused (`unused_variables`, `unused_imports`, `dead_code`) exactly where no
/// read reaches it. A binding rustc reports is never known used; one it does not report is never
/// known unused, outside what it does not judge: `_` names, `self`, public items, items of a file
/// whose child modules may use them, `as _` imports, `pub use` re-exports, modules, bodiless
/// signatures' parameters, and code under an `allow` attribute.
#[test]
fn rust_usedness_agrees_with_rustc_unused_lints() {
    let oracle: Value =
        serde_json::from_str(oracle!("rust/scope/scope.expected.json")).expect("oracle JSON");
    let (mut judged, mut agreed, mut unknown) = (0, 0, 0);
    let mut failures = Vec::new();
    for path in oracle["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(Value::as_str)
    {
        let text = source(path);
        // rustc places `unused_imports` at the path's start and `unused_variables` at a `mut`
        // pattern's start: a lint names the binding by its line and the name in its message.
        let reported: Vec<(usize, String)> = oracle["lints"]
            .as_array()
            .expect("lints")
            .iter()
            .filter(|lint| {
                lint["path"].as_str() == Some(path)
                    && matches!(
                        lint["lint"].as_str(),
                        Some("unused_variables" | "unused_imports" | "dead_code")
                    )
            })
            .map(|lint| {
                (
                    lint["line"].as_u64().unwrap_or(0) as usize,
                    lint["message"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let root = GrammarDialect::Rust.ast_grep(text);
        let root = root.root();
        let table = build_for_test(text);
        let mut steps = Steps {
            cost: 0,
            cap: usize::MAX,
            deadline: &|| false,
        };
        let usage = Usage::rust(&table, &root, &mut steps).expect("usage");
        let child_modules = root
            .dfs()
            .any(|node| node.kind() == "mod_item" && node.field("body").is_none());
        for (scope_index, scope) in table.scopes.iter().enumerate() {
            for (name, chain) in &scope.bindings {
                for (index, binding) in chain.iter().enumerate() {
                    let Some(name_node) = binding.name else {
                        continue;
                    };
                    let Some(site) = binding.site.node.and_then(|site| site.find(root.clone()))
                    else {
                        continue;
                    };
                    let local =
                        matches!(binding.site.form, Form::Parameter | Form::Match | Form::For)
                            || (binding.site.form == Form::Assignment
                                && binding.offset != scope.range.start);
                    let item = !local
                        && matches!(
                            binding.site.form,
                            Form::Definition | Form::Static | Form::Assignment
                        )
                        && scope.kind == ScopeKind::Module;
                    let import = binding.site.form == Form::Import;
                    let allowed =
                        site.ancestors()
                            .chain(std::iter::once(site.clone()))
                            .any(|node| {
                                node.prev_all()
                                    .take_while(|sibling| {
                                        sibling.kind() == "attribute_item"
                                            || sibling.kind().ends_with("comment")
                                    })
                                    .any(|sibling| sibling.text().contains("allow("))
                            });
                    // rustc judges neither a bodiless signature's parameters, a `pub use`
                    // re-export nor a module's name.
                    let signature = binding.site.form == Form::Parameter
                        && site
                            .ancestors()
                            .any(|node| node.kind() == "function_signature_item");
                    let exempt = signature
                        || site.kind() == "mod_item"
                        || (import && site.text().starts_with("pub"));
                    let judged_by_rustc = !exempt
                        && !name.starts_with('_')
                        && name.as_ref() != "self"
                        && !allowed
                        && (local
                            || import
                            || (item && !child_modules && !site.text().starts_with("pub")));
                    if !judged_by_rustc {
                        continue;
                    }
                    judged += 1;
                    let line = text[..name_node.start as usize].matches('\n').count() + 1;
                    let unused = reported.iter().any(|(at, message)| {
                        *at == line
                            && (message.contains(&format!("`{name}`"))
                                || message.contains(&format!("::{name}`")))
                    });
                    let id = BindingId {
                        scope: scope_index as u32,
                        name,
                        index,
                    };
                    match usage.facts(id).used {
                        None => unknown += 1,
                        Some(used) if used != unused => agreed += 1,
                        Some(used) => failures.push(format!(
                            "{path}:{}: `{name}` used: {used}, rustc reports it unused: {unused}",
                            name_node.start
                        )),
                    }
                }
            }
        }
    }
    println!("Rust usage oracle: {judged} bindings judged, {agreed} agree, {unknown} unknown");
    assert!(judged > 60, "only {judged} bindings judged");
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn repeated_or_pattern_binders_are_no_reads() {
    let used = |text: &str| {
        let root = GrammarDialect::Rust.ast_grep(text);
        let root = root.root();
        let table = build_for_test(text);
        let mut steps = Steps {
            cost: 0,
            cap: usize::MAX,
            deadline: &|| false,
        };
        let usage = Usage::rust(&table, &root, &mut steps).expect("usage");
        let (scope, index) = table
            .scopes
            .iter()
            .enumerate()
            .find_map(|(scope, entry)| {
                entry
                    .bindings
                    .get("x")
                    .map(|chain| (scope, chain.len() - 1))
            })
            .expect("a binding of x");
        usage
            .facts(BindingId {
                scope: scope as u32,
                name: "x",
                index,
            })
            .used
    };
    // rustc 1.99: "unused variable: `x`" at both binders.
    let arm = |body: &str| {
        format!(
            "fn main() {{ let v: Result<i32, i32> = Ok(1); match v {{ Ok(x) | Err(x) => {{{body}}} }} }}\n"
        )
    };
    assert_eq!(used(&arm("")), Some(false));
    assert_eq!(used(&arm(" let _ = x; ")), Some(true));
    let fields = "struct P { x: i32 }\nstruct Q { x: i32 }\nenum E { P(P), Q(Q) }\n\
        fn main() { let e = E::P(P { x: 1 }); match e { E::P(P { x }) | E::Q(Q { x }) => {} } }\n";
    assert_eq!(used(fields), Some(false));
}

/// The usage of `text`'s bindings.
fn usage_of(text: &str) -> (crate::code_rule_dsl::scope::ScopeTable, Usage) {
    let root = GrammarDialect::Rust.ast_grep(text);
    let table = build_for_test(text);
    let mut steps = Steps {
        cost: 0,
        cap: usize::MAX,
        deadline: &|| false,
    };
    let usage = Usage::rust(&table, &root.root(), &mut steps).expect("usage");
    (table, usage)
}

/// Whether the file-level binding `name` of `text` is used.
fn module_binding_used(text: &str, name: &str) -> Option<bool> {
    let (table, usage) = usage_of(text);
    let index = table.scopes[0].bindings[name].len() - 1;
    usage
        .facts(BindingId {
            scope: 0,
            name,
            index,
        })
        .used
}

/// A 1,500-line file's thousands of references find their nodes in one walk of the tree, not one
/// walk per reference.
#[test]
fn a_long_file_is_indexed_in_one_walk() {
    use crate::code_rule_dsl::test_support::{Walk, reset_walks, walks};
    let mut text = String::from("fn f0(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
    for n in 1..301 {
        text.push_str(&format!(
            "fn f{n}(a: i32, b: i32) -> i32 {{\n    let c = a + b * a;\n    let d = c - a + b;\n    d + c + f{}(a, b)\n}}\n",
            n - 1
        ));
    }
    assert!(text.lines().count() >= 1_500);
    let nodes = GrammarDialect::Rust.ast_grep(&text).root().dfs().count();
    reset_walks();
    let (table, _) = usage_of(&text);
    assert!(table.references.len() > 3_000);
    assert!(
        walks(Walk::UsageNode) <= nodes,
        "{} walks",
        walks(Walk::UsageNode)
    );
}

#[test]
fn imported_constants_and_variants_in_patterns_are_reads() {
    // rustc 1.99.0: no `unused_imports` for `Less`, `Greater` or `MAX`; `max` binds over the
    // imported function, which is unused.
    let source = "use std::cmp::Ordering::{self, Greater, Less};\nuse std::i32::MAX;\nuse std::cmp::max;\n\
        pub fn f(o: Ordering, n: i32) -> i32 { let max = match o { Less => 1, Greater => 2, _ => 0 }; \
        match n { MAX => max, _ => 0 } }\n";
    assert_eq!(module_binding_used(source, "Less"), Some(true));
    assert_eq!(module_binding_used(source, "Greater"), Some(true));
    assert_eq!(module_binding_used(source, "MAX"), Some(true));
    assert_eq!(module_binding_used(source, "max"), Some(false));
}
