//! Port-shaped rules for Ruff's Pyflakes usedness and resolution rules, against where Ruff 0.16.9
//! itself reports them on the fixtures in `fixtures/python/scope/usage` (`fixtures/python/scope/generate_usage.py`).
//! Where the engine is conservative by design (a read `eval` may make) the port reports less, and
//! the test lists each such range.

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("python/scope/usage.expected.json");

/// Where a Python rule reports on a fixture (`.pyi` for a key that ends so): each finding's byte
/// range, sorted. A first line `# target-version: py3XY` sets the query's `pythonVersion`.
fn ranges(key: &str, rule: Value, source: &str) -> Vec<(u64, u64)> {
    let mut ast = json!({ "language": "python", "rule": rule });
    if let Some(minor) = source
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("# target-version: py3"))
    {
        ast["languageOptions"] = json!({ "pythonVersion": format!("3.{}", minor.trim()) });
    }
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": ast },
        "violation": { "anchor": "witness.span", "code": "PORT", "message": "port" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let path = if key.ends_with(".pyi") {
        "fixture.pyi"
    } else {
        "fixture.py"
    };
    let result = evaluate_file(&compiled, "python", path, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut ranges = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            (
                finding["span"]["startByte"].as_u64().expect("start"),
                finding["span"]["endByte"].as_u64().expect("end"),
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

fn expected(code: &str) -> Vec<(u64, u64)> {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["ruff"], "0.16.9");
    oracle[code]["ranges"]
        .as_array()
        .expect("code")
        .iter()
        .map(|pair| {
            (
                pair[0].as_u64().expect("start"),
                pair[1].as_u64().expect("end"),
            )
        })
        .collect()
}

/// Checks a port against Ruff's ranges, but for the `conservative` ones (as `line:text`), which
/// the engine leaves unknown by design and the port therefore does not report.
fn check(code: &str, rule: Value, source: &str, conservative: &[&str]) {
    let label = |(start, end): &(u64, u64)| {
        let (start, end) = (*start as usize, *end as usize);
        format!(
            "{}:{}",
            source[..start].matches('\n').count() + 1,
            &source[start..end]
        )
    };
    let ours = ranges(code, rule, source);
    let theirs = expected(code);
    let only_ours = ours
        .iter()
        .filter(|range| !theirs.contains(range))
        .map(label)
        .collect::<Vec<_>>();
    let only_theirs = theirs
        .iter()
        .filter(|range| !ours.contains(range))
        .map(label)
        .collect::<Vec<_>>();
    assert_eq!(
        only_ours,
        Vec::<String>::new(),
        "{code}: reported only by the port"
    );
    assert_eq!(only_theirs, conservative, "{code}: reported only by Ruff");
}

/// Ruff's default `dummy-variable-rgx`.
const DUMMY: &str = "^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$";

/// F841: an assignment, walrus, `with` name, match capture or `except` name of a function or
/// lambda that nothing reads, as the latest binding of its name, not under `global`/`nonlocal`,
/// not a dummy or traceback name, and not an unpacking target (unless its value is a display).
pub(in crate::policy_dsl::compile) fn f841() -> Value {
    // `(x) = v` is a plain name: tree-sitter reads its parentheses as a tuple pattern with no comma.
    let targets = json!([
        { "kind": "pattern_list" },
        { "kind": "list_pattern" },
        { "kind": "tuple_pattern", "has": { "regex": "^,$" } },
    ]);
    json!({
        "kind": "identifier",
        "any": [
            { "binds": {
                "form": ["assignment", "annotated", "augmented", "named", "with", "match"],
                "scope": ["function", "lambda"], "latest": true, "outer": false, "used": false,
            } },
            { "binds": { "form": "except", "used": false } },
        ],
        "not": { "any": [
            { "regex": DUMMY },
            { "regex": "^(__tracebackhide__|__traceback_info__|__traceback_supplement__|__debuggerskip__)$" },
            { "all": [
                { "inside": { "any": targets, "stopBy": { "kind": "assignment" } } },
                { "binds": { "site": { "kind": "assignment",
                    "not": { "has": { "any": [{ "kind": "tuple" }, { "kind": "expression_list" }, { "kind": "list" }, { "kind": "set" }] } } } } },
            ] },
            // `w = x, y = 1, 2`: a target that is no tuple makes the chain an unpacking.
            { "all": [
                { "inside": { "any": targets, "stopBy": { "kind": "assignment" } } },
                { "binds": { "site": { "kind": "assignment",
                    "inside": { "kind": "assignment", "has": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }, { "kind": "subscript" }] } } } } },
            ] },
            { "all": [
                { "inside": { "any": [{ "kind": "tuple" }, { "kind": "tuple_pattern" }, { "kind": "list" }, { "kind": "list_pattern" }] } },
                { "inside": { "kind": "as_pattern_target", "stopBy": "end" } },
            ] },
        ] },
    })
}

/// F401: an import of a module or function scope that nothing reads, as the latest binding of its
/// name, not an explicit re-export, not under `global`, and not a `__future__` import.
pub(in crate::policy_dsl::compile) fn f401() -> Value {
    json!({
        "any": [{ "kind": "dotted_name" }, { "kind": "identifier" }],
        "binds": {
            "form": "import", "scope": ["module", "function"],
            "latest": true, "used": false, "exported": false, "outer": false,
        },
        "not": { "inside": { "kind": "future_import_statement", "stopBy": "end" } },
    })
}

#[test]
fn f841_unused_local_variables() {
    let source = oracle!("python/scope/usage/F841.py");
    check("F841", f841(), source, &[]);
}

#[test]
fn f401_unused_imports() {
    let source = oracle!("python/scope/usage/F401.py");
    // `decimal` names a local import in a module-level string annotation, which reads the module's
    // names: the local import is unused.
    check("F401", f401(), source, &[]);
    // An `__all__` that is `None`, a number, a boolean or `...` lists no names: imports stay
    // known.
    let source = oracle!("python/scope/usage/F401_all_none.py");
    check("F401_all_none", f401(), source, &[]);
}

#[test]
fn f401_forward_references_read_the_module_first() {
    let source = oracle!("python/scope/usage/F401_forward.py");
    check("F401_forward", f401(), source, &[]);
    let source = oracle!("python/scope/usage/F401_forward_py314.py");
    check("F401_forward_py314", f401(), source, &[]);
}

/// F821 (and F822 for `__all__` entries): a name with no binding, builtin or star import, unless
/// a `try` around it handles `NameError`.
fn f821() -> Value {
    json!({
        "any": [{ "kind": "identifier" }, { "kind": "string" }],
        "resolves": { "unresolved": true },
        "not": { "inside": { "kind": "try_statement", "stopBy": "end",
            "has": { "kind": "except_clause", "has": { "kind": "identifier", "regex": "^NameError$" } } } },
    })
}

/// F405: a name, or a module `__all__` entry, only a star import may bind.
fn f405() -> Value {
    json!({
        "any": [{ "kind": "identifier" }, { "kind": "string" }, { "kind": "concatenated_string" }],
        "resolves": { "unresolved": "star" },
    })
}

#[test]
fn f821_undefined_names() {
    let source = oracle!("python/scope/usage/F821.py");
    check("F821", f821(), source, &[]);
}

#[test]
fn f821_module_annotations_a_function_declares_global() {
    // Ruff's `set_globals` gives a name a function or class body declares `global` a module
    // binding where the module has only a bare annotation of it (M10a's fix round reported
    // these reads; M11a's integration restored the placeholder).
    let source = oracle!("python/scope/usage/F821_global_annotation.py");
    check("F821_global_annotation", f821(), source, &[]);
    let source = oracle!("python/scope/usage/F821_global_annotation_py314.py");
    check("F821_global_annotation_py314", f821(), source, &[]);
}

#[test]
fn f821_expired_except_names_fall_back_to_the_builtin() {
    // Ruff's `UnboundException` keeps what the name resolved to when the handler began, the
    // builtin included; with nothing there the name is undefined.
    let source = oracle!("python/scope/usage/F821_except_builtin.py");
    check("F821_except_builtin", f821(), source, &[]);
    let source = oracle!("python/scope/usage/F821_except_builtin_py39.py");
    check("F821_except_builtin_py39", f821(), source, &[]);
}

#[test]
fn f821_undefined_names_under_a_star_import() {
    // An `except` name after its handler is unbound even where a star import may bind the name.
    let source = oracle!("python/scope/usage/F821_star.py");
    check("F821_star", f821(), source, &[]);
}

/// F842: a bare annotation of a function that nothing reads, as the latest binding of its name,
/// not a dummy name.
fn f842() -> Value {
    json!({
        "kind": "identifier",
        "binds": { "form": "annotation", "scope": ["function", "lambda"], "latest": true, "used": false },
        "not": { "regex": DUMMY },
    })
}

#[test]
fn f842_unused_annotations() {
    let source = oracle!("python/scope/usage/F842.py");
    check("F842", f842(), source, &[]);
}

/// F823: the first read in a function of the binding its own later binding of the name shadows.
fn f823() -> Value {
    json!({ "kind": "identifier", "resolves": { "beforeLocal": true } })
}

#[test]
fn f823_locals_read_before_their_binding() {
    let source = oracle!("python/scope/usage/F823.py");
    check("F823", f823(), source, &[]);
}

#[test]
fn f823_locals_shadowing_a_builtin() {
    // Ruff materializes the builtin in the module scope, so the local's shadowed binding is it,
    // under a star import too; the builtins are the target version's.
    let source = oracle!("python/scope/usage/F823_builtin.py");
    check("F823_builtin", f823(), source, &[]);
    let source = oracle!("python/scope/usage/F823_builtin_py39.py");
    check("F823_builtin_py39", f823(), source, &[]);
}

#[test]
fn f823_skips_reads_in_a_definitions_type_scope() {
    // A class's bases and keywords and a function's annotations are read in the type scope Ruff
    // opens around the definition; defaults and decorators are the enclosing function's.
    let source = oracle!("python/scope/usage/F823_type_scope.py");
    check("F823_type_scope", f823(), source, &[]);
}

#[test]
fn f823_skips_forward_references_the_module_resolves() {
    // Ruff reads a forward reference the module binds from the module's scope (PEP 563): a
    // deferred annotation (3.14, `from __future__ import annotations`) or a string one.
    let source = oracle!("python/scope/usage/F823_deferred_py314.py");
    check("F823_deferred_py314", f823(), source, &[]);
    let source = oracle!("python/scope/usage/F823_future.py");
    check("F823_future", f823(), source, &[]);
    let source = oracle!("python/scope/usage/F823_string.py");
    check("F823_string", f823(), source, &[]);
}

/// F822: a module `__all__` entry that names nothing the module binds; an implicit
/// concatenation is one entry.
fn f822() -> Value {
    json!({ "any": [{ "kind": "string" }, { "kind": "concatenated_string" }], "resolves": { "unresolved": true } })
}

#[test]
fn f822_undefined_exports() {
    let source = oracle!("python/scope/usage/F822.py");
    check("F822", f822(), source, &[]);
    // A `list(...)` whose callee is no builtin gives no entries.
    let source = oracle!("python/scope/usage/F822_shadowed.py");
    check("F822_shadowed", f822(), source, &[]);
}

#[test]
fn stubs_read_names_after_the_module_and_bind_bare_annotations() {
    let source = oracle!("python/scope/usage/F821_stub.pyi");
    check("F821_stub.pyi", f821(), source, &[]);
    let source = oracle!("python/scope/usage/F822_stub.pyi");
    check("F822_stub.pyi", f822(), source, &[]);
    let source = oracle!("python/scope/usage/F401_stub.pyi");
    check("F401_stub.pyi", f401(), source, &[]);
}

/// A stub defers only what Ruff reads as a type definition: annotations, class bases, `TypeAlias`
/// values, the type arguments of `typing`'s special calls (`cast`, `TypeVar`, `NewType`,
/// `NamedTuple`, `TypedDict`, ...) and the slices of generic subscripts. Every other name is read in
/// order, as in a `.py` file.
#[test]
fn stubs_defer_only_type_definitions_and_class_bases() {
    let source = oracle!("python/scope/usage/F821_stub_positions.pyi");
    check("F821_stub_positions.pyi", f821(), source, &[]);
    // A shadowed `cast`, `List` or `list` is no typing callable or generic.
    let source = oracle!("python/scope/usage/F821_stub_shadowed.pyi");
    check("F821_stub_shadowed.pyi", f821(), source, &[]);
    // TypedDict's keyword fields are types before 3.13; `frozendict` is a builtin generic from 3.15.
    let source = oracle!("python/scope/usage/F821_stub_py313.pyi");
    check("F821_stub_py313.pyi", f821(), source, &[]);
    let source = oracle!("python/scope/usage/F821_stub_py315.pyi");
    check("F821_stub_py315.pyi", f821(), source, &[]);
}

#[test]
fn f405_names_only_a_star_import_may_bind() {
    let source = oracle!("python/scope/usage/F405.py");
    check("F405", f405(), source, &[]);
}

/// `import a.b` without an alias (Ruff's `SubmoduleImport`), as a site rule.
fn submodule_site() -> Value {
    json!({ "kind": "dotted_name", "regex": "\\.", "inside": { "kind": "import_statement", "stopBy": "neighbor" } })
}

/// `import m`, `import m as x` or `from m import x` (Ruff's `Import` and `FromImport`).
fn module_or_member_site() -> Value {
    json!({ "not": { "any": [
        submodule_site(),
        { "inside": { "kind": "future_import_statement", "stopBy": "neighbor" } },
    ] } })
}

/// A binding class of Ruff's `Binding::redefines`: forms, and a site rule where forms do not
/// decide.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `Import`, `FromImport`.
    Import,
    /// `SubmoduleImport`.
    Submodule,
    /// `Assignment`, `NamedExprAssignment` (comprehension targets too, loop targets not).
    Assignment,
    /// `ClassDefinition`, `FunctionDefinition`.
    Definition,
    /// Every other binding that may redefine (a parameter, `with` or `except` name, ...).
    Other,
}

impl Kind {
    /// Filters selecting the kind: one or two (form, site) pairs.
    fn filters(self) -> Vec<(Value, Option<Value>)> {
        match self {
            Kind::Import => vec![(json!("import"), Some(module_or_member_site()))],
            Kind::Submodule => vec![(json!("import"), Some(submodule_site()))],
            Kind::Assignment => vec![
                (
                    json!(["assignment", "annotated", "augmented", "match", "named"]),
                    None,
                ),
                (json!("for"), Some(json!({ "kind": "for_in_clause" }))),
            ],
            Kind::Definition => vec![(json!("definition"), None)],
            Kind::Other => vec![(
                json!([
                    "with",
                    "except",
                    "parameter",
                    "global",
                    "typeParameter",
                    "typeAlias"
                ]),
                None,
            )],
        }
    }
}

/// Ruff's `Binding::redefines`: what each kind redefines, and whether only the same qualified
/// name does.
fn redefinable(binding: Kind) -> Vec<(Kind, bool)> {
    use Kind::{Assignment, Definition, Import, Other, Submodule};
    match binding {
        Import => vec![
            (Submodule, true),
            (Import, false),
            (Definition, false),
            (Assignment, false),
        ],
        Submodule => vec![
            (Import, true),
            (Submodule, true),
            (Definition, false),
            (Assignment, false),
        ],
        Assignment => vec![(Import, false), (Definition, false)],
        Definition | Other => vec![(Import, false), (Definition, false), (Assignment, false)],
    }
}

/// F811: a binding that redefines an unused one of its own scope, or an unused import of an
/// enclosing function or module scope (not from a class body, not an explicit re-export), on the
/// same fork, as Ruff's `Binding::redefines` allows; not a loop variable, not a dummy name that
/// redefines a non-import, not a function redefining an `@overload`.
fn f811() -> Value {
    use Kind::{Assignment, Definition, Import, Other, Submodule};
    let overload = json!({ "inside": { "kind": "decorated_definition", "stopBy": "neighbor",
        "has": { "kind": "decorator", "regex": "overload" } } });
    let mut terms = Vec::new();
    for binding in [Import, Submodule, Assignment, Definition, Other] {
        for (previous, by_name) in redefinable(binding) {
            let import = matches!(previous, Import | Submodule);
            for same_scope in [true, false] {
                if !same_scope && !import {
                    continue;
                }
                for (form, site) in binding.filters() {
                    for (previous_form, previous_site) in previous.filters() {
                        let mut redefines = json!({ "form": previous_form, "used": false, "sameScope": same_scope });
                        if by_name {
                            redefines["sameImport"] = json!(true);
                        }
                        if !same_scope {
                            redefines["exported"] = json!(false);
                        }
                        let previous_site = match (previous, same_scope) {
                            (Definition, true) => Some(json!({ "not": overload })),
                            _ => previous_site.clone(),
                        };
                        if let Some(site) = previous_site {
                            redefines["site"] = site;
                        }
                        let mut binds = json!({ "form": form, "redefines": redefines });
                        if let Some(site) = &site {
                            binds["site"] = site.clone();
                        }
                        if !same_scope {
                            binds["scope"] = json!([
                                "module",
                                "function",
                                "lambda",
                                "comprehension",
                                "typeParameters"
                            ]);
                        }
                        let mut term = json!({ "binds": binds });
                        if same_scope && !import {
                            term["not"] = json!({ "regex": DUMMY });
                        }
                        terms.push(term);
                    }
                }
            }
        }
    }
    json!({ "any": [{ "kind": "identifier" }, { "kind": "dotted_name" }], "all": [{ "any": terms }] })
}

/// F402: a loop variable of a `for` statement that shadows an import (used or not) of its scope or
/// an enclosing one, on the same fork.
fn f402() -> Value {
    json!({
        "kind": "identifier",
        "binds": { "form": "for", "site": { "kind": "for_statement" }, "redefines": { "form": "import" } },
    })
}

#[test]
fn f811_redefinitions_of_unused_bindings() {
    let source = oracle!("python/scope/usage/F811.py");
    check("F811", f811(), source, &[]);
}

#[test]
fn f402_imports_shadowed_by_loop_variables() {
    let source = oracle!("python/scope/usage/F402.py");
    check("F402", f402(), source, &[]);
}
