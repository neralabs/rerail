use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{FileScopeSlot, UtilLimit};
use crate::code_rule_dsl::scope::ScopeTable;
use crate::code_rule_dsl::test_support::*;
use ast_grep_language::LanguageExt;
use napi::Result;
use serde_json::{Value, json};

fn compile(language: &str, ast: Value) -> Result<CompiledCodeRuleDsl> {
    let mut ast = ast;
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation() }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn python(rule: Value) -> CompiledCodeRuleDsl {
    compile("python", json!({ "rule": rule })).unwrap_or_else(|error| panic!("{}", error.reason))
}

/// Each violation as `line:text` (1-based lines).
fn found(result: &Value, source: &str) -> Vec<String> {
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let end = violation["span"]["endByte"].as_u64().expect("end") as usize;
            let line = source[..start].matches('\n').count() + 1;
            format!("{line}:{}", &source[start..end])
        })
        .collect()
}

fn run(rule: Value, source: &str) -> Vec<String> {
    let compiled = python(rule);
    found(&evaluate_file(&compiled, "python", "a.py", source), source)
}

/// F841's shape: the latest assignment, walrus or `with` name of a function that nothing reads.
fn unused_local() -> Value {
    json!({ "kind": "identifier", "binds": {
        "form": ["assignment", "annotated", "named", "with"],
        "scope": "function", "latest": true, "outer": false, "used": false,
    } })
}

#[test]
fn a_binding_is_used_by_reads_it_or_the_binding_it_shadows_sees() {
    let source = "\
def f():
    x = 1
    y = 2
    print(y)
    z = 3
    z = 4
    a = 1
    print(a)
    a = 2
    b: int = 5
    with open(p) as c:
        pass
    (d := 1)
    e = 1
    del e
    g = 1
    del g
    g = 2
";
    assert_eq!(
        run(unused_local(), source),
        ["2:x", "6:z", "10:b", "11:c", "13:d", "18:g"]
    );
    // Only the latest binding of a name is `latest`; the one it shadows is not.
    let rule = json!({ "kind": "identifier", "binds": { "scope": "function", "latest": false } });
    assert_eq!(run(rule, source), ["5:z", "7:a", "14:e", "16:g", "17:g"]);
    // A deletion reads the name: `e` and the first `g` are used.
    let rule = json!({ "kind": "identifier", "binds": { "form": "assignment", "used": true } });
    assert_eq!(run(rule, source), ["3:y", "7:a", "9:a", "14:e", "16:g"]);
    let rule = json!({ "kind": "identifier", "binds": { "form": "deletion" } });
    assert_eq!(run(rule, source), ["15:e", "17:g"]);
}

#[test]
fn referenced_counts_a_bindings_own_references_only() {
    // The second `x` inherits the first's read (used), but nothing references it itself.
    let source = "def f():\n    x = 1\n    print(x)\n    x = 2\n";
    let rule = |filter: Value| json!({ "kind": "identifier", "binds": filter });
    assert_eq!(
        run(rule(json!({ "form": "assignment", "used": true })), source),
        ["2:x", "4:x"]
    );
    assert_eq!(
        run(
            rule(json!({ "form": "assignment", "referenced": true })),
            source
        ),
        ["2:x"]
    );
    // An unseen read leaves it unknown, as `used`: `locals()` hides its own function's
    // assignments, not the module's `f`.
    let source = "def f():\n    x = 1\n    return locals()\n";
    assert_eq!(
        run(
            rule(json!({ "form": "assignment", "referenced": false })),
            source
        ),
        Vec::<String>::new()
    );
    assert_eq!(run(rule(json!({ "referenced": false })), source), ["1:f"]);
}

#[test]
fn global_and_nonlocal_bindings_are_outer_and_their_reads_count() {
    let source = "\
x = 1
def f():
    global x
    print(x)
    x = 2
def outer():
    y = 1
    def inner():
        nonlocal y
        y = 2
    return inner
";
    let rule = json!({ "kind": "identifier", "binds": { "outer": true } });
    assert_eq!(run(rule, source), ["3:x", "5:x", "10:y"]);
    // `print(x)` reads the module's `x` through the `global` declaration; the `nonlocal`
    // statement reads `outer`'s `y`.
    let rule = json!({ "kind": "identifier", "binds": { "used": true } });
    assert_eq!(run(rule, source), ["1:x", "3:x", "5:x", "7:y", "8:inner"]);
    assert_eq!(run(unused_local(), source), Vec::<String>::new());
    let rule = json!({ "kind": "identifier", "binds": { "nestedUse": true } });
    assert_eq!(run(rule, source), ["1:x", "7:y"]);
}

#[test]
fn imports_are_used_through_dunder_all_submodule_aliases_and_nested_reads() {
    // F401's shape: the latest import of a module or function scope that nothing reads.
    let rule = json!({
        "any": [{ "kind": "dotted_name" }, { "kind": "identifier" }],
        "binds": {
            "form": "import", "scope": ["module", "function"],
            "latest": true, "used": false, "exported": false,
        },
    });
    let source = "\
import os
import sys
import json as json
import pyarrow as pa
import pyarrow.csv
from a import b
from a import c
import d.e
__all__ = [\"b\"]
__all__ += (\"x\",)
pa.x
def f():
    import g
    return c
";
    assert_eq!(
        run(rule.clone(), source),
        ["1:os", "2:sys", "8:d.e", "13:g"]
    );
    let exported = json!({ "any": [{ "kind": "dotted_name" }, { "kind": "identifier" }], "binds": { "exported": true } });
    assert_eq!(run(exported, source), ["3:json", "6:b"]);
    // An `__all__` whose entries are not all known leaves module usedness unknown: neither
    // `used: false` nor its negation matches.
    let source = "import os\nimport sys\n__all__ = [\"os\"] + names()\ndef f():\n    import g\n";
    assert_eq!(run(rule, source), ["5:g"]);
    let used = json!({ "kind": "dotted_name", "not": { "binds": { "used": false } } });
    assert_eq!(run(used, source), ["1:os"]);
}

#[test]
fn reads_that_may_be_unseen_leave_usedness_unknown() {
    let source = "\
def f():
    x = 1
    return eval('x')
def g():
    y = 1
    def h():
        return locals()
def k(v: \"Seq\"):
    Seq = 1
    w = 1
    return cast(\"list[w]\", v)
def m():
    eval = print
    z = 1
    return eval()
def n(p):
    hidden = 1
    try:
        pass
    except E as e:
        pass
    return locals()
";
    // Only `locals()` hides reads, as Ruff reads it: in the function that calls it, for its
    // assignments and parameters (not `except` names); `eval`, `exec`, `vars` and `globals` hide
    // nothing. A string annotation's names are read where the string is (the parameter's, the
    // module's): the local `Seq` is unused; `cast` is not typing's, so `"list[w]"` is no type and
    // `w` is unused. Ruff 0.16.9 reports both.
    assert_eq!(
        run(unused_local(), source),
        ["2:x", "5:y", "9:Seq", "10:w", "14:z"]
    );
    let negated = json!({ "kind": "identifier", "binds": { "form": "assignment", "scope": "function" },
                          "not": { "binds": { "used": true } } });
    assert_eq!(
        run(negated, source),
        ["2:x", "5:y", "9:Seq", "10:w", "14:z"]
    );
    let names = json!({ "kind": "identifier", "binds": { "form": ["except", "parameter"], "used": false } });
    assert_eq!(run(names, source), ["20:e"]);
}

#[test]
fn sub_rules_match_the_bindings_nodes_and_bind_captures() {
    let compiled = compile(
        "python",
        json!({ "rule": { "kind": "identifier",
                          "binds": { "form": "assignment", "value": { "pattern": "$V" }, "site": { "kind": "assignment" } } },
                "constraints": { "V": { "kind": "integer" } } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "a = 1\nb = c\nd, e = 1, 2\n";
    let result = evaluate_file(&compiled, "python", "a.py", source);
    // An unpacked name's value is the display element it pairs with.
    assert_eq!(matched_texts(&result, source), ["a V=1", "d V=1", "e V=2"]);
}

#[test]
fn binds_checks_its_filters() {
    for (rule, expected) in [
        (
            json!({ "kind": "identifier", "binds": { "form": "assign" } }),
            "/clauses/0/forbid/ast/rule/binds/form: `\"assign\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor, static, relation, cte, target",
        ),
        (
            json!({ "kind": "identifier", "binds": { "scope": [] } }),
            "/clauses/0/forbid/ast/rule/binds/scope must be a scope kind name or a non-empty array of them",
        ),
        (
            json!({ "kind": "identifier", "binds": { "used": "no" } }),
            "/clauses/0/forbid/ast/rule/binds/used must be true or false",
        ),
        (
            json!({ "kind": "identifier", "binds": { "shadows": true } }),
            "unknown field `shadows` at /clauses/0/forbid/ast/rule/binds",
        ),
        (
            json!({ "kind": "identifier", "binds": { "redefines": { "redefines": {} } } }),
            "unknown field `redefines` at /clauses/0/forbid/ast/rule/binds/redefines",
        ),
        (
            json!({ "kind": "identifier", "binds": { "redefines": { "sameScope": 1 } } }),
            "/clauses/0/forbid/ast/rule/binds/redefines/sameScope must be true or false",
        ),
    ] {
        assert_eq!(
            rejection(compile("python", json!({ "rule": rule }))),
            expected,
            "{rule}"
        );
    }
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "binds": { "directives": "eslint" } } })
        )),
        "/clauses/0/forbid/ast/rule/binds/directives: /* exported */ comments are ESLint's; Python has none"
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "binds": { "directives": "jshint" } } })
        )),
        "/clauses/0/forbid/ast/rule/binds/directives must be \"eslint\" (honor /* exported */ comments)"
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "binds": { "scope": "lambda" } } })
        )),
        "/clauses/0/forbid/ast/rule/binds/scope: `\"lambda\"` is not a scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName"
    );
}

#[test]
fn before_local_is_pythons_and_alone_in_its_resolves() {
    for (rule, expected) in [
        (
            json!({ "kind": "identifier", "resolves": { "beforeLocal": false } }),
            "/clauses/0/forbid/ast/rule/resolves/beforeLocal must be true",
        ),
        (
            json!({ "kind": "identifier", "resolves": { "beforeLocal": true, "global": true } }),
            "/clauses/0/forbid/ast/rule/resolves/global: `beforeLocal` asks about a read before the function binds the name; write other `resolves` terms in a resolves of their own",
        ),
    ] {
        assert_eq!(
            rejection(compile("python", json!({ "rule": rule }))),
            expected,
            "{rule}"
        );
    }
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "resolves": { "beforeLocal": true } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/beforeLocal reads Python's binding order; TypeScript has none"
    );
    // Only the first read, not in a nested scope, and not once the function binds the name twice
    // (the latest binding then shadows its own scope's).
    let source = "\
x = 0
def f():
    print(x, x)
    g = lambda: x
    x = 1
def h():
    print(x)
    x = 1
    x = 2
";
    let rule = json!({ "kind": "identifier", "resolves": { "beforeLocal": true } });
    assert_eq!(run(rule, source), ["3:x"]);
}

#[test]
fn binds_is_source_incomplete_under_syntax_errors() {
    let compiled = python(unused_local());
    let result = evaluate_file(
        &compiled,
        "python",
        "a.py",
        "def f():\n    x = 1\ndef g(:\n",
    );
    assert_eq!(result["violations"], json!([]), "{result}");
    assert_eq!(
        result["failures"][0]["code"], "source-incomplete",
        "{result}"
    );
}

#[test]
fn the_usage_index_is_charged_to_the_budget() {
    let source = "def f():\n    x = 1\n".repeat(20);
    let root = GrammarDialect::Python.ast_grep(source.as_str());
    let name = root
        .root()
        .dfs()
        .find(|node| node.kind() == "identifier" && node.text() == "x")
        .expect("x");
    let table = ScopeTable::build(root.root(), GrammarDialect::Python, usize::MAX, &|| false)
        .expect("table");
    let compiled = python(unused_local());
    let grammar = &grammar_matchers(&compiled)[0];
    // The table fits the budget; its usage index does not.
    let mut work = table.cost + 1;
    let _slot = FileScopeSlot::install(Vec::new(), None, false);
    assert_eq!(
        grammar.match_root(name.clone(), &mut work).err(),
        Some(UtilLimit::Scope)
    );
    let mut work = 100_000;
    assert!(
        grammar
            .match_root(name.clone(), &mut work)
            .expect("within budget")
            .is_some()
    );
    assert!(work < 100_000);
}

#[test]
fn resolves_binding_filters_the_binding_a_reference_reads() {
    let source = "\
import os
counter = 0
def f():
    global counter
    counter += 1
    local = os
    return local, counter
x = 1
x = 2
print(x)
del x
";
    let reads = |filter: Value| {
        run(
            json!({ "kind": "identifier", "resolves": { "binding": filter } }),
            source,
        )
    };
    assert_eq!(reads(json!({ "form": "import" })), ["6:os"]);
    assert_eq!(
        reads(json!({ "scope": "module", "nestedUse": true })),
        ["5:counter", "6:os"]
    );
    assert_eq!(reads(json!({ "outer": true })), ["7:counter"]);
    // The last `x` is a deletion, so `x = 2` is not the live binding.
    assert_eq!(
        reads(json!({ "form": "assignment", "latest": true })),
        ["5:counter", "7:local"]
    );
    // A reference reads its binding, so `used: false` never holds on one.
    assert_eq!(reads(json!({ "used": false })), Vec::<String>::new());
    // A name `del` unbound has no binding.
    let source = "x = 1\ndel x\nprint(x)\n";
    let rule = json!({ "kind": "identifier", "resolves": { "binding": {} } });
    assert_eq!(run(rule, source), ["2:x"]);
}

#[test]
fn typescript_binding_filters_read_forms_scopes_and_usedness() {
    let compiled = compile(
        "typescript",
        json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "form": "import" } } } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "import { a } from 'm';\nconst b = 1;\nf(a, b);\n";
    let result = evaluate_file(&compiled, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["3:a"]);
    let reads = |filter: Value, source: &str| {
        let compiled = compile(
            "typescript",
            json!({ "rule": { "kind": "identifier", "resolves": { "binding": filter } } }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        found(
            &evaluate_file(&compiled, "typescript", "a.ts", source),
            source,
        )
    };
    let source = "const a = 1;\nfunction f(p) { return a + p; }\nexport const b = a;\n";
    assert_eq!(reads(json!({ "scope": "module" }), source), ["2:a", "3:a"]);
    assert_eq!(reads(json!({ "scope": "function" }), source), ["2:p"]);
    assert_eq!(reads(json!({ "nestedUse": true }), source), ["2:a", "3:a"]);
    assert_eq!(
        reads(json!({ "exported": false, "used": true }), source),
        ["2:a", "2:p", "3:a"]
    );
    for (key, value, expected) in [
        (
            "latest",
            json!(true),
            "`latest` reads Python's binding order and declarations; TypeScript has none",
        ),
        (
            "outer",
            json!(false),
            "`outer` reads Python's binding order and declarations; TypeScript has none",
        ),
    ] {
        let rule =
            json!({ "rule": { "kind": "identifier", "resolves": { "binding": { key: value } } } });
        assert_eq!(
            rejection(compile("typescript", rule)),
            format!("/clauses/0/forbid/ast/rule/resolves/binding/{key}: {expected}")
        );
    }
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "form": "let" } } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/binding/form: `\"let\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor, static, relation, cte, target"
    );
}

#[test]
fn redefines_reads_the_binding_shadowed_in_the_scope_or_an_enclosing_one_on_the_same_fork() {
    let source = "\
import os
import os
import a.b
import a.c
if c:
    import json
else:
    import json
try:
    import csv
finally:
    import csv
def f():
    import os
    x = 1
    x = 2
    del x
    x = 3
";
    let redefines = |redefines: Value| {
        run(
            json!({ "any": [{ "kind": "identifier" }, { "kind": "dotted_name" }], "binds": { "redefines": redefines } }),
            source,
        )
    };
    // Each binding and the one before it in its scope; `f`'s first `os`, the module's latest.
    // `else` and `finally` are other forks. A deletion shadows, and is shadowed, like any binding.
    assert_eq!(
        redefines(json!({})),
        ["2:os", "4:a.c", "14:os", "16:x", "17:x", "18:x"]
    );
    assert_eq!(redefines(json!({ "sameScope": false })), ["14:os"]);
    assert_eq!(redefines(json!({ "sameImport": true })), ["2:os", "14:os"]);
    assert_eq!(
        redefines(json!({ "sameImport": false })),
        ["4:a.c", "16:x", "17:x", "18:x"]
    );
    assert_eq!(redefines(json!({ "form": "deletion" })), ["18:x"]);
    assert_eq!(
        redefines(json!({ "form": "import", "used": false })),
        ["2:os", "4:a.c", "14:os"]
    );
    // The shadowed binding's sub-rules bind captures.
    let compiled = python(json!({ "kind": "identifier", "binds": {
        "form": "assignment", "redefines": { "site": { "kind": "assignment", "has": { "pattern": "$OLD", "kind": "integer" } } } } }));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(matched_texts(&result, source), ["x OLD=1"]);
    // An unknown usedness is unknown here too.
    let source = "import os\nimport os\n__all__ = names()\n";
    let rule = json!({ "kind": "dotted_name", "binds": { "redefines": { "used": false } } });
    assert_eq!(run(rule.clone(), source), Vec::<String>::new());
    let negated = json!({ "kind": "dotted_name", "binds": { "form": "import" }, "not": rule });
    assert_eq!(run(negated, source), ["1:os"]);
}

#[test]
fn ble001_logger_candidates_by_qualified_name() {
    // Ruff's `is_logger_candidate`: an object with a qualified name must be `logging` (or Flask's
    // logger); one without is judged by its name.
    let rule = json!({ "pattern": "$L.exception($$$)", "constraints": { "L": { "any": [
        { "resolves": { "import": ["logging", "flask.current_app.logger"] } },
        { "regex": "^_?log", "not": { "resolves": { "import": true } } },
    ] } } });
    let source = "import logging\nimport logbook\nlogger = logging.getLogger()\ndef log(): pass\nlogging.exception(1)\nlogger.exception(2)\nlogbook.exception(3)\nlog.exception(4)\n";
    let compiled = compile(
        "python",
        json!({ "rule": { "pattern": rule["pattern"] }, "constraints": rule["constraints"] }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["5:logging.exception(1)", "6:logger.exception(2)"]
    );
}

#[test]
fn only_counts_a_deletion_as_a_binding_the_next_one_shadows() {
    // Ruff's `del x` outside a branch adds a `Deletion` binding, which a later binding shadows, so
    // `only_binding` fails for it; a read the deletion unbound has no binding at all.
    let source = "\
x = 1
del x
x = 2
print(x)
def f():
    y = 1
    print(y)
    del y
    print(y)
def g():
    if c:
        del z
    z = 1
    print(z)
";
    let only = |extra: Value| {
        let mut binding = json!({ "only": true });
        binding
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        run(
            json!({ "kind": "identifier", "inside": { "kind": "call", "stopBy": "end" },
                    "not": { "regex": "^print$" }, "resolves": { "binding": binding } }),
            source,
        )
    };
    // `z`: a conditional `del` binds nothing (Ruff's `on_conditional_branch`).
    assert_eq!(only(json!({})), ["7:y", "14:z"]);
    // The filters read the binding `only` picked.
    assert_eq!(
        only(json!({ "form": "assignment", "latest": false })),
        ["7:y"]
    );
    assert_eq!(only(json!({ "form": "deletion" })), Vec::<String>::new());
}

#[test]
fn stop_at_global_reads_the_global_statement_which_is_outer() {
    let source = "\
v = 1
def f():
    global v
    print(v)
    v = 2
    print(v)
print(v)
";
    let reads = |binding: Value| {
        run(
            json!({ "kind": "identifier", "inside": { "kind": "argument_list" }, "resolves": { "binding": binding } }),
            source,
        )
    };
    // By default the first read in `f` continues through `global` to the module's `v = 1`; the
    // second reads `f`'s own `v = 2`, made under `global`, so outer.
    assert_eq!(reads(json!({ "outer": true })), ["6:v"]);
    assert_eq!(reads(json!({ "outer": false })), ["4:v", "7:v"]);
    assert_eq!(reads(json!({ "form": "global" })), Vec::<String>::new());
    assert_eq!(reads(json!({ "latest": true })), ["4:v", "6:v", "7:v"]);
    // `stopAtGlobal` keeps the first read at `f`'s `global` statement: outer, in a function, and
    // not `f`'s live binding once `v = 2` shadows it.
    assert_eq!(
        reads(
            json!({ "stopAtGlobal": true, "form": "global", "outer": true, "scope": "function" })
        ),
        ["4:v"]
    );
    assert_eq!(
        reads(json!({ "stopAtGlobal": true, "outer": false })),
        ["7:v"]
    );
    assert_eq!(
        reads(json!({ "stopAtGlobal": true, "latest": true })),
        ["6:v", "7:v"]
    );
    // The `global` binding is `f`'s first; `v = 2` shadows it.
    assert_eq!(
        reads(json!({ "stopAtGlobal": true, "only": true })),
        ["4:v", "7:v"]
    );
}

/// Where a TypeScript rule reports on a file, as `line:text`; a `.js` file is read in
/// tree-sitter-javascript, a dialect TypeScript rules span.
fn run_typescript(ast: Value, path: &str, source: &str) -> Vec<String> {
    let language = if path.ends_with(".tsx") {
        "tsx"
    } else if path.ends_with(".js") {
        "javascript"
    } else {
        "typescript"
    };
    let compiled = compile("typescript", ast).unwrap_or_else(|error| panic!("{}", error.reason));
    found(&evaluate_file(&compiled, language, path, source), source)
}

/// Where a JavaScript rule reports on `a.js`, as `line:text`.
fn run_javascript(ast: Value, source: &str) -> Vec<String> {
    let compiled = compile("javascript", ast).unwrap_or_else(|error| panic!("{}", error.reason));
    found(
        &evaluate_file(&compiled, "javascript", "a.js", source),
        source,
    )
}

/// Every name node of a TypeScript binding.
fn typescript_names() -> Value {
    json!([
        { "kind": "identifier" },
        { "kind": "type_identifier" },
        { "kind": "property_identifier" },
        { "kind": "shorthand_property_identifier_pattern" },
    ])
}

#[test]
fn typescript_usedness_reads_like_eslint() {
    // typescript-eslint 8.70.1's no-unused-vars (`args: "all"`) reports the same names, the
    // first eight at their last write.
    let source = "\
let a = 0;
a = a + 1;
let b = 0;
b += 1;
let c = 0;
c++;
let d = 0;
foo(d++);
let e = 0;
e = function () { return e; };
let f2 = 0;
f2 = () => f2;
function g() { g(); }
const h = () => h();
for (const k in foo) return;
for (const m in foo) { foo(); }
let n = 0;
while (foo) { n = n + 1; }
export const p = 1;
import { q, type Q2 } from \"m\";
type R = typeof q;
let s: Q2;
interface I { self: I }
enum E { A, B = A }
namespace N { N; }
function t(this: Window, u: number) {}
class C { constructor(private v: number) {} set w(x: number) {} }
declare function y(z: number): void;
const o1 = 0, o2 = 0;
o1 ||= o2;
const l = (0, foo)(l);
function sequence() { let r = 0; r = (r, 1); }
";
    let rule = json!({
        "rule": { "any": typescript_names(), "binds": { "used": false, "exported": false } },
    });
    assert_eq!(
        run_typescript(rule, "a.ts", source),
        [
            "1:a",
            "3:b",
            "5:c",
            "9:e",
            "11:f2",
            "13:g",
            "14:h",
            "16:m",
            "21:R",
            "22:s",
            "23:I",
            "24:E",
            "25:N",
            "26:t",
            "26:u",
            "27:C",
            "28:y",
            "32:sequence",
            "32:r",
        ]
    );
}

#[test]
fn typescript_usedness_reads_writes_exports_and_unknowns() {
    let source = "\
function f(a, b = 1, c) { eval(''); return c; }
var x;
export { x };
export default function d() {}
";
    let filter = |binds: Value| json!({ "rule": { "any": typescript_names(), "binds": binds } });
    // `b` has a write, its default; `a` none.
    assert_eq!(
        run_typescript(
            filter(json!({ "form": "parameter", "referenced": false })),
            "a.ts",
            source
        ),
        Vec::<String>::new(),
        "eval leaves the usedness of what its scope sees unknown"
    );
    let source = source.replace("eval(''); ", "");
    assert_eq!(
        run_typescript(
            filter(json!({ "form": "parameter", "referenced": false })),
            "a.ts",
            &source
        ),
        ["1:a"]
    );
    assert_eq!(
        run_typescript(
            filter(json!({ "form": "parameter", "referenced": true, "used": false })),
            "a.ts",
            &source
        ),
        ["1:b"]
    );
    assert_eq!(
        run_typescript(filter(json!({ "exported": true })), "a.ts", &source),
        ["4:d"],
        "`export {{ x }}` reads `x`; it exports no definition"
    );
    assert_eq!(
        run_typescript(
            filter(json!({ "used": true, "scope": "module" })),
            "a.ts",
            &source
        ),
        ["2:x"]
    );
}

#[test]
fn unpacked_values_leave_the_usage_index_alone() {
    let source = "\
def f():
    a, b = 1, 2
    c, d = d, c
    return b
";
    // A display element is the unpacked name's value, and reading it is no read of the target.
    let compiled = python(json!({ "kind": "identifier", "binds": {
        "scope": "function", "used": false, "value": { "pattern": "$V" } } }));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(matched_texts(&result, source), ["a V=1", "c V=d", "d V=c"]);
    let reads = |binding: Value| {
        run(
            json!({ "kind": "identifier", "inside": { "kind": "return_statement" }, "resolves": { "binding": binding } }),
            source,
        )
    };
    assert_eq!(
        reads(
            json!({ "used": true, "latest": true, "value": { "kind": "integer", "regex": "^2$" } })
        ),
        ["4:b"]
    );
}

#[test]
fn binds_has_no_lookup_and_redefines_is_not_only() {
    // `stopAtGlobal` and `only` choose which binding a reference reads; `binds` starts from the
    // binding itself, so it refuses both.
    for key in ["only", "stopAtGlobal"] {
        let rule = json!({ "rule": { "kind": "identifier", "binds": { key: true } } });
        assert_eq!(
            rejection(compile("python", rule)),
            format!("unknown field `{key}` at /clauses/0/forbid/ast/rule/binds")
        );
    }
    // `only` fails for any earlier binding in the scope; `redefines` reads only the one shadowed
    // on the same fork (Ruff's F811 `Binding::redefines`), so the two disagree across branches.
    let source = "\
if c:
    x = 1
else:
    x = 2
print(x)
y = 1
del y
y = 2
print(y)
";
    let only = json!({ "kind": "identifier", "inside": { "kind": "argument_list" },
                       "resolves": { "binding": { "only": true } } });
    assert_eq!(run(only, source), Vec::<String>::new());
    let redefines_nothing = json!({ "kind": "identifier", "binds": { "form": "assignment" },
                                    "not": { "binds": { "redefines": {} } } });
    assert_eq!(run(redefines_nothing, source), ["2:x", "4:x", "6:y"]);
}

#[test]
fn all_goes_on_past_an_unknown_binds_term_that_binds_nothing() {
    // `locals()` may read `x`, so `used: false` is unknown; `all` still tests the next term, whose
    // false verdict makes the conjunction false, and its negation reports.
    let source = "def f():\n    x = 1\n    locals()\n";
    let unknown = json!({ "binds": { "used": false } });
    let rule = json!({ "kind": "identifier", "binds": { "form": "assignment" },
                       "not": { "all": [unknown, { "regex": "^y$" }] } });
    assert_eq!(run(rule, source), ["2:x"]);
    // A `binds` whose sub-rule binds a capture withholds it: the false term that names none of it
    // still decides, while one that reads it stays open.
    let binding = json!({ "binds": { "used": false, "site": { "pattern": "$S" } } });
    let rule = json!({ "kind": "identifier", "binds": { "form": "assignment" },
                       "not": { "all": [binding.clone(), { "regex": "^y$" }] } });
    assert_eq!(run(rule, source), ["2:x"]);
    let rule = json!({ "kind": "identifier", "binds": { "form": "assignment" },
                       "not": { "all": [binding, { "has": { "stopBy": "end", "pattern": "$S", "regex": "^y$" } }] } });
    assert_eq!(run(rule, source), Vec::<String>::new());
}

/// `redefines` holds at an earlier definition that passes past one whose sub-rules are unknown,
/// and leaves no unknown verdict behind: a false term beside it decides the conjunction false at
/// every definition.
#[test]
fn redefines_decides_at_a_later_definition_past_an_unknown_one() {
    let false_beside = |kind: &str, unknown: &str| {
        let binds = json!({ "redefines": { "site": { "testUnknown": unknown } } });
        json!({ "rule": { "kind": kind, "not": { "all": [{ "binds": binds }, { "regex": "^never$" }] } } })
    };
    let source = "var a = 1;\nvar a = 2;\nvar a = 3;\n";
    assert_eq!(
        run_typescript(false_beside("identifier", "a = 1"), "a.ts", source),
        ["1:a", "2:a", "3:a"]
    );
    let source = "class C {\n  get #p() { return 1; }\n  set #p(v) {}\n  #p = 2;\n}\n";
    assert_eq!(
        run_typescript(
            false_beside("private_property_identifier", "^get"),
            "a.ts",
            source
        ),
        ["2:#p", "3:#p", "4:#p"]
    );
}

#[test]
fn bindings_counts_the_scopes_bindings_of_the_name_within_a_capture() {
    let source = "\
def f(y):
    i = 0
    for x in y:
        i += 1
        print([(i := z) for z in x], [i for i in x], g(i=1))
        match x:
            case [i]:
                pass
        del i

        def h():
            i = 5
    j = 0
    for x in y:
        j += 1
";
    // The loop holds four of `f`'s bindings of `i`: the augmented assignment, the walrus, the
    // match capture and the deletion. The comprehension's `i` and `h`'s are other scopes', and a
    // keyword argument binds nothing.
    let counted = |bindings: Value| {
        run(
            json!({ "kind": "identifier", "inside": { "kind": "for_statement", "pattern": "$LOOP", "stopBy": "end" },
                    "binds": { "form": "augmented", "bindings": bindings } }),
            source,
        )
    };
    assert_eq!(
        counted(json!({ "within": "LOOP", "min": 4, "max": 4 })),
        ["4:i"]
    );
    assert_eq!(counted(json!({ "within": "LOOP", "max": 1 })), ["15:j"]);
    // Without `within`, every binding of the name in the scope: `i = 0` too.
    assert_eq!(counted(json!({ "min": 5 })), ["4:i"]);
    assert_eq!(counted(json!({ "max": 2 })), ["15:j"]);
    // resolves.binding counts around the binding the reference reads.
    let reads = json!({ "kind": "identifier", "inside": { "kind": "for_statement", "pattern": "$LOOP", "stopBy": "end" },
                        "not": { "inside": { "kind": "augmented_assignment" } },
                        "resolves": { "binding": { "bindings": { "within": "LOOP", "min": 4 } } } });
    assert_eq!(run(reads, source), ["9:i"]);
    // A capture an `any` branch left unbound holds no binding.
    let optional = json!({ "kind": "identifier", "any": [{ "inside": { "kind": "while_statement", "pattern": "$LOOP", "stopBy": "end" } }, { "regex": "." }],
                           "binds": { "form": "augmented", "bindings": { "within": "LOOP", "max": 0 } } });
    assert_eq!(run(optional, source), ["4:i", "15:j"]);
}

#[test]
fn bindings_checks_its_bounds_and_capture_and_is_python_only() {
    let at = "/clauses/0/forbid/ast/rule/binds/bindings";
    for (bindings, expected) in [
        (
            json!({ "within": "LOOP" }),
            format!("{at} must set min to at least 1, or set max; otherwise every binding passes"),
        ),
        (
            json!({ "min": 0 }),
            format!("{at} must set min to at least 1, or set max; otherwise every binding passes"),
        ),
        (
            json!({ "min": 2, "max": 1 }),
            format!("{at}: min must not exceed max"),
        ),
        (
            json!({ "max": -1 }),
            format!("{at}/max must be a non-negative integer"),
        ),
        (
            json!({ "within": "$LOOP", "max": 1 }),
            format!(
                "{at}/within: `\"$LOOP\"` is not a metavariable name (A-Z, 0-9, _, without `$`)"
            ),
        ),
        (
            json!({ "max": 1, "near": "A" }),
            format!("unknown field `near` at {at}"),
        ),
        (
            json!({ "within": "LOOP", "max": 1 }),
            format!(
                "{at}/within: `$LOOP` is never bound where this binding filter runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
            ),
        ),
    ] {
        let rule = json!({ "rule": { "kind": "identifier", "binds": { "bindings": bindings } } });
        assert_eq!(rejection(compile("python", rule)), expected);
    }
    let rule = json!({ "rule": { "kind": "identifier", "inside": { "pattern": "f($$$ARGS)", "stopBy": "end" },
                                 "binds": { "bindings": { "within": "ARGS", "max": 1 } } } });
    assert_eq!(
        rejection(compile("python", rule)),
        format!(
            "{at}/within: `$$$ARGS` captures a node list; within reads single-node captures only"
        )
    );
    // TypeScript counts a scope's declarations of the name (eslint-scope's `defs`).
    let rule = json!({ "rule": { "kind": "identifier", "resolves": { "binding": { "bindings": { "max": 1 } } } } });
    assert!(compile("typescript", rule).is_ok());
}

#[test]
fn exported_comments_count_under_directives_in_a_script() {
    let source = "/* exported a, b */\nvar a = 1, b = 2, c = 3;\nfunction f() { var b; }\n";
    let rule = |directives: bool, source_type: &str| {
        let mut binds = json!({ "used": false, "exported": false, "form": "assignment" });
        if directives {
            binds["directives"] = json!("eslint");
        }
        json!({
            "languageOptions": { "sourceType": source_type },
            "rule": { "kind": "identifier", "binds": binds },
        })
    };
    assert_eq!(
        run_typescript(rule(false, "script"), "a.ts", source),
        ["2:a", "2:b", "2:c", "3:b"]
    );
    assert_eq!(
        run_typescript(rule(true, "script"), "a.ts", source),
        ["2:c", "3:b"]
    );
    // In a module the top level is no global scope, so the comment exports nothing.
    assert_eq!(
        run_typescript(rule(true, "module"), "a.ts", source),
        ["2:a", "2:b", "2:c", "3:b"]
    );
    let scope = |scope: &str| {
        json!({
            "languageOptions": { "sourceType": "script" },
            "rule": { "kind": "identifier", "binds": { "scope": scope, "form": "assignment" } },
        })
    };
    assert_eq!(
        run_typescript(scope("global"), "a.ts", source),
        ["2:a", "2:b", "2:c"]
    );
    assert_eq!(
        run_typescript(scope("module"), "a.ts", source),
        Vec::<String>::new()
    );
}

#[test]
fn the_jsx_pragma_reads_its_variable() {
    let source = "import React from 'react';\nimport h from 'h';\nexport const x = <div />;\n";
    let rule = |options: Value| {
        json!({
            "languageOptions": options,
            "rule": { "kind": "identifier", "binds": { "form": "import", "used": false } },
        })
    };
    assert_eq!(run_typescript(rule(json!({})), "a.tsx", source), ["2:h"]);
    assert_eq!(
        run_typescript(rule(json!({ "jsxPragma": "h" })), "a.tsx", source),
        ["1:React"]
    );
    assert_eq!(
        run_typescript(rule(json!({ "jsxPragma": null })), "a.tsx", source),
        ["1:React", "2:h"]
    );
    let fragment = "import Fragment from 'react';\nexport const x = <></>;\n";
    assert_eq!(
        run_typescript(
            rule(json!({ "jsxPragma": null, "jsxFragmentName": "Fragment" })),
            "a.tsx",
            fragment
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        rejection(compile("typescript", rule(json!({ "jsxPragma": "a.b" })))),
        "/clauses/0/forbid/ast/languageOptions/jsxPragma must be an identifier of at most 256 bytes, or null"
    );
}

#[test]
fn typescript_redefines_reads_the_variables_other_definitions() {
    // ESLint 10.11.0's no-redeclare (script) reports the same four.
    let source = "\
var a = 1; var a = 2;
function f(b) { var b; }
var Object = 0;
var x; function x() {}
{ let c; } let c;
const g = function g2() { var g2; };
function h() { var arguments; }
";
    let rule = json!({
        "languageOptions": { "sourceType": "script" },
        "rule": { "kind": "identifier", "any": [
            { "binds": { "redefines": {} } },
            { "binds": { "builtin": true } },
        ] },
    });
    assert_eq!(
        run_typescript(rule, "a.ts", source),
        ["1:a", "2:b", "3:Object", "4:x"]
    );
    // In a module the top level holds no global variable.
    let rule = json!({ "rule": { "kind": "identifier", "binds": { "builtin": true } } });
    assert_eq!(run_typescript(rule, "a.ts", source), Vec::<String>::new());
    // Some earlier definition passes, or with `every` each other one does.
    let source = "interface A {}\ninterface A {}\nclass B {}\ninterface B {}\nclass B {}\n";
    let redefines = |redefines: Value| json!({ "rule": { "kind": "type_identifier", "binds": { "redefines": redefines } } });
    assert_eq!(
        run_typescript(
            redefines(json!({ "site": { "kind": "class_declaration" } })),
            "a.ts",
            source
        ),
        ["4:B", "5:B"]
    );
    assert_eq!(
        run_typescript(
            redefines(json!({ "every": true, "site": { "kind": "interface_declaration" } })),
            "a.ts",
            source
        ),
        ["1:A", "2:A"]
    );
    assert_eq!(
        run_typescript(redefines(json!({ "sameScope": false })), "a.ts", source),
        Vec::<String>::new()
    );
    for (rule, expected) in [
        (
            json!({ "kind": "identifier", "binds": { "redefines": { "sameImport": true } } }),
            "/clauses/0/forbid/ast/rule/binds/redefines/sameImport: qualified import names are Python's; TypeScript has none",
        ),
        (
            json!({ "kind": "identifier", "binds": { "redefines": { "every": 1 } } }),
            "/clauses/0/forbid/ast/rule/binds/redefines/every must be true or false",
        ),
    ] {
        assert_eq!(
            rejection(compile("typescript", json!({ "rule": rule }))),
            expected
        );
    }
    for (rule, expected) in [
        (
            json!({ "kind": "identifier", "binds": { "redefines": { "every": true } } }),
            "/clauses/0/forbid/ast/rule/binds/redefines/every: Python's redefines reads the one binding shadowed; `every` is TypeScript's",
        ),
        (
            json!({ "kind": "identifier", "binds": { "builtin": true } }),
            "/clauses/0/forbid/ast/rule/binds/builtin: configured globals are ESLint's; Python has none",
        ),
    ] {
        assert_eq!(
            rejection(compile("python", json!({ "rule": rule }))),
            expected
        );
    }
}

#[test]
fn write_last_anchors_where_eslint_reports() {
    // ESLint 10.11.0's no-unused-vars reports at these four nodes.
    let source = "\
let a;
a = 1;
var b = 1;
let c = 0;
function f() { c = 2; }
let d;
d = 1;
d = 2;
export { f };
";
    let rule = json!({ "rule": { "any": typescript_names(), "resolves": {
        "write": "last", "binding": { "used": false, "exported": false },
    } } });
    assert_eq!(
        run_typescript(rule, "a.ts", source),
        ["2:a", "3:b", "4:c", "8:d"]
    );
    let every = json!({ "rule": { "any": typescript_names(), "resolves": { "write": "last" } } });
    assert_eq!(
        run_typescript(every, "a.ts", source),
        ["2:a", "3:b", "4:c", "5:f", "8:d"]
    );
    for (resolves, expected) in [
        (
            json!({ "write": "first" }),
            "/clauses/0/forbid/ast/rule/resolves/write must be \"last\"",
        ),
        (
            json!({ "write": "last", "global": ["a"] }),
            "/clauses/0/forbid/ast/rule/resolves/global: `write` asks where ESLint reports a variable; filter it with `binding`, and write other `resolves` terms in a resolves of their own",
        ),
    ] {
        let rule = json!({ "rule": { "kind": "identifier", "resolves": resolves } });
        assert_eq!(rejection(compile("typescript", rule)), expected);
    }
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": { "write": "last" } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/write: where ESLint reports a variable is TypeScript's; Python has none"
    );
}

#[test]
fn occurrence_matches_a_definition_name_or_reference_of_the_variable() {
    // ESLint's destructuredArrayIgnorePattern reads references in array patterns anywhere.
    let source = "let _a, b;\nfoo(() => { [_a, b] = x; b; });\nlet _c;\nexport { b };\n";
    let rule = |occurrence: Value| json!({ "rule": { "kind": "identifier", "binds": { "used": false, "occurrence": occurrence } } });
    assert_eq!(
        run_typescript(
            rule(json!({ "inside": { "kind": "array_pattern" } })),
            "a.ts",
            source
        ),
        ["1:_a"]
    );
    assert_eq!(
        run_typescript(
            rule(json!({ "inside": { "kind": "variable_declarator" } })),
            "a.ts",
            source
        ),
        ["1:_a", "3:_c"]
    );
    let captured = json!({ "rule": { "kind": "identifier", "resolves": { "binding": {
        "occurrence": { "pattern": "$N", "inside": { "kind": "array_pattern" } },
    } } } });
    assert_eq!(
        run_typescript(captured, "a.ts", source),
        ["2:_a", "2:b", "2:b", "4:b"]
    );
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "binds": { "occurrence": { "kind": "identifier" } } } })
        )),
        "/clauses/0/forbid/ast/rule/binds/occurrence: a variable's occurrences are TypeScript's; Python has none"
    );
}

#[test]
fn builtin_counts_global_comments_under_directives() {
    // typescript-eslint's no-redeclare reports `b` after the comment declared it.
    let source = "/*global b:false*/ var b = 1;\nvar c = 2;\n";
    let rule = |directives: bool| {
        let mut binds = json!({ "builtin": true });
        if directives {
            binds["directives"] = json!("eslint");
        }
        json!({ "languageOptions": { "sourceType": "script" }, "rule": { "kind": "identifier", "binds": binds } })
    };
    assert_eq!(
        run_typescript(rule(false), "a.ts", source),
        Vec::<String>::new()
    );
    assert_eq!(run_typescript(rule(true), "a.ts", source), ["1:b"]);
}

#[test]
fn builtin_reads_the_lib_globals_in_either_namespace() {
    // typescript-eslint's no-redeclare (`builtinGlobals`) under `lib: ["dom"]`: a script's top-level
    // declaration of a lib name merges into its lib variable, a type (`NodeListOf`) or a value
    // (`NodeList`) whatever the declaration's namespace.
    let source = "type NodeListOf = 1;\nvar NodeList = 1;\ninterface Node {}\nvar local = 1;\n";
    let rule = |options: Value| {
        json!({ "languageOptions": options, "rule": {
            "any": [{ "kind": "identifier" }, { "kind": "type_identifier" }],
            "binds": { "builtin": true },
        } })
    };
    assert_eq!(
        run_typescript(rule(json!({ "sourceType": "script" })), "a.ts", source),
        Vec::<String>::new()
    );
    assert_eq!(
        run_typescript(
            rule(json!({ "sourceType": "script", "lib": ["dom"] })),
            "a.ts",
            source
        ),
        ["1:NodeListOf", "2:NodeList", "3:Node"]
    );
    // In a module the top level holds no global variable.
    assert_eq!(
        run_typescript(rule(json!({ "lib": ["dom"] })), "a.ts", source),
        Vec::<String>::new()
    );
}

#[test]
fn binds_reads_private_names_per_class_body() {
    // ESLint 10.11.0's no-unused-private-class-members: a read in the class body uses a private
    // name; a plain write, a statement-level compound assignment or update, and a destructuring
    // target do not; any reference uses an accessor.
    let source = "\
class A {
  #field = 1;
  #written;
  #method() {}
  get #pair() { return 1; }
  set #pair(v) {}
  #tested;
  #never: number;
  m(o) { this.#field; this.#written = 2; this.#written += 1; this.#method(); #tested in o; this.#pair = 1; }
}
";
    let run = |binds: Value| {
        run_typescript(
            json!({ "rule": { "kind": "private_property_identifier", "binds": binds } }),
            "a.ts",
            source,
        )
    };
    let all = [
        "2:#field",
        "3:#written",
        "4:#method",
        "5:#pair",
        "6:#pair",
        "7:#tested",
        "8:#never",
    ];
    assert_eq!(run(json!({})), all);
    assert_eq!(run(json!({ "used": false })), ["3:#written", "8:#never"]);
    assert_eq!(run(json!({ "referenced": false })), ["8:#never"]);
    assert_eq!(
        run(json!({ "form": "field" })),
        ["2:#field", "3:#written", "7:#tested", "8:#never"]
    );
    assert_eq!(run(json!({ "form": "method" })), ["4:#method"]);
    assert_eq!(run(json!({ "form": "accessor" })), ["5:#pair", "6:#pair"]);
    assert_eq!(run(json!({ "scope": "class" })), all);
    assert_eq!(run(json!({ "scope": "function" })), Vec::<String>::new());
    assert_eq!(run(json!({ "exported": false, "builtin": false })), all);
    assert_eq!(run(json!({ "builtin": true })), Vec::<String>::new());
    // A getter and setter are one member of two declarations.
    assert_eq!(run(json!({ "redefines": {} })), ["6:#pair"]);
    assert_eq!(
        run(json!({ "bindings": { "min": 2 } })),
        ["5:#pair", "6:#pair"]
    );
    // Sub-rules read the member, its value, its annotation and its occurrences.
    assert_eq!(
        run(json!({ "site": { "kind": "method_definition" } })),
        ["4:#method", "5:#pair", "6:#pair"]
    );
    assert_eq!(run(json!({ "value": { "kind": "number" } })), ["2:#field"]);
    assert_eq!(
        run(json!({ "annotation": { "kind": "predefined_type" } })),
        ["8:#never"]
    );
    assert_eq!(
        run(json!({ "occurrence": { "inside": { "kind": "binary_expression" } } })),
        ["7:#tested"]
    );
    // `nestedUse` has no reading for a private name: unknown, so no violation either way.
    assert_eq!(run(json!({ "nestedUse": false })), Vec::<String>::new());
    let negated = run_typescript(
        json!({ "rule": { "kind": "private_property_identifier", "not": { "binds": { "nestedUse": false } } } }),
        "a.ts",
        source,
    );
    assert_eq!(
        negated,
        [
            "9:#field",
            "9:#written",
            "9:#written",
            "9:#method",
            "9:#tested",
            "9:#pair"
        ]
    );
    // A private name's own class body decides: an inner class that declares it shadows it.
    let source =
        "class A { #a; #b; m() { return class B { #a; n() { return this.#a + this.#b; } }; } }\n";
    let rule =
        json!({ "rule": { "kind": "private_property_identifier", "binds": { "used": false } } });
    assert_eq!(run_typescript(rule.clone(), "a.ts", source), ["1:#a"]);
    // A private name's escapes are decoded, as espree decodes them: `#\u0061` is `#a`.
    for source in [
        "class A { #\\u0061 = 1; m() { return this.#a; } }\n",
        "class A { #a = 1; m() { return this.#\\u{61}; } }\n",
    ] {
        assert_eq!(
            run_typescript(rule.clone(), "a.ts", source),
            Vec::<String>::new(),
            "{source}"
        );
    }
}

#[test]
fn typescript_marks_what_typescript_eslint_marks_used() {
    // typescript-eslint 8.70.1's no-unused-vars reports only `unusedHere`.
    let source = "\
enum Foo { a }
export type T = { [Foo.a]: 1 };
namespace foo.bar { export interface User {} }
declare global { namespace jest { interface Matchers<R> { seven: () => R; } } }
export declare namespace Outer { namespace Inner { const x: number; } }
declare module 'm' { type Hidden = 1; }
const unusedHere = 1;
";
    let rule = json!({ "rule": { "any": typescript_names(), "binds": { "used": false, "exported": false } } });
    assert_eq!(run_typescript(rule, "a.ts", source), ["7:unusedHere"]);
}

/// M9b's `bindings` and `only` on TypeScript (M10b): `bindings` counts a scope's declarations of the
/// name (eslint-scope's `defs`), within a capture or in all; `resolves.binding`'s `only` is one
/// declaration, beside `write: "last"` too.
#[test]
fn typescript_binds_count_declarations_and_only_reads_one() {
    let source = "\
var a = 1; var a = 2;
function f(b) { var b; return b; }
let c = 0; c = 1;
{ let d = 1; }
function e() {}
function e() {}
";
    let binds = |binds: Value| {
        run_typescript(
            json!({ "rule": { "any": typescript_names(), "binds": binds } }),
            "a.ts",
            source,
        )
    };
    assert_eq!(
        binds(json!({ "bindings": { "min": 2 } })),
        ["1:a", "1:a", "2:b", "2:b", "5:e", "6:e"]
    );
    // `c = 1` writes `c`; it declares nothing.
    assert_eq!(
        binds(json!({ "bindings": { "max": 1 }, "used": false })),
        ["2:f", "3:c", "4:d"]
    );
    assert_eq!(
        binds(json!({ "bindings": { "max": 1 }, "scope": "block" })),
        ["4:d"]
    );
    // `within` counts the declarations inside the captured function.
    let within = run_typescript(
        json!({ "rule": { "any": typescript_names(),
            "inside": { "kind": "function_declaration", "pattern": "$F", "stopBy": "end" },
            "binds": { "bindings": { "within": "F", "min": 2 } } } }),
        "a.ts",
        source,
    );
    assert_eq!(within, ["2:b", "2:b"]);
    let resolves = |binding: Value| {
        run_typescript(
            json!({ "rule": { "any": typescript_names(), "resolves": { "write": "last", "binding": binding } } }),
            "a.ts",
            source,
        )
    };
    let unused_alone = ["2:f", "3:c", "4:d"];
    assert_eq!(
        resolves(json!({ "used": false, "only": true })),
        unused_alone
    );
    assert_eq!(
        resolves(json!({ "used": false, "bindings": { "max": 1 } })),
        unused_alone
    );
    assert_eq!(
        resolves(json!({ "used": false, "only": false })),
        resolves(json!({ "used": false }))
    );
}

/// M10b's private-name binds beside M9's `member` compare: `binds` keeps `#g` and `g` apart (each
/// its own member of the class body), while `member` names `#g` as typescript-eslint's
/// getStaticMemberAccessValue does, without its `#`, so it equals the public `g`. A relation that
/// compares against the binding's `site` runs after `binds` binds it. tree-sitter-javascript keys
/// a field `property` where tree-sitter-typescript keys it `name`; both read alike.
#[test]
fn member_compare_names_private_members_that_binds_keeps_apart() {
    let source = "\
class C {
  #g = 1;
  g = 2;
  #h = 3;
  h() { return this.#h; }
  #k() {}
  k() {}
  #n = 4;
}
";
    // An unused private member with a public namesake in the same body.
    let shadowed = |field: &str| {
        let members = json!([{ "kind": field }, { "kind": "method_definition" }]);
        json!({ "rule": {
            "kind": "private_property_identifier",
            "binds": { "used": false, "site": { "pattern": "$P" } },
            "inside": { "kind": "class_body", "stopBy": "end", "has": {
                "stopBy": "neighbor", "any": members, "pattern": "$Q",
                "not": { "has": { "stopBy": "neighbor", "kind": "private_property_identifier" } },
                "compare": { "left": "Q", "op": "eq", "right": "P", "text": "member" } } },
        } })
    };
    assert_eq!(
        run_typescript(shadowed("public_field_definition"), "a.ts", source),
        ["2:#g", "6:#k"]
    );
    assert_eq!(
        run_javascript(shadowed("field_definition"), source),
        ["2:#g", "6:#k"]
    );
    // The public namesake is no other definition of the private member.
    assert_eq!(
        run_typescript(
            json!({ "rule": { "kind": "private_property_identifier", "binds": { "redefines": {} } } }),
            "a.ts",
            source
        ),
        Vec::<String>::new()
    );
    for path in ["a.ts", "a.js"] {
        assert_eq!(
            run_typescript(
                json!({ "rule": { "kind": "private_property_identifier", "binds": { "used": false } } }),
                path,
                source
            ),
            ["2:#g", "6:#k", "8:#n"],
            "{path}"
        );
    }
}

/// M10a's fix round's bare annotations under M10b's filters: `referenced` counts the reads that
/// resolve to the annotation itself, the assignment that shadows it inherits them, `redefines`
/// and `bindings` count it as a binding, and TypeScript has no bare annotations.
#[test]
fn bare_annotations_meet_referenced_redefines_and_bindings() {
    let source = "\
def f():
    x: int
    x = 1
    print(x)
def g():
    y: int
    print(y)
def h():
    z: int
";
    let rule = |filter: Value| run(json!({ "kind": "identifier", "binds": filter }), source);
    // `print(y)` ends at `g`'s bare annotation (an unbound local) and reads it.
    assert_eq!(rule(json!({ "form": "annotation", "used": true })), ["6:y"]);
    assert_eq!(
        rule(json!({ "form": "annotation", "referenced": true })),
        ["6:y"]
    );
    assert_eq!(
        rule(json!({ "form": "annotation", "used": false })),
        ["2:x", "9:z"]
    );
    // F842's shape reads only the latest binding: `x = 1` shadows `x: int`.
    assert_eq!(
        rule(json!({ "form": "annotation", "latest": true, "used": false })),
        ["9:z"]
    );
    assert_eq!(
        rule(json!({ "form": "assignment", "referenced": true })),
        ["3:x"]
    );
    assert_eq!(
        rule(json!({ "redefines": { "form": "annotation" } })),
        ["3:x"]
    );
    assert_eq!(
        rule(json!({ "form": "assignment", "bindings": { "min": 2, "max": 2 } })),
        ["3:x"]
    );
    // TypeScript declares no bare annotation: `let a: number;` is a declaration.
    let source = "let a: number;\nf(a);\ndeclare let b: string;\n";
    let rule = json!({ "kind": "identifier", "binds": { "form": "annotation" } });
    assert_eq!(
        run_typescript(json!({ "rule": rule }), "a.ts", source),
        Vec::<String>::new()
    );
}

/// `locals()` hides only its own function's assignments, parameters and the like, so M10b's
/// `referenced` reads the module's bindings and an `except` name there as known.
#[test]
fn locals_leaves_referenced_known_outside_its_functions_assignments() {
    let source = "\
import os
def f(p):
    q = 1
    try:
        pass
    except E as e:
        pass
    return locals()
";
    let rule = |filter: Value| {
        run(
            json!({ "any": [{ "kind": "identifier" }, { "kind": "dotted_name" }], "binds": filter }),
            source,
        )
    };
    assert_eq!(rule(json!({ "referenced": false })), ["1:os", "2:f", "6:e"]);
    // `f` is never read; `p` and `q` are unknown under `locals()`.
    assert_eq!(rule(json!({ "referenced": true })), Vec::<String>::new());
}

/// `form: "annotation"` came from M10a's fix round and from M11a; the table keeps one binding, so
/// a string type definition's name meets a bare annotation exactly as a load does: unbound in a
/// function unless a nested `nonlocal` initializes it, passed over at module level, and a use of
/// it either way. Each list is Ruff 0.16.9's (F821, F842; F822 reports nothing).
#[test]
fn string_names_and_loads_read_bare_annotations_alike() {
    let source = "\
import os


def plain():
    x: int
    print(x)


def stringly():
    x: int
    y: \"x\"


def initialized():
    x: int
    y: \"x\"
    print(x)

    def inner():
        nonlocal x
        x = 1

    return inner


def unread():
    x: int
    y: \"os\"


z: int
__all__ = [\"z\"]
w: \"z\"
";
    let unresolved = json!({ "kind": "identifier", "resolves": { "unresolved": true } });
    let strings = json!([{ "kind": "string" }, { "kind": "concatenated_string" }]);
    let in_strings = json!({ "any": strings, "parse": { "grammar": "annotation",
        "has": { "kind": "identifier", "resolves": { "unresolved": true }, "stopBy": "end" } } });
    assert_eq!(run(unresolved, source), ["6:x"]);
    assert_eq!(run(in_strings, source), ["11:\"x\"", "33:\"z\""]);
    let unused = json!({ "kind": "identifier",
        "binds": { "form": "annotation", "scope": "function", "latest": true, "used": false } });
    assert_eq!(run(unused, source), ["11:y", "16:y", "27:x", "28:y"]);
    // The same bindings through either key: `binds` lists them, `resolves.binding` passes over
    // them outside a stub, and `__all__` sees the module's.
    let annotations = json!({ "kind": "identifier", "binds": { "form": "annotation" } });
    assert_eq!(
        run(annotations, source),
        [
            "5:x", "10:x", "11:y", "15:x", "16:y", "27:x", "28:y", "31:z", "33:w"
        ]
    );
    let resolved =
        json!({ "kind": "identifier", "resolves": { "binding": { "form": "annotation" } } });
    assert_eq!(run(resolved, source), Vec::<String>::new());
    let exported =
        json!({ "kind": "identifier", "binds": { "form": "annotation", "exported": true } });
    assert_eq!(run(exported, source), ["31:z"]);
}
